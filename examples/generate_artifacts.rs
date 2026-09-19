use std::path::Path;

use anyhow::Result;
use chromiator::document::{Document, ExportDefaults, Recipe};
use chromiator::export::{self, ExportFormat};
use chromiator::{processing, project, raster};

fn main() -> Result<()> {
    let destination = Path::new("tests/artifacts/exports");
    std::fs::create_dir_all(destination)?;
    let source_path = Path::new("archive/webapp/Demo1.png");
    let (bytes, decoded) = raster::decode_file(source_path)?;
    let mut recipe = Recipe::default();
    let proxy = processing::bounded_preview(&decoded.pixels);
    recipe.voronoi = chromiator::voronoi::auto_initialize(&proxy, &decoded.pixels);
    recipe.set_hue_degrees(12.5);
    let result = processing::process(&decoded.pixels, &recipe);
    export::export(
        &destination.join("result-8.png"),
        &result,
        ExportFormat::Png8,
    )?;
    export::export(
        &destination.join("result-16.png"),
        &result,
        ExportFormat::Png16,
    )?;
    export::export(
        &destination.join("result-32f.exr"),
        &result,
        ExportFormat::OpenExr32Float,
    )?;
    let project_path = destination.join("example.chromiator");
    project::save(
        &project_path,
        &Document {
            source_name: "Demo1.png".into(),
            source_bytes: bytes,
            source: decoded.pixels,
            interpretation: decoded.interpretation,
            recipe,
            export_defaults: ExportDefaults::default(),
            dirty: true,
        },
    )?;
    let reopened = project::open(&project_path)?;
    anyhow::ensure!(reopened.recipe.hue_degrees() == 12.5);
    Ok(())
}
