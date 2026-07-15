use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::document::{
    ComponentOperation, Document, Method, ProcessingStep, Recipe, ThresholdState, VoronoiState,
};
use crate::export::atomic_write_checked;

pub const PRESET_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PresetProcessing {
    pub active_method: Method,
    pub threshold: ThresholdState,
    pub voronoi: VoronoiState,
    pub steps: Vec<ProcessingStep>,
}

impl From<&Recipe> for PresetProcessing {
    fn from(recipe: &Recipe) -> Self {
        let mut voronoi = recipe.voronoi.clone();
        detach_sites(&mut voronoi);
        Self {
            active_method: recipe.active_method,
            threshold: recipe.threshold.clone(),
            voronoi,
            steps: recipe.steps.clone(),
        }
    }
}

impl PresetProcessing {
    pub fn recipe(&self) -> Recipe {
        let mut recipe = Recipe {
            active_method: self.active_method,
            threshold: self.threshold.clone(),
            voronoi: self.voronoi.clone(),
            steps: self.steps.clone(),
        };
        detach_sites(&mut recipe.voronoi);
        recipe
    }

    pub fn validate(&self) -> Result<()> {
        self.threshold
            .validate()
            .map_err(anyhow::Error::msg)
            .context("invalid Threshold state")?;
        self.voronoi
            .validate()
            .map_err(anyhow::Error::msg)
            .context("invalid Voronoi state")?;
        validate_steps(&self.steps)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preset {
    pub version: u32,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub processing: PresetProcessing,
}

impl Preset {
    pub fn new(name: &str, description: Option<String>, recipe: &Recipe) -> Result<Self> {
        let name = validate_name(name)?;
        let preset = Self {
            version: PRESET_VERSION,
            name,
            description,
            processing: PresetProcessing::from(recipe),
        };
        preset.validate()?;
        Ok(preset)
    }

    pub fn validate(&self) -> Result<()> {
        if self.version != PRESET_VERSION {
            bail!("unsupported preset version {}", self.version);
        }
        if validate_name(&self.name)? != self.name {
            bail!("preset name cannot have surrounding whitespace");
        }
        self.processing.validate()
    }

    pub fn recipe(&self) -> Recipe {
        self.processing.recipe()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresetDiagnostic {
    pub path: PathBuf,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PresetEntry {
    pub path: PathBuf,
    pub preset: Preset,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PresetScan {
    pub entries: Vec<PresetEntry>,
    pub diagnostics: Vec<PresetDiagnostic>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresetStore {
    directory: PathBuf,
}

impl PresetStore {
    pub fn system() -> Self {
        Self::at(glib::user_data_dir().join("threshiator/presets"))
    }

    pub fn at(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn prepare_directory(&self) -> Result<&Path> {
        fs::create_dir_all(&self.directory)
            .with_context(|| format!("cannot create preset folder {}", self.directory.display()))?;
        Ok(&self.directory)
    }

    pub fn scan(&self) -> Result<PresetScan> {
        self.prepare_directory()?;
        let mut paths = fs::read_dir(&self.directory)?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .collect::<Vec<_>>();
        paths.sort();
        let mut scan = PresetScan::default();
        let mut candidates = Vec::new();
        for path in paths {
            match read_preset(&path) {
                Ok(preset) => candidates.push(PresetEntry { path, preset }),
                Err(error) => scan.diagnostics.push(PresetDiagnostic {
                    path,
                    reason: format!("{error:#}"),
                }),
            }
        }
        let mut by_name = BTreeMap::<String, Vec<PresetEntry>>::new();
        for entry in candidates {
            by_name
                .entry(entry.preset.name.clone())
                .or_default()
                .push(entry);
        }
        for (name, mut entries) in by_name {
            entries.sort_by(|a, b| a.path.cmp(&b.path));
            if entries.len() == 1 {
                scan.entries.push(entries.remove(0));
                continue;
            }
            let files = entries
                .iter()
                .map(|entry| {
                    entry
                        .path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                })
                .collect::<Vec<_>>()
                .join(", ");
            for entry in entries {
                scan.diagnostics.push(PresetDiagnostic {
                    path: entry.path,
                    reason: format!("duplicate preset name {name:?}; conflicting files: {files}"),
                });
            }
        }
        scan.entries.sort_by(|a, b| {
            a.preset
                .name
                .to_lowercase()
                .cmp(&b.preset.name.to_lowercase())
                .then_with(|| a.preset.name.cmp(&b.preset.name))
                .then_with(|| a.path.file_name().cmp(&b.path.file_name()))
        });
        scan.diagnostics.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(scan)
    }

    pub fn save(&self, preset: &Preset, overwrite: bool) -> Result<PathBuf> {
        self.save_checked(preset, overwrite, || true)
    }

    pub fn save_checked(
        &self,
        preset: &Preset,
        overwrite: bool,
        should_commit: impl FnOnce() -> bool,
    ) -> Result<PathBuf> {
        self.prepare_directory()?;
        let mut canonical = preset.clone();
        detach_sites(&mut canonical.processing.voronoi);
        canonical.name = validate_name(&canonical.name)?;
        canonical.validate()?;
        let path = self.directory.join(filename_for_name(&canonical.name));
        let scan = self.scan()?;
        if scan.diagnostics.iter().any(|diagnostic| {
            diagnostic
                .reason
                .starts_with(&format!("duplicate preset name {:?};", canonical.name))
        }) {
            bail!(
                "preset name {:?} is ambiguous across multiple files; remove or rename the conflicting files first",
                canonical.name
            );
        }
        let duplicate = scan
            .entries
            .iter()
            .find(|entry| entry.preset.name == canonical.name);
        if path.exists() {
            let existing = read_preset(&path).with_context(|| {
                format!(
                    "cannot safely replace preset file {}; it may be a hash collision",
                    path.display()
                )
            })?;
            if existing.name != canonical.name {
                bail!(
                    "preset filename collision: {} belongs to preset {:?}, not {:?}",
                    path.display(),
                    existing.name,
                    canonical.name
                );
            }
        }
        if !overwrite && (path.exists() || duplicate.is_some()) {
            bail!("a preset named {:?} already exists", canonical.name);
        }
        if overwrite
            && let Some(entry) = duplicate
            && entry.path != path
        {
            bail!(
                "preset name conflicts with unexpected file {}",
                entry.path.display()
            );
        }
        let json = serde_json::to_vec_pretty(&canonical)?;
        atomic_write_checked(
            &path,
            |temporary| {
                let mut file = fs::File::create(temporary)?;
                file.write_all(&json)?;
                Ok(())
            },
            should_commit,
        )?;
        Ok(path)
    }
}

pub fn apply_to_document(document: &mut Document, preset: &Preset) -> Result<bool> {
    let mut canonical = preset.clone();
    detach_sites(&mut canonical.processing.voronoi);
    canonical.validate()?;
    let recipe = canonical.recipe();
    if document.recipe == recipe {
        return Ok(false);
    }
    document.recipe = recipe;
    document.dirty = true;
    Ok(true)
}

pub fn validate_name(name: &str) -> Result<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        bail!("preset name cannot be empty");
    }
    if trimmed.chars().count() > 128 {
        bail!("preset name is longer than 128 characters");
    }
    if trimmed.chars().any(char::is_control) {
        bail!("preset name cannot contain control characters");
    }
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains("..") {
        bail!("preset name cannot contain path separators or '..'");
    }
    Ok(trimmed.to_owned())
}

fn filename_for_name(name: &str) -> String {
    // FNV-1a is deliberately used only as a stable opaque filename, never as identity.
    // The embedded validated name remains authoritative and collisions are rejected above.
    let mut digest = 0xcbf2_9ce4_8422_2325_u64;
    for byte in name.as_bytes() {
        digest ^= u64::from(*byte);
        digest = digest.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("preset-{digest:016x}.json")
}

fn read_preset(path: &Path) -> Result<Preset> {
    let bytes = fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    let mut preset: Preset = serde_json::from_slice(&bytes)
        .with_context(|| format!("{} is not a valid preset JSON file", path.display()))?;
    detach_sites(&mut preset.processing.voronoi);
    preset.validate()?;
    Ok(preset)
}

fn detach_sites(voronoi: &mut VoronoiState) {
    for site in &mut voronoi.sites {
        site.position = None;
    }
}

fn validate_steps(steps: &[ProcessingStep]) -> Result<()> {
    for (index, step) in steps.iter().enumerate() {
        match &step.operation {
            ComponentOperation::ThreeBandQuantize {
                thresholds,
                outputs,
                ..
            } => {
                if thresholds
                    .iter()
                    .chain(outputs)
                    .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
                    || thresholds[0] >= thresholds[1]
                {
                    bail!("processing step {} has invalid quantizer values", index + 1);
                }
            }
            ComponentOperation::RotateHue { degrees } if !degrees.is_finite() => {
                bail!(
                    "processing step {} has a non-finite hue rotation",
                    index + 1
                );
            }
            ComponentOperation::RotateHue { .. } => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filename_digest_is_stable_and_embedded_name_collision_is_rejected() {
        assert_eq!(
            filename_for_name("Audit Look"),
            "preset-b44c3c737dbb4e9b.json"
        );
        let directory = tempfile::tempdir().unwrap();
        let store = PresetStore::at(directory.path());
        let intended = Preset::new("Intended", None, &Recipe::default()).unwrap();
        let occupant = Preset::new("Different", None, &Recipe::default()).unwrap();
        let collision_path = directory.path().join(filename_for_name(&intended.name));
        fs::write(
            collision_path,
            serde_json::to_vec_pretty(&occupant).unwrap(),
        )
        .unwrap();
        let error = store.save(&intended, false).unwrap_err().to_string();
        assert!(error.contains("filename collision"), "{error}");
        assert!(error.contains("Different"), "{error}");
        assert!(error.contains("Intended"), "{error}");
    }
}
