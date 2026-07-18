use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use zip::write::SimpleFileOptions;

use crate::document::{Document, ExportDefaults, Recipe, SourceInterpretation};
use crate::export::atomic_write_checked;
use crate::raster;
use crate::scheduler::JobToken;

const PROJECT_VERSION: u32 = 5;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    document
        .export_defaults
        .validate()
        .map_err(anyhow::Error::msg)
        .context("cannot save invalid export defaults")?;
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
    let mut entry_names = (0..archive.len())
        .map(|index| archive.by_index(index).map(|entry| entry.name().to_owned()))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    entry_names.sort();
    if entry_names != ["manifest.json", "source/original"] {
        bail!("project archive must contain exactly manifest.json and source/original");
    }
    let manifest: Manifest = {
        let mut manifest_file = archive.by_name("manifest.json")?;
        let mut json = String::new();
        manifest_file.read_to_string(&mut json)?;
        let value: serde_json::Value =
            serde_json::from_str(&json).context("project manifest is not valid JSON")?;
        let version = value
            .get("version")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| anyhow::anyhow!("project manifest has no numeric version"))?
            as u32;
        if version != PROJECT_VERSION {
            bail!(
                "unsupported pre-release project version {version}; this build opens only version {PROJECT_VERSION}"
            );
        }
        serde_json::from_value(value)
            .with_context(|| format!("project version {version} manifest is malformed"))?
    };
    if manifest.source_entry != "source/original" {
        bail!("project source entry must be source/original");
    }
    manifest
        .recipe
        .threshold
        .validate()
        .map_err(|message| anyhow::anyhow!("invalid Threshold state in project: {message}"))?;
    manifest
        .recipe
        .voronoi
        .validate()
        .map_err(|message| anyhow::anyhow!("invalid Voronoi state in project: {message}"))?;
    manifest
        .export_defaults
        .validate()
        .map_err(|message| anyhow::anyhow!("invalid export defaults in project: {message}"))?;
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
