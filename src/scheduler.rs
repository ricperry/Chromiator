use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;

use crossbeam_channel::{Receiver, Sender, TrySendError, bounded, unbounded};

use crate::document::{PixelImage, Recipe};
use crate::processing::{
    Coverage, DisplayBuffer, process_cancellable_with_progress_and_coverage, to_display_rgba8,
};

#[derive(Clone, Default)]
pub struct LatestGeneration(Arc<AtomicU64>);
impl LatestGeneration {
    pub fn begin(&self) -> u64 {
        self.0.fetch_add(1, Ordering::AcqRel) + 1
    }
    pub fn is_current(&self, generation: u64) -> bool {
        self.0.load(Ordering::Acquire) == generation
    }

    pub fn begin_token(&self) -> JobToken {
        JobToken {
            generation: self.begin(),
            current: self.0.clone(),
        }
    }

    pub fn cancel(&self) {
        self.begin();
    }
}

#[derive(Clone)]
pub struct JobToken {
    generation: u64,
    current: Arc<AtomicU64>,
}

impl JobToken {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn is_current(&self) -> bool {
        self.current.load(Ordering::Acquire) == self.generation
    }

    pub fn current(&self) -> &AtomicU64 {
        &self.current
    }
}

#[derive(Default)]
pub struct ProgressTracker {
    generation: u64,
    fraction: f64,
}

#[derive(Default)]
pub struct JobCoordinator {
    generations: LatestGeneration,
    in_flight: Option<u64>,
}

impl JobCoordinator {
    pub fn begin(&mut self) -> Option<JobToken> {
        if self.in_flight.is_some() {
            return None;
        }
        let token = self.generations.begin_token();
        self.in_flight = Some(token.generation());
        Some(token)
    }

    pub fn cancel(&self) {
        if self.in_flight.is_some() {
            self.generations.cancel();
        }
    }

    pub fn is_current(&self, generation: u64) -> bool {
        self.generations.is_current(generation)
    }

    pub fn acknowledge(&mut self, generation: u64) -> bool {
        if self.in_flight == Some(generation) {
            self.in_flight = None;
            true
        } else {
            false
        }
    }

    pub fn is_busy(&self) -> bool {
        self.in_flight.is_some()
    }
}

impl ProgressTracker {
    pub fn start(&mut self, generation: u64) {
        self.generation = generation;
        self.fraction = 0.0;
    }

    pub fn accept(&mut self, generation: u64, fraction: f64) -> bool {
        if generation != self.generation || !(self.fraction..=1.0).contains(&fraction) {
            return false;
        }
        self.fraction = fraction;
        true
    }
}

pub struct PreviewResult {
    pub generation: u64,
    pub image: PixelImage,
    pub display: DisplayBuffer,
    pub coverage: Coverage,
}

struct Request {
    generation: u64,
    source: PixelImage,
    recipe: Recipe,
}

pub struct PreviewScheduler {
    generation: Arc<AtomicU64>,
    requests: Sender<Request>,
    request_drain: Receiver<Request>,
    results: Receiver<PreviewResult>,
}

impl PreviewScheduler {
    pub fn new() -> Self {
        let generation = Arc::new(AtomicU64::new(0));
        let (requests, worker_requests) = bounded::<Request>(1);
        let request_drain = worker_requests.clone();
        let (result_sender, results) = unbounded();
        let worker_generation = generation.clone();
        thread::spawn(move || {
            while let Ok(request) = worker_requests.recv() {
                if let Some((image, coverage)) = process_cancellable_with_progress_and_coverage(
                    &request.source,
                    &request.recipe,
                    request.generation,
                    &worker_generation,
                    |_| {},
                ) {
                    let _ = result_sender.send(PreviewResult {
                        generation: request.generation,
                        display: to_display_rgba8(&image),
                        image,
                        coverage,
                    });
                }
            }
        });
        Self {
            generation,
            requests,
            request_drain,
            results,
        }
    }

    pub fn schedule(&self, source: PixelImage, recipe: Recipe) -> u64 {
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        let mut request = Request {
            generation,
            source,
            recipe,
        };
        loop {
            match self.requests.try_send(request) {
                Ok(()) => break,
                Err(TrySendError::Full(returned)) => {
                    request = returned;
                    let _ = self.request_drain.try_recv();
                }
                Err(TrySendError::Disconnected(_)) => break,
            }
        }
        generation
    }

    pub fn current_generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    pub fn try_latest(&self) -> Option<PreviewResult> {
        let current = self.generation.load(Ordering::Acquire);
        self.results
            .try_iter()
            .filter(|result| result.generation == current)
            .last()
    }
}

impl Default for PreviewScheduler {
    fn default() -> Self {
        Self::new()
    }
}
