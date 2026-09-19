use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::document::{PixelImage, Recipe};
use crate::processing::{linear_to_srgb, process, process_cancellable_with_progress};
use crate::scheduler::JobToken;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormat {
    Png8,
    Png16,
    OpenExr32Float,
}

impl ExportFormat {
    pub fn description(self) -> &'static str {
        match self {
            Self::Png8 => "PNG — 8-bit integer per channel, encoded sRGB",
            Self::Png16 => "PNG — 16-bit integer per channel, encoded sRGB",
            Self::OpenExr32Float => "OpenEXR — 32-bit float per channel, linear sRGB",
        }
    }
}

pub fn export(path: &Path, image: &PixelImage, format: ExportFormat) -> Result<()> {
    atomic_write(path, |temporary| match format {
        ExportFormat::Png8 => write_png8(temporary, image),
        ExportFormat::Png16 => write_png16(temporary, image),
        ExportFormat::OpenExr32Float => write_exr(temporary, image),
    })
}

/// Reprocess the authoritative full-resolution source immediately before export.
pub fn export_recipe(
    path: &Path,
    source: &PixelImage,
    recipe: &Recipe,
    format: ExportFormat,
) -> Result<()> {
    export(path, &process(source, recipe), format)
}

pub fn atomic_write(path: &Path, write: impl FnOnce(&Path) -> Result<()>) -> Result<()> {
    atomic_write_checked(path, write, || true)
}

pub fn atomic_write_checked(
    path: &Path,
    write: impl FnOnce(&Path) -> Result<()>,
    should_commit: impl FnOnce() -> bool,
) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut temporary = tempfile::Builder::new()
        .prefix(".chromiator-")
        .tempfile_in(parent)
        .with_context(|| format!("cannot create temporary output in {}", parent.display()))?;
    write(temporary.path())?;
    temporary.as_file_mut().flush()?;
    temporary.as_file().sync_all()?;
    if !should_commit() {
        bail!("operation cancelled before destination replacement");
    }
    temporary
        .persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("cannot atomically replace {}", path.display()))?;
    Ok(())
}

pub fn export_recipe_cancellable(
    path: &Path,
    source: &PixelImage,
    recipe: &Recipe,
    format: ExportFormat,
    token: &JobToken,
    mut progress: impl FnMut(f64, &'static str),
) -> Result<()> {
    if !token.is_current() {
        bail!("export cancelled");
    }
    progress(0.0, "Processing full-resolution rows…");
    let image = process_cancellable_with_progress(
        source,
        recipe,
        token.generation(),
        token.current(),
        |fraction| progress(fraction * 0.72, "Processing full-resolution rows…"),
    )
    .ok_or_else(|| anyhow::anyhow!("export cancelled"))?;
    progress(0.74, "Encoding image…");
    atomic_write_checked(
        path,
        |temporary| match format {
            ExportFormat::Png8 => write_png8(temporary, &image),
            ExportFormat::Png16 => write_png16(temporary, &image),
            ExportFormat::OpenExr32Float => write_exr(temporary, &image),
        },
        || {
            progress(0.94, "Atomically replacing destination…");
            token.is_current()
        },
    )?;
    progress(1.0, "Export complete");
    Ok(())
}

fn write_png8(path: &Path, image: &PixelImage) -> Result<()> {
    let file = BufWriter::new(File::create(path)?);
    let mut encoder = png::Encoder::new(file, image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    let mut writer = encoder.write_header()?;
    let mut data = Vec::with_capacity(image.pixels.len() * 4);
    for pixel in image.pixels.iter() {
        data.extend(pixel.iter().enumerate().map(|(index, value)| {
            let encoded = if index == 3 {
                value.clamp(0.0, 1.0)
            } else {
                linear_to_srgb(value.clamp(0.0, 1.0))
            };
            (encoded * 255.0).round() as u8
        }));
    }
    writer.write_image_data(&data)?;
    Ok(())
}

fn write_png16(path: &Path, image: &PixelImage) -> Result<()> {
    let file = BufWriter::new(File::create(path)?);
    let mut encoder = png::Encoder::new(file, image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Sixteen);
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    let mut writer = encoder.write_header()?;
    let mut data = Vec::with_capacity(image.pixels.len() * 8);
    for pixel in image.pixels.iter() {
        for (index, value) in pixel.iter().enumerate() {
            let encoded = if index == 3 {
                value.clamp(0.0, 1.0)
            } else {
                linear_to_srgb(value.clamp(0.0, 1.0))
            };
            data.extend_from_slice(&((encoded * 65535.0).round() as u16).to_be_bytes());
        }
    }
    writer.write_image_data(&data)?;
    Ok(())
}

fn write_exr(path: &Path, image: &PixelImage) -> Result<()> {
    exr::prelude::write_rgba_file(path, image.width as usize, image.height as usize, |x, y| {
        let [r, g, b, a] = image.pixels[y * image.width as usize + x];
        (r, g, b, a)
    })
    .context("OpenEXR 32-bit float export failed")
}

pub fn infer_format(path: &Path) -> Result<ExportFormat> {
    match path.extension().and_then(|value| value.to_str()) {
        Some("png") => Ok(ExportFormat::Png16),
        Some("exr") => Ok(ExportFormat::OpenExr32Float),
        _ => bail!("choose .png or .exr; no precision fallback was performed"),
    }
}
