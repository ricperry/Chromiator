use anyhow::Result;

use crate::document::{Document, ExportDefaults, Recipe};
use crate::processing::bounded_preview;
use crate::{raster, voronoi};

pub const SPECTRUM_BYTES: &[u8] = include_bytes!("../assets/examples/SpectrumBreakpoint.png");

pub fn spectrum_document() -> Result<Document> {
    let decoded = raster::decode(SPECTRUM_BYTES, None)?;
    let source = decoded.pixels;
    let proxy = bounded_preview(&source);
    let recipe = Recipe {
        voronoi: voronoi::auto_initialize(&proxy, &source),
        ..Recipe::default()
    };
    Ok(Document {
        source_name: "Spectrum Example.png".into(),
        source_bytes: SPECTRUM_BYTES.into(),
        source,
        interpretation: decoded.interpretation,
        recipe,
        export_defaults: ExportDefaults::default(),
        dirty: false,
    })
}
