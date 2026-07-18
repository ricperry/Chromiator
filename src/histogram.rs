//! Bounded, source-owned distributions used by the Threshold mapping editor.

use crate::color::{encoded_to_hsv, hsv_to_encoded};
use crate::document::{ComponentQuantizer, PixelImage, ThresholdEditTarget};
use crate::processing::linear_to_srgb;

pub const THRESHOLD_HISTOGRAM_BINS: usize = 512;

#[derive(Clone, Debug, PartialEq)]
pub struct ThresholdHistograms {
    pub encoded_rgb: [Vec<f64>; 3],
    /// Absolute Hue in degrees, Saturation and Value in normalized encoded-sRGB HSV.
    pub hsv: [Vec<f64>; 3],
    pub visible_alpha: f64,
    pub representative_hue_degrees: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HsvHistogramComponent {
    Hue,
    Saturation,
    Value,
}

impl ThresholdHistograms {
    pub fn build(source: &PixelImage) -> Self {
        Self::build_cancellable(source, || true).expect("unconditionally current histogram build")
    }

    /// Build the current encoded-sRGB and HSV distributions from the authoritative source.
    ///
    /// Source RGB remains straight and is never composited. Alpha is the contribution weight,
    /// so transparent RGB contributes nothing and partially transparent pixels contribute
    /// proportionally. The source values remain floating point until the final bounded display
    /// bin is selected.
    pub fn build_cancellable(
        source: &PixelImage,
        mut is_current: impl FnMut() -> bool,
    ) -> Option<Self> {
        if !is_current() {
            return None;
        }
        let mut encoded_rgb = std::array::from_fn(|_| vec![0.0; THRESHOLD_HISTOGRAM_BINS]);
        let mut hsv = std::array::from_fn(|_| vec![0.0; THRESHOLD_HISTOGRAM_BINS]);
        let mut visible_alpha = 0.0;
        let mut hue_x = 0.0_f64;
        let mut hue_y = 0.0_f64;
        let mut hue_weight = 0.0_f64;

        for (index, pixel) in source.pixels.iter().enumerate() {
            if index % 16_384 == 0 && !is_current() {
                return None;
            }
            let alpha = f64::from(pixel[3].clamp(0.0, 1.0));
            if alpha <= 0.0 {
                continue;
            }
            visible_alpha += alpha;
            let mut encoded = [0.0; 3];
            for channel in 0..3 {
                let linear = pixel[channel].clamp(0.0, 1.0);
                encoded[channel] = f64::from(linear_to_srgb(linear).clamp(0.0, 1.0));
                encoded_rgb[channel][unit_histogram_bin(encoded[channel])] += alpha;
            }
            let components = encoded_to_hsv(encoded);
            let saturation = components[1].clamp(0.0, 1.0);
            let value = components[2].clamp(0.0, 1.0);
            if saturation > 1.0e-6 {
                hsv[0][hue_histogram_bin(components[0])] += alpha;
                let weight = alpha * saturation;
                let radians = components[0].to_radians();
                hue_x += radians.cos() * weight;
                hue_y += radians.sin() * weight;
                hue_weight += weight;
            }
            hsv[1][unit_histogram_bin(saturation)] += alpha;
            hsv[2][unit_histogram_bin(value)] += alpha;
        }
        let representative_hue_degrees =
            if hue_weight <= f64::EPSILON || hue_x.hypot(hue_y) <= hue_weight * 1.0e-12 {
                0.0
            } else {
                hue_y.atan2(hue_x).to_degrees().rem_euclid(360.0)
            };
        is_current().then_some(Self {
            encoded_rgb,
            hsv,
            visible_alpha,
            representative_hue_degrees,
        })
    }
}

/// Series visibility is a property of the explicit semantic edit target, not the Link policy.
/// This keeps individual inspection honest while Link remains enabled.
pub fn threshold_histogram_series_mask(target: ThresholdEditTarget) -> [bool; 3] {
    match target {
        ThresholdEditTarget::Red => [true, false, false],
        ThresholdEditTarget::Green => [false, true, false],
        ThresholdEditTarget::Blue => [false, false, true],
        ThresholdEditTarget::LinkedRgb => [true, true, true],
        ThresholdEditTarget::Hue => [true, false, false],
        ThresholdEditTarget::Saturation => [false, true, false],
        ThresholdEditTarget::Value => [false, false, true],
        ThresholdEditTarget::LinkedSaturationValue => [false, true, true],
    }
}

pub fn relative_hue_histogram(absolute: &[f64], origin_degrees: f64) -> Vec<f64> {
    let mut relative = vec![0.0; absolute.len()];
    if absolute.is_empty() {
        return relative;
    }
    for (index, count) in absolute.iter().copied().enumerate() {
        let absolute_degrees = index as f64 * 360.0 / absolute.len() as f64;
        let relative_degrees = (absolute_degrees - origin_degrees).rem_euclid(360.0);
        let relative_index =
            ((relative_degrees / 360.0) * relative.len() as f64).floor() as usize % relative.len();
        relative[relative_index] += count;
    }
    relative
}

pub fn hsv_input_strip_color(
    component: HsvHistogramComponent,
    input: f64,
    hue_origin_degrees: f64,
    representative_hue_degrees: f64,
) -> [f64; 3] {
    let input = input.clamp(0.0, 1.0);
    hsv_to_encoded(match component {
        HsvHistogramComponent::Hue => [
            (hue_origin_degrees + input * 360.0).rem_euclid(360.0),
            1.0,
            1.0,
        ],
        HsvHistogramComponent::Saturation => {
            [representative_hue_degrees.rem_euclid(360.0), input, 1.0]
        }
        HsvHistogramComponent::Value => [0.0, 0.0, input],
    })
}

pub fn hsv_output_strip_color(
    component: HsvHistogramComponent,
    input: f64,
    quantizer: &ComponentQuantizer,
    hue_origin_degrees: f64,
    representative_hue_degrees: f64,
) -> [f64; 3] {
    let input = input.clamp(0.0, 1.0);
    let output = if quantizer.enabled {
        f64::from(quantizer.quantize(input as f32))
    } else {
        input
    };
    hsv_input_strip_color(
        component,
        output,
        hue_origin_degrees,
        representative_hue_degrees,
    )
}

fn unit_histogram_bin(value: f64) -> usize {
    (value.clamp(0.0, 1.0) * (THRESHOLD_HISTOGRAM_BINS - 1) as f64).round() as usize
}

fn hue_histogram_bin(degrees: f64) -> usize {
    ((degrees.rem_euclid(360.0) / 360.0) * THRESHOLD_HISTOGRAM_BINS as f64).floor() as usize
        % THRESHOLD_HISTOGRAM_BINS
}
