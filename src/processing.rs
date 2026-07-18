use std::sync::atomic::{AtomicU64, Ordering};

use crate::color::{encoded_to_hsv, hsv_to_encoded, linear_to_okhsl_cylinder};
use crate::document::{
    ColorComponent, ComponentOperation, ComponentSet, Method, PixelImage, Recipe, ThresholdSpace,
    VoronoiMatching,
};
use crate::voronoi::{distance2, linear_rgb_to_oklab};

pub const PREVIEW_MAX_DIMENSION: u32 = 1600;

#[derive(Clone, Debug)]
pub struct DisplayBuffer {
    pub width: u32,
    pub height: u32,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Coverage {
    pub site_counts: Vec<(u64, u64)>,
    pub visible_total: u64,
}

#[inline]
fn band(value: f32, thresholds: [f32; 2], outputs: [f32; 3]) -> f32 {
    if value <= thresholds[0] {
        outputs[0]
    } else if value <= thresholds[1] {
        outputs[1]
    } else {
        outputs[2]
    }
}

pub fn process(source: &PixelImage, recipe: &Recipe) -> PixelImage {
    process_cancellable(source, recipe, 0, &AtomicU64::new(0)).expect("fixed generation")
}

pub fn process_cancellable(
    source: &PixelImage,
    recipe: &Recipe,
    generation: u64,
    current: &AtomicU64,
) -> Option<PixelImage> {
    process_cancellable_with_progress(source, recipe, generation, current, |_| {})
}

pub fn process_cancellable_with_progress(
    source: &PixelImage,
    recipe: &Recipe,
    generation: u64,
    current: &AtomicU64,
    progress: impl FnMut(f64),
) -> Option<PixelImage> {
    process_cancellable_with_progress_and_coverage(source, recipe, generation, current, progress)
        .map(|(image, _)| image)
}

pub fn process_cancellable_with_progress_and_coverage(
    source: &PixelImage,
    recipe: &Recipe,
    generation: u64,
    current: &AtomicU64,
    mut progress: impl FnMut(f64),
) -> Option<(PixelImage, Coverage)> {
    let smoothing = recipe.threshold.input_smoothing;
    let smoothing_rows = usize::from(smoothing > 0.0) * 2;
    let active_steps = recipe
        .steps
        .iter()
        .filter(|step| !matches!(step.operation, ComponentOperation::ThreeBandQuantize { .. }))
        .count();
    let method_rows = 1usize;
    let total_rows =
        ((active_steps + method_rows + smoothing_rows).max(1) * source.height as usize) as f64;
    let mut completed_rows = 0usize;
    let mut pixels = if smoothing > 0.0 {
        gaussian_blur_cancellable(source, smoothing, generation, current, |rows| {
            completed_rows += rows;
            progress(completed_rows as f64 / total_rows);
        })?
        .pixels
        .to_vec()
    } else {
        source.pixels.to_vec()
    };
    let mut coverage = Coverage {
        site_counts: recipe
            .voronoi
            .sites
            .iter()
            .map(|site| (site.id, 0))
            .collect(),
        visible_total: 0,
    };
    if recipe.active_method == Method::Voronoi {
        let sites: Vec<_> = recipe
            .voronoi
            .sites
            .iter()
            .map(|site| {
                let source_rgb = [
                    site.source_color[0],
                    site.source_color[1],
                    site.source_color[2],
                ];
                let encoded = source_rgb.map(|channel| linear_to_srgb(channel) as f64);
                let hsv = encoded_to_hsv(encoded);
                CompiledSite {
                    id: site.id,
                    order: site.order,
                    oklab: linear_rgb_to_oklab(source_rgb),
                    okhsl_cylinder: if recipe.voronoi.matching == VoronoiMatching::Okhsl {
                        linear_to_okhsl_cylinder(source_rgb.map(f64::from))
                            .expect("validated bounded site colors have OKHSL coordinates")
                    } else {
                        [0.0; 3]
                    },
                    encoded,
                    hsv_cone: hsv_cone_point(hsv),
                    influence: site.influence,
                    target: site.target_color,
                }
            })
            .collect();
        for (row, scanline) in pixels.chunks_mut(source.width as usize).enumerate() {
            if row % 8 == 0 && current.load(Ordering::Acquire) != generation {
                return None;
            }
            for pixel in scanline {
                if pixel[3] == 0.0 {
                    pixel[..3].fill(0.0);
                    continue;
                }
                coverage.visible_total += 1;
                if let Some(site) = nearest_site(*pixel, &sites, recipe.voronoi.matching) {
                    if let Some((_, count)) = coverage
                        .site_counts
                        .iter_mut()
                        .find(|(id, _)| *id == site.id)
                    {
                        *count += 1;
                    }
                    pixel[..3].copy_from_slice(&site.target);
                }
            }
            completed_rows += 1;
            progress(completed_rows as f64 / total_rows);
        }
    }
    if recipe.active_method == Method::Thresholds {
        for (row, scanline) in pixels.chunks_mut(source.width as usize).enumerate() {
            if row % 8 == 0 && current.load(Ordering::Acquire) != generation {
                return None;
            }
            for pixel in scanline {
                apply_threshold(pixel, recipe);
            }
            completed_rows += 1;
            progress(completed_rows as f64 / total_rows);
        }
    }
    for step in &recipe.steps {
        if matches!(step.operation, ComponentOperation::ThreeBandQuantize { .. }) {
            continue;
        }
        for (row, scanline) in pixels.chunks_mut(source.width as usize).enumerate() {
            if row % 8 == 0 && current.load(Ordering::Acquire) != generation {
                return None;
            }
            for pixel in scanline {
                match (&step.components, &step.operation) {
                    (
                        components @ (ComponentSet::AllRgb | ComponentSet::SelectedRgb(_)),
                        ComponentOperation::ThreeBandQuantize {
                            thresholds,
                            outputs,
                            ..
                        },
                    ) => {
                        for (index, component) in [
                            ColorComponent::Red,
                            ColorComponent::Green,
                            ColorComponent::Blue,
                        ]
                        .into_iter()
                        .enumerate()
                        {
                            let selected = matches!(components, ComponentSet::AllRgb)
                                || matches!(components, ComponentSet::SelectedRgb(selected) if selected.contains(&component));
                            if selected {
                                pixel[index] = band(pixel[index], *thresholds, *outputs);
                            }
                        }
                    }
                    (ComponentSet::Hue, ComponentOperation::RotateHue { degrees }) => {
                        let rotated = rotate_oklch([pixel[0], pixel[1], pixel[2]], *degrees);
                        pixel[..3].copy_from_slice(&rotated);
                    }
                    _ => {}
                }
            }
            completed_rows += 1;
            progress(completed_rows as f64 / total_rows);
        }
    }
    if current.load(Ordering::Acquire) != generation {
        return None;
    }
    progress(1.0);
    Some((
        PixelImage {
            width: source.width,
            height: source.height,
            pixels: pixels.into(),
        },
        coverage,
    ))
}

pub fn gaussian_blur_cancellable(
    source: &PixelImage,
    amount: f32,
    generation: u64,
    current: &AtomicU64,
    mut rows_done: impl FnMut(usize),
) -> Option<PixelImage> {
    if amount <= 0.0 {
        return Some(source.clone());
    }
    let sigma = amount.max(0.5);
    let radius = (3.0 * sigma).ceil() as i32;
    let mut weights = (-radius..=radius)
        .map(|offset| (-(offset * offset) as f32 / (2.0 * sigma * sigma)).exp())
        .collect::<Vec<_>>();
    let sum: f32 = weights.iter().sum();
    for weight in &mut weights {
        *weight /= sum;
    }
    let width = source.width as usize;
    let height = source.height as usize;
    let mut horizontal = vec![[0.0; 4]; source.pixels.len()];
    for y in 0..height {
        if y % 4 == 0 && current.load(Ordering::Acquire) != generation {
            return None;
        }
        for x in 0..width {
            let mut premul = [0.0; 4];
            for (kernel, offset) in weights.iter().zip(-radius..=radius) {
                let sx = (x as i32 + offset).clamp(0, width as i32 - 1) as usize;
                let pixel = source.pixels[y * width + sx];
                premul[0] += pixel[0] * pixel[3] * kernel;
                premul[1] += pixel[1] * pixel[3] * kernel;
                premul[2] += pixel[2] * pixel[3] * kernel;
                premul[3] += pixel[3] * kernel;
            }
            horizontal[y * width + x] = premul;
        }
        rows_done(1);
    }
    let mut output = vec![[0.0; 4]; source.pixels.len()];
    for y in 0..height {
        if y % 4 == 0 && current.load(Ordering::Acquire) != generation {
            return None;
        }
        for x in 0..width {
            let mut premul = [0.0; 4];
            for (kernel, offset) in weights.iter().zip(-radius..=radius) {
                let sy = (y as i32 + offset).clamp(0, height as i32 - 1) as usize;
                let pixel = horizontal[sy * width + x];
                for channel in 0..4 {
                    premul[channel] += pixel[channel] * kernel;
                }
            }
            let alpha = premul[3].clamp(0.0, 1.0);
            output[y * width + x] = if alpha > 1.0e-8 {
                [
                    premul[0] / alpha,
                    premul[1] / alpha,
                    premul[2] / alpha,
                    alpha,
                ]
            } else {
                [0.0, 0.0, 0.0, 0.0]
            };
        }
        rows_done(1);
    }
    PixelImage::new(source.width, source.height, output).ok()
}

fn apply_threshold(pixel: &mut [f32; 4], recipe: &Recipe) {
    if pixel[3] == 0.0 {
        pixel[..3].fill(0.0);
        return;
    }
    let threshold = &recipe.threshold;
    match threshold.active_space {
        ThresholdSpace::Rgb => {
            let mut components = [
                linear_to_srgb(pixel[0]),
                linear_to_srgb(pixel[1]),
                linear_to_srgb(pixel[2]),
            ];
            for (value, quantizer) in components.iter_mut().zip(&threshold.rgb_state.components) {
                if quantizer.enabled {
                    *value = quantizer.quantize(*value);
                }
            }
            for index in 0..3 {
                pixel[index] = srgb_to_linear(components[index]);
            }
        }
        ThresholdSpace::Hsv => {
            let encoded = [
                linear_to_srgb(pixel[0]) as f64,
                linear_to_srgb(pixel[1]) as f64,
                linear_to_srgb(pixel[2]) as f64,
            ];
            let original = encoded_to_hsv(encoded);
            let mut hsv = original;
            let state = &threshold.hsv_state;
            if state.hue.enabled && original[1] > 1.0e-6 {
                let origin = (state.hue_origin_degrees as f64).rem_euclid(360.0);
                let relative_unit = ((original[0] - origin).rem_euclid(360.0) / 360.0) as f32;
                hsv[0] =
                    (state.hue.quantize(relative_unit) as f64 * 360.0 + origin).rem_euclid(360.0);
            }
            if state.saturation.enabled {
                hsv[1] = state.saturation.quantize(original[1] as f32) as f64;
            }
            if state.value.enabled {
                hsv[2] = state.value.quantize(original[2] as f32) as f64;
            }
            // Quantizing hue must never create chroma from an achromatic source.
            if original[1] <= 1.0e-6 {
                hsv[0] = original[0];
                if !state.saturation.enabled {
                    hsv[1] = original[1];
                }
            }
            let rgb = hsv_to_encoded(hsv);
            for index in 0..3 {
                pixel[index] = srgb_to_linear(rgb[index].clamp(0.0, 1.0) as f32);
            }
        }
    }
}

struct CompiledSite {
    id: u64,
    order: u64,
    oklab: [f64; 3],
    okhsl_cylinder: [f64; 3],
    encoded: [f64; 3],
    hsv_cone: [f64; 3],
    influence: f64,
    target: [f32; 3],
}

fn nearest_site(
    pixel: [f32; 4],
    sites: &[CompiledSite],
    matching: VoronoiMatching,
) -> Option<&CompiledSite> {
    let coordinates = match matching {
        VoronoiMatching::Perceptual => linear_rgb_to_oklab([pixel[0], pixel[1], pixel[2]]),
        VoronoiMatching::Okhsl => {
            linear_to_okhsl_cylinder([pixel[0] as f64, pixel[1] as f64, pixel[2] as f64])
                .expect("bounded processing pixels have OKHSL coordinates")
        }
        VoronoiMatching::Rgb => [
            linear_to_srgb(pixel[0]) as f64,
            linear_to_srgb(pixel[1]) as f64,
            linear_to_srgb(pixel[2]) as f64,
        ],
        VoronoiMatching::Hsv => {
            let encoded = [
                linear_to_srgb(pixel[0]) as f64,
                linear_to_srgb(pixel[1]) as f64,
                linear_to_srgb(pixel[2]) as f64,
            ];
            let hsv = encoded_to_hsv(encoded);
            hsv_cone_point(hsv)
        }
    };
    sites.iter().min_by(|a, b| {
        let metric = |site: &CompiledSite| {
            let site_coordinates = match matching {
                VoronoiMatching::Perceptual => site.oklab,
                VoronoiMatching::Okhsl => site.okhsl_cylinder,
                VoronoiMatching::Rgb => site.encoded,
                VoronoiMatching::Hsv => site.hsv_cone,
            };
            distance2(coordinates, site_coordinates)
        };
        // Influence is dimensionless and scales the complete native metric, never individual
        // axes. Each +1 halves weighted distance², expanding that site's radial reach by sqrt(2);
        // the -4..=4 UI range therefore spans radius factors 1/4..=4 around neutral.
        let da = metric(a) * 2.0_f64.powf(-a.influence.clamp(-4.0, 4.0));
        let db = metric(b) * 2.0_f64.powf(-b.influence.clamp(-4.0, 4.0));
        let scale = da.abs().max(db.abs()).max(1.0);
        if (da - db).abs() <= scale * 1.0e-12 {
            a.order.cmp(&b.order).then_with(|| a.id.cmp(&b.id))
        } else {
            da.total_cmp(&db)
        }
    })
}

fn hsv_cone_point(hsv: [f64; 3]) -> [f64; 3] {
    let radians = hsv[0].to_radians();
    let radius = hsv[1] * hsv[2];
    [radius * radians.cos(), radius * radians.sin(), hsv[2]]
}

/// Create a bounded linear-f32 preview. Full-resolution authoritative pixels remain untouched.
pub fn bounded_preview(source: &PixelImage) -> PixelImage {
    let largest = source.width.max(source.height);
    if largest <= PREVIEW_MAX_DIMENSION {
        return source.clone();
    }
    let scale = PREVIEW_MAX_DIMENSION as f64 / largest as f64;
    let width = (source.width as f64 * scale).round().max(1.0) as u32;
    let height = (source.height as f64 * scale).round().max(1.0) as u32;
    let mut pixels = Vec::with_capacity(width as usize * height as usize);
    for y in 0..height {
        let sy = ((y as u64 * source.height as u64) / height as u64).min(source.height as u64 - 1)
            as usize;
        for x in 0..width {
            let sx = ((x as u64 * source.width as u64) / width as u64).min(source.width as u64 - 1)
                as usize;
            pixels.push(source.pixels[sy * source.width as usize + sx]);
        }
    }
    PixelImage::new(width, height, pixels).expect("preview dimensions match")
}

pub fn rotate_oklch(rgb: [f32; 3], degrees: f32) -> [f32; 3] {
    if degrees.rem_euclid(360.0).abs() < f32::EPSILON {
        return rgb;
    }
    let l = 0.412_221_46 * rgb[0] + 0.536_332_55 * rgb[1] + 0.051_445_995 * rgb[2];
    let m = 0.211_903_5 * rgb[0] + 0.680_699_5 * rgb[1] + 0.107_396_96 * rgb[2];
    let s = 0.088_302_46 * rgb[0] + 0.281_718_85 * rgb[1] + 0.629_978_7 * rgb[2];
    let (l_, m_, s_) = (l.cbrt(), m.cbrt(), s.cbrt());
    let light = 0.210_454_26 * l_ + 0.793_617_8 * m_ - 0.004_072_047 * s_;
    let aa = 1.977_998_5 * l_ - 2.428_592_2 * m_ + 0.450_593_7 * s_;
    let bb = 0.025_904_037 * l_ + 0.782_771_77 * m_ - 0.808_675_77 * s_;
    if aa.hypot(bb) < 1.0e-6 {
        return rgb;
    }
    let (sin, cos) = degrees.rem_euclid(360.0).to_radians().sin_cos();
    let (a2, b2) = (aa * cos - bb * sin, aa * sin + bb * cos);
    let l2 = light + 0.396_337_78 * a2 + 0.215_803_76 * b2;
    let m2 = light - 0.105_561_346 * a2 - 0.063_854_17 * b2;
    let s2 = light - 0.089_484_18 * a2 - 1.291_485_5 * b2;
    let (l3, m3, s3) = (l2 * l2 * l2, m2 * m2 * m2, s2 * s2 * s2);
    [
        (4.076_741_7 * l3 - 3.307_711_6 * m3 + 0.230_969_94 * s3).clamp(0.0, 1.0),
        (-1.268_438 * l3 + 2.609_757_4 * m3 - 0.341_319_38 * s3).clamp(0.0, 1.0),
        (-0.004_196_086_3 * l3 - 0.703_418_6 * m3 + 1.707_614_7 * s3).clamp(0.0, 1.0),
    ]
}

pub fn linear_to_srgb(value: f32) -> f32 {
    if value <= 0.003_130_8 {
        12.92 * value
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}
pub fn srgb_to_linear(value: f32) -> f32 {
    if value <= 0.040_45 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

/// Straight encoded-sRGB RGBA8 for GdkPixbuf. Low alpha never changes RGB values.
pub fn to_display_rgba8(image: &PixelImage) -> DisplayBuffer {
    let mut bytes = Vec::with_capacity(image.pixels.len() * 4);
    for &[r, g, b, a] in image.pixels.iter() {
        for channel in [r, g, b] {
            bytes.push((linear_to_srgb(channel.clamp(0.0, 1.0)) * 255.0).round() as u8);
        }
        bytes.push((a.clamp(0.0, 1.0) * 255.0).round() as u8);
    }
    DisplayBuffer {
        width: image.width,
        height: image.height,
        bytes,
    }
}
