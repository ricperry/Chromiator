use std::collections::BTreeMap;

use crate::document::{PixelImage, SampleSize, VoronoiSite, VoronoiState};

const DISTINCT_D2: f64 = 0.0025;

pub fn linear_rgb_to_oklab(rgb: [f32; 3]) -> [f64; 3] {
    let r = rgb[0] as f64;
    let g = rgb[1] as f64;
    let b = rgb[2] as f64;
    let l = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
    let m = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
    let s = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();
    [
        0.210_454_255_3 * l + 0.793_617_785 * m - 0.004_072_046_8 * s,
        1.977_998_495_1 * l - 2.428_592_205 * m + 0.450_593_709_9 * s,
        0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766 * s,
    ]
}

pub fn distance2(a: [f64; 3], b: [f64; 3]) -> f64 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
}

pub fn sample_color(image: &PixelImage, position: [f64; 2], size: SampleSize) -> Option<[f32; 4]> {
    let x = (position[0].clamp(0.0, 1.0) * (image.width.saturating_sub(1)) as f64).round() as i32;
    let y = (position[1].clamp(0.0, 1.0) * (image.height.saturating_sub(1)) as f64).round() as i32;
    let mut premul = [0.0_f64; 3];
    let mut alpha = 0.0_f64;
    let mut count = 0_u32;
    for yy in (y - size.radius())..=(y + size.radius()) {
        for xx in (x - size.radius())..=(x + size.radius()) {
            if xx < 0 || yy < 0 || xx >= image.width as i32 || yy >= image.height as i32 {
                continue;
            }
            let pixel = image.pixels[yy as usize * image.width as usize + xx as usize];
            let a = pixel[3] as f64;
            for channel in 0..3 {
                premul[channel] += pixel[channel] as f64 * a;
            }
            alpha += a;
            count += 1;
        }
    }
    if alpha <= f64::EPSILON || count == 0 {
        return None;
    }
    Some([
        (premul[0] / alpha) as f32,
        (premul[1] / alpha) as f32,
        (premul[2] / alpha) as f32,
        (alpha / count as f64) as f32,
    ])
}

pub fn add_site_at(
    state: &mut VoronoiState,
    source: &PixelImage,
    position: [f64; 2],
) -> Option<u64> {
    let color = sample_color(source, position, SampleSize::ThreeByThree)?;
    let site_id = state.next_site_id;
    state.next_site_id += 1;
    state.sites.push(VoronoiSite {
        id: site_id,
        order: site_id,
        source_color: color,
        target_color: [color[0], color[1], color[2]],
        influence: 0.0,
        locked: false,
        position: Some(position),
        size: SampleSize::ThreeByThree,
    });
    Some(site_id)
}

pub fn reattach_site(
    state: &mut VoronoiState,
    source: &PixelImage,
    site_id: u64,
    position: [f64; 2],
) -> bool {
    let Some(site) = state.site(site_id) else {
        return false;
    };
    if site.locked {
        return false;
    }
    let size = site.size;
    let Some(color) = sample_color(source, position, size) else {
        return false;
    };
    state.set_source(site_id, color, Some(position))
}

pub fn auto_initialize(proxy: &PixelImage, source: &PixelImage) -> VoronoiState {
    let mut colors = BTreeMap::<(u32, u32, u32), (usize, [f32; 3], f64)>::new();
    for (index, pixel) in proxy.pixels.iter().enumerate() {
        if pixel[3] <= 0.0 {
            continue;
        }
        let key = (pixel[0].to_bits(), pixel[1].to_bits(), pixel[2].to_bits());
        let entry = colors
            .entry(key)
            .or_insert((index, [pixel[0], pixel[1], pixel[2]], 0.0));
        entry.0 = entry.0.min(index);
        entry.2 += pixel[3] as f64;
    }
    let candidates: Vec<_> = colors
        .into_values()
        .map(|(index, rgb, alpha)| (index, linear_rgb_to_oklab(rgb), alpha))
        .collect();
    let mut state = VoronoiState::default();
    if candidates.is_empty() {
        return state;
    }
    let total_alpha: f64 = candidates.iter().map(|candidate| candidate.2).sum();
    let mut global = [0.0_f64; 3];
    for &(_, lab, alpha) in &candidates {
        for component in 0..3 {
            global[component] += lab[component] * alpha;
        }
    }
    for component in &mut global {
        *component /= total_alpha;
    }
    let first = candidates
        .iter()
        .min_by(|a, b| {
            distance2(a.1, global)
                .total_cmp(&distance2(b.1, global))
                .then_with(|| a.0.cmp(&b.0))
        })
        .expect("candidates are nonempty")
        .1;
    let mut centers = vec![first];
    while centers.len() < 8.min(candidates.len()) {
        let next = candidates
            .iter()
            .map(|&(index, lab, alpha)| {
                let nearest = centers
                    .iter()
                    .map(|&center| distance2(lab, center))
                    .fold(f64::INFINITY, f64::min);
                (index, nearest * alpha.sqrt())
            })
            .max_by(|a, b| a.1.total_cmp(&b.1).then_with(|| b.0.cmp(&a.0)));
        let Some((index, d2)) = next else { break };
        if d2 < DISTINCT_D2 {
            break;
        }
        centers.push(
            candidates
                .iter()
                .find(|(candidate, _, _)| *candidate == index)
                .unwrap()
                .1,
        );
    }

    let mut assignments = vec![0usize; candidates.len()];
    let mut support = vec![0.0_f64; centers.len()];
    for _ in 0..12 {
        support.fill(0.0);
        let mut sums = vec![[0.0_f64; 3]; centers.len()];
        for (candidate_index, &(_, lab, alpha)) in candidates.iter().enumerate() {
            let cluster = centers
                .iter()
                .enumerate()
                .min_by(|(ia, a), (ib, b)| {
                    distance2(lab, **a)
                        .total_cmp(&distance2(lab, **b))
                        .then_with(|| ia.cmp(ib))
                })
                .map(|(index, _)| index)
                .unwrap_or(0);
            assignments[candidate_index] = cluster;
            support[cluster] += alpha;
            for component in 0..3 {
                sums[cluster][component] += lab[component] * alpha;
            }
        }
        for (index, center) in centers.iter_mut().enumerate() {
            if support[index] > 0.0 {
                for component in 0..3 {
                    center[component] = sums[index][component] / support[index];
                }
            }
        }
    }

    // A cluster must own more than 0.5% of visible alpha. For nontrivial images,
    // at least two fully opaque proxy pixels are required as an additional floor.
    let minimum_support = if total_alpha > 64.0 {
        (total_alpha * 0.005).max(2.0)
    } else {
        0.0
    };
    let mut retained: Vec<_> = support
        .iter()
        .enumerate()
        .filter(|(_, value)| **value > minimum_support)
        .map(|(index, value)| (index, *value))
        .collect();
    retained.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    retained.truncate(4);
    retained.sort_by_key(|(index, _)| *index);
    let mut accepted = Vec::<[f64; 3]>::new();
    for (cluster, _) in retained {
        let center = centers[cluster];
        if accepted
            .iter()
            .any(|&other| distance2(center, other) < DISTINCT_D2)
        {
            continue;
        }
        let Some((index, _)) = proxy
            .pixels
            .iter()
            .enumerate()
            .filter(|(_, pixel)| pixel[3] > 0.0)
            .min_by(|(index_a, a), (index_b, b)| {
                let da = distance2(linear_rgb_to_oklab([a[0], a[1], a[2]]), center);
                let db = distance2(linear_rgb_to_oklab([b[0], b[1], b[2]]), center);
                da.total_cmp(&db)
                    .then_with(|| {
                        local_same_color_support(proxy, *index_b)
                            .total_cmp(&local_same_color_support(proxy, *index_a))
                    })
                    .then_with(|| index_a.cmp(index_b))
            })
        else {
            continue;
        };
        let x = index % proxy.width as usize;
        let y = index / proxy.width as usize;
        let position = [
            if proxy.width > 1 {
                x as f64 / (proxy.width - 1) as f64
            } else {
                0.0
            },
            if proxy.height > 1 {
                y as f64 / (proxy.height - 1) as f64
            } else {
                0.0
            },
        ];
        if add_site_at(&mut state, source, position).is_some() {
            accepted.push(center);
        }
    }
    state
}

fn local_same_color_support(image: &PixelImage, index: usize) -> f64 {
    let x = index % image.width as usize;
    let y = index / image.width as usize;
    let target = image.pixels[index];
    let key = (
        target[0].to_bits(),
        target[1].to_bits(),
        target[2].to_bits(),
    );
    let mut support = 0.0;
    for yy in y.saturating_sub(1)..=(y + 1).min(image.height as usize - 1) {
        for xx in x.saturating_sub(1)..=(x + 1).min(image.width as usize - 1) {
            let pixel = image.pixels[yy * image.width as usize + xx];
            if (pixel[0].to_bits(), pixel[1].to_bits(), pixel[2].to_bits()) == key {
                support += pixel[3] as f64;
            }
        }
    }
    support
}
