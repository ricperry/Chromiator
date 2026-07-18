use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// Authoritative straight-alpha, linear-sRGB RGBA f32 pixels. Cloning is O(1).
#[derive(Clone, Debug, PartialEq)]
pub struct PixelImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<[[f32; 4]]>,
}

impl PixelImage {
    pub fn new(width: u32, height: u32, pixels: Vec<[f32; 4]>) -> Result<Self, String> {
        if pixels.len() != width as usize * height as usize {
            return Err("pixel count does not match dimensions".into());
        }
        Ok(Self {
            width,
            height,
            pixels: pixels.into(),
        })
    }

    pub fn shares_storage_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.pixels, &other.pixels)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    pub steps: Vec<ProcessingStep>,
    pub active_method: Method,
    pub threshold: ThresholdState,
    pub voronoi: VoronoiState,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThresholdSpace {
    #[default]
    Rgb,
    Hsv,
}

/// A creator-facing Threshold edit target.  This is deliberately semantic rather than a
/// dropdown index so Link controls can change the available targets without changing meaning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThresholdEditTarget {
    Red,
    Green,
    Blue,
    LinkedRgb,
    Hue,
    Saturation,
    Value,
    LinkedSaturationValue,
}

impl ThresholdEditTarget {
    pub fn label(self) -> &'static str {
        match self {
            Self::Red => "Red",
            Self::Green => "Green",
            Self::Blue => "Blue",
            Self::LinkedRgb => "RGB linked",
            Self::Hue => "Hue",
            Self::Saturation => "Saturation",
            Self::Value => "Value",
            Self::LinkedSaturationValue => "S + V linked",
        }
    }

    pub fn is_hue(self) -> bool {
        self == Self::Hue
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThresholdEncoding {
    #[default]
    EncodedSrgb,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinkPolicy {
    #[default]
    Linked,
    Independent,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentQuantizer {
    pub enabled: bool,
    pub boundaries: Vec<f32>,
    pub outputs: Vec<f32>,
}

impl ComponentQuantizer {
    /// Equal-width scalar input bands with linearly spaced endpoint output levels.
    pub fn automatic_scalar(bands: usize) -> Self {
        let bands = bands.clamp(2, 32);
        Self {
            enabled: true,
            boundaries: (1..bands)
                .map(|index| index as f32 / bands as f32)
                .collect(),
            outputs: (0..bands)
                .map(|index| index as f32 / (bands - 1) as f32)
                .collect(),
        }
    }

    /// Equal circular input bands whose output levels sit at the center of each interval.
    pub fn automatic_circular(bands: usize) -> Self {
        Self::evenly_spaced(bands)
    }

    pub fn evenly_spaced(bands: usize) -> Self {
        let bands = bands.clamp(2, 32);
        Self {
            enabled: true,
            boundaries: (1..bands)
                .map(|index| index as f32 / bands as f32)
                .collect(),
            outputs: (0..bands)
                .map(|index| (index as f32 + 0.5) / bands as f32)
                .collect(),
        }
    }

    pub fn validate(&self, name: &str) -> Result<(), String> {
        let bands = self.outputs.len();
        if !(2..=32).contains(&bands) {
            return Err(format!("{name}: Bands must be between 2 and 32"));
        }
        if self.boundaries.len() + 1 != bands {
            return Err(format!(
                "{name}: Outputs must contain exactly one more value than Boundaries"
            ));
        }
        if self
            .boundaries
            .iter()
            .chain(&self.outputs)
            .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        {
            return Err(format!(
                "{name}: Boundaries and Outputs must be finite values from 0 to 1"
            ));
        }
        if self.boundaries.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(format!("{name}: Boundaries must be strictly ordered"));
        }
        Ok(())
    }

    pub fn quantize(&self, value: f32) -> f32 {
        let band = self
            .boundaries
            .iter()
            .position(|boundary| value <= *boundary)
            .unwrap_or(self.boundaries.len());
        self.outputs[band]
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RgbThresholdState {
    pub link: LinkPolicy,
    pub encoding: ThresholdEncoding,
    pub locks: [bool; 3],
    pub components: [ComponentQuantizer; 3],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HsvThresholdState {
    pub sv_link: LinkPolicy,
    pub hue_origin_degrees: f32,
    pub locks: [bool; 3],
    pub hue: ComponentQuantizer,
    pub saturation: ComponentQuantizer,
    pub value: ComponentQuantizer,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThresholdState {
    pub active_space: ThresholdSpace,
    pub rgb_state: RgbThresholdState,
    pub hsv_state: HsvThresholdState,
    pub alpha_policy: AlphaPolicy,
    /// Global pre-engine smoothing shared by Threshold and Voronoi.
    pub input_smoothing: f32,
}

impl Default for ThresholdState {
    fn default() -> Self {
        let quantizer = ComponentQuantizer::evenly_spaced(3);
        Self {
            active_space: ThresholdSpace::Rgb,
            rgb_state: RgbThresholdState {
                link: LinkPolicy::Linked,
                encoding: ThresholdEncoding::EncodedSrgb,
                locks: [false; 3],
                components: [quantizer.clone(), quantizer.clone(), quantizer.clone()],
            },
            hsv_state: HsvThresholdState {
                sv_link: LinkPolicy::Linked,
                hue_origin_degrees: 0.0,
                locks: [false; 3],
                hue: quantizer.clone(),
                saturation: quantizer.clone(),
                value: quantizer,
            },
            alpha_policy: AlphaPolicy::PassThroughStraight,
            input_smoothing: 0.0,
        }
    }
}

impl ThresholdState {
    pub fn edit_targets(&self) -> Vec<ThresholdEditTarget> {
        match (self.active_space, self.active_link()) {
            (ThresholdSpace::Rgb, LinkPolicy::Linked) => vec![
                ThresholdEditTarget::LinkedRgb,
                ThresholdEditTarget::Red,
                ThresholdEditTarget::Green,
                ThresholdEditTarget::Blue,
            ],
            (ThresholdSpace::Rgb, LinkPolicy::Independent) => vec![
                ThresholdEditTarget::Red,
                ThresholdEditTarget::Green,
                ThresholdEditTarget::Blue,
            ],
            (ThresholdSpace::Hsv, LinkPolicy::Linked) => vec![
                ThresholdEditTarget::Hue,
                ThresholdEditTarget::LinkedSaturationValue,
                ThresholdEditTarget::Saturation,
                ThresholdEditTarget::Value,
            ],
            (ThresholdSpace::Hsv, LinkPolicy::Independent) => vec![
                ThresholdEditTarget::Hue,
                ThresholdEditTarget::Saturation,
                ThresholdEditTarget::Value,
            ],
        }
    }

    pub fn active_link(&self) -> LinkPolicy {
        match self.active_space {
            ThresholdSpace::Rgb => self.rgb_state.link,
            ThresholdSpace::Hsv => self.hsv_state.sv_link,
        }
    }

    pub fn reconcile_edit_target(&self, previous: ThresholdEditTarget) -> ThresholdEditTarget {
        let targets = self.edit_targets();
        if targets.contains(&previous) {
            return previous;
        }
        match previous {
            ThresholdEditTarget::Red | ThresholdEditTarget::Green | ThresholdEditTarget::Blue => {
                targets.iter().copied().find(|target| {
                    matches!(
                        target,
                        ThresholdEditTarget::Red
                            | ThresholdEditTarget::Green
                            | ThresholdEditTarget::Blue
                    )
                })
            }
            ThresholdEditTarget::Saturation | ThresholdEditTarget::Value => targets
                .iter()
                .copied()
                .find(|target| *target == ThresholdEditTarget::Saturation),
            ThresholdEditTarget::LinkedSaturationValue => targets
                .iter()
                .copied()
                .find(|target| *target == ThresholdEditTarget::Saturation),
            ThresholdEditTarget::LinkedRgb => targets
                .iter()
                .copied()
                .find(|target| *target == ThresholdEditTarget::Red),
            ThresholdEditTarget::Hue => None,
        }
        .unwrap_or(targets[0])
    }

    pub fn edit_quantizer(&self, target: ThresholdEditTarget) -> ComponentQuantizer {
        match target {
            ThresholdEditTarget::Red | ThresholdEditTarget::LinkedRgb => {
                self.rgb_state.components[0].clone()
            }
            ThresholdEditTarget::Green => self.rgb_state.components[1].clone(),
            ThresholdEditTarget::Blue => self.rgb_state.components[2].clone(),
            ThresholdEditTarget::Hue => self.hsv_state.hue.clone(),
            ThresholdEditTarget::Saturation | ThresholdEditTarget::LinkedSaturationValue => {
                self.hsv_state.saturation.clone()
            }
            ThresholdEditTarget::Value => self.hsv_state.value.clone(),
        }
    }

    pub fn target_index(target: ThresholdEditTarget) -> usize {
        match target {
            ThresholdEditTarget::Red
            | ThresholdEditTarget::Hue
            | ThresholdEditTarget::LinkedRgb => 0,
            ThresholdEditTarget::Green
            | ThresholdEditTarget::Saturation
            | ThresholdEditTarget::LinkedSaturationValue => 1,
            ThresholdEditTarget::Blue | ThresholdEditTarget::Value => 2,
        }
    }

    pub fn is_locked(&self, target: ThresholdEditTarget) -> bool {
        let index = Self::target_index(target);
        match target {
            ThresholdEditTarget::Red
            | ThresholdEditTarget::Green
            | ThresholdEditTarget::Blue
            | ThresholdEditTarget::LinkedRgb => self.rgb_state.locks[index],
            _ => self.hsv_state.locks[index],
        }
    }

    pub fn set_locked(&mut self, target: ThresholdEditTarget, locked: bool) -> bool {
        let index = Self::target_index(target);
        let value = match target {
            ThresholdEditTarget::Red
            | ThresholdEditTarget::Green
            | ThresholdEditTarget::Blue
            | ThresholdEditTarget::LinkedRgb => &mut self.rgb_state.locks[index],
            _ => &mut self.hsv_state.locks[index],
        };
        if *value == locked {
            false
        } else {
            *value = locked;
            true
        }
    }

    pub fn set_hue_origin_degrees(&mut self, degrees: f32) -> bool {
        if self.hsv_state.locks[0] || !degrees.is_finite() {
            return false;
        }
        let canonical = degrees.rem_euclid(360.0);
        if self.hsv_state.hue_origin_degrees == canonical {
            false
        } else {
            self.hsv_state.hue_origin_degrees = canonical;
            true
        }
    }

    pub fn set_edit_quantizer(
        &mut self,
        target: ThresholdEditTarget,
        edited: ComponentQuantizer,
    ) -> bool {
        match target {
            ThresholdEditTarget::Red | ThresholdEditTarget::LinkedRgb => {
                self.set_rgb_component(0, edited)
            }
            ThresholdEditTarget::Green => self.set_rgb_component(1, edited),
            ThresholdEditTarget::Blue => self.set_rgb_component(2, edited),
            ThresholdEditTarget::Hue => self.set_hsv_component(0, edited),
            ThresholdEditTarget::Saturation | ThresholdEditTarget::LinkedSaturationValue => {
                self.set_hsv_component(1, edited)
            }
            ThresholdEditTarget::Value => self.set_hsv_component(2, edited),
        }
    }

    /// Restore the selected component to its automatic baseline without changing its Band count.
    /// Ordinary component setters retain Process and apply the active Link policy while locks
    /// remain authoritative.
    pub fn auto_map(&mut self, target: ThresholdEditTarget) -> bool {
        if self.is_locked(target) {
            return false;
        }
        let bands = self.edit_quantizer(target).outputs.len();
        let automatic = if target.is_hue() {
            ComponentQuantizer::automatic_circular(bands)
        } else {
            ComponentQuantizer::automatic_scalar(bands)
        };
        self.set_edit_quantizer(target, automatic)
    }
    pub fn validate(&self) -> Result<(), String> {
        if !self.input_smoothing.is_finite() || !(0.0..=10.0).contains(&self.input_smoothing) {
            return Err("Smooth source must be a finite value from 0 to 10".into());
        }
        if !self.hsv_state.hue_origin_degrees.is_finite() {
            return Err("Hue origin must be finite".into());
        }
        for (name, component) in ["Red", "Green", "Blue"]
            .into_iter()
            .zip(&self.rgb_state.components)
        {
            component.validate(name)?;
        }
        self.hsv_state.hue.validate("Hue")?;
        self.hsv_state.saturation.validate("Saturation")?;
        self.hsv_state.value.validate("Value")
    }

    /// Propagate an edit according to Link controls, without copying Process/Bypass flags.
    pub fn set_rgb_component(&mut self, index: usize, edited: ComponentQuantizer) -> bool {
        if self.rgb_state.locks[index] {
            return false;
        }
        let before = self.rgb_state.components.clone();
        let enabled = self.rgb_state.components[index].enabled;
        self.rgb_state.components[index] = edited.clone();
        self.rgb_state.components[index].enabled = enabled;
        if self.rgb_state.link == LinkPolicy::Linked {
            for (other_index, component) in self.rgb_state.components.iter_mut().enumerate() {
                if other_index != index && !self.rgb_state.locks[other_index] {
                    let other_enabled = component.enabled;
                    *component = edited.clone();
                    component.enabled = other_enabled;
                }
            }
        }
        self.rgb_state.components != before
    }

    pub fn set_hsv_component(&mut self, index: usize, edited: ComponentQuantizer) -> bool {
        if self.hsv_state.locks[index] {
            return false;
        }
        let before = [
            self.hsv_state.hue.clone(),
            self.hsv_state.saturation.clone(),
            self.hsv_state.value.clone(),
        ];
        match index {
            0 => {
                let enabled = self.hsv_state.hue.enabled;
                self.hsv_state.hue = edited;
                self.hsv_state.hue.enabled = enabled;
            }
            1 | 2 => {
                let target = if index == 1 {
                    &mut self.hsv_state.saturation
                } else {
                    &mut self.hsv_state.value
                };
                let enabled = target.enabled;
                *target = edited.clone();
                target.enabled = enabled;
                if self.hsv_state.sv_link == LinkPolicy::Linked
                    && !self.hsv_state.locks[if index == 1 { 2 } else { 1 }]
                {
                    let other = if index == 1 {
                        &mut self.hsv_state.value
                    } else {
                        &mut self.hsv_state.saturation
                    };
                    let other_enabled = other.enabled;
                    *other = edited;
                    other.enabled = other_enabled;
                }
            }
            _ => unreachable!("HSV component index"),
        }
        before
            != [
                self.hsv_state.hue.clone(),
                self.hsv_state.saturation.clone(),
                self.hsv_state.value.clone(),
            ]
    }

    /// Copy the selected mapping to its synchronization peers. Process and lock state remain
    /// independent. Returns `(copied, locked_destinations_skipped)`.
    pub fn sync_from(&mut self, target: ThresholdEditTarget) -> (usize, usize) {
        let source = self.edit_quantizer(target);
        let source_index = Self::target_index(target);
        let mut copied = 0;
        let mut skipped = 0;
        match target {
            ThresholdEditTarget::Red | ThresholdEditTarget::Green | ThresholdEditTarget::Blue => {
                for index in 0..3 {
                    if index == source_index {
                        continue;
                    }
                    if self.rgb_state.locks[index] {
                        skipped += 1;
                        continue;
                    }
                    let enabled = self.rgb_state.components[index].enabled;
                    self.rgb_state.components[index] = source.clone();
                    self.rgb_state.components[index].enabled = enabled;
                    copied += 1;
                }
            }
            ThresholdEditTarget::Saturation | ThresholdEditTarget::Value => {
                let index = if source_index == 1 { 2 } else { 1 };
                if self.hsv_state.locks[index] {
                    skipped = 1;
                } else {
                    let destination = if index == 1 {
                        &mut self.hsv_state.saturation
                    } else {
                        &mut self.hsv_state.value
                    };
                    let enabled = destination.enabled;
                    *destination = source;
                    destination.enabled = enabled;
                    copied = 1;
                }
            }
            ThresholdEditTarget::Hue
            | ThresholdEditTarget::LinkedRgb
            | ThresholdEditTarget::LinkedSaturationValue => {}
        }
        (copied, skipped)
    }
}

impl ComponentQuantizer {
    /// Split a band without changing the transfer function: both halves retain the old output.
    pub fn split_band(&mut self, band: usize) -> bool {
        if self.outputs.len() >= 32 || band >= self.outputs.len() {
            return false;
        }
        let lower = if band == 0 {
            0.0
        } else {
            self.boundaries[band - 1]
        };
        let upper = self.boundaries.get(band).copied().unwrap_or(1.0);
        self.boundaries.insert(band, (lower + upper) / 2.0);
        self.outputs.insert(band + 1, self.outputs[band]);
        true
    }

    pub fn split_widest_band(&mut self) -> bool {
        let band = (0..self.outputs.len())
            .max_by(|left, right| {
                self.band_width(*left)
                    .total_cmp(&self.band_width(*right))
                    .then_with(|| right.cmp(left))
            })
            .unwrap_or(0);
        self.split_band(band)
    }

    fn band_width(&self, band: usize) -> f32 {
        let lower = if band == 0 {
            0.0
        } else {
            self.boundaries[band - 1]
        };
        let upper = self.boundaries.get(band).copied().unwrap_or(1.0);
        upper - lower
    }

    /// Remove a boundary and merge its adjacent bands. The larger interval's output wins; an
    /// exact width tie keeps the lower interval's output.
    pub fn remove_boundary(&mut self, boundary: usize) -> bool {
        if self.outputs.len() <= 2 || boundary >= self.boundaries.len() {
            return false;
        }
        let lower_width = self.band_width(boundary);
        let upper_width = self.band_width(boundary + 1);
        self.boundaries.remove(boundary);
        if lower_width >= upper_width {
            self.outputs.remove(boundary + 1);
        } else {
            self.outputs.remove(boundary);
        }
        true
    }

    /// Merge the adjacent pair with the smallest output error, then the narrowest combined
    /// interval, then the lowest boundary index.
    pub fn remove_least_error_boundary(&mut self) -> bool {
        let Some(boundary) = (0..self.boundaries.len()).min_by(|left, right| {
            let left_error = (self.outputs[*left] - self.outputs[*left + 1]).abs();
            let right_error = (self.outputs[*right] - self.outputs[*right + 1]).abs();
            left_error
                .total_cmp(&right_error)
                .then_with(|| {
                    (self.band_width(*left) + self.band_width(*left + 1))
                        .total_cmp(&(self.band_width(*right) + self.band_width(*right + 1)))
                })
                .then_with(|| left.cmp(right))
        }) else {
            return false;
        };
        self.remove_boundary(boundary)
    }

    pub fn resize_preserving_mapping(
        &mut self,
        bands: usize,
        selected_band: Option<usize>,
        selected_boundary: Option<usize>,
    ) -> bool {
        let bands = bands.clamp(2, 32);
        if bands == self.outputs.len() {
            return false;
        }
        let mut changed = false;
        let mut first = true;
        while self.outputs.len() < bands {
            changed |= if first {
                selected_band
                    .filter(|band| *band < self.outputs.len())
                    .is_some_and(|band| self.split_band(band))
                    || self.split_widest_band()
            } else {
                self.split_widest_band()
            };
            first = false;
        }
        first = true;
        while self.outputs.len() > bands {
            changed |= if first {
                selected_boundary
                    .filter(|boundary| *boundary < self.boundaries.len())
                    .is_some_and(|boundary| self.remove_boundary(boundary))
                    || self.remove_least_error_boundary()
            } else {
                self.remove_least_error_boundary()
            };
            first = false;
        }
        changed
    }
}

const THRESHOLD_HANDLE_GAP: f32 = 0.000_001;

/// Clamp a Boundary drag between its neighbors while keeping the model strictly ordered.
pub fn clamp_threshold_boundary(boundaries: &[f32], index: usize, candidate: f32) -> f32 {
    let lower = index
        .checked_sub(1)
        .and_then(|previous| boundaries.get(previous))
        .map_or(0.0, |value| value + THRESHOLD_HANDLE_GAP);
    let upper = boundaries
        .get(index + 1)
        .map_or(1.0, |value| value - THRESHOLD_HANDLE_GAP);
    candidate.clamp(lower, upper)
}

pub fn threshold_display_value(value: f32, hue_degrees: bool) -> f64 {
    f64::from(value) * if hue_degrees { 360.0 } else { 1.0 }
}

pub fn threshold_normalized_value(value: f64, hue_degrees: bool) -> f32 {
    (value / if hue_degrees { 360.0 } else { 1.0 }) as f32
}

/// Small pure coordinator used by pointer and keyboard editing. Motion is draft-only; a
/// completed adjustment emits at most one commit, and an unchanged gesture emits none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ThresholdEditGesture {
    changed: bool,
    commits: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ThresholdEditTransaction {
    snapshot: ThresholdState,
    pre_dirty: bool,
    completed_edits: usize,
}

impl ThresholdEditTransaction {
    pub fn new(state: &ThresholdState, dirty: bool) -> Self {
        Self {
            snapshot: state.clone(),
            pre_dirty: dirty,
            completed_edits: 0,
        }
    }

    pub fn record_completed_edit(&mut self) {
        self.completed_edits += 1;
    }

    /// Returns whether a restoring preview is needed.
    pub fn cancel(self, state: &mut ThresholdState, dirty: &mut bool) -> bool {
        if self.completed_edits == 0 {
            return false;
        }
        *state = self.snapshot;
        *dirty = self.pre_dirty;
        true
    }
}

impl ThresholdEditGesture {
    pub fn motion(&mut self, changed: bool) {
        self.changed |= changed;
    }

    pub fn complete(&mut self) -> bool {
        if !std::mem::take(&mut self.changed) {
            return false;
        }
        self.commits += 1;
        true
    }

    pub fn commits(self) -> usize {
        self.commits
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Method {
    #[default]
    Voronoi,
    Thresholds,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SampleSize {
    Point,
    #[default]
    ThreeByThree,
    FiveByFive,
}

impl SampleSize {
    pub fn radius(self) -> i32 {
        match self {
            Self::Point => 0,
            Self::ThreeByThree => 1,
            Self::FiveByFive => 2,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoronoiSite {
    pub id: u64,
    pub order: u64,
    pub source_color: [f32; 4],
    pub target_color: [f32; 3],
    pub influence: f64,
    pub locked: bool,
    pub position: Option<[f64; 2]>,
    pub size: SampleSize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoronoiState {
    pub sites: Vec<VoronoiSite>,
    pub next_site_id: u64,
    pub matching: VoronoiMatching,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum VoronoiMatching {
    #[default]
    Perceptual,
    Okhsl,
    Rgb,
    Hsv,
}

impl Default for VoronoiState {
    fn default() -> Self {
        Self {
            sites: Vec::new(),
            next_site_id: 1,
            matching: VoronoiMatching::Perceptual,
        }
    }
}

impl VoronoiState {
    pub fn site(&self, id: u64) -> Option<&VoronoiSite> {
        self.sites.iter().find(|site| site.id == id)
    }

    pub fn site_mut(&mut self, id: u64) -> Option<&mut VoronoiSite> {
        self.sites.iter_mut().find(|site| site.id == id)
    }

    pub fn set_source(
        &mut self,
        id: u64,
        source_color: [f32; 4],
        position: Option<[f64; 2]>,
    ) -> bool {
        let Some(site) = self.site_mut(id) else {
            return false;
        };
        if site.locked {
            return false;
        }
        site.source_color = source_color;
        site.target_color.copy_from_slice(&source_color[..3]);
        site.position = position;
        true
    }

    pub fn set_target(&mut self, id: u64, target: [f32; 3]) -> bool {
        let Some(site) = self.site_mut(id) else {
            return false;
        };
        if site.locked {
            return false;
        }
        site.target_color = target;
        true
    }

    pub fn set_influence(&mut self, id: u64, influence: f64) -> bool {
        let Some(site) = self.site_mut(id) else {
            return false;
        };
        if site.locked {
            return false;
        }
        site.influence = influence;
        true
    }

    pub fn set_size(&mut self, id: u64, size: SampleSize, color: [f32; 4]) -> bool {
        let Some(site) = self.site_mut(id) else {
            return false;
        };
        if site.locked {
            return false;
        }
        site.size = size;
        site.source_color = color;
        site.target_color.copy_from_slice(&color[..3]);
        true
    }

    pub fn set_locked(&mut self, id: u64, locked: bool) -> bool {
        let Some(site) = self.site_mut(id) else {
            return false;
        };
        if site.locked == locked {
            return false;
        }
        site.locked = locked;
        true
    }

    pub fn delete_site(&mut self, id: u64) -> bool {
        let before = self.sites.len();
        self.sites.retain(|site| site.id != id || site.locked);
        self.sites.len() != before
    }

    pub fn validate(&self) -> Result<(), String> {
        let mut ids = std::collections::BTreeSet::new();
        let mut orders = std::collections::BTreeSet::new();
        for site in &self.sites {
            if site.id == 0 || !ids.insert(site.id) {
                return Err("Voronoi site IDs must be unique and non-zero".into());
            }
            if !orders.insert(site.order) {
                return Err("Voronoi site order values must be unique".into());
            }
            if site
                .source_color
                .iter()
                .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
                || site
                    .target_color
                    .iter()
                    .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
            {
                return Err(
                    "Voronoi Source and Target colors must be finite values from 0 to 1".into(),
                );
            }
            if !site.influence.is_finite() || !(-4.0..=4.0).contains(&site.influence) {
                return Err("Voronoi Influence must be finite and between -4 and 4".into());
            }
            if let Some(position) = site.position
                && position
                    .iter()
                    .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
            {
                return Err("Voronoi site positions must be finite normalized coordinates".into());
            }
        }
        if self.next_site_id == 0 || self.sites.iter().any(|site| site.id >= self.next_site_id) {
            return Err("Voronoi next site ID must exceed every existing site ID".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessingStep {
    pub working_space: WorkingColorSpace,
    pub components: ComponentSet,
    pub operation: ComponentOperation,
    pub pass_through: PassThroughPolicy,
    pub alpha: AlphaPolicy,
    pub conversion_back: ConversionBack,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkingColorSpace {
    LinearSrgb,
    Oklch,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComponentSet {
    AllRgb,
    SelectedRgb(Vec<ColorComponent>),
    Hue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColorComponent {
    Red,
    Green,
    Blue,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum ComponentOperation {
    ThreeBandQuantize {
        linked: bool,
        thresholds: [f32; 2],
        outputs: [f32; 3],
    },
    RotateHue {
        degrees: f32,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PassThroughPolicy {
    PreserveUnselectedComponents,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlphaPolicy {
    PassThroughStraight,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConversionBack {
    KeepLinearSrgbF32,
}

impl Default for Recipe {
    fn default() -> Self {
        Self {
            steps: vec![
                ProcessingStep {
                    working_space: WorkingColorSpace::LinearSrgb,
                    components: ComponentSet::AllRgb,
                    operation: ComponentOperation::ThreeBandQuantize {
                        linked: true,
                        thresholds: [1.0 / 3.0, 2.0 / 3.0],
                        outputs: [1.0 / 6.0, 1.0 / 2.0, 5.0 / 6.0],
                    },
                    pass_through: PassThroughPolicy::PreserveUnselectedComponents,
                    alpha: AlphaPolicy::PassThroughStraight,
                    conversion_back: ConversionBack::KeepLinearSrgbF32,
                },
                ProcessingStep {
                    working_space: WorkingColorSpace::Oklch,
                    components: ComponentSet::Hue,
                    operation: ComponentOperation::RotateHue { degrees: 0.0 },
                    pass_through: PassThroughPolicy::PreserveUnselectedComponents,
                    alpha: AlphaPolicy::PassThroughStraight,
                    conversion_back: ConversionBack::KeepLinearSrgbF32,
                },
            ],
            active_method: Method::Voronoi,
            threshold: ThresholdState::default(),
            voronoi: VoronoiState::default(),
        }
    }
}

impl Recipe {
    pub fn thresholds_default() -> Self {
        Self {
            active_method: Method::Thresholds,
            ..Self::default()
        }
    }
    pub fn quantize(&self) -> ([f32; 2], [f32; 3]) {
        let component = &self.threshold.rgb_state.components[0];
        if component.boundaries.len() == 2 && component.outputs.len() == 3 {
            (
                [component.boundaries[0], component.boundaries[1]],
                [
                    component.outputs[0],
                    component.outputs[1],
                    component.outputs[2],
                ],
            )
        } else {
            ([1.0 / 3.0, 2.0 / 3.0], [1.0 / 6.0, 0.5, 5.0 / 6.0])
        }
    }
    pub fn set_quantize(&mut self, thresholds: [f32; 2], outputs: [f32; 3]) {
        let enabled = self.threshold.rgb_state.components[0].enabled;
        self.threshold.set_rgb_component(
            0,
            ComponentQuantizer {
                enabled,
                boundaries: thresholds.to_vec(),
                outputs: outputs.to_vec(),
            },
        );
        if let Some(step) = self
            .steps
            .iter_mut()
            .find(|step| matches!(step.operation, ComponentOperation::ThreeBandQuantize { .. }))
        {
            step.operation = ComponentOperation::ThreeBandQuantize {
                linked: true,
                thresholds,
                outputs,
            };
        }
    }
    pub fn hue_degrees(&self) -> f32 {
        self.steps
            .iter()
            .find_map(|step| match step.operation {
                ComponentOperation::RotateHue { degrees } => Some(degrees),
                _ => None,
            })
            .unwrap_or(0.0)
    }
    pub fn set_hue_degrees(&mut self, degrees: f32) {
        if let Some(step) = self
            .steps
            .iter_mut()
            .find(|step| matches!(step.operation, ComponentOperation::RotateHue { .. }))
        {
            step.operation = ComponentOperation::RotateHue { degrees };
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProfileInterpretation {
    UntaggedAssumedSrgb,
    EmbeddedProfileConvertedToSrgb,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceInterpretation {
    pub original_format: String,
    pub profile: ProfileInterpretation,
    pub expanded_layout: String,
    pub animation_policy: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportDefaults {
    pub format: String,
    pub depth: String,
}
impl Default for ExportDefaults {
    fn default() -> Self {
        Self {
            format: "PNG".into(),
            depth: "16-bit integer per channel".into(),
        }
    }
}

impl ExportDefaults {
    pub fn validate(&self) -> Result<(), String> {
        match (self.format.as_str(), self.depth.as_str()) {
            ("PNG", "8-bit integer per channel")
            | ("PNG", "16-bit integer per channel")
            | ("OpenEXR", "32-bit float per channel") => Ok(()),
            _ => Err(format!(
                "unsupported export defaults: {} / {}",
                self.format, self.depth
            )),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Document {
    pub source_name: String,
    pub source_bytes: Arc<[u8]>,
    pub source: PixelImage,
    pub interpretation: SourceInterpretation,
    pub recipe: Recipe,
    pub export_defaults: ExportDefaults,
    pub dirty: bool,
}

impl Document {
    pub fn set_hue(&mut self, degrees: f32) {
        if (self.recipe.hue_degrees() - degrees).abs() > f32::EPSILON {
            self.recipe.set_hue_degrees(degrees);
            self.dirty = true;
        }
    }
}
