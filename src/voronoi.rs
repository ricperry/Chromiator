use crate::document::{PixelImage, SampleSize, VoronoiSite, VoronoiState};

const DISTINCT_D2: f64 = 0.0025;
const MAX_AUTO_SITES: usize = 6;
const OKLAB_GRID_STEP: f64 = 0.025;
const OKLAB_L_MIN: f64 = 0.0;
const OKLAB_AB_MIN: f64 = -0.5;
const OKLAB_GRID_SIZE: usize = 40;
const OKLAB_GRID_CELLS: usize = OKLAB_GRID_SIZE * OKLAB_GRID_SIZE * OKLAB_GRID_SIZE;
const DENSITY_CORE_RADIUS_CELLS: isize = 2;
const DENSITY_SHELL_RADIUS_CELLS: isize = 4;
const DENSITY_PROMINENCE: f64 = 1.5;

#[derive(Clone, Copy)]
struct VoxelAccum {
    alpha: f64,
    lab_sum: [f64; 3],
    first_index: usize,
}

impl Default for VoxelAccum {
    fn default() -> Self {
        Self {
            alpha: 0.0,
            lab_sum: [0.0; 3],
            first_index: usize::MAX,
        }
    }
}

struct ColorCandidate {
    first_index: usize,
    sample_index: usize,
    lab: [f64; 3],
    alpha: f64,
    sample_distance2: f64,
}

struct ClusterFamily {
    first_cluster: usize,
    clusters: Vec<usize>,
    center: [f64; 3],
    support: f64,
}

#[derive(Clone)]
struct PaletteCandidate {
    stable_order: (usize, u8),
    sample_index: usize,
    swatch: [f32; 4],
    swatch_lab: [f64; 3],
    support: f64,
}

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

fn oklab_voxel_index(lab: [f64; 3]) -> usize {
    let coordinate = |value: f64, minimum: f64| {
        ((value - minimum) / OKLAB_GRID_STEP)
            .floor()
            .clamp(0.0, (OKLAB_GRID_SIZE - 1) as f64) as usize
    };
    let l = coordinate(lab[0], OKLAB_L_MIN);
    let a = coordinate(lab[1], OKLAB_AB_MIN);
    let b = coordinate(lab[2], OKLAB_AB_MIN);
    (l * OKLAB_GRID_SIZE + a) * OKLAB_GRID_SIZE + b
}

fn oklab_voxel_coordinates(index: usize) -> [usize; 3] {
    let b = index % OKLAB_GRID_SIZE;
    let a = (index / OKLAB_GRID_SIZE) % OKLAB_GRID_SIZE;
    let l = index / (OKLAB_GRID_SIZE * OKLAB_GRID_SIZE);
    [l, a, b]
}

fn oklab_voxel_neighbor(coordinates: [usize; 3], offset: [isize; 3]) -> Option<usize> {
    let mut neighbor = [0usize; 3];
    for axis in 0..3 {
        let value = coordinates[axis] as isize + offset[axis];
        if !(0..OKLAB_GRID_SIZE as isize).contains(&value) {
            return None;
        }
        neighbor[axis] = value as usize;
    }
    Some((neighbor[0] * OKLAB_GRID_SIZE + neighbor[1]) * OKLAB_GRID_SIZE + neighbor[2])
}

fn core_kernel_offsets() -> Vec<([isize; 3], f64)> {
    let mut offsets = Vec::new();
    for l in -DENSITY_CORE_RADIUS_CELLS..=DENSITY_CORE_RADIUS_CELLS {
        for a in -DENSITY_CORE_RADIUS_CELLS..=DENSITY_CORE_RADIUS_CELLS {
            for b in -DENSITY_CORE_RADIUS_CELLS..=DENSITY_CORE_RADIUS_CELLS {
                let radius2 = l * l + a * a + b * b;
                if radius2 <= DENSITY_CORE_RADIUS_CELLS * DENSITY_CORE_RADIUS_CELLS {
                    // A bounded Gaussian kernel with sigma equal to one voxel. Its fixed
                    // normalization includes empty cells and keeps peak density comparable.
                    offsets.push(([l, a, b], (-0.5 * radius2 as f64).exp()));
                }
            }
        }
    }
    offsets
}

fn shell_offsets() -> Vec<[isize; 3]> {
    let mut offsets = Vec::new();
    for l in -DENSITY_SHELL_RADIUS_CELLS..=DENSITY_SHELL_RADIUS_CELLS {
        for a in -DENSITY_SHELL_RADIUS_CELLS..=DENSITY_SHELL_RADIUS_CELLS {
            for b in -DENSITY_SHELL_RADIUS_CELLS..=DENSITY_SHELL_RADIUS_CELLS {
                let radius2 = l * l + a * a + b * b;
                if radius2 > DENSITY_CORE_RADIUS_CELLS * DENSITY_CORE_RADIUS_CELLS
                    && radius2 <= DENSITY_SHELL_RADIUS_CELLS * DENSITY_SHELL_RADIUS_CELLS
                {
                    offsets.push([l, a, b]);
                }
            }
        }
    }
    offsets
}

fn density_equal(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1.0e-12 * a.abs().max(b.abs()).max(1.0)
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

/// Add an explicitly chosen color, optionally attached to a Source-image pixel.
/// New colors start identity-mapped; later Source edits preserve Target.
pub fn add_site_color(
    state: &mut VoronoiState,
    color: [f32; 4],
    position: Option<[f64; 2]>,
) -> u64 {
    let id = state.next_site_id;
    state.next_site_id += 1;
    state.sites.push(VoronoiSite {
        id,
        order: id,
        source_color: color,
        target_color: [color[0], color[1], color[2]],
        influence: 0.0,
        locked: false,
        position,
        size: SampleSize::Point,
    });
    id
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

pub fn auto_initialize(_proxy: &PixelImage, source: &PixelImage) -> VoronoiState {
    // Accumulate the full-resolution image directly into fixed-volume OKLab voxels. The
    // 40³ grid uses 0.025 units per axis over canonical sRGB OKLab bounds; unlike coarse
    // linear-RGB bins, every cell represents the same perceptual volume.
    let mut accum = vec![VoxelAccum::default(); OKLAB_GRID_CELLS];
    for (index, pixel) in source.pixels.iter().enumerate() {
        if pixel[3] <= 0.0 {
            continue;
        }
        let lab = linear_rgb_to_oklab([pixel[0], pixel[1], pixel[2]]);
        let voxel_index = oklab_voxel_index(lab);
        let voxel = &mut accum[voxel_index];
        let alpha = pixel[3] as f64;
        voxel.alpha += alpha;
        voxel.first_index = voxel.first_index.min(index);
        for (sum, component) in voxel.lab_sum.iter_mut().zip(lab) {
            *sum += component * alpha;
        }
    }

    let mut voxel_labs = vec![[0.0_f64; 3]; OKLAB_GRID_CELLS];
    let mut candidate_by_voxel = vec![usize::MAX; OKLAB_GRID_CELLS];
    let mut candidates = Vec::<ColorCandidate>::new();
    for (voxel_index, voxel) in accum.iter().enumerate() {
        if voxel.alpha <= 0.0 {
            continue;
        }
        let lab = voxel.lab_sum.map(|sum| sum / voxel.alpha);
        voxel_labs[voxel_index] = lab;
        candidate_by_voxel[voxel_index] = candidates.len();
        candidates.push(ColorCandidate {
            first_index: voxel.first_index,
            sample_index: voxel.first_index,
            lab,
            alpha: voxel.alpha,
            sample_distance2: f64::INFINITY,
        });
    }
    let mut state = VoronoiState::default();
    if candidates.is_empty() {
        return state;
    }

    // Select a real source pixel nearest each occupied voxel's weighted OKLab center. This
    // second bounded full-source pass replaces the former local 3×3 exact-color tie-break and
    // keeps a one-pixel outlier from representing a much denser voxel.
    for (index, pixel) in source.pixels.iter().enumerate() {
        if pixel[3] <= 0.0 {
            continue;
        }
        let lab = linear_rgb_to_oklab([pixel[0], pixel[1], pixel[2]]);
        let candidate_index = candidate_by_voxel[oklab_voxel_index(lab)];
        let candidate = &mut candidates[candidate_index];
        let sample_distance2 = distance2(lab, candidate.lab);
        let ordering = sample_distance2.total_cmp(&candidate.sample_distance2);
        if ordering.is_lt() || (ordering.is_eq() && index < candidate.sample_index) {
            candidate.sample_index = index;
            candidate.sample_distance2 = sample_distance2;
        }
    }

    let total_alpha: f64 = candidates.iter().map(|candidate| candidate.alpha).sum();
    let minimum_support = if total_alpha > 64.0 { 2.0 } else { 0.0 };
    let mut global = [0.0_f64; 3];
    for candidate in &candidates {
        for (sum, component) in global.iter_mut().zip(candidate.lab) {
            *sum += component * candidate.alpha;
        }
    }
    for component in &mut global {
        *component /= total_alpha;
    }
    let first = candidates
        .iter()
        .min_by(|a, b| {
            distance2(a.lab, global)
                .total_cmp(&distance2(b.lab, global))
                .then_with(|| a.first_index.cmp(&b.first_index))
        })
        .expect("candidates are nonempty")
        .lab;
    let mut centers = vec![first];
    while centers.len() < 8.min(candidates.len()) {
        let next = candidates
            .iter()
            .map(|candidate| {
                let nearest = centers
                    .iter()
                    .map(|&center| distance2(candidate.lab, center))
                    .fold(f64::INFINITY, f64::min);
                (candidate.first_index, nearest * candidate.alpha.sqrt())
            })
            .max_by(|a, b| a.1.total_cmp(&b.1).then_with(|| b.0.cmp(&a.0)));
        let Some((first_index, score)) = next else {
            break;
        };
        if score < DISTINCT_D2 {
            break;
        }
        centers.push(
            candidates
                .iter()
                .find(|candidate| candidate.first_index == first_index)
                .expect("seed index belongs to a candidate")
                .lab,
        );
    }

    let mut assignments = vec![0usize; candidates.len()];
    let mut cluster_support = vec![0.0_f64; centers.len()];
    for _ in 0..12 {
        cluster_support.fill(0.0);
        let mut sums = vec![[0.0_f64; 3]; centers.len()];
        for (candidate_index, candidate) in candidates.iter().enumerate() {
            let cluster = nearest_center(candidate.lab, &centers);
            assignments[candidate_index] = cluster;
            cluster_support[cluster] += candidate.alpha;
            for (sum, component) in sums[cluster].iter_mut().zip(candidate.lab) {
                *sum += component * candidate.alpha;
            }
        }
        for (index, center) in centers.iter_mut().enumerate() {
            if cluster_support[index] > 0.0 {
                for component in 0..3 {
                    center[component] = sums[index][component] / cluster_support[index];
                }
            }
        }
    }
    cluster_support.fill(0.0);
    for (candidate_index, candidate) in candidates.iter().enumerate() {
        let cluster = nearest_center(candidate.lab, &centers);
        assignments[candidate_index] = cluster;
        cluster_support[cluster] += candidate.alpha;
    }

    // Coalesce nearby K-means centers with deterministic complete-link grouping. A whole family
    // must stay within the 0.05 OKLab radius, so a gradual chain cannot merge distinct endpoints.
    let mut families = Vec::<ClusterFamily>::new();
    for cluster in 0..centers.len() {
        let family_index = families.iter().position(|family| {
            family
                .clusters
                .iter()
                .all(|&other| distance2(centers[cluster], centers[other]) < DISTINCT_D2)
        });
        let family_index = family_index.unwrap_or_else(|| {
            families.push(ClusterFamily {
                first_cluster: cluster,
                clusters: Vec::new(),
                center: [0.0; 3],
                support: 0.0,
            });
            families.len() - 1
        });
        families[family_index].clusters.push(cluster);
    }
    for family in &mut families {
        family.support = family
            .clusters
            .iter()
            .map(|&cluster| cluster_support[cluster])
            .sum();
        if family.support > 0.0 {
            for &cluster in &family.clusters {
                for (sum, component) in family.center.iter_mut().zip(centers[cluster]) {
                    *sum += component * cluster_support[cluster] / family.support;
                }
            }
        }
    }

    let family_candidates: Vec<_> = families
        .iter()
        .filter_map(|family| {
            family_palette_candidate(family, &candidates, &assignments, source, minimum_support)
        })
        .collect();
    let Some(anchor_index) = family_candidates
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| {
            a.support
                .total_cmp(&b.support)
                .then_with(|| b.stable_order.cmp(&a.stable_order))
        })
        .map(|(index, _)| index)
    else {
        return state;
    };
    let density_modes = density_mode_candidates(
        &accum,
        &voxel_labs,
        &candidate_by_voxel,
        &candidates,
        source,
        minimum_support,
    );

    // Keep the strongest alpha family as the main-artwork anchor even when it is broad and has
    // little local prominence. Add compact density modes as the remaining choices. If the image
    // has no compact modes (for example, a smooth gradient), use the other coalesced families as
    // a deterministic fallback instead of returning an empty or single-site palette.
    let mut palette_candidates = vec![family_candidates[anchor_index].clone()];
    if density_modes.is_empty() {
        palette_candidates.extend(
            family_candidates
                .into_iter()
                .enumerate()
                .filter_map(|(index, candidate)| (index != anchor_index).then_some(candidate)),
        );
    } else {
        palette_candidates.extend(density_modes);
    }
    let selected = select_palette_candidates(&palette_candidates);
    for palette_index in selected {
        let candidate = &palette_candidates[palette_index];
        let index = candidate.sample_index;
        let pixel = candidate.swatch;
        let x = index % source.width as usize;
        let y = index / source.width as usize;
        let position = [
            if source.width > 1 {
                x as f64 / (source.width - 1) as f64
            } else {
                0.0
            },
            if source.height > 1 {
                y as f64 / (source.height - 1) as f64
            } else {
                0.0
            },
        ];
        let site_id = state.next_site_id;
        state.next_site_id += 1;
        state.sites.push(VoronoiSite {
            id: site_id,
            order: site_id,
            source_color: pixel,
            target_color: [pixel[0], pixel[1], pixel[2]],
            influence: 0.0,
            locked: false,
            position: Some(position),
            size: SampleSize::Point,
        });
    }
    state
}

fn nearest_center(lab: [f64; 3], centers: &[[f64; 3]]) -> usize {
    centers
        .iter()
        .enumerate()
        .min_by(|(ia, a), (ib, b)| {
            distance2(lab, **a)
                .total_cmp(&distance2(lab, **b))
                .then_with(|| ia.cmp(ib))
        })
        .map(|(index, _)| index)
        .unwrap_or(0)
}

fn family_palette_candidate(
    family: &ClusterFamily,
    candidates: &[ColorCandidate],
    assignments: &[usize],
    source: &PixelImage,
    minimum_support: f64,
) -> Option<PaletteCandidate> {
    if family.support <= minimum_support {
        return None;
    }
    let members: Vec<_> = candidates
        .iter()
        .enumerate()
        .filter(|(index, _)| family.clusters.contains(&assignments[*index]))
        .map(|(_, candidate)| candidate)
        .collect();
    let eligible: Vec<_> = members
        .iter()
        .copied()
        .filter(|candidate| candidate.alpha > minimum_support)
        .collect();
    let sample = if eligible.is_empty() {
        members.into_iter().max_by(|a, b| {
            a.alpha
                .total_cmp(&b.alpha)
                .then_with(|| b.first_index.cmp(&a.first_index))
        })
    } else {
        eligible.into_iter().min_by(|a, b| {
            let a_pixel = source.pixels[a.sample_index];
            let b_pixel = source.pixels[b.sample_index];
            let a_lab = linear_rgb_to_oklab([a_pixel[0], a_pixel[1], a_pixel[2]]);
            let b_lab = linear_rgb_to_oklab([b_pixel[0], b_pixel[1], b_pixel[2]]);
            distance2(a_lab, family.center)
                .total_cmp(&distance2(b_lab, family.center))
                .then_with(|| b.alpha.total_cmp(&a.alpha))
                .then_with(|| a.first_index.cmp(&b.first_index))
        })
    }?;
    let sample_index = sample.sample_index;
    let swatch = source.pixels[sample_index];
    Some(PaletteCandidate {
        stable_order: (family.first_cluster, 0),
        sample_index,
        swatch,
        swatch_lab: linear_rgb_to_oklab([swatch[0], swatch[1], swatch[2]]),
        support: family.support,
    })
}

fn density_mode_candidates(
    voxels: &[VoxelAccum],
    voxel_labs: &[[f64; 3]],
    candidate_by_voxel: &[usize],
    candidates: &[ColorCandidate],
    source: &PixelImage,
    minimum_support: f64,
) -> Vec<PaletteCandidate> {
    let core = core_kernel_offsets();
    let shell = shell_offsets();
    let core_weight: f64 = core.iter().map(|(_, weight)| weight).sum();
    let shell_volume = shell.len() as f64;
    let mut core_mass = vec![0.0_f64; OKLAB_GRID_CELLS];
    let mut core_density = vec![0.0_f64; OKLAB_GRID_CELLS];
    let mut shell_density = vec![0.0_f64; OKLAB_GRID_CELLS];
    let mut active = vec![false; OKLAB_GRID_CELLS];

    // Splat each occupied voxel into its bounded neighborhoods. The symmetric kernels make
    // this the same fixed-volume convolution as visiting every possible center, while keeping
    // one-color and small-palette imports proportional to occupied cells rather than 64k cells.
    for (index, voxel) in voxels.iter().enumerate() {
        if voxel.alpha <= 0.0 {
            continue;
        }
        let coordinates = oklab_voxel_coordinates(index);
        for &(offset, weight) in &core {
            if let Some(center) = oklab_voxel_neighbor(coordinates, offset) {
                active[center] = true;
                core_mass[center] += voxel.alpha;
                core_density[center] += voxel.alpha * weight / core_weight;
            }
        }
        for offset in &shell {
            if let Some(center) = oklab_voxel_neighbor(coordinates, *offset) {
                active[center] = true;
                shell_density[center] += voxel.alpha / shell_volume;
            }
        }
    }
    let active_indices: Vec<_> = active
        .iter()
        .enumerate()
        .filter_map(|(index, &is_active)| is_active.then_some(index))
        .collect();

    // Collapse equal-density local-max plateaus before checking prominence. Choose their
    // representative with the greatest surrounding shell support, preventing an arbitrary
    // boundary voxel from appearing denser than the plateau interior.
    let mut adjacent = Vec::<[isize; 3]>::new();
    for l in -1..=1 {
        for a in -1..=1 {
            for b in -1..=1 {
                if l != 0 || a != 0 || b != 0 {
                    adjacent.push([l, a, b]);
                }
            }
        }
    }
    let mut visited = vec![false; OKLAB_GRID_CELLS];
    let mut peaks = Vec::<PaletteCandidate>::new();
    for start in active_indices {
        if visited[start] || core_density[start] <= 0.0 {
            continue;
        }
        let level = core_density[start];
        let mut pending = vec![start];
        let mut plateau = Vec::new();
        visited[start] = true;
        while let Some(index) = pending.pop() {
            plateau.push(index);
            let coordinates = oklab_voxel_coordinates(index);
            for offset in &adjacent {
                let Some(neighbor) = oklab_voxel_neighbor(coordinates, *offset) else {
                    continue;
                };
                if !visited[neighbor]
                    && core_density[neighbor] > 0.0
                    && density_equal(core_density[neighbor], level)
                {
                    visited[neighbor] = true;
                    pending.push(neighbor);
                }
            }
        }

        let mut is_local_maximum = true;
        let mut minimum = [usize::MAX; 3];
        let mut maximum = [0usize; 3];
        for &index in &plateau {
            let coordinates = oklab_voxel_coordinates(index);
            for axis in 0..3 {
                minimum[axis] = minimum[axis].min(coordinates[axis]);
                maximum[axis] = maximum[axis].max(coordinates[axis]);
            }
            for offset in &adjacent {
                if let Some(neighbor) = oklab_voxel_neighbor(coordinates, *offset)
                    && core_density[neighbor] > level
                    && !density_equal(core_density[neighbor], level)
                {
                    is_local_maximum = false;
                    break;
                }
            }
            if !is_local_maximum {
                break;
            }
        }
        if !is_local_maximum {
            continue;
        }
        let plateau_diameter2: f64 = (0..3)
            .map(|axis| ((maximum[axis] - minimum[axis]) as f64 * OKLAB_GRID_STEP).powi(2))
            .sum();
        if plateau_diameter2 > DISTINCT_D2 {
            continue;
        }
        let representative = plateau
            .iter()
            .copied()
            .filter(|&index| core_mass[index] > minimum_support)
            .max_by(|&a, &b| {
                shell_density[a]
                    .total_cmp(&shell_density[b])
                    .then_with(|| core_mass[a].total_cmp(&core_mass[b]))
                    .then_with(|| b.cmp(&a))
            });
        let Some(representative) = representative else {
            continue;
        };
        let core_value = core_density[representative];
        let shell_value = shell_density[representative];
        if shell_value > 0.0 && core_value < shell_value * DENSITY_PROMINENCE {
            continue;
        }

        let coordinates = oklab_voxel_coordinates(representative);
        let mut center_sum = [0.0_f64; 3];
        let mut center_weight = 0.0;
        let mut sample_candidates = Vec::new();
        for &(offset, weight) in &core {
            let Some(neighbor) = oklab_voxel_neighbor(coordinates, offset) else {
                continue;
            };
            let alpha = voxels[neighbor].alpha;
            if alpha <= 0.0 {
                continue;
            }
            let local_weight = alpha * weight;
            center_weight += local_weight;
            for component in 0..3 {
                center_sum[component] += voxel_labs[neighbor][component] * local_weight;
            }
            let candidate_index = candidate_by_voxel[neighbor];
            if candidate_index != usize::MAX {
                sample_candidates.push(candidate_index);
            }
        }
        if center_weight <= 0.0 {
            continue;
        }
        let center = center_sum.map(|sum| sum / center_weight);
        let eligible: Vec<_> = sample_candidates
            .iter()
            .copied()
            .filter(|&index| candidates[index].alpha > minimum_support)
            .collect();
        let sample_index = if eligible.is_empty() {
            sample_candidates.into_iter().max_by(|&a, &b| {
                candidates[a]
                    .alpha
                    .total_cmp(&candidates[b].alpha)
                    .then_with(|| candidates[b].first_index.cmp(&candidates[a].first_index))
            })
        } else {
            eligible.into_iter().min_by(|&a, &b| {
                let a_pixel = source.pixels[candidates[a].sample_index];
                let b_pixel = source.pixels[candidates[b].sample_index];
                let a_lab = linear_rgb_to_oklab([a_pixel[0], a_pixel[1], a_pixel[2]]);
                let b_lab = linear_rgb_to_oklab([b_pixel[0], b_pixel[1], b_pixel[2]]);
                distance2(a_lab, center)
                    .total_cmp(&distance2(b_lab, center))
                    .then_with(|| candidates[b].alpha.total_cmp(&candidates[a].alpha))
                    .then_with(|| candidates[a].first_index.cmp(&candidates[b].first_index))
            })
        };
        let Some(sample_index) = sample_index else {
            continue;
        };
        let selected = &candidates[sample_index];
        let swatch = source.pixels[selected.sample_index];
        peaks.push(PaletteCandidate {
            stable_order: (selected.first_index, 1),
            sample_index: selected.sample_index,
            swatch,
            swatch_lab: linear_rgb_to_oklab([swatch[0], swatch[1], swatch[2]]),
            support: core_value,
        });
    }
    peaks
}

fn select_palette_candidates(candidates: &[PaletteCandidate]) -> Vec<usize> {
    if candidates.is_empty() {
        return Vec::new();
    }
    let mut selected = vec![0usize];
    while selected.len() < MAX_AUTO_SITES {
        let next = candidates
            .iter()
            .enumerate()
            .filter(|(index, _)| !selected.contains(index))
            .map(|(index, candidate)| {
                let nearest = selected
                    .iter()
                    .map(|&selected_index| {
                        distance2(candidate.swatch_lab, candidates[selected_index].swatch_lab)
                    })
                    .fold(f64::INFINITY, f64::min);
                (index, nearest, candidate.support, candidate.stable_order)
            })
            .max_by(|a, b| {
                a.1.total_cmp(&b.1)
                    .then_with(|| a.2.total_cmp(&b.2))
                    .then_with(|| b.3.cmp(&a.3))
            });
        let Some((index, separation, _, _)) = next else {
            break;
        };
        if separation < DISTINCT_D2 {
            break;
        }
        selected.push(index);
    }
    selected
}
