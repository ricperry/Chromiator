//! Opt-in workload evidence, not wall-clock assertions in the regular test suite.
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use chromiator::document::{
    BlendSpace, PixelImage, Recipe, SampleSize, TransitionProfile,
    VoronoiMatching, VoronoiSite,
};
use chromiator::processing::{process, process_cancellable_with_progress_and_coverage};

fn workload(site_count: usize, side: u32) -> (PixelImage, Recipe) {
    let mut seed = 0x1876_abcd_u32;
    let mut next = || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        ((seed >> 8) as f32) / 16_777_215.0
    };
    let mut recipe = Recipe::default();
    recipe.steps.clear();
    recipe.voronoi.matching = VoronoiMatching::Perceptual;
    recipe.voronoi.sites = (0..site_count).map(|i| VoronoiSite {
        id: i as u64 + 1,
        order: i as u64,
        source_color: [next(), next(), next(), 1.0],
        target_color: [next(), next(), next()],
        influence: (i % 5) as f64 - 2.0,
        locked: false,
        position: None,
        size: SampleSize::Point,
    }).collect();
    recipe.voronoi.next_site_id = site_count as u64 + 1;
    let pixels = (0..side * side).map(|i| {
        let x = (i % side) as f32 / (side - 1) as f32;
        let y = (i / side) as f32 / (side - 1) as f32;
        [x, y, (x + y) * 0.5, 1.0]
    }).collect();
    (PixelImage::new(side, side, pixels).unwrap(), recipe)
}

#[test]
#[ignore = "Run in release mode with --nocapture for representative transition timings"]
fn transition_workload_timings() {
    for (count, side) in [(5, 1024), (16, 512), (64, 256)] {
        let (source, mut recipe) = workload(count, side);
        for space in [BlendSpace::Oklab, BlendSpace::LinearRgb] {
            for (name, width) in [
                ("hard", 0.0),
                ("blended", 0.5),
            ] {
                recipe.voronoi.transition = TransitionProfile {
                    blend_space: space,
                    ..TransitionProfile::HARD.with_width(width)
                };
                let mut times = Vec::new();
                let reference = process(&source, &recipe);
                for _ in 0..3 {
                    let start = Instant::now();
                    let output = process(&source, &recipe);
                    times.push(start.elapsed().as_secs_f64() * 1000.0);
                    assert_eq!(output, reference, "nondeterministic repeated render");
                }
                times.sort_by(f64::total_cmp);
                println!("sites={count} image={side}x{side} space={space:?} scope={name} width={width} median_ms={:.3} min_ms={:.3} max_ms={:.3}", times[1], times[0], times[2]);
            }
        }
    }
}

#[test]
#[ignore = "Larger workload cancellation check; run explicitly in release mode"]
fn transitions_cancel_after_progress_without_publishing_pixels() {
    let (source, mut recipe) = workload(64, 512);
    for space in [BlendSpace::Oklab, BlendSpace::LinearRgb] {
        recipe.voronoi.transition = TransitionProfile {
            blend_space: space, ..TransitionProfile::HARD.with_width(0.5)
        };
        let generation = AtomicU64::new(1);
        let mut cancellation = None;
        let result = process_cancellable_with_progress_and_coverage(
            &source, &recipe, 1, &generation, |_| {
                if cancellation.is_none() {
                    cancellation = Some(Instant::now());
                    generation.store(2, Ordering::Release);
                }
            },
        );
        assert!(result.is_none(), "cancelled generation must not publish output");
        println!("space={space:?} sites=64 cancellation_to_return_ms={:.3}", cancellation.unwrap().elapsed().as_secs_f64() * 1000.0);
    }
}
