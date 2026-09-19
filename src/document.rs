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
    pub preprocessing: Preprocessing,
    pub steps: Vec<ProcessingStep>,
    pub voronoi: VoronoiState,
}

/// Source preparation shared by preview and full-resolution export.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preprocessing {
    /// Gaussian blur sigma in evaluation-input pixels. Zero disables smoothing.
    pub input_smoothing: f32,
}

impl Preprocessing {
    pub fn validate(&self) -> Result<(), String> {
        if !self.input_smoothing.is_finite() || !(0.0..=10.0).contains(&self.input_smoothing) {
            return Err("Smooth source must be a finite value from 0 to 10".into());
        }
        Ok(())
    }
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
    pub transition: TransitionProfile,
}

/// Symmetric boundary transition, independent of the Source matching metric.
/// Width is `end - start`; defaulted options preserve existing v6/v3 hard files.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitionProfile {
    pub start: f32,
    pub midpoint: f32,
    pub end: f32,
    #[serde(default, skip_serializing_if = "BlendSpace::is_default")]
    pub blend_space: BlendSpace,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlendSpace {
    #[default]
    Oklab,
    LinearRgb,
}

impl BlendSpace {
    fn is_default(&self) -> bool { *self == Self::default() }
}

impl TransitionProfile {
    pub const HARD: Self = Self {
        start: 0.5,
        midpoint: 0.5,
        end: 0.5,
        blend_space: BlendSpace::Oklab,
    };

    pub fn width(self) -> f32 { self.end - self.start }

    /// A width edit preserves blend options; admission validates rather than clamps.
    pub fn with_width(self, width: f32) -> Self {
        Self { start: (1.0 - width) * 0.5, midpoint: 0.5, end: (1.0 + width) * 0.5, ..self }
    }

    pub fn validate(self) -> Result<(), String> {
        let values = [self.start, self.midpoint, self.end];
        if values
            .iter()
            .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        {
            return Err(
                "Voronoi transition values must be finite and normalized from 0 to 1".into(),
            );
        }
        if self.start > self.midpoint || self.midpoint > self.end {
            return Err(
                "Voronoi transition values must be ordered start <= midpoint <= end".into(),
            );
        }
        if self.midpoint != 0.5 || (self.start + self.end - 1.0).abs() > 1.0e-6 {
            return Err(
                "Voronoi transition must be symmetric around midpoint 0.5"
                    .into(),
            );
        }
        Ok(())
    }
}

impl Default for TransitionProfile {
    fn default() -> Self {
        Self::HARD
    }
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
            transition: TransitionProfile::default(),
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

    /// Update the matching color and attachment without changing the authored Target.
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

    /// Resample the Source footprint while preserving the authored Target.
    pub fn set_size(&mut self, id: u64, size: SampleSize, color: [f32; 4]) -> bool {
        let Some(site) = self.site_mut(id) else {
            return false;
        };
        if site.locked {
            return false;
        }
        site.size = size;
        site.source_color = color;
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
        self.transition.validate()?;
        let mut ids = std::collections::BTreeSet::new();
        let mut orders = std::collections::BTreeSet::new();
        for site in &self.sites {
            if site.id == 0 || !ids.insert(site.id) {
                return Err("Voronoi site IDs must be unique and non-zero".into());
            }
            if !orders.insert(site.order) {
                return Err("Voronoi site order values must be unique".into());
            }
            validate_site_values(site)?;
        }
        if self.next_site_id == 0 || self.sites.iter().any(|site| site.id >= self.next_site_id) {
            return Err("Voronoi next site ID must exceed every existing site ID".into());
        }
        Ok(())
    }

    /// Validate only the state consumed by pixel evaluation.
    ///
    /// Site identity, order uniqueness, and the next-ID allocator are editor and persistence
    /// invariants, so rendering remains independent of that bookkeeping.
    pub(crate) fn validate_for_processing(&self) -> Result<(), String> {
        self.transition.validate()?;
        for site in &self.sites {
            validate_site_values(site)?;
        }
        Ok(())
    }
}

fn validate_site_values(site: &VoronoiSite) -> Result<(), String> {
    if site
        .source_color
        .iter()
        .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
        || site
            .target_color
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
    {
        return Err("Voronoi Source and Target colors must be finite values from 0 to 1".into());
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
    Ok(())
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
    Oklch,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComponentSet {
    Hue,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum ComponentOperation {
    RotateHue { degrees: f32 },
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
            preprocessing: Preprocessing::default(),
            steps: vec![ProcessingStep {
                working_space: WorkingColorSpace::Oklch,
                components: ComponentSet::Hue,
                operation: ComponentOperation::RotateHue { degrees: 0.0 },
                pass_through: PassThroughPolicy::PreserveUnselectedComponents,
                alpha: AlphaPolicy::PassThroughStraight,
                conversion_back: ConversionBack::KeepLinearSrgbF32,
            }],
            voronoi: VoronoiState::default(),
        }
    }
}

impl Recipe {
    /// Validate the complete authoritative processing recipe at an admission boundary.
    pub fn validate(&self) -> Result<(), String> {
        self.preprocessing.validate()?;
        self.voronoi.validate()?;
        self.validate_steps()
    }

    /// Validate the portion of a recipe consumed by pixel evaluation.
    pub(crate) fn validate_for_processing(&self) -> Result<(), String> {
        self.preprocessing.validate()?;
        self.voronoi.validate_for_processing()?;
        self.validate_steps()
    }

    fn validate_steps(&self) -> Result<(), String> {
        for (index, step) in self.steps.iter().enumerate() {
            let ComponentOperation::RotateHue { degrees } = step.operation;
            if !degrees.is_finite() {
                return Err(format!(
                    "processing step {} has a non-finite hue rotation",
                    index + 1
                ));
            }
            if step.working_space != WorkingColorSpace::Oklch
                || step.components != ComponentSet::Hue
                || step.pass_through != PassThroughPolicy::PreserveUnselectedComponents
                || step.alpha != AlphaPolicy::PassThroughStraight
                || step.conversion_back != ConversionBack::KeepLinearSrgbF32
            {
                return Err(format!(
                    "processing step {} is not a supported OKLCH hue rotation",
                    index + 1
                ));
            }
        }
        Ok(())
    }

    pub fn hue_degrees(&self) -> f32 {
        self.steps
            .iter()
            .map(|step| match step.operation {
                ComponentOperation::RotateHue { degrees } => degrees,
            })
            .next()
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
