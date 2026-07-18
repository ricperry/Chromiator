use std::io::{BufReader, Cursor};
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use image::{AnimationDecoder, DynamicImage, ImageDecoder, ImageFormat, ImageReader};
use moxcms::{ColorProfile, DataColorSpace, Layout, RenderingIntent, TransformOptions};

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
    let embedded_profile = decoder.icc_profile()?;
    let image = DynamicImage::from_decoder(decoder)?;
    let width = image.width();
    let height = image.height();
    let layout = format!("{:?} expanded to straight RGBA", image.color());
    let rgba = image.into_rgba32f();
    let mut encoded_rgb: Vec<f32> = rgba
        .pixels()
        .flat_map(|pixel| pixel.0[..3].iter().copied())
        .collect();
    if let Some(profile_bytes) = embedded_profile.as_deref() {
        convert_embedded_rgb_to_srgb(&mut encoded_rgb, profile_bytes)?;
    }
    let pixels = rgba
        .pixels()
        .zip(encoded_rgb.chunks_exact(3))
        .map(|(p, rgb)| {
            [
                srgb_to_linear(rgb[0].clamp(0.0, 1.0)),
                srgb_to_linear(rgb[1].clamp(0.0, 1.0)),
                srgb_to_linear(rgb[2].clamp(0.0, 1.0)),
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
            profile: if embedded_profile.is_some() {
                ProfileInterpretation::EmbeddedProfileConvertedToSrgb
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

/// Convert encoded RGB samples from an embedded ICC profile into the app's canonical encoded
/// sRGB working space. Perceptual Voronoi code receives linear sRGB later and never interprets
/// picker coordinates or profile-native channel values as OKLab.
fn convert_embedded_rgb_to_srgb(encoded_rgb: &mut [f32], profile_bytes: &[u8]) -> Result<()> {
    let source = ColorProfile::new_from_slice(profile_bytes)
        .context("embedded ICC profile could not be parsed")?;
    if source.color_space != DataColorSpace::Rgb {
        bail!(
            "embedded ICC profile uses unsupported {:?} source channels; provide an RGB raster",
            source.color_space
        );
    }
    let destination = ColorProfile::new_srgb();
    let options = TransformOptions {
        rendering_intent: RenderingIntent::RelativeColorimetric,
        // Keep the f32 path as precise as the profile permits; the decoded raster can be 16-bit.
        prefer_fixed_point: false,
        ..TransformOptions::default()
    };
    let transform = source
        .create_transform_f32(Layout::Rgb, &destination, Layout::Rgb, options)
        .context("embedded ICC profile cannot be transformed to sRGB")?;
    let input = encoded_rgb.to_vec();
    transform
        .transform(&input, encoded_rgb)
        .context("embedded ICC conversion to sRGB failed")
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
