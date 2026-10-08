use std::sync::atomic::{AtomicU64, Ordering};

use crate::color::{encoded_to_hsv, linear_to_okhsl_cylinder};
use crate::document::{
    ComponentOperation, ComponentSet, MAX_INPUT_SMOOTHING_SIGMA, PixelImage, Recipe,
    VoronoiMatching,
};
use crate::voronoi::{distance2, linear_rgb_to_oklab};
use crate::transitions::{BlendScratch, CompiledBlend};

pub const PREVIEW_MAX_DIMENSION: u32 = 1600;
const BOX_BLUR_ONLY_SIGMA: f32 = 12.0;
const FIR_TRANSITION_RADIUS: i32 = 30; // The established 3σ support at the legacy 10px ceiling.

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
    let compiled = CompiledVoronoi::compile_cancellable(recipe, &|| current.load(Ordering::Acquire) == generation)
        .expect("cannot process an invalid Voronoi recipe")?;
    let mut blend_scratch = BlendScratch::new(if compiled.blend.is_some() { compiled.sites.len() } else { 0 });
    let smoothing = recipe.preprocessing.input_smoothing;
    let smoothing_rows = blur_work_units(source, smoothing);
    let active_steps = recipe.steps.len();
    let total_rows = ((active_steps + 1) * source.height as usize + smoothing_rows).max(1) as f64;
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
        site_counts: compiled
            .sites
            .iter()
            .map(|site| (site.context.id, 0))
            .collect(),
        visible_total: 0,
    };
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
            if let Some(index) = compiled.winner_index(*pixel) {
                coverage.site_counts[index].1 += 1;
                *pixel = if let Some(blend) = &compiled.blend {
                    let coordinates = compiled.match_coordinates(*pixel);
                    for (score, site) in blend_scratch.scores.iter_mut().zip(&compiled.sites) {
                        *score = distance2(coordinates, compiled.site_coordinates(site)) * site.distance_weight;
                    }
                    blend.resolve(*pixel, &mut blend_scratch)
                } else {
                    compiled.resolve_hard(*pixel, index)
                };
            }
        }
        completed_rows += 1;
        progress(completed_rows as f64 / total_rows);
    }
    for step in &recipe.steps {
        for (row, scanline) in pixels.chunks_mut(source.width as usize).enumerate() {
            if row % 8 == 0 && current.load(Ordering::Acquire) != generation {
                return None;
            }
            for pixel in scanline {
                let (ComponentSet::Hue, ComponentOperation::RotateHue { degrees }) =
                    (&step.components, &step.operation);
                let rotated = rotate_oklch([pixel[0], pixel[1], pixel[2]], *degrees);
                pixel[..3].copy_from_slice(&rotated);
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

/// Process the authoritative full-resolution source, then publish a bounded result image.
/// The preview therefore uses the same processing path and sigma units as export.
pub fn process_preview_cancellable_with_progress_and_coverage(
    source: &PixelImage,
    recipe: &Recipe,
    generation: u64,
    current: &AtomicU64,
    mut progress: impl FnMut(f64),
) -> Option<(PixelImage, Coverage)> {
    let (full_result, coverage) = process_cancellable_with_progress_and_coverage(
        source,
        recipe,
        generation,
        current,
        |fraction| progress(fraction * 0.9),
    )?;
    let preview = bounded_preview_cancellable(&full_result, generation, current, |fraction| {
        progress(0.9 + fraction * 0.1);
    })?;
    Some((preview, coverage))
}

/// Blur with σ in original source-image pixels, preserving legacy FIR output through σ=10.
/// A short FIR/three-box crossfade avoids a step at the bounded-cost path transition.
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
    if !amount.is_finite() || amount > MAX_INPUT_SMOOTHING_SIGMA {
        return None;
    }
    let sigma = amount;
    if sigma >= BOX_BLUR_ONLY_SIGMA {
        return gaussian_blur_boxes_cancellable(source, sigma, generation, current, &mut rows_done);
    }
    if sigma > 10.0 {
        let exact = gaussian_blur_fir_with_radius_cancellable(
            source,
            sigma,
            FIR_TRANSITION_RADIUS,
            generation,
            current,
            &mut rows_done,
        )?;
        let boxes = gaussian_blur_boxes_cancellable(source, sigma, generation, current, &mut rows_done)?;
        return blend_blur_results(
            source.width,
            source.height,
            exact,
            boxes,
            (sigma - 10.0) / (BOX_BLUR_ONLY_SIGMA - 10.0),
            generation,
            current,
            &mut rows_done,
        );
    }
    if sigma < 0.5 {
        return gaussian_blur_small_sigma_cancellable(
            source,
            sigma,
            generation,
            current,
            &mut rows_done,
        );
    }

    gaussian_blur_fir_cancellable(source, sigma, generation, current, &mut rows_done)
}

fn gaussian_blur_fir_cancellable(
    source: &PixelImage,
    sigma: f32,
    generation: u64,
    current: &AtomicU64,
    rows_done: &mut impl FnMut(usize),
) -> Option<PixelImage> {
    gaussian_blur_fir_with_radius_cancellable(
        source,
        sigma,
        (3.0 * sigma).ceil() as i32,
        generation,
        current,
        rows_done,
    )
}

fn gaussian_blur_fir_with_radius_cancellable(
    source: &PixelImage,
    sigma: f32,
    radius: i32,
    generation: u64,
    current: &AtomicU64,
    rows_done: &mut impl FnMut(usize),
) -> Option<PixelImage> {
    // Keep the established FIR kernel and accumulation order for the legacy range.
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

#[allow(clippy::too_many_arguments)]
fn blend_blur_results(
    width: u32,
    height: u32,
    exact: PixelImage,
    boxes: PixelImage,
    amount: f32,
    generation: u64,
    current: &AtomicU64,
    rows_done: &mut impl FnMut(usize),
) -> Option<PixelImage> {
    let blend = f64::from(amount.clamp(0.0, 1.0));
    let keep = 1.0 - blend;
    let mut pixels = Vec::with_capacity(exact.pixels.len());
    let width_usize = width as usize;
    let height_usize = height as usize;
    for y in 0..height_usize {
        if current.load(Ordering::Acquire) != generation {
            return None;
        }
        let row_start = y * width_usize;
        for x in 0..width_usize {
            if x % 4096 == 0 && current.load(Ordering::Acquire) != generation {
                return None;
            }
            let index = row_start + x;
            let exact = exact.pixels[index];
            let boxes = boxes.pixels[index];
            let alpha = f64::from(exact[3]) * keep + f64::from(boxes[3]) * blend;
            let premul: [f64; 3] = std::array::from_fn(|channel| {
                f64::from(exact[channel]) * f64::from(exact[3]) * keep
                    + f64::from(boxes[channel]) * f64::from(boxes[3]) * blend
            });
            pixels.push(unpremultiply([premul[0], premul[1], premul[2], alpha]));
        }
        rows_done(1);
    }
    if current.load(Ordering::Acquire) != generation {
        return None;
    }
    PixelImage::new(width, height, pixels).ok()
}

fn gaussian_blur_small_sigma_cancellable(
    source: &PixelImage,
    sigma: f32,
    generation: u64,
    current: &AtomicU64,
    rows_done: &mut impl FnMut(usize),
) -> Option<PixelImage> {
    let radius = (3.0 * sigma).ceil() as i32;
    let sigma64 = f64::from(sigma);
    let mut weights = (-radius..=radius)
        .map(|offset| {
            let offset = f64::from(offset);
            (-(offset * offset) / (2.0 * sigma64 * sigma64)).exp()
        })
        .collect::<Vec<_>>();
    let sum: f64 = weights.iter().sum();
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
            let mut premul = [0.0_f64; 4];
            for (kernel, offset) in weights.iter().zip(-radius..=radius) {
                let sx = (x as i64 + i64::from(offset)).clamp(0, width as i64 - 1) as usize;
                let pixel = source.pixels[y * width + sx];
                premul[0] += f64::from(pixel[0] * pixel[3]) * kernel;
                premul[1] += f64::from(pixel[1] * pixel[3]) * kernel;
                premul[2] += f64::from(pixel[2] * pixel[3]) * kernel;
                premul[3] += f64::from(pixel[3]) * kernel;
            }
            horizontal[y * width + x] = premul.map(|channel| channel as f32);
        }
        rows_done(1);
    }
    let mut output = vec![[0.0; 4]; source.pixels.len()];
    for y in 0..height {
        if y % 4 == 0 && current.load(Ordering::Acquire) != generation {
            return None;
        }
        for x in 0..width {
            let mut premul = [0.0_f64; 4];
            for (kernel, offset) in weights.iter().zip(-radius..=radius) {
                let sy = (y as i64 + i64::from(offset)).clamp(0, height as i64 - 1) as usize;
                let pixel = horizontal[sy * width + x];
                for channel in 0..4 {
                    premul[channel] += f64::from(pixel[channel]) * kernel;
                }
            }
            output[y * width + x] = unpremultiply(premul);
        }
        rows_done(1);
    }
    PixelImage::new(source.width, source.height, output).ok()
}

fn gaussian_blur_boxes_cancellable(
    source: &PixelImage,
    sigma: f32,
    generation: u64,
    current: &AtomicU64,
    rows_done: &mut impl FnMut(usize),
) -> Option<PixelImage> {
    let width = source.width as usize;
    let height = source.height as usize;
    if width == 0 || height == 0 {
        return Some(source.clone());
    }
    let mut working = Vec::with_capacity(source.pixels.len());
    for (index, pixel) in source.pixels.iter().enumerate() {
        if index % 8192 == 0 && current.load(Ordering::Acquire) != generation {
            return None;
        }
        working.push([pixel[0] * pixel[3], pixel[1] * pixel[3], pixel[2] * pixel[3], pixel[3]]);
    }
    let mut scratch = vec![[0.0; 4]; source.pixels.len()];
    let radii = gaussian_box_radii(f64::from(sigma));
    for (box_index, radius) in radii.into_iter().enumerate() {
        if !box_pass_cancellable(
            &working,
            &mut scratch,
            width,
            height,
            radius,
            true,
            false,
            generation,
            current,
            rows_done,
        ) {
            return None;
        }
        if !box_pass_cancellable(
            &scratch,
            &mut working,
            width,
            height,
            radius,
            false,
            box_index == 2,
            generation,
            current,
            rows_done,
        ) {
            return None;
        }
    }
    PixelImage::new(source.width, source.height, working).ok()
}

fn gaussian_box_radii(sigma: f64) -> [usize; 3] {
    let ideal_width = (4.0 * sigma * sigma + 1.0).sqrt();
    let mut lower_width = ideal_width.floor() as usize;
    if lower_width.is_multiple_of(2) {
        lower_width = lower_width.saturating_sub(1);
    }
    lower_width = lower_width.max(1);
    let upper_width = lower_width + 2;
    let lower = lower_width as f64;
    let lower_count = ((12.0 * sigma * sigma - 3.0 * lower * lower - 12.0 * lower - 9.0)
        / (-4.0 * lower - 4.0))
        .round()
        .clamp(0.0, 3.0) as usize;
    std::array::from_fn(|index| {
        let width = if index < lower_count { lower_width } else { upper_width };
        (width - 1) / 2
    })
}

#[allow(clippy::too_many_arguments)]
fn box_pass_cancellable(
    input: &[[f32; 4]],
    output: &mut [[f32; 4]],
    width: usize,
    height: usize,
    radius: usize,
    horizontal: bool,
    unpremultiply_output: bool,
    generation: u64,
    current: &AtomicU64,
    rows_done: &mut impl FnMut(usize),
) -> bool {
    let line_count = if horizontal { height } else { width };
    let line_len = if horizontal { width } else { height };
    let radius_i64 = radius as i64;
    let divisor = (radius.saturating_mul(2).saturating_add(1)) as f64;
    let inv_divisor = 1.0 / divisor;
    for line in 0..line_count {
        if current.load(Ordering::Acquire) != generation {
            return false;
        }
        let at = |position: usize| {
            if horizontal {
                line * width + position
            } else {
                position * width + line
            }
        };
        let mut sum = [0.0_f64; 4];
        let edge_count = radius.saturating_add(1) as f64;
        for channel in 0..4 {
            sum[channel] = f64::from(input[at(0)][channel]) * edge_count;
        }
        let unique_right = radius.min(line_len - 1);
        for position in 1..=unique_right {
            let pixel = input[at(position)];
            for channel in 0..4 {
                sum[channel] += f64::from(pixel[channel]);
            }
        }
        if radius >= line_len - 1 {
            let replicated = (radius - (line_len - 1)) as f64;
            let pixel = input[at(line_len - 1)];
            for channel in 0..4 {
                sum[channel] += f64::from(pixel[channel]) * replicated;
            }
        }
        for position in 0..line_len {
            if position % 4096 == 0 && current.load(Ordering::Acquire) != generation {
                return false;
            }
            let average = sum.map(|value| (value * inv_divisor) as f32);
            output[at(position)] = if unpremultiply_output {
                unpremultiply(average.map(f64::from))
            } else {
                average
            };
            if position + 1 < line_len {
                let leaving = (position as i64 - radius_i64).clamp(0, line_len as i64 - 1) as usize;
                let entering = (position as i64 + radius_i64 + 1).clamp(0, line_len as i64 - 1) as usize;
                let left = input[at(leaving)];
                let right = input[at(entering)];
                for channel in 0..4 {
                    sum[channel] += f64::from(right[channel]) - f64::from(left[channel]);
                }
            }
        }
        rows_done(1);
    }
    true
}

fn unpremultiply(premul: [f64; 4]) -> [f32; 4] {
    let alpha = premul[3].clamp(0.0, 1.0);
    if alpha > 1.0e-8 {
        [
            (premul[0] / alpha) as f32,
            (premul[1] / alpha) as f32,
            (premul[2] / alpha) as f32,
            alpha as f32,
        ]
    } else {
        [0.0, 0.0, 0.0, 0.0]
    }
}

fn blur_work_units(source: &PixelImage, sigma: f32) -> usize {
    if sigma <= 0.0 {
        0
    } else if sigma >= BOX_BLUR_ONLY_SIGMA {
        3 * (source.width as usize + source.height as usize)
    } else if sigma > 10.0 {
        3 * source.height as usize
            + 3 * (source.width as usize + source.height as usize)
    } else {
        2 * source.height as usize
    }
}

/// Source and output context retained for the winning site and future boundary resolution.
#[derive(Clone, Debug, PartialEq)]
pub struct CompiledSiteContext {
    /// Stable persisted site identifier.
    pub id: u64,
    /// Stable tie-break order, lower values winning coincident boundaries.
    pub order: u64,
    /// Source color in straight-alpha linear-sRGB units, without alpha.
    pub source_linear: [f32; 3],
    /// Target color in linear-sRGB units.
    pub target_linear: [f32; 3],
    /// Dimensionless reach control retained for future boundary profiles.
    pub influence: f64,
}

struct CompiledSite {
    context: CompiledSiteContext,
    oklab: [f64; 3],
    okhsl_cylinder: [f64; 3],
    encoded: [f64; 3],
    hsv_cone: [f64; 3],
    distance_weight: f64,
}

/// Immutable Voronoi matching partition compiled once before any pixel work.
pub struct CompiledVoronoi {
    matching: VoronoiMatching,
    sites: Vec<CompiledSite>,
    blend: Option<CompiledBlend>,
}

impl CompiledVoronoi {
    /// Validate processing state and precompute every site coordinate used by the selected metric.
    pub fn compile(recipe: &Recipe) -> Result<Self, String> {
        Self::compile_cancellable(recipe, &|| true).map(|compiled| compiled.expect("uncancellable compilation"))
    }

    fn compile_cancellable(recipe: &Recipe, current: &impl Fn() -> bool) -> Result<Option<Self>, String> {
        recipe.validate_for_processing()?;
        if !current() { return Ok(None); }
        let matching = recipe.voronoi.matching;
        let sites = recipe
            .voronoi
            .sites
            .iter()
            .map(|site| {
                let source_linear = [
                    site.source_color[0],
                    site.source_color[1],
                    site.source_color[2],
                ];
                let encoded = source_linear.map(|channel| f64::from(linear_to_srgb(channel)));
                let hsv = encoded_to_hsv(encoded);
                CompiledSite {
                    context: CompiledSiteContext {
                        id: site.id,
                        order: site.order,
                        source_linear,
                        target_linear: site.target_color,
                        influence: site.influence,
                    },
                    oklab: linear_rgb_to_oklab(source_linear),
                    okhsl_cylinder: if matching == VoronoiMatching::Okhsl {
                        linear_to_okhsl_cylinder(source_linear.map(f64::from))
                            .expect("validated bounded site colors have OKHSL coordinates")
                    } else {
                        [0.0; 3]
                    },
                    encoded,
                    hsv_cone: hsv_cone_point(hsv),
                    distance_weight: 2.0_f64.powf(-site.influence.clamp(-4.0, 4.0)),
                }
            })
            .collect();
        let mut compiled = Self { matching, sites, blend: None };
        if recipe.voronoi.transition.width() > 0.0 {
            let targets = compiled.sites.iter().map(|site| site.context.target_linear).collect();
            compiled.blend = Some(CompiledBlend::compile(recipe.voronoi.transition, targets));
        }
        if !current() { return Ok(None); }
        Ok(Some(compiled))
    }

    fn site_coordinates(&self, site: &CompiledSite) -> [f64; 3] {
        match self.matching {
            VoronoiMatching::Perceptual => site.oklab,
            VoronoiMatching::Okhsl => site.okhsl_cylinder,
            VoronoiMatching::Rgb => site.encoded,
            VoronoiMatching::Hsv => site.hsv_cone,
        }
    }

    fn match_coordinates(&self, pixel: [f32; 4]) -> [f64; 3] {
        let linear = [pixel[0], pixel[1], pixel[2]];
        match self.matching {
            VoronoiMatching::Perceptual => linear_rgb_to_oklab(linear),
            VoronoiMatching::Okhsl => linear_to_okhsl_cylinder(linear.map(f64::from))
                .expect("bounded processing pixels have OKHSL coordinates"),
            VoronoiMatching::Rgb => linear.map(|v| f64::from(linear_to_srgb(v))),
            VoronoiMatching::Hsv => hsv_cone_point(encoded_to_hsv(linear.map(|v| f64::from(linear_to_srgb(v))))),
        }
    }

    /// Stable site contexts in direct coverage-index order.
    pub fn site_contexts(&self) -> impl ExactSizeIterator<Item = &CompiledSiteContext> {
        self.sites.iter().map(|site| &site.context)
    }

    /// Return the stable winning site index without allocating candidate storage.
    pub fn winner_index(&self, pixel: [f32; 4]) -> Option<usize> {
        let coordinates = match self.matching {
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
        self.sites
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                let metric = |site: &CompiledSite| {
                    let site_coordinates = match self.matching {
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
                let da = metric(a) * a.distance_weight;
                let db = metric(b) * b.distance_weight;
                let scale = da.abs().max(db.abs()).max(1.0);
                if (da - db).abs() <= scale * 1.0e-12 {
                    a.context
                        .order
                        .cmp(&b.context.order)
                        .then_with(|| a.context.id.cmp(&b.context.id))
                } else {
                    da.total_cmp(&db)
                }
            })
            .map(|(index, _)| index)
    }

    /// Resolves a hard boundary winner by copying Target RGB and preserving straight alpha.
    ///
    /// Fully transparent pixels are normalized before this boundary in the processing pipeline and
    /// never require winner resolution.
    ///
    /// # Panics
    /// Panics when `winner_index` was not returned by [`Self::winner_index`] for this partition.
    pub fn resolve_hard(&self, pixel: [f32; 4], winner_index: usize) -> [f32; 4] {
        let target = self.sites[winner_index].context.target_linear;
        [target[0], target[1], target[2], pixel[3]]
    }
}

fn hsv_cone_point(hsv: [f64; 3]) -> [f64; 3] {
    let radians = hsv[0].to_radians();
    let radius = hsv[1] * hsv[2];
    [radius * radians.cos(), radius * radians.sin(), hsv[2]]
}

/// Create a bounded linear-f32 preview. Full-resolution authoritative pixels remain untouched.
pub fn bounded_preview(source: &PixelImage) -> PixelImage {
    bounded_preview_cancellable(source, 0, &AtomicU64::new(0), |_| {})
        .expect("fixed generation")
}

fn bounded_preview_cancellable(
    source: &PixelImage,
    generation: u64,
    current: &AtomicU64,
    mut progress: impl FnMut(f64),
) -> Option<PixelImage> {
    let largest = source.width.max(source.height);
    if largest <= PREVIEW_MAX_DIMENSION {
        if current.load(Ordering::Acquire) != generation {
            return None;
        }
        progress(1.0);
        return Some(source.clone());
    }
    let scale = PREVIEW_MAX_DIMENSION as f64 / largest as f64;
    let width = (source.width as f64 * scale).round().max(1.0) as u32;
    let height = (source.height as f64 * scale).round().max(1.0) as u32;
    let mut pixels = Vec::with_capacity(width as usize * height as usize);
    for y in 0..height {
        if current.load(Ordering::Acquire) != generation {
            return None;
        }
        let sy = ((y as u64 * source.height as u64) / height as u64).min(source.height as u64 - 1)
            as usize;
        for x in 0..width {
            let sx = ((x as u64 * source.width as u64) / width as u64).min(source.width as u64 - 1)
                as usize;
            pixels.push(source.pixels[sy * source.width as usize + sx]);
        }
        progress((y + 1) as f64 / height as f64);
    }
    PixelImage::new(width, height, pixels).ok()
}

/// Rotates one linear-sRGB color in OKLCh using the persisted f32 operation kernel.
///
/// Voronoi matching intentionally retains its established f64 conversion in `voronoi`: forcing
/// both paths through one precision would change the ordered, per-step clipping contract.
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
