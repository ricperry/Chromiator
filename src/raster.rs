use std::io::{BufReader, Cursor};
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use image::{AnimationDecoder, DynamicImage, ImageDecoder, ImageFormat, ImageReader};

use crate::document::{PixelImage, ProfileInterpretation, SourceInterpretation};
use crate::processing::srgb_to_linear;

pub struct DecodedRaster {
    pub pixels: PixelImage,
    pub interpretation: SourceInterpretation,
}

pub fn decode(bytes: &[u8], _path_hint: Option<&Path>) -> Result<DecodedRaster> {
    // Trust recognized raster magic before considering textual formats. Compressed image
    // payloads can contain arbitrary byte sequences, including the literal bytes `<svg`.
    let format = match image::guess_format(bytes) {
        Ok(format) => format,
        Err(_) if is_svg_text_prefix(bytes) => {
            bail!("SVG is intentionally unsupported; open a raster image")
        }
        Err(error) => return Err(error).context("unrecognized raster format"),
    };
    if !matches!(
        format,
        ImageFormat::Png
            | ImageFormat::Jpeg
            | ImageFormat::Tiff
            | ImageFormat::WebP
            | ImageFormat::Bmp
            | ImageFormat::Gif
    ) {
        bail!("unsupported raster format: {format:?}")
    }
    if format == ImageFormat::Gif {
        let decoder = image::codecs::gif::GifDecoder::new(BufReader::new(Cursor::new(bytes)))?;
        if decoder.into_frames().take(2).count() > 1 {
            bail!("animated GIF is unsupported; provide a still raster")
        }
    }

    let mut reader = ImageReader::new(Cursor::new(bytes));
    reader.set_format(format);
    let mut decoder = reader.into_decoder()?;
    let has_profile = decoder.icc_profile()?.is_some();
    let image = DynamicImage::from_decoder(decoder)?;
    let width = image.width();
    let height = image.height();
    let layout = format!("{:?} expanded to straight RGBA", image.color());
    let rgba = image.into_rgba32f();
    let pixels = rgba
        .pixels()
        .map(|p| {
            [
                srgb_to_linear(p.0[0].clamp(0.0, 1.0)),
                srgb_to_linear(p.0[1].clamp(0.0, 1.0)),
                srgb_to_linear(p.0[2].clamp(0.0, 1.0)),
                p.0[3].clamp(0.0, 1.0),
            ]
        })
        .collect();
    let pixels = PixelImage::new(width, height, pixels).map_err(anyhow::Error::msg)?;
    Ok(DecodedRaster {
        pixels,
        interpretation: SourceInterpretation {
            original_format: format
                .extensions_str()
                .first()
                .copied()
                .unwrap_or("raster")
                .to_uppercase(),
            profile: if has_profile {
                ProfileInterpretation::EmbeddedProfileNotConverted
            } else {
                ProfileInterpretation::UntaggedAssumedSrgb
            },
            expanded_layout: layout,
            animation_policy: if format == ImageFormat::Gif {
                "verified single-frame GIF".into()
            } else {
                "not animated".into()
            },
        },
    })
}

fn is_svg_text_prefix(bytes: &[u8]) -> bool {
    const PREFIX_LIMIT: usize = 8 * 1024;
    let prefix = &bytes[..bytes.len().min(PREFIX_LIMIT)];
    let valid_length = match std::str::from_utf8(prefix) {
        Ok(_) => prefix.len(),
        Err(error) => error.valid_up_to(),
    };
    let Ok(text) = std::str::from_utf8(&prefix[..valid_length]) else {
        return false;
    };
    let lowercase = text
        .strip_prefix('\u{feff}')
        .unwrap_or(text)
        .trim_start()
        .to_ascii_lowercase();
    let mut remainder = lowercase.as_str();

    for _ in 0..3 {
        remainder = remainder.trim_start();
        if let Some(after) = remainder.strip_prefix("<svg") {
            return after.chars().next().is_none_or(|character| {
                character.is_ascii_whitespace() || matches!(character, '>' | '/')
            });
        }
        if remainder.starts_with("<?xml") {
            let Some(end) = remainder.find("?>") else {
                return false;
            };
            remainder = &remainder[end + 2..];
            continue;
        }
        if remainder.starts_with("<!doctype") {
            let Some(end) = remainder.find('>') else {
                return false;
            };
            remainder = &remainder[end + 1..];
            continue;
        }
        return false;
    }
    false
}

pub fn decode_file(path: &Path) -> Result<(Arc<[u8]>, DecodedRaster)> {
    let bytes =
        std::fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let decoded = decode(&bytes, Some(path))?;
    Ok((bytes.into(), decoded))
}
