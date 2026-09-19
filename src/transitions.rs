//! Target-color blending over the unchanged weighted Source partition.
//! All competitive sites contribute through normalized compact-support activations.
use crate::color::{linear_to_oklab, oklab_to_linear};
use crate::document::{BlendSpace, TransitionProfile};

pub(crate) struct CompiledBlend {
    width: f64,
    space: BlendSpace,
    targets: Vec<[f32; 3]>,
    coordinates: Vec<[f64; 3]>,
}

/// Reused for an entire processing job, never allocated per pixel.
pub(crate) struct BlendScratch {
    pub scores: Vec<f64>,
    weights: Vec<f64>,
}

impl BlendScratch {
    pub fn new(count: usize) -> Self {
        Self { scores: vec![0.0; count], weights: vec![0.0; count] }
    }
}

impl CompiledBlend {
    pub fn compile(profile: TransitionProfile, targets: Vec<[f32; 3]>) -> Self {
        let coordinates = targets.iter().map(|rgb| match profile.blend_space {
            BlendSpace::Oklab => linear_to_oklab(rgb.map(f64::from)),
            BlendSpace::LinearRgb => rgb.map(f64::from),
        }).collect();
        Self { width: f64::from(profile.width()), space: profile.blend_space, targets, coordinates }
    }

    pub fn resolve(&self, pixel: [f32; 4], scratch: &mut BlendScratch) -> [f32; 4] {
        let minimum = scratch.scores.iter().copied().fold(f64::INFINITY, f64::min);
        for (index, &score) in scratch.scores.iter().enumerate() {
            let gap = if score + minimum == 0.0 { 0.0 } else { (score - minimum) / (score + minimum) };
            let t = (gap / self.width).clamp(0.0, 1.0);
            scratch.weights[index] = smoothstep(1.0 - t);
        }
        let mut total = 0.0;
        let mut mixed = [0.0; 3];
        let mut count = 0;
        let mut last = 0;
        for (i, &weight) in scratch.weights.iter().enumerate() {
            if weight <= 0.0 { continue; }
            count += 1;
            last = i;
            total += weight;
            for (channel, coordinate) in mixed.iter_mut().zip(self.coordinates[i]) { *channel += weight * coordinate; }
        }
        let rgb = if count == 1 {
            self.targets[last]
        } else {
            mixed = mixed.map(|v| v / total);
            match self.space {
                BlendSpace::LinearRgb => mixed.map(|v| v as f32),
                BlendSpace::Oklab => bounded_oklab(mixed).map(|v| v as f32),
            }
        };
        [rgb[0], rgb[1], rgb[2], pixel[3]]
    }
}

fn smoothstep(t: f64) -> f64 { t * t * (3.0 - 2.0 * t) }

/// Constant-lightness/hue chroma reduction; preserve in-gamut mixtures unchanged.
fn bounded_oklab(lab: [f64; 3]) -> [f64; 3] {
    let rgb = oklab_to_linear(lab);
    let in_gamut = |rgb: [f64; 3]| rgb.into_iter().all(|v| (0.0..=1.0).contains(&v));
    if in_gamut(rgb) { return rgb; }
    let l = lab[0].clamp(0.0, 1.0);
    let (mut low, mut high) = (0.0, 1.0);
    for _ in 0..24 {
        let middle = (low + high) * 0.5;
        if in_gamut(oklab_to_linear([l, lab[1] * middle, lab[2] * middle])) { low = middle; }
        else { high = middle; }
    }
    oklab_to_linear([l, lab[1] * low, lab[2] * low]).map(|v| v.clamp(0.0, 1.0))
}
