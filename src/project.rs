use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use zip::write::SimpleFileOptions;

use crate::document::{
    AlphaPolicy, ColorComponent, ComponentOperation, ComponentQuantizer, ComponentSet, Document,
    ExportDefaults, LinkPolicy, Recipe, SourceInterpretation, ThresholdEncoding, ThresholdSpace,
};
use crate::export::atomic_write_checked;
use crate::raster;
use crate::scheduler::JobToken;

const PROJECT_VERSION: u32 = 4;

#[derive(Serialize, Deserialize)]
struct Manifest {
    version: u32,
    source_name: String,
    source_entry: String,
    source_interpretation: SourceInterpretation,
    recipe: Recipe,
    export_defaults: ExportDefaults,
}

pub fn save(path: &Path, document: &Document) -> Result<()> {
    save_impl(path, document, None, |_, _| {})
}

pub fn save_cancellable(
    path: &Path,
    document: &Document,
    token: &JobToken,
    progress: impl FnMut(f64, &'static str),
) -> Result<()> {
    save_impl(path, document, Some(token), progress)
}

fn save_impl(
    path: &Path,
    document: &Document,
    token: Option<&JobToken>,
    mut progress: impl FnMut(f64, &'static str),
) -> Result<()> {
    if token.is_some_and(|token| !token.is_current()) {
        bail!("save cancelled");
    }
    document
        .recipe
        .threshold
        .validate()
        .map_err(anyhow::Error::msg)
        .context("cannot save invalid Threshold state")?;
    document
        .recipe
        .voronoi
        .validate()
        .map_err(anyhow::Error::msg)
        .context("cannot save invalid Voronoi state")?;
    progress(0.05, "Serializing project manifest…");
    let manifest = Manifest {
        version: PROJECT_VERSION,
        source_name: document.source_name.clone(),
        source_entry: "source/original".into(),
        source_interpretation: document.interpretation.clone(),
        recipe: document.recipe.clone(),
        export_defaults: document.export_defaults.clone(),
    };
    let json = serde_json::to_vec_pretty(&manifest)?;
    progress(0.25, "Writing embedded source archive…");
    atomic_write_checked(
        path,
        |temporary| {
            let file = File::create(temporary)?;
            let mut archive = zip::ZipWriter::new(file);
            let options =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            archive.start_file("manifest.json", options)?;
            archive.write_all(&json)?;
            archive.start_file(&manifest.source_entry, options)?;
            archive.write_all(document.source_bytes.as_ref())?;
            archive.finish()?.sync_all()?;
            Ok(())
        },
        || {
            progress(0.92, "Atomically replacing project…");
            token.is_none_or(JobToken::is_current)
        },
    )?;
    progress(1.0, "Project saved");
    Ok(())
}

pub fn open(path: &Path) -> Result<Document> {
    let file = File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file)?;
    let (mut manifest, version): (Manifest, u32) = {
        let mut manifest_file = archive.by_name("manifest.json")?;
        let mut json = String::new();
        manifest_file.read_to_string(&mut json)?;
        let mut value: serde_json::Value =
            serde_json::from_str(&json).context("project manifest is not valid JSON")?;
        let version = value
            .get("version")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| anyhow::anyhow!("project manifest has no numeric version"))?
            as u32;
        if matches!(version, 2 | 3) {
            migrate_legacy_voronoi(&mut value)?;
        }
        let manifest = serde_json::from_value(value)
            .with_context(|| format!("project version {version} manifest is malformed"))?;
        (manifest, version)
    };
    match version {
        2 => migrate_v2(&mut manifest.recipe),
        3 | PROJECT_VERSION => {
            manifest.recipe.threshold.validate().map_err(|message| {
                anyhow::anyhow!("invalid Threshold state in project: {message}")
            })?;
        }
        _ => bail!("unsupported project version {version}"),
    }
    manifest
        .recipe
        .voronoi
        .validate()
        .map_err(|message| anyhow::anyhow!("invalid Voronoi state in project: {message}"))?;
    let mut source_bytes = Vec::new();
    archive
        .by_name(&manifest.source_entry)?
        .read_to_end(&mut source_bytes)?;
    let decoded = raster::decode(&source_bytes, None)?;
    Ok(Document {
        source_name: manifest.source_name,
        source_bytes: source_bytes.into(),
        source: decoded.pixels,
        interpretation: manifest.source_interpretation,
        recipe: manifest.recipe,
        export_defaults: manifest.export_defaults,
        dirty: false,
    })
}

fn migrate_legacy_voronoi(manifest: &mut serde_json::Value) -> Result<()> {
    let voronoi = manifest
        .get_mut("recipe")
        .and_then(|recipe| recipe.get_mut("voronoi"))
        .ok_or_else(|| anyhow::anyhow!("legacy project has no Voronoi state"))?;
    // Test/development archives briefly used a legacy manifest version with the
    // current independent-site shape. It needs no group flattening.
    if voronoi.get("sites").is_some() {
        return Ok(());
    }
    let matching = voronoi
        .get("matching")
        .cloned()
        .unwrap_or_else(|| serde_json::json!("Perceptual"));
    let groups = voronoi
        .get("groups")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("legacy project Voronoi groups are malformed"))?;
    let mut sites = Vec::new();
    let mut max_id = 0_u64;
    for group in groups {
        let target = group
            .get("output_color")
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("legacy Color group has no Output color"))?;
        let samples = group
            .get("samples")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("legacy Color group samples are malformed"))?;
        for sample in samples {
            let id = sample
                .get("id")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| anyhow::anyhow!("legacy Source sample has no numeric ID"))?;
            max_id = max_id.max(id);
            sites.push(serde_json::json!({
                "id": id,
                "order": sample.get("order").cloned().unwrap_or_else(|| serde_json::json!(id)),
                "source_color": sample.get("source_color").cloned().ok_or_else(|| anyhow::anyhow!("legacy Source sample has no color"))?,
                "target_color": target.clone(),
                "influence": sample.get("influence").cloned().unwrap_or_else(|| serde_json::json!(0.0)),
                "locked": false,
                "position": sample.get("position").cloned(),
                "size": sample.get("size").cloned().unwrap_or_else(|| serde_json::json!("ThreeByThree")),
            }));
        }
    }
    let legacy_next = voronoi
        .get("next_sample_id")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(1);
    *voronoi = serde_json::json!({
        "sites": sites,
        "next_site_id": legacy_next.max(max_id.saturating_add(1)).max(1),
        "matching": matching,
    });
    Ok(())
}

fn migrate_v2(recipe: &mut Recipe) {
    let (boundaries, outputs, components) = recipe
        .steps
        .iter()
        .find_map(|step| match (&step.operation, &step.components) {
            (
                ComponentOperation::ThreeBandQuantize {
                    thresholds,
                    outputs,
                    ..
                },
                components,
            ) => Some((*thresholds, *outputs, components.clone())),
            _ => None,
        })
        .unwrap_or((
            [1.0 / 3.0, 2.0 / 3.0],
            [1.0 / 6.0, 0.5, 5.0 / 6.0],
            ComponentSet::AllRgb,
        ));
    recipe.threshold.active_space = ThresholdSpace::Rgb;
    recipe.threshold.rgb_state.encoding = ThresholdEncoding::LinearSrgbLegacy;
    recipe.threshold.rgb_state.link = LinkPolicy::Linked;
    for (index, component) in recipe.threshold.rgb_state.components.iter_mut().enumerate() {
        let color = [
            ColorComponent::Red,
            ColorComponent::Green,
            ColorComponent::Blue,
        ][index];
        let enabled = matches!(components, ComponentSet::AllRgb)
            || matches!(&components, ComponentSet::SelectedRgb(selected) if selected.contains(&color));
        *component = ComponentQuantizer {
            enabled,
            boundaries: boundaries.to_vec(),
            outputs: outputs.to_vec(),
        };
    }
    recipe.threshold.alpha_policy = AlphaPolicy::PassThroughStraight;
    recipe.threshold.input_smoothing = 0.0;
}
