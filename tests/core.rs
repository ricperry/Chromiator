use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use image::{ImageBuffer, ImageFormat, Rgba};
use threshiator::color::{
    ColorModel, DraftColor, PickerTransaction, PlaneKey, adjust_oklab_plane, encoded_to_hsl,
    encoded_to_hsv, hsl_to_encoded, hsv_to_encoded, in_srgb_gamut, linear_to_oklab,
    max_oklab_chroma, oklab_plane, oklab_plane_coords, oklab_plane_projected, oklab_to_linear,
    parse_hex,
};
use threshiator::document::{
    ComponentQuantizer, Document, ExportDefaults, LinkPolicy, Method, Recipe, SampleSize,
    ThresholdEditGesture, ThresholdEditTarget, ThresholdEditTransaction, ThresholdEncoding,
    ThresholdSpace, VoronoiMatching, VoronoiSite, VoronoiState, clamp_threshold_boundary,
    threshold_display_value, threshold_normalized_value,
};
use threshiator::export::{self, ExportFormat};
use threshiator::preset::{Preset, PresetStore, apply_to_document, validate_name};
use threshiator::processing::{
    PREVIEW_MAX_DIMENSION, bounded_preview, process,
    process_cancellable_with_progress_and_coverage, rotate_oklch, srgb_to_linear, to_display_rgba8,
};
use threshiator::project;
use threshiator::raster;
use threshiator::scheduler::{JobCoordinator, LatestGeneration, PreviewScheduler, ProgressTracker};
use threshiator::voronoi::{auto_initialize, linear_rgb_to_oklab, reattach_site, sample_color};
use threshiator::workflow::{
    OpenKind, ReplacementDecision, SaveResolution, classify_open_path, ensure_project_extension,
    replacement_decision, resolve_pending_after_save,
};
use zip::write::SimpleFileOptions;

fn tiny_source() -> (Vec<u8>, raster::DecodedRaster) {
    let image = ImageBuffer::from_pixel(2, 1, Rgba([64_u8, 128, 192, 127]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, ImageFormat::Png).unwrap();
    let bytes = bytes.into_inner();
    let decoded = raster::decode(&bytes, None).unwrap();
    (bytes, decoded)
}

fn preset_recipe() -> Recipe {
    let mut recipe = Recipe::default();
    let mut site = sample(1, [0.2, 0.3, 0.4], [0.8, 0.7, 0.6], 1.5);
    site.position = Some([0.25, 0.75]);
    site.locked = true;
    recipe.voronoi.sites = vec![site];
    recipe.voronoi.next_site_id = 2;
    recipe
}

#[test]
fn preset_store_first_run_unicode_duplicate_and_live_rescan() {
    let root = tempfile::tempdir().unwrap();
    let store = PresetStore::at(root.path().join("nested/presets"));
    assert!(!store.directory().exists());
    let preset = Preset::new(
        "  Étude 蓝  ",
        Some("A useful look".into()),
        &preset_recipe(),
    )
    .unwrap();
    let path = store.save(&preset, false).unwrap();
    assert!(path.starts_with(store.directory()));
    assert!(
        path.file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("preset-")
    );
    let before = fs::read(&path).unwrap();
    assert!(store.save(&preset, false).is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
    let mut replacement = preset.clone();
    replacement.description = Some("replacement".into());
    assert!(store.save_checked(&replacement, true, || false).is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
    let other = Preset::new("alpha", None, &Recipe::default()).unwrap();
    store.save(&other, false).unwrap();
    let names: Vec<_> = store
        .scan()
        .unwrap()
        .entries
        .into_iter()
        .map(|e| e.preset.name)
        .collect();
    assert_eq!(names, vec!["alpha", "Étude 蓝"]);
}

#[test]
fn preset_names_reject_traversal_controls_and_unreasonable_values() {
    for invalid in ["", "  ", "../escape", "a/b", "a\\b", "bad\nname"] {
        assert!(validate_name(invalid).is_err(), "{invalid:?}");
    }
    assert!(validate_name(&"x".repeat(129)).is_err());
    assert_eq!(validate_name("  Fine Look ").unwrap(), "Fine Look");
}

#[test]
fn preset_filenames_are_short_for_maximum_ascii_and_unicode_names() {
    let root = tempfile::tempdir().unwrap();
    let store = PresetStore::at(root.path());
    for name in ["a".repeat(128), "🦀".repeat(128)] {
        let path = store
            .save(
                &Preset::new(&name, None, &Recipe::default()).unwrap(),
                false,
            )
            .unwrap();
        let filename = path.file_name().unwrap().to_string_lossy();
        assert_eq!(filename.len(), "preset-".len() + 16 + ".json".len());
    }
    assert_eq!(store.scan().unwrap().entries.len(), 2);
}

#[test]
fn preset_json_is_leak_free_and_positions_are_always_detached() {
    let root = tempfile::tempdir().unwrap();
    let store = PresetStore::at(root.path());
    let preset = Preset::new("Detached", None, &preset_recipe()).unwrap();
    let path = store.save(&preset, false).unwrap();
    let json = fs::read_to_string(path).unwrap();
    for forbidden in [
        "source_bytes",
        "source_name",
        "export_defaults",
        "dirty",
        "project_path",
    ] {
        assert!(!json.contains(forbidden));
    }
    assert!(json.contains("\"position\": null"));
    let loaded = &store.scan().unwrap().entries[0].preset;
    assert_eq!(loaded.processing.voronoi.sites[0].position, None);
    assert!(loaded.processing.voronoi.sites[0].locked);
    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
    value["processing"]["voronoi"]["sites"][0]["position"] = serde_json::json!([99.0, -2.0]);
    fs::write(
        root.path().join("external.json"),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    assert!(
        store
            .scan()
            .unwrap()
            .entries
            .iter()
            .all(|entry| entry.preset.processing.voronoi.sites[0].position.is_none())
    );
}

#[test]
fn preset_scan_keeps_valid_files_when_others_are_malformed_or_future() {
    let root = tempfile::tempdir().unwrap();
    let store = PresetStore::at(root.path());
    store
        .save(&Preset::new("Good", None, &preset_recipe()).unwrap(), false)
        .unwrap();
    fs::write(root.path().join("bad.json"), b"not json").unwrap();
    fs::write(root.path().join("missing.json"), b"{}").unwrap();
    let mut future =
        serde_json::to_value(Preset::new("Future", None, &preset_recipe()).unwrap()).unwrap();
    future["version"] = serde_json::json!(99);
    fs::write(
        root.path().join("future.json"),
        serde_json::to_vec(&future).unwrap(),
    )
    .unwrap();
    fs::write(root.path().join("ignored.tmp"), b"bad").unwrap();
    let scan = store.scan().unwrap();
    assert_eq!(scan.entries.len(), 1);
    assert_eq!(scan.diagnostics.len(), 3);
    assert!(
        scan.diagnostics
            .iter()
            .any(|d| d.reason.contains("version 99"))
    );
}

#[test]
fn preset_scan_rejects_unknown_fields_at_nested_schema_levels() {
    let root = tempfile::tempdir().unwrap();
    let store = PresetStore::at(root.path());
    let preset = Preset::new("Typed", None, &preset_recipe()).unwrap();
    let mut matching_typo = serde_json::to_value(&preset).unwrap();
    matching_typo["processing"]["voronoi"]["matchng"] = serde_json::json!("Rgb");
    fs::write(
        root.path().join("matching-typo.json"),
        serde_json::to_vec(&matching_typo).unwrap(),
    )
    .unwrap();
    let mut locked_typo = serde_json::to_value(&preset).unwrap();
    locked_typo["processing"]["voronoi"]["sites"][0]["lokced"] = serde_json::json!(true);
    fs::write(
        root.path().join("locked-typo.json"),
        serde_json::to_vec(&locked_typo).unwrap(),
    )
    .unwrap();

    let scan = store.scan().unwrap();
    assert!(scan.entries.is_empty());
    assert_eq!(scan.diagnostics.len(), 2);
    assert!(scan.diagnostics.iter().any(|diagnostic| {
        diagnostic.path.ends_with("matching-typo.json")
            && diagnostic.reason.contains("unknown field `matchng`")
    }));
    assert!(scan.diagnostics.iter().any(|diagnostic| {
        diagnostic.path.ends_with("locked-typo.json")
            && diagnostic.reason.contains("unknown field `lokced`")
    }));
}

#[test]
fn preset_scan_excludes_every_file_with_an_ambiguous_embedded_name() {
    let root = tempfile::tempdir().unwrap();
    let store = PresetStore::at(root.path());
    let path = store
        .save(
            &Preset::new("Duplicate", None, &preset_recipe()).unwrap(),
            false,
        )
        .unwrap();
    fs::copy(path, root.path().join("user-copy.json")).unwrap();

    let scan = store.scan().unwrap();
    assert!(scan.entries.is_empty());
    assert_eq!(scan.diagnostics.len(), 2);
    assert!(scan.diagnostics.iter().all(|diagnostic| {
        diagnostic
            .reason
            .contains("duplicate preset name \"Duplicate\"")
    }));
    assert!(scan.diagnostics[0].path < scan.diagnostics[1].path);
}

#[test]
fn current_recipe_json_rejects_unknown_fields_without_breaking_roundtrip() {
    let recipe = preset_recipe();
    let encoded = serde_json::to_value(&recipe).unwrap();
    assert_eq!(
        serde_json::from_value::<Recipe>(encoded.clone()).unwrap(),
        recipe
    );
    let mut typo = encoded;
    typo["voronoi"]["sites"][0]["lokced"] = serde_json::json!(true);
    assert!(serde_json::from_value::<Recipe>(typo).is_err());
}

#[test]
fn preset_roundtrips_threshold_voronoi_metrics_locks_and_steps() {
    let root = tempfile::tempdir().unwrap();
    let store = PresetStore::at(root.path());
    let mut expected = Vec::new();
    for matching in [
        VoronoiMatching::Perceptual,
        VoronoiMatching::Rgb,
        VoronoiMatching::Hsv,
    ] {
        let mut recipe = preset_recipe();
        recipe.voronoi.matching = matching;
        recipe.threshold.active_space = ThresholdSpace::Hsv;
        let preset = Preset::new(&format!("Mode {matching:?}"), None, &recipe).unwrap();
        expected.push(preset.clone());
        store.save(&preset, false).unwrap();
    }
    let scan = store.scan().unwrap();
    assert_eq!(scan.entries.len(), 3);
    assert!(
        scan.entries
            .iter()
            .all(|entry| entry.preset.processing.voronoi.sites[0].locked)
    );
    for preset in expected {
        assert_eq!(
            scan.entries
                .iter()
                .find(|entry| entry.preset.name == preset.name)
                .unwrap()
                .preset,
            preset
        );
    }
}

#[test]
fn preset_apply_preserves_document_identity_detaches_and_noops_canonically() {
    let (bytes, decoded) = tiny_source();
    let mut document = Document {
        source_name: "identity.png".into(),
        source_bytes: bytes.clone().into(),
        source: decoded.pixels.clone(),
        interpretation: decoded.interpretation.clone(),
        recipe: Recipe::default(),
        export_defaults: ExportDefaults::default(),
        dirty: false,
    };
    let interpretation = document.interpretation.clone();
    let export_defaults = document.export_defaults.clone();
    let preset = Preset::new("Apply", None, &preset_recipe()).unwrap();
    assert!(apply_to_document(&mut document, &preset).unwrap());
    assert_eq!(document.source_name, "identity.png");
    assert_eq!(document.source_bytes.as_ref(), bytes.as_slice());
    assert!(document.source.shares_storage_with(&decoded.pixels));
    assert_eq!(document.interpretation, interpretation);
    assert_eq!(document.export_defaults, export_defaults);
    assert_eq!(document.recipe.voronoi.sites[0].position, None);
    assert!(document.dirty);
    document.dirty = false;
    assert!(!apply_to_document(&mut document, &preset).unwrap());
    assert!(!document.dirty);
}

#[test]
fn system_preset_store_uses_platform_data_suffix() {
    assert!(
        PresetStore::system()
            .directory()
            .ends_with("threshiator/presets")
    );
}

#[test]
fn preset_store_reports_conflicting_folder_and_nonfinite_steps() {
    let root = tempfile::tempdir().unwrap();
    let conflict = root.path().join("presets");
    fs::write(&conflict, b"not a directory").unwrap();
    assert!(PresetStore::at(&conflict).scan().is_err());
    let mut recipe = preset_recipe();
    recipe.steps[1].operation =
        threshiator::document::ComponentOperation::RotateHue { degrees: f32::NAN };
    assert!(Preset::new("Bad step", None, &recipe).is_err());
}

fn write_project_fixture(path: &Path, manifest: &serde_json::Value, source: &[u8]) {
    let file = fs::File::create(path).unwrap();
    let mut archive = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default();
    archive.start_file("manifest.json", options).unwrap();
    archive
        .write_all(serde_json::to_string_pretty(manifest).unwrap().as_bytes())
        .unwrap();
    archive.start_file("source/original", options).unwrap();
    archive.write_all(source).unwrap();
    archive.finish().unwrap();
}

fn hue_recipe(degrees: f32) -> Recipe {
    let mut recipe = Recipe::default();
    recipe.set_hue_degrees(degrees);
    recipe
}

#[test]
fn boundary_belongs_to_lower_band_and_alpha_survives() {
    let source = threshiator::document::PixelImage::new(
        1,
        1,
        vec![[
            srgb_to_linear(1.0 / 3.0),
            srgb_to_linear(2.0 / 3.0),
            srgb_to_linear(0.9),
            0.42,
        ]],
    )
    .unwrap();
    let output = process(&source, &Recipe::thresholds_default());
    assert!((output.pixels[0][0] - srgb_to_linear(1.0 / 6.0)).abs() < 0.0001);
    assert!((output.pixels[0][1] - srgb_to_linear(0.5)).abs() < 0.0001);
    assert_eq!(output.pixels[0][3], 0.42);
}

#[test]
fn fractional_hue_is_not_quantized() {
    let a = rotate_oklch([0.7, 0.2, 0.1], 12.0);
    let b = rotate_oklch([0.7, 0.2, 0.1], 12.5);
    assert_ne!(a, b);
    let recipe = hue_recipe(12.5);
    let json = serde_json::to_string(&recipe).unwrap();
    let reopened: Recipe = serde_json::from_str(&json).unwrap();
    assert_eq!(reopened.hue_degrees(), 12.5);
}

#[test]
fn processing_model_can_target_components_and_hue_is_circular_with_neutral_semantics() {
    let mut recipe = Recipe::thresholds_default();
    recipe.threshold.rgb_state.components[1].enabled = false;
    recipe.threshold.rgb_state.components[2].enabled = false;
    recipe.set_hue_degrees(12.5);
    let reopened: Recipe = serde_json::from_str(&serde_json::to_string(&recipe).unwrap()).unwrap();
    assert_eq!(reopened.steps, recipe.steps);
    let source = threshiator::document::PixelImage::new(1, 1, vec![[0.2, 0.4, 0.9, 0.7]]).unwrap();
    let targeted = process(&source, &recipe);
    let expected = rotate_oklch([srgb_to_linear(0.5), 0.4, 0.9], 12.5);
    for (index, expected) in expected.iter().enumerate() {
        assert!((targeted.pixels[0][index] - expected).abs() < 1.0e-6);
    }
    assert_eq!(targeted.pixels[0][3], 0.7);
    let color = [0.7, 0.2, 0.1];
    let a = rotate_oklch(color, 12.5);
    let b = rotate_oklch(color, 372.5);
    for index in 0..3 {
        assert!((a[index] - b[index]).abs() < 1.0e-5);
    }
    let neutral = [0.4, 0.4, 0.4];
    let rotated = rotate_oklch(neutral, 137.25);
    for index in 0..3 {
        assert!((neutral[index] - rotated[index]).abs() < 1.0e-5);
    }
}

#[test]
fn project_embeds_source_and_reopens_after_external_deletion() {
    let directory = tempfile::tempdir().unwrap();
    let external = directory.path().join("source.png");
    let project_path = directory.path().join("work.threshiator");
    let (bytes, decoded) = tiny_source();
    fs::write(&external, &bytes).unwrap();
    let mut recipe = hue_recipe(12.5);
    recipe.set_quantize([0.2, 0.79], [0.11, 0.51, 0.91]);
    recipe.active_method = Method::Thresholds;
    recipe
        .voronoi
        .sites
        .push(sample(7, [0.2, 0.4, 0.6], [0.7, 0.3, 0.1], 1.25));
    recipe.voronoi.next_site_id = 8;
    let document = Document {
        source_name: "source.png".into(),
        source_bytes: bytes.into(),
        source: decoded.pixels,
        interpretation: decoded.interpretation,
        recipe: recipe.clone(),
        export_defaults: ExportDefaults::default(),
        dirty: true,
    };
    project::save(&project_path, &document).unwrap();
    fs::remove_file(external).unwrap();
    let reopened = project::open(&project_path).unwrap();
    assert_eq!(reopened.recipe.hue_degrees(), 12.5);
    assert_eq!(reopened.recipe.voronoi, recipe.voronoi);
    assert_eq!(reopened.recipe.quantize(), recipe.quantize());
    assert_eq!(reopened.recipe.active_method, Method::Thresholds);
    assert_eq!(
        reopened.source_bytes.as_ref(),
        document.source_bytes.as_ref()
    );
    assert_eq!(reopened.source.width, 2);
    assert!(!reopened.dirty);
}

#[test]
fn embedded_example_decodes_with_default_voronoi_and_is_clean() {
    let document = threshiator::example::spectrum_document().unwrap();
    assert_eq!(
        document.source_bytes.as_ref(),
        threshiator::example::SPECTRUM_BYTES
    );
    assert_eq!(document.recipe.active_method, Method::Voronoi);
    assert!(!document.recipe.voronoi.sites.is_empty());
    assert!(!document.dirty);
}

#[test]
fn stale_preview_cannot_win() {
    let scheduler = PreviewScheduler::new();
    let source =
        threshiator::document::PixelImage::new(256, 256, vec![[0.4, 0.5, 0.6, 1.0]; 65536])
            .unwrap();
    scheduler.schedule(source.clone(), Recipe::default());
    let expected = scheduler.schedule(source, hue_recipe(12.5));
    let start = Instant::now();
    loop {
        if let Some(result) = scheduler.try_latest() {
            assert_eq!(result.generation, expected);
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn stale_preview_coverage_cannot_replace_latest_generation() {
    let scheduler = PreviewScheduler::new();
    let source =
        threshiator::document::PixelImage::new(512, 32, vec![[0.4, 0.4, 0.4, 1.0]; 16384]).unwrap();
    let mut older = Recipe::default();
    older
        .voronoi
        .sites
        .push(sample(1, [0.4; 3], [0.1, 0.2, 0.3], 0.0));
    scheduler.schedule(source.clone(), older);
    let mut latest = Recipe::default();
    latest
        .voronoi
        .sites
        .push(sample(9, [0.4; 3], [0.8, 0.7, 0.6], 0.0));
    let generation = scheduler.schedule(source, latest);
    let start = Instant::now();
    loop {
        if let Some(result) = scheduler.try_latest() {
            assert_eq!(result.generation, generation);
            assert_eq!(result.coverage.site_counts, vec![(9, 16384)]);
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn stale_open_generation_cannot_replace_latest_selection() {
    let gate = LatestGeneration::default();
    let older = gate.begin();
    let latest = gate.begin();
    assert!(!gate.is_current(older));
    assert!(gate.is_current(latest));
}

#[test]
fn cancelled_job_token_is_immediately_stale() {
    let gate = LatestGeneration::default();
    let token = gate.begin_token();
    assert!(token.is_current());
    gate.cancel();
    assert!(!token.is_current());
}

#[test]
fn cancelled_coordinator_stays_busy_until_matching_acknowledgement() {
    let mut jobs = JobCoordinator::default();
    let token = jobs.begin().unwrap();
    jobs.cancel();
    assert!(!token.is_current());
    assert!(jobs.is_busy());
    assert!(jobs.begin().is_none());
    assert!(!jobs.acknowledge(token.generation() + 1));
    assert!(jobs.is_busy());
    assert!(jobs.acknowledge(token.generation()));
    assert!(!jobs.is_busy());
    assert!(jobs.begin().is_some());
}

#[test]
fn progress_is_monotonic_and_rejects_stale_jobs() {
    let mut progress = ProgressTracker::default();
    progress.start(2);
    assert!(progress.accept(2, 0.25));
    assert!(!progress.accept(1, 0.50));
    assert!(!progress.accept(2, 0.20));
    assert!(progress.accept(2, 0.75));
}

#[test]
fn failed_atomic_export_preserves_existing_destination() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("existing.png");
    fs::write(&destination, b"keep me").unwrap();
    let result = export::atomic_write(&destination, |temporary| {
        fs::write(temporary, b"partial")?;
        anyhow::bail!("simulated encoder failure")
    });
    assert!(result.is_err());
    assert_eq!(fs::read(destination).unwrap(), b"keep me");
}

#[test]
fn cancelled_export_preserves_existing_destination() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("existing.png");
    fs::write(&destination, b"keep me").unwrap();
    let source =
        threshiator::document::PixelImage::new(4, 4, vec![[0.2, 0.4, 0.6, 1.0]; 16]).unwrap();
    let gate = LatestGeneration::default();
    let token = gate.begin_token();
    let result = export::export_recipe_cancellable(
        &destination,
        &source,
        &Recipe::default(),
        ExportFormat::Png8,
        &token,
        |fraction, _| {
            if fraction >= 0.74 {
                gate.cancel();
            }
        },
    );
    assert!(result.is_err());
    assert_eq!(fs::read(destination).unwrap(), b"keep me");
}

#[test]
fn representative_rgba_decode_and_png_depths_and_exr_float() {
    let (_, decoded) = tiny_source();
    assert_eq!(decoded.pixels.pixels[0][3], 127.0 / 255.0);
    let directory = tempfile::tempdir().unwrap();
    let png8 = directory.path().join("out8.png");
    let png16 = directory.path().join("out16.png");
    let exr = directory.path().join("out.exr");
    export::export(&png8, &decoded.pixels, ExportFormat::Png8).unwrap();
    export::export(&png16, &decoded.pixels, ExportFormat::Png16).unwrap();
    export::export(&exr, &decoded.pixels, ExportFormat::OpenExr32Float).unwrap();
    assert_eq!(image::open(png8).unwrap().color(), image::ColorType::Rgba8);
    assert_eq!(
        image::open(png16).unwrap().color(),
        image::ColorType::Rgba16
    );
    assert_eq!(
        image::open(&exr).unwrap().color(),
        image::ColorType::Rgba32F
    );
    let metadata = exr::prelude::MetaData::read_from_file(&exr, true).unwrap();
    assert!(
        metadata.headers[0]
            .channels
            .list
            .iter()
            .all(|channel| channel.sample_type == exr::meta::attribute::SampleType::F32)
    );
}

#[test]
fn baseline_still_formats_and_grayscale_alpha_layout_decode() {
    let image =
        image::DynamicImage::ImageRgba8(ImageBuffer::from_pixel(3, 2, Rgba([20_u8, 90, 180, 120])));
    for format in [
        ImageFormat::Png,
        ImageFormat::Jpeg,
        ImageFormat::Tiff,
        ImageFormat::WebP,
        ImageFormat::Bmp,
        ImageFormat::Gif,
    ] {
        let mut encoded = std::io::Cursor::new(Vec::new());
        image.write_to(&mut encoded, format).unwrap();
        let decoded = raster::decode(encoded.get_ref(), None).unwrap();
        assert_eq!((decoded.pixels.width, decoded.pixels.height), (3, 2));
    }

    let gray_alpha =
        image::DynamicImage::ImageLumaA8(ImageBuffer::from_pixel(1, 1, image::LumaA([100_u8, 77])));
    let mut encoded = std::io::Cursor::new(Vec::new());
    gray_alpha.write_to(&mut encoded, ImageFormat::Png).unwrap();
    let decoded = raster::decode(encoded.get_ref(), None).unwrap();
    let pixel = decoded.pixels.pixels[0];
    assert_eq!(pixel[0], pixel[1]);
    assert_eq!(pixel[1], pixel[2]);
    assert_eq!(pixel[3], 77.0 / 255.0);
    for svg in [
        "<svg xmlns='http://www.w3.org/2000/svg'/>",
        "\u{feff}  <SVG viewBox='0 0 1 1'></SVG>",
        "<?xml version='1.0'?>\n<svg/>",
        "<!DOCTYPE svg PUBLIC '-//W3C//DTD SVG 1.1//EN'>\n<svg/>",
    ] {
        let error = match raster::decode(svg.as_bytes(), None) {
            Ok(_) => panic!("SVG prefix unexpectedly decoded"),
            Err(error) => error,
        };
        assert!(
            error
                .to_string()
                .contains("SVG is intentionally unsupported")
        );
    }
}

#[test]
fn spectrum_breakpoint_png_with_svg_bytes_decodes_as_raster() {
    let path = std::path::Path::new("archive/webapp/SpectrumBreakpoint.png");
    let (bytes, decoded) = raster::decode_file(path).unwrap();
    assert!(bytes.windows(4).any(|window| window == b"<svg"));
    assert!(decoded.pixels.width > 0);
    assert!(decoded.pixels.height > 0);
    assert_eq!(decoded.interpretation.original_format, "PNG");
}

#[test]
fn pixel_and_source_storage_clone_cheaply_and_preview_is_bounded() {
    let source =
        threshiator::document::PixelImage::new(2000, 2, vec![[0.1, 0.2, 0.3, 1.0]; 4000]).unwrap();
    let clone = source.clone();
    assert!(source.shares_storage_with(&clone));
    let preview = bounded_preview(&source);
    assert_eq!(preview.width.max(preview.height), PREVIEW_MAX_DIMENSION);
    assert_eq!((source.width, source.height), (2000, 2));

    let bytes: Arc<[u8]> = vec![1, 2, 3].into();
    let clone = bytes.clone();
    assert!(Arc::ptr_eq(&bytes, &clone));
}

#[test]
fn low_alpha_display_keeps_straight_color_accuracy() {
    let image = threshiator::document::PixelImage::new(
        1,
        1,
        vec![[
            srgb_to_linear(0.8),
            srgb_to_linear(0.2),
            srgb_to_linear(0.4),
            1.0 / 255.0,
        ]],
    )
    .unwrap();
    let display = to_display_rgba8(&image);
    assert_eq!(display.bytes, vec![204, 51, 102, 1]);
}

#[test]
fn png16_input_process_export_retains_more_than_eight_bit_precision_and_full_dimensions() {
    let mut encoded = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut encoded, 3, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Sixteen);
        let mut writer = encoder.write_header().unwrap();
        let samples = [
            1_u16, 257, 1000, 65535, 12345, 23456, 34567, 45678, 54321, 22222, 33333, 44444,
        ];
        let bytes: Vec<u8> = samples.into_iter().flat_map(u16::to_be_bytes).collect();
        writer.write_image_data(&bytes).unwrap();
    }
    let decoded = raster::decode(&encoded, None).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("roundtrip.png");
    export::export_recipe(
        &path,
        &decoded.pixels,
        &Recipe {
            steps: vec![],
            ..Recipe::default()
        },
        ExportFormat::Png16,
    )
    .unwrap();
    let output = image::open(&path).unwrap().to_rgba16();
    assert_eq!(output.dimensions(), (3, 1));
    let values: Vec<u16> = output.into_raw();
    assert!(values.iter().any(|value| value % 257 != 0));
    assert!((values[2] as i32 - 1000).abs() <= 1);
}

#[test]
fn exr_f32_preserves_non_eight_bit_and_extended_range_values() {
    let image = threshiator::document::PixelImage::new(1, 1, vec![[1.25, 0.123_456_7, -0.1, 0.75]])
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("extended.exr");
    export::export(&path, &image, ExportFormat::OpenExr32Float).unwrap();
    let decoded = image::open(&path).unwrap().into_rgba32f();
    let pixel = decoded.get_pixel(0, 0).0;
    assert!((pixel[0] - 1.25).abs() < 1.0e-6);
    assert!((pixel[1] - 0.123_456_7).abs() < 1.0e-6);
    assert!((pixel[2] + 0.1).abs() < 1.0e-6);
    let metadata = exr::prelude::MetaData::read_from_file(path, true).unwrap();
    assert!(
        metadata.headers[0]
            .channels
            .list
            .iter()
            .all(|channel| channel.sample_type == exr::meta::attribute::SampleType::F32)
    );
}

fn sample(id: u64, color: [f32; 3], output: [f32; 3], influence: f64) -> VoronoiSite {
    VoronoiSite {
        id,
        order: id,
        source_color: [color[0], color[1], color[2], 1.0],
        target_color: output,
        influence,
        locked: false,
        position: Some([0.0, 0.0]),
        size: SampleSize::ThreeByThree,
    }
}

#[test]
fn site_mutations_reset_pairing_and_lock_protects_every_parameter() {
    let mut state = VoronoiState {
        sites: vec![sample(1, [0.1, 0.2, 0.3], [0.8, 0.7, 0.6], 1.0)],
        next_site_id: 2,
        matching: VoronoiMatching::Perceptual,
    };
    let replacement = [0.25, 0.5, 0.75, 0.4];
    assert!(state.set_source(1, replacement, None));
    assert_eq!(state.sites[0].target_color, replacement[..3]);
    assert_eq!(state.sites[0].position, None);
    assert!(state.set_target(1, [0.9, 0.1, 0.2]));
    assert!(state.set_locked(1, true));
    let locked = state.sites[0].clone();
    assert!(!state.set_source(1, [0.0; 4], Some([1.0, 1.0])));
    assert!(!state.set_target(1, [0.0; 3]));
    assert!(!state.set_influence(1, -4.0));
    assert!(!state.set_size(1, SampleSize::Point, [0.0; 4]));
    assert!(!state.delete_site(1));
    assert_eq!(state.sites[0], locked);
    assert!(state.set_locked(1, false));
    assert!(state.delete_site(1));
}

#[test]
fn hsv_matching_uses_true_cylinder_geometry() {
    let linear_hsv =
        |h, s, v| hsv_to_encoded([h, s, v]).map(|channel| srgb_to_linear(channel as f32));
    let source_rgb = linear_hsv(0.0, 1.0, 1.0);
    let source = threshiator::document::PixelImage::new(
        1,
        1,
        vec![[source_rgb[0], source_rgb[1], source_rgb[2], 1.0]],
    )
    .unwrap();
    let mut recipe = Recipe::default();
    recipe.voronoi.matching = VoronoiMatching::Hsv;
    recipe.voronoi.sites = vec![
        sample(1, linear_hsv(90.0, 1.0, 1.0), [0.9, 0.1, 0.1], 0.0),
        sample(2, linear_hsv(0.0, 0.0, 1.0), [0.1, 0.9, 0.1], 0.0),
    ];
    assert_eq!(&process(&source, &recipe).pixels[0][..3], &[0.1, 0.9, 0.1]);
}

#[test]
fn hsv_archived_neutral_weight_fixture_is_exact_and_alpha_safe() {
    let linear_hsv = |h, s, v| hsv_to_encoded([h, s, v]).map(|c| srgb_to_linear(c as f32));
    let site =
        |id, hsv: [f64; 3], target| sample(id, linear_hsv(hsv[0], hsv[1], hsv[2]), target, 0.0);
    let mut sites = vec![
        site(1, [1.0, 1.0, 0.8], [0.1, 0.0, 0.0]),
        site(2, [180.0, 1.0, 0.8], [0.0, 0.2, 0.0]),
        site(3, [0.0, 0.0, 0.35], [0.0, 0.0, 0.3]),
        site(4, [240.0, 0.0, 0.75], [0.4, 0.4, 0.0]),
    ];
    let cases = [
        ([359.0, 1.0, 0.8], 1, 1.0),
        ([181.0, 1.0, 0.8], 2, 1.0),
        ([300.0, 0.0, 0.30], 3, 1.0),
        ([60.0, 0.0, 0.80], 4, 1.0),
        ([20.0, 0.05, 0.80], 4, 0.08),
        ([170.0, 0.8, 0.65], 2, 1.0),
    ];
    let pixels = cases
        .iter()
        .map(|(hsv, _, alpha)| {
            let rgb = linear_hsv(hsv[0], hsv[1], hsv[2]);
            [rgb[0], rgb[1], rgb[2], *alpha]
        })
        .collect();
    let source = threshiator::document::PixelImage::new(3, 2, pixels).unwrap();
    let mut recipe = Recipe::default();
    recipe.voronoi.matching = VoronoiMatching::Hsv;
    recipe.voronoi.sites = sites.clone();
    let output = process(&source, &recipe);
    for (index, (_, expected, alpha)) in cases.iter().enumerate() {
        let target = recipe.voronoi.site(*expected).unwrap().target_color;
        assert_eq!(&output.pixels[index][..3], &target);
        assert_eq!(output.pixels[index][3], *alpha);
    }
    sites.reverse();
    recipe.voronoi.sites = sites;
    assert_eq!(
        process(&source, &recipe),
        output,
        "stable order must not depend on vector order"
    );
    let transparent =
        threshiator::document::PixelImage::new(1, 1, vec![[0.8, 0.2, 0.1, 0.0]]).unwrap();
    assert_eq!(process(&transparent, &recipe).pixels[0], [0.0; 4]);
}

#[test]
fn hsv_full_resolution_export_matches_authoritative_processing_and_alpha() {
    let rgb = hsv_to_encoded([359.0, 1.0, 0.8]).map(|c| srgb_to_linear(c as f32));
    let source =
        threshiator::document::PixelImage::new(2001, 1, vec![[rgb[0], rgb[1], rgb[2], 0.37]; 2001])
            .unwrap();
    let mut recipe = Recipe::default();
    recipe.voronoi.matching = VoronoiMatching::Hsv;
    recipe.voronoi.sites = vec![
        sample(1, rgb, [0.123, 0.456, 0.789], 0.0),
        sample(2, [0.0; 3], [0.8, 0.1, 0.2], 0.0),
    ];
    let authoritative = process(&source, &recipe);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hsv.png");
    export::export_recipe(&path, &source, &recipe, ExportFormat::Png16).unwrap();
    let decoded = raster::decode(&fs::read(path).unwrap(), None).unwrap();
    for channel in 0..4 {
        assert!(
            (decoded.pixels.pixels[1000][channel] - authoritative.pixels[1000][channel]).abs()
                < 2.0 / 65535.0
        );
    }
    let preview = bounded_preview(&source);
    assert_eq!(
        process(&preview, &recipe).pixels[0],
        authoritative.pixels[0]
    );
}

#[test]
fn point_and_three_by_three_sampling_are_authoritative_and_alpha_weighted() {
    let image = threshiator::document::PixelImage::new(
        3,
        3,
        vec![
            [1.0, 0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0, 0.5],
            [0.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 1.0],
            [0.25, 0.5, 0.75, 0.0],
            [1.0, 1.0, 1.0, 0.5],
            [0.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 0.0],
        ],
    )
    .unwrap();
    assert!(sample_color(&image, [0.5, 0.5], SampleSize::Point).is_none());
    let sampled = sample_color(&image, [0.5, 0.5], SampleSize::ThreeByThree).unwrap();
    assert!((sampled[0] - 0.5).abs() < 1e-6);
    assert!((sampled[1] - 1.0 / 3.0).abs() < 1e-6);
    assert!((sampled[2] - 0.5).abs() < 1e-6);
    assert!((sampled[3] - 3.0 / 9.0).abs() < 1e-6);
}

#[test]
fn five_by_five_sampling_uses_full_clipped_alpha_weighted_footprint() {
    let mut pixels = vec![[0.0, 0.0, 0.0, 0.0]; 25];
    for (index, pixel) in pixels.iter_mut().enumerate() {
        *pixel = [
            index as f32 / 24.0,
            0.25,
            0.75,
            if index % 2 == 0 { 1.0 } else { 0.5 },
        ];
    }
    let image = threshiator::document::PixelImage::new(5, 5, pixels.clone()).unwrap();
    let sampled = sample_color(&image, [0.5, 0.5], SampleSize::FiveByFive).unwrap();
    let alpha: f32 = pixels.iter().map(|pixel| pixel[3]).sum();
    let expected_red: f32 = pixels.iter().map(|pixel| pixel[0] * pixel[3]).sum::<f32>() / alpha;
    assert!((sampled[0] - expected_red).abs() < 1.0e-6);
    assert!((sampled[1] - 0.25).abs() < 1.0e-6);
    assert!((sampled[2] - 0.75).abs() < 1.0e-6);
}

#[test]
fn reattach_uses_the_sites_persisted_sampling_footprint_and_resets_target() {
    let pixels = (0..25)
        .map(|index| [index as f32 / 24.0, 0.25, 0.75, 1.0])
        .collect();
    let image = threshiator::document::PixelImage::new(5, 5, pixels).unwrap();
    let position = [0.5, 0.5];
    for size in [
        SampleSize::Point,
        SampleSize::ThreeByThree,
        SampleSize::FiveByFive,
    ] {
        let mut state = VoronoiState {
            sites: vec![VoronoiSite {
                size,
                target_color: [1.0, 0.0, 0.0],
                ..sample(1, [0.0; 3], [1.0, 0.0, 0.0], 0.0)
            }],
            next_site_id: 2,
            ..VoronoiState::default()
        };
        let expected = sample_color(&image, position, size).unwrap();
        assert!(reattach_site(&mut state, &image, 1, position));
        let site = state.site(1).unwrap();
        assert_eq!(site.size, size);
        assert_eq!(site.position, Some(position));
        assert_eq!(site.source_color, expected);
        assert_eq!(site.target_color, expected[..3]);
    }
}

#[test]
fn auto_sites_are_deterministic_representative_and_ignore_rare_noise() {
    let mut pixels = vec![[0.1, 0.1, 0.1, 1.0]];
    pixels.extend(vec![[0.8, 0.1, 0.1, 1.0]; 40]);
    pixels.extend(vec![[0.1, 0.8, 0.1, 1.0]; 40]);
    pixels.extend(vec![[0.1, 0.1, 0.8, 1.0]; 40]);
    pixels.push([1.0, 1.0, 1.0, 0.0]);
    let image = threshiator::document::PixelImage::new(122, 1, pixels).unwrap();
    let a = auto_initialize(&image, &image);
    let b = auto_initialize(&image, &image);
    assert_eq!(a, b);
    assert_eq!(a.sites.len(), 3);
    assert!(a.sites.iter().all(|site| site.source_color[3] > 0.0));
    assert!(
        a.sites
            .iter()
            .all(|site| site.target_color == site.source_color[..3])
    );
}

#[test]
fn large_proxy_alpha_support_keeps_four_dominant_colors_not_noise() {
    let dominant = [
        [0.70, 0.05, 0.05, 1.0],
        [0.05, 0.70, 0.05, 1.0],
        [0.05, 0.05, 0.70, 1.0],
        [0.65, 0.65, 0.08, 1.0],
    ];
    let mut pixels = Vec::with_capacity(1600 * 1600);
    for index in 0..(1600 * 1600) {
        pixels.push(dominant[index / (640 * 1000)]);
    }
    pixels[0] = [0.0, 1.0, 1.0, 1.0];
    pixels[1] = [1.0, 0.0, 1.0, 1.0];
    for pixel in pixels.iter_mut().take(102).skip(2) {
        *pixel = [1.0, 1.0, 1.0, 0.01];
    }
    let proxy = threshiator::document::PixelImage::new(1600, 1600, pixels).unwrap();
    let sites = auto_initialize(&proxy, &proxy);
    assert_eq!(sites.sites.len(), 4);
    for expected in dominant {
        assert!(sites.sites.iter().any(|site| {
            site.target_color
                .iter()
                .zip(expected)
                .all(|(actual, expected)| (actual - expected).abs() < 0.01)
        }));
    }
}

#[test]
fn coverage_updates_with_influence_and_is_generated_with_pixels() {
    let source = threshiator::document::PixelImage::new(
        5,
        1,
        vec![
            [0.1, 0.1, 0.1, 1.0],
            [0.35, 0.35, 0.35, 1.0],
            [0.5, 0.5, 0.5, 1.0],
            [0.7, 0.7, 0.7, 1.0],
            [0.9, 0.9, 0.9, 1.0],
        ],
    )
    .unwrap();
    let mut recipe = Recipe::default();
    recipe.voronoi.sites = vec![
        sample(1, [0.1; 3], [0.2, 0.0, 0.0], 0.0),
        sample(2, [0.9; 3], [0.0, 0.2, 0.0], 0.0),
    ];
    let generation = std::sync::atomic::AtomicU64::new(1);
    let (_, neutral) =
        process_cancellable_with_progress_and_coverage(&source, &recipe, 1, &generation, |_| {})
            .unwrap();
    recipe.voronoi.sites[1].influence = 4.0;
    let (_, biased) =
        process_cancellable_with_progress_and_coverage(&source, &recipe, 1, &generation, |_| {})
            .unwrap();
    assert_eq!(neutral.visible_total, 5);
    assert_eq!(biased.visible_total, 5);
    assert!(biased.site_counts[1].1 > neutral.site_counts[1].1);
}

#[test]
fn influence_preserves_exact_weighted_distance_boundary() {
    let encoded = |value: f32| srgb_to_linear(value);
    let source = threshiator::document::PixelImage::new(
        2,
        1,
        vec![
            [encoded(0.58), encoded(0.58), encoded(0.58), 1.0],
            [encoded(0.59), encoded(0.59), encoded(0.59), 1.0],
        ],
    )
    .unwrap();
    let mut recipe = Recipe::default();
    recipe.voronoi.matching = VoronoiMatching::Rgb;
    recipe.voronoi.sites = vec![
        sample(1, [0.0; 3], [0.1, 0.2, 0.3], 1.0),
        sample(2, [1.0; 3], [0.8, 0.7, 0.6], 0.0),
    ];
    let output = process(&source, &recipe);
    assert_eq!(&output.pixels[0][..3], &[0.1, 0.2, 0.3]);
    assert_eq!(&output.pixels[1][..3], &[0.8, 0.7, 0.6]);
}

#[test]
fn every_voronoi_metric_agrees_between_bounded_preview_and_full_source() {
    let pixels: Vec<_> = (0..3200)
        .map(|x| {
            if x % 4 < 2 {
                [0.1, 0.3, 0.8, 1.0]
            } else {
                [0.8, 0.2, 0.1, 1.0]
            }
        })
        .collect();
    let source = threshiator::document::PixelImage::new(3200, 1, pixels).unwrap();
    let preview = bounded_preview(&source);
    let mut recipe = Recipe::default();
    recipe.voronoi.sites = vec![
        sample(1, [0.1, 0.3, 0.8], [0.2, 0.4, 0.6], -1.0),
        sample(2, [0.8, 0.2, 0.1], [0.9, 0.7, 0.5], 1.0),
    ];
    for matching in [
        VoronoiMatching::Perceptual,
        VoronoiMatching::Rgb,
        VoronoiMatching::Hsv,
    ] {
        recipe.voronoi.matching = matching;
        let full = process(&source, &recipe);
        let small = process(&preview, &recipe);
        for x in 0..preview.width as usize {
            assert_eq!(small.pixels[x], full.pixels[x * 2], "{matching:?} at {x}");
        }
    }
}

#[test]
fn oklab_sites_have_stable_ties_influence_and_float_outputs() {
    let source = threshiator::document::PixelImage::new(
        2,
        1,
        vec![[0.5, 0.5, 0.5, 0.25], [0.0, 0.0, 0.0, 0.0]],
    )
    .unwrap();
    let mut recipe = Recipe {
        voronoi: VoronoiState {
            sites: vec![
                sample(1, [0.5; 3], [0.123_456_7, 0.2, 0.3], 0.0),
                sample(2, [0.5; 3], [0.9, 0.8, 0.7], 0.0),
            ],
            next_site_id: 3,
            matching: Default::default(),
        },
        ..Recipe::default()
    };
    let tied = process(&source, &recipe);
    assert_eq!(&tied.pixels[0][..3], &[0.123_456_7, 0.2, 0.3]);
    assert_eq!(tied.pixels[0][3], 0.25);
    assert_eq!(tied.pixels[1], [0.0; 4]);
    recipe.voronoi.sites[0].source_color = [0.0, 0.0, 0.0, 1.0];
    recipe.voronoi.sites[1].source_color = [1.0, 1.0, 1.0, 1.0];
    recipe.voronoi.sites[0].influence = -4.0;
    recipe.voronoi.sites[1].influence = 4.0;
    assert_eq!(&process(&source, &recipe).pixels[0][..3], &[0.9, 0.8, 0.7]);
}

#[test]
fn independent_sites_can_share_targets_and_method_states_remain_independent() {
    let mut recipe = Recipe::default();
    recipe.voronoi.sites = vec![
        sample(1, [0.0; 3], [0.37, 0.41, 0.53], 0.0),
        sample(2, [1.0; 3], [0.37, 0.41, 0.53], 0.0),
    ];
    recipe.voronoi.sites[1].position = Some([1.0, 0.0]);
    recipe.voronoi.sites[1].size = SampleSize::Point;
    let before_position = recipe.voronoi.sites[0].position;
    recipe.voronoi.sites[0].target_color[0] = 0.333_333_34;
    assert_eq!(recipe.voronoi.sites[0].position, before_position);
    let source = threshiator::document::PixelImage::new(
        2,
        1,
        vec![[0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0]],
    )
    .unwrap();
    let output = process(&source, &recipe);
    assert_ne!(output.pixels[0], output.pixels[1]);
    let sites = recipe.voronoi.clone();
    recipe.active_method = Method::Thresholds;
    recipe.set_quantize([0.2, 0.8], [0.1, 0.5, 0.9]);
    recipe.active_method = Method::Voronoi;
    assert_eq!(recipe.voronoi, sites);
}

#[test]
fn oklab_reference_fixture_and_preview_full_classification_agree() {
    let red = linear_rgb_to_oklab([1.0, 0.0, 0.0]);
    assert!((red[0] - 0.627_955_36).abs() < 1e-7);
    assert!((red[1] - 0.224_863_06).abs() < 1e-7);
    let source =
        threshiator::document::PixelImage::new(4, 1, vec![[0.1, 0.2, 0.3, 1.0]; 4]).unwrap();
    let mut recipe = Recipe::default();
    recipe
        .voronoi
        .sites
        .push(sample(1, [0.1, 0.2, 0.3], [0.7, 0.6, 0.5], 0.0));
    assert_eq!(
        process(&source, &recipe),
        process(&bounded_preview(&source), &recipe)
    );
}

#[test]
fn bounded_proxy_uses_full_resolution_samples_and_corresponding_labels_agree() {
    let pixels: Vec<_> = (0..3200)
        .map(|x| {
            if x < 1600 {
                [0.12, 0.22, 0.32, 1.0]
            } else {
                [0.82, 0.62, 0.22, 1.0]
            }
        })
        .collect();
    let source = threshiator::document::PixelImage::new(3200, 1, pixels).unwrap();
    let proxy = bounded_preview(&source);
    let recipe = Recipe {
        voronoi: auto_initialize(&proxy, &source),
        ..Recipe::default()
    };
    for site in &recipe.voronoi.sites {
        assert_eq!(
            site.source_color,
            sample_color(&source, site.position.unwrap(), site.size).unwrap()
        );
    }
    let full = process(&source, &recipe);
    let preview = process(&proxy, &recipe);
    for x in 0..proxy.width as usize {
        assert_eq!(preview.pixels[x], full.pixels[x * 2]);
    }
}

#[test]
fn project_routing_and_extension_are_case_insensitive() {
    assert_eq!(
        classify_open_path(Path::new("work.THRESHIATOR")),
        OpenKind::Project
    );
    assert_eq!(classify_open_path(Path::new("photo.png")), OpenKind::Image);
    assert_eq!(
        ensure_project_extension(PathBuf::from("work")),
        PathBuf::from("work.threshiator")
    );
    assert_eq!(
        ensure_project_extension(PathBuf::from("work.ThReShIaToR")),
        PathBuf::from("work.ThReShIaToR")
    );
}

#[test]
fn replacement_gate_only_replaces_after_explicit_resolution() {
    assert_eq!(
        replacement_decision(false, "cancel"),
        ReplacementDecision::ReplaceNow
    );
    assert_eq!(
        replacement_decision(true, "save"),
        ReplacementDecision::SaveThenReplace
    );
    assert_eq!(
        replacement_decision(true, "discard"),
        ReplacementDecision::ReplaceNow
    );
    assert_eq!(
        replacement_decision(true, "cancel"),
        ReplacementDecision::KeepCurrent
    );
    assert_eq!(
        replacement_decision(true, "save-failed"),
        ReplacementDecision::KeepCurrent
    );
}

#[test]
fn color_models_roundtrip_references_and_retain_latent_hue() {
    let red = [1.0, 0.0, 0.0];
    assert_eq!(encoded_to_hsv(red), [0.0, 1.0, 1.0]);
    assert_eq!(encoded_to_hsl(red), [0.0, 1.0, 0.5]);
    for rgb in [[0.2, 0.4, 0.8], [1.0, 0.0, 0.0], [0.25, 0.25, 0.25]] {
        let hsv = hsv_to_encoded(encoded_to_hsv(rgb));
        let hsl = hsl_to_encoded(encoded_to_hsl(rgb));
        for channel in 0..3 {
            assert!((hsv[channel] - rgb[channel]).abs() < 1e-10);
            assert!((hsl[channel] - rgb[channel]).abs() < 1e-10);
        }
    }
    let mut draft = DraftColor::new([0.5, 0.5, 0.5]);
    draft.set_values(ColorModel::Hsv, [287.0, 0.0, 0.7]);
    assert!((draft.values(ColorModel::Hsv)[0] - 287.0).abs() < 1e-9);
}

#[test]
fn oklab_plane_mapping_projection_and_marker_are_coherent() {
    let reference = [0.12, 0.53, 0.81];
    let roundtrip = oklab_to_linear(linear_to_oklab(reference));
    for channel in 0..3 {
        assert!((roundtrip[channel] - reference[channel]).abs() < 1e-6);
    }
    let valid = oklab_plane(0.65, 0.05, -0.05);
    assert!(in_srgb_gamut(oklab_to_linear(valid)));
    assert_eq!(oklab_plane_projected(0.65, 0.05, -0.05), valid);
    let coordinates = oklab_plane_coords(valid);
    assert!((coordinates[0] - 0.05).abs() < 1e-12);
    assert!((coordinates[1] + 0.05).abs() < 1e-12);
    assert_eq!(oklab_plane(valid[0], coordinates[0], coordinates[1]), valid);

    let raw_invalid = oklab_plane(0.65, 1.0, 0.0);
    assert!(!in_srgb_gamut(oklab_to_linear(raw_invalid)));
    let projected = oklab_plane_projected(0.65, 1.0, 0.0);
    assert!(in_srgb_gamut(oklab_to_linear(projected)));
    let max = max_oklab_chroma(0.65, 0.0);
    assert!((projected[1] - max).abs() < 1e-8);
    assert!(projected[2].abs() < 1e-12);
    assert_eq!(parse_hex("#abc").unwrap(), parse_hex("#AABBCC").unwrap());
    assert!(parse_hex("#abcd").is_err());
    assert!(parse_hex("#GG0000").is_err());
}

#[test]
fn oklab_plane_keyboard_adjusts_axes_projects_and_preserves_lightness() {
    let neutral = [0.65, 0.0, 0.0];
    assert_eq!(
        adjust_oklab_plane(neutral, PlaneKey::Right, false),
        [0.65, 0.005, 0.0]
    );
    assert_eq!(
        adjust_oklab_plane(neutral, PlaneKey::Left, true),
        [0.65, -0.0005, 0.0]
    );
    assert_eq!(
        adjust_oklab_plane(neutral, PlaneKey::Up, false),
        [0.65, 0.0, 0.005]
    );
    assert_eq!(
        adjust_oklab_plane(neutral, PlaneKey::Down, true),
        [0.65, 0.0, -0.0005]
    );
    let saturated = oklab_plane_projected(0.65, 1.0, 0.0);
    let adjusted = adjust_oklab_plane(saturated, PlaneKey::Right, false);
    assert_eq!(adjusted[0], saturated[0]);
    assert!(adjusted.iter().all(|value| value.is_finite()));
    assert!(in_srgb_gamut(oklab_to_linear(adjusted)));
    assert_eq!(
        adjust_oklab_plane(saturated, PlaneKey::Home, false),
        [0.65, 0.0, 0.0]
    );
}

#[test]
fn cancelled_or_failed_save_clears_replacement_continuation() {
    let mut pending = Some("open-project");
    assert_eq!(
        resolve_pending_after_save(&mut pending, SaveResolution::Cancelled),
        None
    );
    assert_eq!(pending, None);
    assert_eq!(
        resolve_pending_after_save(&mut pending, SaveResolution::CurrentSuccess),
        None
    );

    pending = Some("try-example");
    assert_eq!(
        resolve_pending_after_save(&mut pending, SaveResolution::Failed),
        None
    );
    assert_eq!(pending, None);

    pending = Some("open-image");
    assert_eq!(
        resolve_pending_after_save(&mut pending, SaveResolution::CurrentSuccess),
        Some("open-image")
    );
    assert_eq!(pending, None);
}

#[test]
fn color_picker_transaction_cancels_without_commit_and_commits_once() {
    assert!(PickerTransaction::new([0.1, 0.2, 0.3]).cancel().is_none());
    let mut transaction = PickerTransaction::new([0.1, 0.2, 0.3]);
    assert_eq!(transaction.commit(), Some([0.1, 0.2, 0.3]));
    assert!(transaction.commit().is_none());
}

#[test]
fn active_voronoi_method_drives_full_resolution_export() {
    let source =
        threshiator::document::PixelImage::new(3, 1, vec![[0.2, 0.3, 0.4, 1.0]; 3]).unwrap();
    let mut recipe = Recipe::default();
    recipe
        .voronoi
        .sites
        .push(sample(1, [0.2, 0.3, 0.4], [0.71, 0.22, 0.13], 0.0));
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("voronoi.png");
    export::export_recipe(&destination, &source, &recipe, ExportFormat::Png16).unwrap();
    let decoded = raster::decode(&fs::read(destination).unwrap(), None).unwrap();
    assert_eq!((decoded.pixels.width, decoded.pixels.height), (3, 1));
    for channel in 0..3 {
        assert!((decoded.pixels.pixels[0][channel] - [0.71, 0.22, 0.13][channel]).abs() < 0.0001);
    }
}

#[test]
fn variable_band_quantizers_validate_and_keep_exact_boundaries_in_lower_band() {
    for bands in [2, 3, 8, 32] {
        let quantizer = ComponentQuantizer::evenly_spaced(bands);
        quantizer.validate("Test").unwrap();
        assert_eq!(quantizer.outputs.len(), bands);
        assert_eq!(quantizer.boundaries.len(), bands - 1);
        assert_eq!(
            quantizer.quantize(quantizer.boundaries[0]),
            quantizer.outputs[0]
        );
    }
    let mut malformed = ComponentQuantizer::evenly_spaced(3);
    malformed.boundaries = vec![0.6, 0.4];
    assert!(
        malformed
            .validate("Red")
            .unwrap_err()
            .contains("strictly ordered")
    );
    malformed.boundaries = vec![f32::NAN, 0.8];
    assert!(malformed.validate("Red").unwrap_err().contains("finite"));
    malformed = ComponentQuantizer::evenly_spaced(2);
    malformed.outputs.push(0.9);
    assert!(
        malformed
            .validate("Red")
            .unwrap_err()
            .contains("exactly one more")
    );
}

#[test]
fn encoded_rgb_is_float_precise_bypassable_and_distinct_from_legacy_linear() {
    let source =
        threshiator::document::PixelImage::new(1, 1, vec![[0.214_041_14, 0.32, 0.73, 0.37]])
            .unwrap();
    let mut encoded = Recipe::thresholds_default();
    encoded.threshold.rgb_state.link = LinkPolicy::Independent;
    encoded.threshold.rgb_state.components[0] = ComponentQuantizer {
        enabled: true,
        boundaries: vec![0.49],
        outputs: vec![0.123_456_7, 0.876_543_2],
    };
    encoded.threshold.rgb_state.components[1].enabled = false;
    encoded.threshold.rgb_state.components[2].enabled = false;
    let output = process(&source, &encoded);
    assert!((output.pixels[0][0] - srgb_to_linear(0.876_543_2)).abs() < 1.0e-6);
    assert!((output.pixels[0][1] - source.pixels[0][1]).abs() < 1.0e-6);
    assert!((output.pixels[0][2] - source.pixels[0][2]).abs() < 1.0e-6);
    assert_eq!(output.pixels[0][3], 0.37);
    let mut legacy = encoded.clone();
    legacy.threshold.rgb_state.encoding = ThresholdEncoding::LinearSrgbLegacy;
    assert_ne!(process(&source, &legacy).pixels[0][0], output.pixels[0][0]);
}

#[test]
fn linking_propagates_edits_not_enabled_flags_and_spaces_restore_exactly() {
    let mut threshold = Recipe::thresholds_default().threshold;
    threshold.rgb_state.components[1].enabled = false;
    threshold.set_rgb_component(0, ComponentQuantizer::evenly_spaced(8));
    assert_eq!(threshold.rgb_state.components[1].outputs.len(), 8);
    assert!(!threshold.rgb_state.components[1].enabled);
    let rgb_before = threshold.rgb_state.clone();
    threshold.active_space = ThresholdSpace::Hsv;
    threshold.hsv_state.hue = ComponentQuantizer::evenly_spaced(2);
    threshold.hsv_state.value.enabled = false;
    threshold.set_hsv_component(1, ComponentQuantizer::evenly_spaced(32));
    assert_eq!(threshold.hsv_state.value.outputs.len(), 32);
    assert!(!threshold.hsv_state.value.enabled);
    assert_eq!(threshold.hsv_state.hue.outputs.len(), 2);
    threshold.active_space = ThresholdSpace::Rgb;
    assert_eq!(threshold.rgb_state, rgb_before);
}

#[test]
fn hsv_hue_wraps_at_seam_keeps_neutrals_and_sv_link_excludes_hue() {
    let near_seam = threshiator::document::PixelImage::new(
        2,
        1,
        vec![
            {
                let rgb = hsv_to_encoded([359.0, 1.0, 0.8]);
                [
                    srgb_to_linear(rgb[0] as f32),
                    srgb_to_linear(rgb[1] as f32),
                    srgb_to_linear(rgb[2] as f32),
                    1.0,
                ]
            },
            {
                let rgb = hsv_to_encoded([1.0, 1.0, 0.8]);
                [
                    srgb_to_linear(rgb[0] as f32),
                    srgb_to_linear(rgb[1] as f32),
                    srgb_to_linear(rgb[2] as f32),
                    1.0,
                ]
            },
        ],
    )
    .unwrap();
    let mut recipe = Recipe::thresholds_default();
    recipe.threshold.active_space = ThresholdSpace::Hsv;
    recipe.threshold.hsv_state.hue_origin_degrees = 0.0;
    recipe.threshold.hsv_state.hue = ComponentQuantizer::evenly_spaced(8);
    recipe.threshold.hsv_state.saturation.enabled = false;
    recipe.threshold.hsv_state.value.enabled = false;
    let seam = process(&near_seam, &recipe);
    let hue0 = encoded_to_hsv([
        threshiator::processing::linear_to_srgb(seam.pixels[0][0]) as f64,
        threshiator::processing::linear_to_srgb(seam.pixels[0][1]) as f64,
        threshiator::processing::linear_to_srgb(seam.pixels[0][2]) as f64,
    ])[0];
    let hue1 = encoded_to_hsv([
        threshiator::processing::linear_to_srgb(seam.pixels[1][0]) as f64,
        threshiator::processing::linear_to_srgb(seam.pixels[1][1]) as f64,
        threshiator::processing::linear_to_srgb(seam.pixels[1][2]) as f64,
    ])[0];
    assert!((hue0 - 337.5).abs() < 1.0e-3);
    assert!((hue1 - 22.5).abs() < 1.0e-3);
    assert!(((hue0 - hue1).abs().min(360.0 - (hue0 - hue1).abs()) - 45.0).abs() < 1.0e-3);

    recipe.threshold.hsv_state.hue = ComponentQuantizer::evenly_spaced(4);
    let known_hues = [10.0, 100.0, 200.0, 300.0];
    let known_pixels = known_hues
        .map(|hue| {
            let rgb = hsv_to_encoded([hue, 1.0, 0.8]);
            [
                srgb_to_linear(rgb[0] as f32),
                srgb_to_linear(rgb[1] as f32),
                srgb_to_linear(rgb[2] as f32),
                1.0,
            ]
        })
        .to_vec();
    let known = threshiator::document::PixelImage::new(4, 1, known_pixels).unwrap();
    let quantized = process(&known, &recipe);
    for (pixel, expected_hue) in quantized.pixels.iter().zip([45.0, 135.0, 225.0, 315.0]) {
        let actual = encoded_to_hsv([
            threshiator::processing::linear_to_srgb(pixel[0]) as f64,
            threshiator::processing::linear_to_srgb(pixel[1]) as f64,
            threshiator::processing::linear_to_srgb(pixel[2]) as f64,
        ])[0];
        assert!((actual - expected_hue).abs() < 1.0e-3);
    }
    let neutral = threshiator::document::PixelImage::new(1, 1, vec![[0.4, 0.4, 0.4, 0.8]]).unwrap();
    let neutral_out = process(&neutral, &recipe);
    for index in 0..3 {
        assert!((neutral_out.pixels[0][index] - neutral.pixels[0][index]).abs() < 1.0e-5);
    }
}

#[test]
fn voronoi_matching_modes_are_deterministic_and_hsv_fades_hue_for_neutrals() {
    let source = threshiator::document::PixelImage::new(1, 1, vec![[0.5, 0.5, 0.5, 0.22]]).unwrap();
    let mut recipe = Recipe::default();
    recipe.voronoi.sites = vec![
        sample(1, [0.5; 3], [0.1, 0.2, 0.3], 0.0),
        sample(2, [0.5; 3], [0.8, 0.7, 0.6], 0.0),
    ];
    for matching in [
        VoronoiMatching::Perceptual,
        VoronoiMatching::Rgb,
        VoronoiMatching::Hsv,
    ] {
        recipe.voronoi.matching = matching;
        let first = process(&source, &recipe);
        let second = process(&source, &recipe);
        assert_eq!(first, second);
        assert_eq!(&first.pixels[0][..3], &[0.1, 0.2, 0.3]);
        assert_eq!(first.pixels[0][3], 0.22);
    }

    let linear_hue =
        |degrees| hsv_to_encoded([degrees, 1.0, 0.8]).map(|value| srgb_to_linear(value as f32));
    let blue = linear_hue(240.0);
    let mut hues = Recipe::default();
    hues.voronoi.matching = VoronoiMatching::Hsv;
    hues.voronoi.sites = vec![
        sample(1, linear_hue(0.0), [0.9, 0.1, 0.1], 0.0),
        sample(2, blue, [0.1, 0.1, 0.9], 0.0),
    ];
    let blue_source =
        threshiator::document::PixelImage::new(1, 1, vec![[blue[0], blue[1], blue[2], 0.6]])
            .unwrap();
    assert_eq!(
        &process(&blue_source, &hues).pixels[0][..3],
        &[0.1, 0.1, 0.9]
    );
    let seam = linear_hue(359.0);
    hues.voronoi.sites = vec![
        sample(1, linear_hue(180.0), [0.9, 0.1, 0.1], 0.0),
        sample(2, linear_hue(1.0), [0.1, 0.9, 0.1], 0.0),
    ];
    let seam_source =
        threshiator::document::PixelImage::new(1, 1, vec![[seam[0], seam[1], seam[2], 1.0]])
            .unwrap();
    assert_eq!(
        &process(&seam_source, &hues).pixels[0][..3],
        &[0.1, 0.9, 0.1]
    );
}

#[test]
fn project_v4_roundtrips_both_method_states_flags_and_ids() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("v3.threshiator");
    let (bytes, decoded) = tiny_source();
    let mut recipe = Recipe::thresholds_default();
    recipe.threshold.rgb_state.link = LinkPolicy::Independent;
    recipe.threshold.rgb_state.components[2].enabled = false;
    recipe
        .threshold
        .set_rgb_component(0, ComponentQuantizer::evenly_spaced(8));
    recipe.threshold.active_space = ThresholdSpace::Hsv;
    recipe.threshold.hsv_state.hue_origin_degrees = 37.5;
    recipe
        .threshold
        .set_hsv_component(1, ComponentQuantizer::evenly_spaced(32));
    recipe.voronoi.matching = VoronoiMatching::Hsv;
    recipe
        .voronoi
        .sites
        .push(sample(41, [0.2, 0.3, 0.4], [0.6, 0.5, 0.4], 1.5));
    recipe.voronoi.next_site_id = 42;
    let document = Document {
        source_name: "fixture.png".into(),
        source_bytes: bytes.into(),
        source: decoded.pixels,
        interpretation: decoded.interpretation,
        recipe: recipe.clone(),
        export_defaults: ExportDefaults::default(),
        dirty: true,
    };
    project::save(&path, &document).unwrap();
    let reopened = project::open(&path).unwrap();
    assert_eq!(reopened.recipe, recipe);
    assert!(!reopened.dirty);
}

#[test]
fn v2_migration_is_clean_and_pixel_identical_to_legacy_processing() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("v2.threshiator");
    let (bytes, decoded) = tiny_source();
    let mut recipe = Recipe::thresholds_default();
    recipe.set_quantize([0.2, 0.79], [0.11, 0.51, 0.91]);
    recipe.set_hue_degrees(12.5);
    let mut recipe_json = serde_json::to_value(&recipe).unwrap();
    recipe_json.as_object_mut().unwrap().remove("threshold");
    recipe_json["voronoi"]
        .as_object_mut()
        .unwrap()
        .remove("matching");
    let manifest = serde_json::json!({
        "version": 2, "source_name": "fixture.png", "source_entry": "source/original",
        "source_interpretation": decoded.interpretation, "recipe": recipe_json,
        "export_defaults": ExportDefaults::default(),
    });
    write_project_fixture(&path, &manifest, &bytes);
    let migrated = project::open(&path).unwrap();
    assert!(!migrated.dirty);
    assert_eq!(
        migrated.recipe.threshold.rgb_state.encoding,
        ThresholdEncoding::LinearSrgbLegacy
    );
    let output = process(&migrated.source, &migrated.recipe);
    let source = migrated.source.pixels[0];
    let legacy_band = |value: f32| {
        if value <= 0.2 {
            0.11
        } else if value <= 0.79 {
            0.51
        } else {
            0.91
        }
    };
    let expected = rotate_oklch(
        [
            legacy_band(source[0]),
            legacy_band(source[1]),
            legacy_band(source[2]),
        ],
        12.5,
    );
    for (index, expected) in expected.iter().enumerate() {
        assert!((output.pixels[0][index] - expected).abs() < 1.0e-6);
    }
    assert_eq!(output.pixels[0][3], source[3]);
}

#[test]
fn malformed_v3_quantizer_arrays_are_rejected_actionably() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("bad.threshiator");
    let (bytes, decoded) = tiny_source();
    let recipe = Recipe::thresholds_default();
    let mut recipe_json = serde_json::to_value(recipe).unwrap();
    recipe_json["threshold"]["rgb_state"]["components"][0]["boundaries"] =
        serde_json::json!([0.8, 0.2]);
    let manifest = serde_json::json!({
        "version": 3, "source_name": "fixture.png", "source_entry": "source/original",
        "source_interpretation": decoded.interpretation, "recipe": recipe_json,
        "export_defaults": ExportDefaults::default(),
    });
    write_project_fixture(&path, &manifest, &bytes);
    let error = project::open(&path).unwrap_err().to_string();
    assert!(error.contains("invalid Threshold state"));
    assert!(error.contains("strictly ordered"));
}

#[test]
fn v3_groups_migrate_to_independent_v4_sites_with_inherited_targets() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("legacy-v3.threshiator");
    let (bytes, decoded) = tiny_source();
    let mut recipe = serde_json::to_value(Recipe::default()).unwrap();
    recipe["voronoi"] = serde_json::json!({
        "groups": [
            {
                "id": 10,
                "name": "Shared blue",
                "output_color": [0.1, 0.2, 0.8],
                "samples": [
                    {"id": 4, "position": [0.25, 0.5], "source_color": [0.2, 0.3, 0.4, 0.5], "oklab": [0.0, 0.0, 0.0], "influence": -1.0, "size": "Point", "order": 4},
                    {"id": 7, "position": [0.75, 0.5], "source_color": [0.6, 0.5, 0.4, 1.0], "oklab": [1.0, 1.0, 1.0], "influence": 2.0, "size": "FiveByFive", "order": 7}
                ]
            },
            {"id": 11, "name": "Empty", "output_color": [1.0, 0.0, 0.0], "samples": []}
        ],
        "next_group_id": 12,
        "next_sample_id": 6,
        "matching": "Hsv"
    });
    let manifest = serde_json::json!({
        "version": 3, "source_name": "fixture.png", "source_entry": "source/original",
        "source_interpretation": decoded.interpretation, "recipe": recipe,
        "export_defaults": ExportDefaults::default(),
    });
    write_project_fixture(&path, &manifest, &bytes);
    let migrated = project::open(&path).unwrap();
    assert_eq!(migrated.recipe.voronoi.sites.len(), 2);
    assert_eq!(migrated.recipe.voronoi.sites[0].id, 4);
    assert_eq!(migrated.recipe.voronoi.sites[1].id, 7);
    assert_eq!(
        migrated.recipe.voronoi.sites[0].target_color,
        [0.1, 0.2, 0.8]
    );
    assert_eq!(
        migrated.recipe.voronoi.sites[1].target_color,
        [0.1, 0.2, 0.8]
    );
    assert!(!migrated.recipe.voronoi.sites.iter().any(|site| site.locked));
    assert_eq!(migrated.recipe.voronoi.next_site_id, 8);
    assert_eq!(migrated.recipe.voronoi.matching, VoronoiMatching::Hsv);
    let mut edited = migrated;
    assert!(edited.recipe.voronoi.set_target(4, [0.9, 0.1, 0.2]));
    let v4_path = directory.path().join("migrated-v4.threshiator");
    project::save(&v4_path, &edited).unwrap();
    let reopened = project::open(&v4_path).unwrap();
    assert_eq!(
        reopened.recipe.voronoi.site(4).unwrap().target_color,
        [0.9, 0.1, 0.2]
    );
    assert_eq!(
        reopened.recipe.voronoi.site(7).unwrap().target_color,
        [0.1, 0.2, 0.8]
    );
}

#[test]
fn threshold_editor_targets_follow_space_and_link_semantics() {
    let mut threshold = Recipe::default().threshold;
    assert_eq!(
        threshold.edit_targets(),
        vec![ThresholdEditTarget::LinkedRgb]
    );
    threshold.active_space = ThresholdSpace::Hsv;
    assert_eq!(
        threshold.edit_targets(),
        vec![
            ThresholdEditTarget::Hue,
            ThresholdEditTarget::LinkedSaturationValue
        ]
    );
    assert_eq!(
        threshold.reconcile_edit_target(ThresholdEditTarget::Value),
        ThresholdEditTarget::LinkedSaturationValue
    );
    threshold.hsv_state.sv_link = LinkPolicy::Independent;
    assert_eq!(
        threshold.reconcile_edit_target(ThresholdEditTarget::LinkedSaturationValue),
        ThresholdEditTarget::Saturation
    );
}

#[test]
fn threshold_drag_motion_commits_only_once_on_release() {
    let mut gesture = ThresholdEditGesture::default();
    for _ in 0..100 {
        gesture.motion(true);
    }
    assert_eq!(gesture.commits(), 0);
    assert!(gesture.complete());
    assert_eq!(gesture.commits(), 1);
    assert!(!gesture.complete());
    assert_eq!(gesture.commits(), 1);
}

#[test]
fn threshold_boundary_clamp_preserves_strict_order_and_hue_units_round_trip() {
    let boundaries = [0.25, 0.5, 0.75];
    let clamped = clamp_threshold_boundary(&boundaries, 1, 0.9);
    assert!(clamped > boundaries[0] && clamped < boundaries[2]);
    assert!((threshold_display_value(0.25, true) - 90.0).abs() < f64::EPSILON);
    assert!((threshold_normalized_value(270.0, true) - 0.75).abs() < f32::EPSILON);
}

#[test]
fn linked_editor_target_copies_mapping_without_copying_process_flags() {
    let mut threshold = Recipe::default().threshold;
    threshold.active_space = ThresholdSpace::Hsv;
    threshold.hsv_state.saturation.enabled = false;
    threshold.hsv_state.value.enabled = true;
    threshold.set_edit_quantizer(
        ThresholdEditTarget::LinkedSaturationValue,
        ComponentQuantizer::evenly_spaced(7),
    );
    assert_eq!(
        threshold.hsv_state.saturation.boundaries,
        threshold.hsv_state.value.boundaries
    );
    assert_eq!(
        threshold.hsv_state.saturation.outputs,
        threshold.hsv_state.value.outputs
    );
    assert!(!threshold.hsv_state.saturation.enabled);
    assert!(threshold.hsv_state.value.enabled);
}

#[test]
fn threshold_dialog_cancel_restores_state_and_prior_dirty_flag_only_after_edits() {
    let original = Recipe::default().threshold;
    let mut state = original.clone();
    let mut dirty = false;
    let no_op = ThresholdEditTransaction::new(&state, dirty);
    assert!(!no_op.cancel(&mut state, &mut dirty));

    let mut transaction = ThresholdEditTransaction::new(&state, dirty);
    state.set_edit_quantizer(
        ThresholdEditTarget::LinkedRgb,
        ComponentQuantizer::evenly_spaced(8),
    );
    dirty = true;
    transaction.record_completed_edit();
    assert!(transaction.cancel(&mut state, &mut dirty));
    assert_eq!(state, original);
    assert!(!dirty);
}
