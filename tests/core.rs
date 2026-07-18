use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::{Duration, Instant};

use image::{ImageBuffer, ImageFormat, Rgba};
use threshiator::color::{
    ColorModel, DraftColor, PickerTransaction, PlaneKey, adjust_okhsl, encoded_to_hsl,
    encoded_to_hsv, encoded_to_linear, hsl_to_encoded, hsv_to_encoded, in_srgb_gamut,
    linear_to_encoded, linear_to_okhsl, linear_to_okhsl_cylinder, linear_to_oklab, okhsl_to_linear,
    oklab_to_linear, parse_hex,
};
use threshiator::document::{
    ComponentQuantizer, Document, ExportDefaults, LinkPolicy, Method, ProfileInterpretation,
    Recipe, SampleSize, ThresholdEditGesture, ThresholdEditTarget, ThresholdEditTransaction,
    ThresholdEncoding, ThresholdSpace, ThresholdState, VoronoiMatching, VoronoiSite, VoronoiState,
    clamp_threshold_boundary, threshold_display_value, threshold_normalized_value,
};
use threshiator::export::{self, ExportFormat};
use threshiator::histogram::{
    HsvHistogramComponent, THRESHOLD_HISTOGRAM_BINS, ThresholdHistograms, hsv_input_strip_color,
    hsv_output_strip_color, relative_hue_histogram, threshold_histogram_series_mask,
};
use threshiator::preset::{Preset, PresetStore, apply_to_document, validate_name};
use threshiator::processing::{
    PREVIEW_MAX_DIMENSION, bounded_preview, gaussian_blur_cancellable, process,
    process_cancellable_with_progress_and_coverage, rotate_oklch, srgb_to_linear, to_display_rgba8,
};
use threshiator::project;
use threshiator::raster;
use threshiator::scheduler::{JobCoordinator, LatestGeneration, PreviewScheduler, ProgressTracker};
use threshiator::starter_looks::{
    STARTER_LOOKS, apply_starter_look, recipe_for_starter_look, reset_active_space,
    reset_component, reset_method, rgb_state,
};
use threshiator::voronoi::{auto_initialize, linear_rgb_to_oklab, reattach_site, sample_color};
use threshiator::workflow::{
    OpenKind, ReplacementDecision, SaveResolution, classify_open_path, ensure_project_extension,
    replacement_decision, resolve_pending_after_save,
};
use zip::write::SimpleFileOptions;

#[test]
fn starter_looks_recreate_exact_archived_normalized_rgb_fixtures() {
    let expected = [
        (true, vec![vec![0, 128, 255]; 3]),
        (false, vec![vec![0, 0], vec![0, 0], vec![0, 255]]),
        (
            false,
            vec![
                vec![112, 176, 240, 255],
                vec![80, 144, 208, 224],
                vec![48, 112, 176, 192],
            ],
        ),
        (true, vec![vec![0, 85, 170, 255]; 3]),
        (false, vec![vec![255, 0], vec![0, 255], vec![0, 255]]),
    ];
    let boundaries = [
        vec![85, 170],
        vec![128],
        vec![64, 128, 192],
        vec![64, 128, 192],
        vec![128],
    ];
    assert!(STARTER_LOOKS.len() >= 13);
    for (index, look) in STARTER_LOOKS[..5].iter().copied().enumerate() {
        let state = rgb_state(look);
        assert_eq!(state.encoding, ThresholdEncoding::EncodedSrgb);
        assert_eq!(state.link == LinkPolicy::Linked, expected[index].0);
        for (channel, component) in state.components.iter().enumerate() {
            assert_eq!(
                component.boundaries,
                boundaries[index]
                    .iter()
                    .map(|value| *value as f32 / 255.0)
                    .collect::<Vec<_>>()
            );
            assert_eq!(
                component.outputs,
                expected[index].1[channel]
                    .iter()
                    .map(|value| *value as f32 / 255.0)
                    .collect::<Vec<_>>()
            );
        }
    }
}

#[test]
fn built_in_presets_have_stable_unique_ids_valid_recipes_and_useful_output() {
    let mut ids = std::collections::BTreeSet::new();
    let mut names = std::collections::BTreeSet::new();
    let document = threshiator::example::spectrum_document().unwrap();
    let mut threshold_count = 0;
    let mut voronoi_count = 0;
    for look in STARTER_LOOKS.iter().copied() {
        assert!(ids.insert(look.id), "duplicate built-in id {}", look.id);
        assert!(
            names.insert(look.name),
            "duplicate built-in name {}",
            look.name
        );
        let recipe = recipe_for_starter_look(look);
        recipe.threshold.validate().unwrap();
        recipe.voronoi.validate().unwrap();
        let preset = Preset::new(look.name, Some(look.description.into()), &recipe).unwrap();
        let preset_json = serde_json::to_vec(&preset).unwrap();
        let reopened: Preset = serde_json::from_slice(&preset_json).unwrap();
        reopened.validate().unwrap();
        assert_eq!(reopened.recipe(), recipe);
        let output = process(&document.source, &recipe);
        let unique = output
            .pixels
            .iter()
            .map(|pixel| pixel.map(f32::to_bits))
            .collect::<std::collections::BTreeSet<_>>();
        assert!(unique.len() >= 2, "{} collapsed to one output", look.name);
        match look.method {
            Method::Thresholds => threshold_count += 1,
            Method::Voronoi => {
                voronoi_count += 1;
                assert!(recipe.voronoi.sites.len() >= 4);
                assert!(
                    recipe
                        .voronoi
                        .sites
                        .iter()
                        .all(|site| site.position.is_none())
                );
            }
        }
    }
    assert!(threshold_count >= 8);
    assert!(voronoi_count >= 5);
}

#[test]
fn starter_apply_is_scoped_dirty_and_idempotent() {
    let mut document = threshiator::example::spectrum_document().unwrap();
    let identity = (
        document.source_name.clone(),
        document.source_bytes.clone(),
        document.source.clone(),
        document.interpretation.clone(),
        document.export_defaults.clone(),
        document.recipe.voronoi.clone(),
        document.recipe.steps.clone(),
    );
    document.recipe.threshold.hsv_state.hue = ComponentQuantizer::evenly_spaced(9);
    document.recipe.threshold.input_smoothing = 4.0;
    assert!(apply_starter_look(&mut document, 2));
    assert_eq!(document.recipe.threshold.input_smoothing, 0.0);
    assert!(document.dirty);
    assert_eq!(document.recipe.active_method, Method::Thresholds);
    assert_eq!(document.recipe.threshold.active_space, ThresholdSpace::Rgb);
    assert_eq!(
        document.recipe.threshold.hsv_state,
        Recipe::default().threshold.hsv_state
    );
    assert_eq!(
        (
            document.source_name.clone(),
            document.source_bytes.clone(),
            document.source.clone(),
            document.interpretation.clone(),
            document.export_defaults.clone(),
            document.recipe.voronoi.clone(),
            document.recipe.steps.clone(),
        ),
        identity
    );
    assert!(!apply_starter_look(&mut document, 2));
    assert!(!apply_starter_look(&mut document, usize::MAX));
}

#[test]
fn gaussian_smoothing_zero_constant_impulse_alpha_and_cancellation() {
    let generation = AtomicU64::new(1);
    let source =
        threshiator::document::PixelImage::new(5, 5, vec![[0.2, 0.4, 0.6, 1.0]; 25]).unwrap();
    assert!(
        gaussian_blur_cancellable(&source, 0.0, 1, &generation, |_| {})
            .unwrap()
            .shares_storage_with(&source)
    );
    let constant = gaussian_blur_cancellable(&source, 2.0, 1, &generation, |_| {}).unwrap();
    for pixel in constant.pixels.iter() {
        for (actual, expected) in pixel.iter().zip(source.pixels[0]) {
            assert!((*actual - expected).abs() < 1.0e-5);
        }
    }
    let mut impulse = vec![[0.0, 0.0, 0.0, 0.0]; 25];
    impulse[12] = [1.0, 0.0, 0.0, 1.0];
    let impulse = threshiator::document::PixelImage::new(5, 5, impulse).unwrap();
    let blurred = gaussian_blur_cancellable(&impulse, 1.0, 1, &generation, |_| {}).unwrap();
    assert!((blurred.pixels[11][3] - blurred.pixels[13][3]).abs() < 1.0e-6);
    assert!((blurred.pixels[7][3] - blurred.pixels[17][3]).abs() < 1.0e-6);
    for pixel in blurred.pixels.iter().filter(|pixel| pixel[3] > 1.0e-8) {
        assert!((pixel[0] - 1.0).abs() < 1.0e-5);
        assert_eq!(pixel[1], 0.0);
        assert_eq!(pixel[2], 0.0);
    }
    let transparent =
        threshiator::document::PixelImage::new(1, 1, vec![[1.0, 1.0, 1.0, 0.0]]).unwrap();
    assert_eq!(
        gaussian_blur_cancellable(&transparent, 1.0, 1, &generation, |_| {})
            .unwrap()
            .pixels[0],
        [0.0; 4]
    );
    generation.store(2, std::sync::atomic::Ordering::Release);
    assert!(gaussian_blur_cancellable(&source, 1.0, 1, &generation, |_| {}).is_none());
}

#[test]
fn smoothing_affects_both_methods_and_validates_range() {
    let source = threshiator::document::PixelImage::new(
        3,
        1,
        vec![
            [0.0, 0.0, 0.0, 1.0],
            [1.0, 1.0, 1.0, 1.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    )
    .unwrap();
    for method in [Method::Thresholds, Method::Voronoi] {
        let mut recipe = Recipe {
            active_method: method,
            ..Recipe::default()
        };
        recipe.threshold.input_smoothing = 2.0;
        let smoothed = process(&source, &recipe);
        recipe.threshold.input_smoothing = 0.0;
        assert_ne!(smoothed, process(&source, &recipe));
    }
    let mut threshold = Recipe::default().threshold;
    threshold.input_smoothing = 11.0;
    assert!(threshold.validate().is_err());
}

#[test]
fn threshold_resets_obey_component_space_method_and_link_scope() {
    let mut threshold = Recipe::default().threshold;
    threshold.rgb_state.link = LinkPolicy::Independent;
    threshold.rgb_state.components[0] = ComponentQuantizer::evenly_spaced(8);
    threshold.rgb_state.components[1] = ComponentQuantizer::evenly_spaced(7);
    threshold.rgb_state.components[2] = ComponentQuantizer::evenly_spaced(6);
    threshold.rgb_state.components[1].enabled = false;
    let blue = threshold.rgb_state.components[2].clone();
    assert!(reset_component(&mut threshold, ThresholdEditTarget::Green));
    assert_eq!(threshold.rgb_state.components[0].outputs.len(), 8);
    assert_eq!(threshold.rgb_state.components[1].outputs.len(), 3);
    assert!(!threshold.rgb_state.components[1].enabled);
    assert_eq!(threshold.rgb_state.components[2], blue);

    threshold.rgb_state.link = LinkPolicy::Linked;
    threshold.rgb_state.components[0] = ComponentQuantizer::evenly_spaced(9);
    threshold.rgb_state.components[1] = ComponentQuantizer::evenly_spaced(9);
    threshold.rgb_state.components[2] = ComponentQuantizer::evenly_spaced(9);
    threshold.rgb_state.components[2].enabled = false;
    assert!(reset_component(&mut threshold, ThresholdEditTarget::Red));
    assert!(
        threshold
            .rgb_state
            .components
            .iter()
            .all(|component| component.outputs.len() == 3)
    );
    assert!(!threshold.rgb_state.components[2].enabled);

    threshold.active_space = ThresholdSpace::Hsv;
    threshold.hsv_state.hue = ComponentQuantizer::evenly_spaced(10);
    let rgb_before = threshold.rgb_state.clone();
    assert!(reset_active_space(&mut threshold));
    assert_eq!(threshold.rgb_state, rgb_before);
    assert_eq!(threshold.hsv_state, Recipe::default().threshold.hsv_state);
    assert!(!reset_active_space(&mut threshold));

    threshold.rgb_state.components[0] = ComponentQuantizer::evenly_spaced(11);
    assert!(reset_method(&mut threshold));
    assert_eq!(threshold, Recipe::default().threshold);
    assert!(!reset_method(&mut threshold));
}

fn tiny_source() -> (Vec<u8>, raster::DecodedRaster) {
    let image = ImageBuffer::from_pixel(2, 1, Rgba([64_u8, 128, 192, 127]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, ImageFormat::Png).unwrap();
    let bytes = bytes.into_inner();
    let decoded = raster::decode(&bytes, None).unwrap();
    (bytes, decoded)
}

fn png_with_icc(rgb: [u8; 3], profile: &moxcms::ColorProfile) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut info = png::Info::with_size(1, 1);
    info.color_type = png::ColorType::Rgb;
    info.bit_depth = png::BitDepth::Eight;
    info.icc_profile = Some(std::borrow::Cow::Owned(profile.encode().unwrap()));
    let encoder = png::Encoder::with_info(&mut bytes, info).unwrap();
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(&rgb).unwrap();
    writer.finish().unwrap();
    bytes
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
        VoronoiMatching::Okhsl,
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
    assert_eq!(scan.entries.len(), 4);
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
fn preset_v1_is_rejected_without_rewriting_and_v2_roundtrips_locks() {
    let root = tempfile::tempdir().unwrap();
    let store = PresetStore::at(root.path());
    let mut legacy =
        serde_json::to_value(Preset::new("Legacy", None, &preset_recipe()).unwrap()).unwrap();
    legacy["version"] = serde_json::json!(1);
    legacy["processing"]["threshold"]["rgb_state"]
        .as_object_mut()
        .unwrap()
        .remove("locks");
    legacy["processing"]["threshold"]["hsv_state"]
        .as_object_mut()
        .unwrap()
        .remove("locks");
    let legacy_path = root.path().join("legacy.json");
    let legacy_bytes = serde_json::to_vec_pretty(&legacy).unwrap();
    fs::write(&legacy_path, &legacy_bytes).unwrap();
    let scan = store.scan().unwrap();
    assert!(scan.entries.is_empty());
    assert_eq!(scan.diagnostics.len(), 1);
    assert!(scan.diagnostics[0].reason.contains("only version 2"));
    assert_eq!(fs::read(&legacy_path).unwrap(), legacy_bytes);

    let mut recipe = preset_recipe();
    recipe.threshold.rgb_state.locks = [true, false, true];
    recipe.threshold.hsv_state.locks = [false, true, false];
    let current = Preset::new("Current", None, &recipe).unwrap();
    assert_eq!(current.version, 2);
    store.save(&current, false).unwrap();
    let scan = store.scan().unwrap();
    let reopened = &scan
        .entries
        .iter()
        .find(|entry| entry.preset.name == "Current")
        .unwrap()
        .preset;
    assert_eq!(
        reopened.processing.threshold.rgb_state.locks,
        [true, false, true]
    );
    assert_eq!(
        reopened.processing.threshold.hsv_state.locks,
        [false, true, false]
    );
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
fn embedded_rgb_profiles_are_converted_to_canonical_srgb_before_linearization() {
    let encoded = [128_u8, 102, 77];
    let srgb = raster::decode(
        &png_with_icc(encoded, &moxcms::ColorProfile::new_srgb()),
        None,
    )
    .unwrap();
    assert_eq!(
        srgb.interpretation.profile,
        ProfileInterpretation::EmbeddedProfileConvertedToSrgb
    );
    for (actual, encoded) in srgb.pixels.pixels[0][..3].iter().zip(encoded) {
        assert!((*actual - srgb_to_linear(encoded as f32 / 255.0)).abs() < 3.0e-3);
    }

    let p3 = raster::decode(
        &png_with_icc(encoded, &moxcms::ColorProfile::new_display_p3()),
        None,
    )
    .unwrap();
    assert_eq!(
        p3.interpretation.profile,
        ProfileInterpretation::EmbeddedProfileConvertedToSrgb
    );

    // Independent Display-P3-linear -> XYZ(D65) -> linear-sRGB reference matrices. This checks
    // the source profile affects the canonical pixels rather than merely exercising the CMS API.
    let p3_linear = encoded.map(|channel| srgb_to_linear(channel as f32 / 255.0) as f64);
    let xyz = [
        0.486_570_948_648_216_2 * p3_linear[0]
            + 0.265_667_693_169_093_06 * p3_linear[1]
            + 0.198_217_285_234_362_5 * p3_linear[2],
        0.228_974_564_069_748_8 * p3_linear[0]
            + 0.691_738_521_836_506_4 * p3_linear[1]
            + 0.079_286_914_093_745 * p3_linear[2],
        0.045_113_381_858_902_64 * p3_linear[1] + 1.043_944_368_900_976 * p3_linear[2],
    ];
    let expected = [
        3.240_969_941_904_522_6 * xyz[0]
            - 1.537_383_177_570_094 * xyz[1]
            - 0.498_610_760_293_003_4 * xyz[2],
        -0.969_243_636_280_879_6 * xyz[0]
            + 1.875_967_501_507_720_2 * xyz[1]
            + 0.041_555_057_407_175 * xyz[2],
        0.055_630_079_696_993_66 * xyz[0] - 0.203_976_958_888_976_52 * xyz[1]
            + 1.056_971_514_242_878_6 * xyz[2],
    ];
    for (actual, expected) in p3.pixels.pixels[0][..3].iter().zip(expected) {
        assert!((*actual as f64 - expected).abs() < 4.0e-3);
    }
    assert_ne!(p3.pixels.pixels[0][..3], srgb.pixels.pixels[0][..3]);
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
fn hsv_matching_cone_maps_near_black_pixels_to_the_darkest_site() {
    let linear_rgb8 = |rgb: [u8; 3]| {
        encoded_to_linear(rgb.map(|channel| f64::from(channel) / 255.0))
            .map(|channel| channel as f32)
    };
    let source_rgb = linear_rgb8([1, 1, 1]);
    let source = threshiator::document::PixelImage::new(
        1,
        1,
        vec![[source_rgb[0], source_rgb[1], source_rgb[2], 1.0]],
    )
    .unwrap();
    let mut recipe = Recipe::default();
    recipe.voronoi.matching = VoronoiMatching::Hsv;
    recipe.voronoi.sites = vec![
        sample(1, linear_rgb8([80, 155, 180]), [0.9, 0.1, 0.1], 0.0),
        sample(2, linear_rgb8([1, 3, 34]), [0.1, 0.9, 0.1], 0.0),
    ];
    assert_eq!(&process(&source, &recipe).pixels[0][..3], &[0.1, 0.9, 0.1]);
}

#[test]
fn hsv_cone_reference_fixture_is_deterministic_and_alpha_safe() {
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
        VoronoiMatching::Okhsl,
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
fn oklab_known_colors_round_trip_and_use_unscaled_cartesian_axes() {
    let references = [
        ([0.0, 0.0, 0.0], [0.0, 0.0, 0.0]),
        ([1.0, 1.0, 1.0], [1.0, 0.0, 0.0]),
        ([1.0, 0.0, 0.0], [0.627_955_36, 0.224_863_06, 0.125_846_30]),
        ([0.0, 1.0, 0.0], [0.866_439_61, -0.233_887_57, 0.179_498_48]),
        (
            [0.0, 0.0, 1.0],
            [0.452_013_72, -0.032_456_98, -0.311_528_15],
        ),
    ];
    for (linear, expected) in references {
        let lab = linear_rgb_to_oklab(linear.map(|value| value as f32));
        for (actual, expected) in lab.into_iter().zip(expected) {
            assert!((actual - expected).abs() < 2.0e-7);
        }
        let round_trip = oklab_to_linear(lab);
        for (actual, expected) in round_trip.into_iter().zip(linear) {
            assert!((actual - expected).abs() < 5.0e-7);
        }
    }

    // Raw OKLab says A wins: dA²=.05²+.03²=.0034; dB²=.07²=.0049. Independently
    // normalizing chroma axes would incorrectly choose B.
    let pixel_lab = [0.65, 0.02, 0.02];
    let site_a_lab = [0.60, 0.05, 0.02];
    let site_b_lab = [0.72, 0.02, 0.02];
    let source_rgb = oklab_to_linear(pixel_lab).map(|value| value as f32);
    let site_a = oklab_to_linear(site_a_lab).map(|value| value as f32);
    let site_b = oklab_to_linear(site_b_lab).map(|value| value as f32);
    assert!(
        source_rgb
            .iter()
            .chain(site_a.iter())
            .chain(site_b.iter())
            .all(|v| (0.0..=1.0).contains(v))
    );
    let source = threshiator::document::PixelImage::new(
        1,
        1,
        vec![[source_rgb[0], source_rgb[1], source_rgb[2], 1.0]],
    )
    .unwrap();
    let mut recipe = Recipe::default();
    recipe.voronoi.matching = VoronoiMatching::Perceptual;
    recipe.voronoi.sites = vec![
        sample(1, site_a, [1.0, 0.0, 0.0], 0.0),
        sample(2, site_b, [0.0, 0.0, 1.0], 0.0),
    ];
    assert_eq!(process(&source, &recipe).pixels[0][..3], [1.0, 0.0, 0.0]);
}

#[test]
fn full_source_distribution_prevents_checkerboard_proxy_aliasing() {
    let colors = [[0.85, 0.05, 0.05, 1.0], [0.05, 0.05, 0.85, 1.0]];
    let pixels = (0..6400).map(|index| colors[index % 2]).collect();
    let source = threshiator::document::PixelImage::new(3200, 2, pixels).unwrap();
    let proxy = bounded_preview(&source);
    assert!(proxy.pixels.iter().all(|pixel| *pixel == colors[0]));
    let sites = auto_initialize(&proxy, &source);
    assert_eq!(sites.sites.len(), 2);
    assert!(
        sites
            .sites
            .iter()
            .all(|site| site.size == SampleSize::Point)
    );
    for color in colors {
        assert!(sites.sites.iter().any(|site| site.source_color == color));
    }
}

#[test]
fn perceptual_influence_scales_complete_oklab_distance_squared() {
    let pixel_lab = [0.60, 0.0, 0.0];
    let far_lab = [0.42, 0.0, 0.0];
    let near_lab = [0.65, 0.0, 0.0];
    let pixel = oklab_to_linear(pixel_lab).map(|value| value as f32);
    let far = oklab_to_linear(far_lab).map(|value| value as f32);
    let near = oklab_to_linear(near_lab).map(|value| value as f32);
    let source =
        threshiator::document::PixelImage::new(1, 1, vec![[pixel[0], pixel[1], pixel[2], 1.0]])
            .unwrap();
    let mut recipe = Recipe::default();
    recipe.voronoi.matching = VoronoiMatching::Perceptual;
    recipe.voronoi.sites = vec![
        sample(1, far, [1.0, 0.0, 0.0], 0.0),
        sample(2, near, [0.0, 0.0, 1.0], 0.0),
    ];
    assert_eq!(process(&source, &recipe).pixels[0][..3], [0.0, 0.0, 1.0]);

    // .18² / 2⁴ = .002025, which beats the near site's .05² = .0025.
    recipe.voronoi.sites[0].influence = 4.0;
    assert_eq!(process(&source, &recipe).pixels[0][..3], [1.0, 0.0, 0.0]);

    // At -4 the same site's weighted score is .18² × 2⁴, so the near site wins again.
    recipe.voronoi.sites[0].influence = -4.0;
    assert_eq!(process(&source, &recipe).pixels[0][..3], [0.0, 0.0, 1.0]);
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
    assert_eq!(ColorModel::default(), ColorModel::Okhsl);
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
        let linear = encoded_to_linear(rgb);
        let okhsl = linear_to_okhsl(linear).unwrap();
        let roundtrip = okhsl_to_linear(okhsl).unwrap();
        for channel in 0..3 {
            assert!((roundtrip[channel] - linear[channel]).abs() < 2e-7);
        }
    }
    let mut draft = DraftColor::new([0.5, 0.5, 0.5]);
    draft.set_values(ColorModel::Okhsl, [287.0, 0.0, 0.7]);
    assert!((draft.values(ColorModel::Okhsl)[0] - 287.0).abs() < 1e-9);
    draft.set_values(ColorModel::Okhsl, [287.0, 0.8, 0.7]);
    let restored_hue = draft.values(ColorModel::Okhsl)[0];
    assert!((restored_hue - 287.0).abs() < 1e-5, "{restored_hue}");
}

#[test]
fn picker_models_share_one_encoded_srgb_color_without_switch_drift() {
    let expected = [
        0x50 as f64 / 255.0,
        0x9b as f64 / 255.0,
        0xb4 as f64 / 255.0,
    ];
    let linear = encoded_to_linear(expected);
    let draft = DraftColor::new(linear.map(|channel| channel as f32));
    let canonical = draft.encoded();
    assert_eq!(draft, {
        let _ = draft.values(ColorModel::Okhsl);
        let _ = draft.values(ColorModel::Hsv);
        let _ = draft.values(ColorModel::Hsl);
        draft
    });
    for model in [ColorModel::Okhsl, ColorModel::Hsv, ColorModel::Hsl] {
        let mut reconstructed = draft;
        reconstructed.set_values(model, draft.values(model));
        for (actual, expected) in reconstructed.encoded().into_iter().zip(canonical) {
            assert!(
                (actual - expected).abs() < 2.0e-6,
                "{model:?}: {actual} != {expected}"
            );
        }
        assert_eq!(
            reconstructed
                .encoded()
                .map(|channel| (channel * 255.0).round() as u8),
            [0x50, 0x9b, 0xb4]
        );
    }
    for (actual, expected) in linear_to_encoded(draft.linear).into_iter().zip(canonical) {
        assert!((actual - expected).abs() < f64::EPSILON);
    }
}

#[test]
fn neutral_hue_memory_is_independent_for_hsv_hsl_and_okhsl() {
    let mut draft = DraftColor::new([0.5, 0.5, 0.5]);
    draft.set_values(ColorModel::Hsv, [25.0, 0.0, 0.5]);
    draft.set_values(ColorModel::Hsl, [145.0, 0.0, 0.5]);
    draft.set_values(ColorModel::Okhsl, [285.0, 0.0, 0.5]);
    assert!((draft.values(ColorModel::Hsv)[0] - 25.0).abs() < 1.0e-9);
    assert!((draft.values(ColorModel::Hsl)[0] - 145.0).abs() < 1.0e-9);
    assert!((draft.values(ColorModel::Okhsl)[0] - 285.0).abs() < 1.0e-9);
}

#[test]
fn okhsl_matches_reference_values_and_rejects_invalid_inputs() {
    let reference = [0.12, 0.53, 0.81];
    let roundtrip = oklab_to_linear(linear_to_oklab(reference));
    for channel in 0..3 {
        assert!(
            (roundtrip[channel] - reference[channel]).abs() < 2e-7,
            "channel {channel}: reference={}, roundtrip={}",
            reference[channel],
            roundtrip[channel]
        );
    }
    let brown = linear_to_okhsl(encoded_to_linear([
        0x83 as f64 / 255.0,
        0x49 as f64 / 255.0,
        0x41 as f64 / 255.0,
    ]))
    .unwrap();
    assert!(
        (brown[0] * 360.0 - 28.773_829_336_976_384).abs() < 1e-5,
        "brown OKHSL: {brown:?}"
    );
    assert!((brown[1] - 0.462_921_718_345_498_6).abs() < 1e-7);
    assert!((brown[2] - 0.390_099_814_614_742_7).abs() < 1e-7);
    let brown_cylinder = linear_to_okhsl_cylinder(encoded_to_linear([
        0x83 as f64 / 255.0,
        0x49 as f64 / 255.0,
        0x41 as f64 / 255.0,
    ]))
    .unwrap();
    for (actual, expected) in brown_cylinder.into_iter().zip([
        0.405_763_216_731_3,
        0.222_828_923_759_5,
        0.390_099_814_614_7,
    ]) {
        assert!((actual - expected).abs() < 2e-6, "{brown_cylinder:?}");
    }
    let blue = linear_to_okhsl([0.0, 0.0, 1.0]).unwrap();
    assert!((blue[0] * 360.0 - 264.052_021_148_125_16).abs() < 1e-5);
    assert!((blue[1] - 0.999_999_989_726_226_1).abs() < 1e-7);
    assert!((blue[2] - 0.366_565_335_813_274).abs() < 1e-7);
    let blue_cylinder = linear_to_okhsl_cylinder([0.0, 0.0, 1.0]).unwrap();
    for (actual, expected) in blue_cylinder.into_iter().zip([
        -0.103_625_452_719_4,
        -0.994_616_380_822_9,
        0.366_565_335_813_3,
    ]) {
        assert!((actual - expected).abs() < 2e-6, "{blue_cylinder:?}");
    }
    let encoded = okhsl_to_linear([0.0, 0.5, 0.5])
        .unwrap()
        .map(|channel| threshiator::processing::linear_to_srgb(channel as f32));
    assert_eq!(
        encoded.map(|channel| (channel * 255.0).round() as u8),
        [0xaa, 0x5a, 0x74]
    );
    assert!(okhsl_to_linear([f64::NAN, 0.5, 0.5]).is_none());
    assert!(linear_to_okhsl([1.1, 0.0, 0.0]).is_none());
    assert_eq!(parse_hex("#abc").unwrap(), parse_hex("#AABBCC").unwrap());
    assert!(parse_hex("#abcd").is_err());
    assert!(parse_hex("#GG0000").is_err());
}

#[test]
fn okhsl_fills_the_complete_picker_domain_and_keyboard_is_polar() {
    for hue_degrees in (0..360).step_by(5) {
        for saturation in [0.0, 0.25, 0.799_999, 0.8, 0.800_001, 1.0] {
            for lightness in [0.0, 0.01, 0.25, 0.5, 0.75, 0.99, 1.0] {
                let linear = okhsl_to_linear([hue_degrees as f64 / 360.0, saturation, lightness])
                    .unwrap_or_else(|| {
                        panic!("gap at H={hue_degrees}, S={saturation}, L={lightness}")
                    });
                assert!(linear.iter().all(|channel| channel.is_finite()));
                assert!(in_srgb_gamut(linear));
            }
        }
    }
    for red in 0..=10 {
        for green in 0..=10 {
            for blue in 0..=10 {
                let linear = [red as f64 / 10.0, green as f64 / 10.0, blue as f64 / 10.0];
                let roundtrip = okhsl_to_linear(linear_to_okhsl(linear).unwrap()).unwrap();
                for channel in 0..3 {
                    assert!(
                        (roundtrip[channel] - linear[channel]).abs() < 5e-7,
                        "linear={linear:?}, roundtrip={roundtrip:?}, channel={channel}"
                    );
                }
            }
        }
    }

    let neutral = [287.0, 0.0, 0.65];
    assert_eq!(
        adjust_okhsl(neutral, PlaneKey::Right, false),
        [288.0, 0.0, 0.65]
    );
    assert_eq!(
        adjust_okhsl(neutral, PlaneKey::Left, true),
        [286.9, 0.0, 0.65]
    );
    assert_eq!(
        adjust_okhsl(neutral, PlaneKey::Up, false),
        [287.0, 0.01, 0.65]
    );
    assert_eq!(
        adjust_okhsl([287.0, 0.5, 0.65], PlaneKey::Down, true),
        [287.0, 0.499, 0.65]
    );
    assert_eq!(
        adjust_okhsl([287.0, 1.0, 0.65], PlaneKey::Home, false),
        neutral
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
fn threshold_histograms_use_float_rgb_and_straight_alpha_weights() {
    let encoded = [0.25, 0.5, 0.75];
    let linear = encoded.map(srgb_to_linear);
    let source = threshiator::document::PixelImage::new(
        3,
        1,
        vec![
            [linear[0], linear[1], linear[2], 1.0],
            [0.25, 0.5, 0.75, 0.5],
            [1.0, 0.0, 1.0, 0.0],
        ],
    )
    .unwrap();
    let histogram = ThresholdHistograms::build(&source);
    let bin = |value: f32| {
        (value.clamp(0.0, 1.0) * (THRESHOLD_HISTOGRAM_BINS - 1) as f32).round() as usize
    };

    assert_eq!(histogram.visible_alpha, 1.5);
    for (channel, encoded_value) in encoded.into_iter().enumerate() {
        assert_eq!(histogram.encoded_rgb[channel][bin(encoded_value)], 1.0);
    }
    let encoded_partial = [0.25, 0.5, 0.75].map(threshiator::processing::linear_to_srgb);
    for (channel, encoded_value) in encoded_partial.into_iter().enumerate() {
        assert_eq!(histogram.encoded_rgb[channel][bin(encoded_value)], 0.5);
    }
    assert_eq!(histogram.encoded_rgb[0][THRESHOLD_HISTOGRAM_BINS - 1], 0.0);
    assert_eq!(histogram.encoded_rgb[1][0], 0.0);
}

#[test]
fn threshold_histograms_keep_linked_rgb_distributions_distinct() {
    let encoded = [0.1, 0.5, 0.9];
    let source = threshiator::document::PixelImage::new(
        1,
        1,
        vec![[
            srgb_to_linear(encoded[0]),
            srgb_to_linear(encoded[1]),
            srgb_to_linear(encoded[2]),
            1.0,
        ]],
    )
    .unwrap();
    let histogram = ThresholdHistograms::build(&source);

    assert_ne!(histogram.encoded_rgb[0], histogram.encoded_rgb[1]);
    assert_ne!(histogram.encoded_rgb[1], histogram.encoded_rgb[2]);
    assert_eq!(
        threshold_histogram_series_mask(ThresholdEditTarget::Red),
        [true, false, false]
    );
    assert_eq!(
        threshold_histogram_series_mask(ThresholdEditTarget::LinkedRgb),
        [true, true, true]
    );
}

#[test]
fn threshold_histogram_visibility_follows_semantic_target_only() {
    for (target, expected) in [
        (ThresholdEditTarget::Red, [true, false, false]),
        (ThresholdEditTarget::Green, [false, true, false]),
        (ThresholdEditTarget::Blue, [false, false, true]),
        (ThresholdEditTarget::LinkedRgb, [true, true, true]),
        (ThresholdEditTarget::Hue, [true, false, false]),
        (ThresholdEditTarget::Saturation, [false, true, false]),
        (ThresholdEditTarget::Value, [false, false, true]),
        (
            ThresholdEditTarget::LinkedSaturationValue,
            [false, true, true],
        ),
    ] {
        assert_eq!(threshold_histogram_series_mask(target), expected);
    }
}

#[test]
fn threshold_histograms_keep_exact_hsv_channels_and_alpha_policy() {
    let linear_hsv = |hue, saturation, value, alpha| {
        let encoded = hsv_to_encoded([hue, saturation, value]);
        [
            srgb_to_linear(encoded[0] as f32),
            srgb_to_linear(encoded[1] as f32),
            srgb_to_linear(encoded[2] as f32),
            alpha,
        ]
    };
    let source = threshiator::document::PixelImage::new(
        4,
        1,
        vec![
            linear_hsv(0.0, 1.0, 1.0, 1.0),
            linear_hsv(120.0, 0.5, 0.75, 0.5),
            linear_hsv(0.0, 0.0, 0.4, 0.25),
            linear_hsv(240.0, 1.0, 1.0, 0.0),
        ],
    )
    .unwrap();
    let histogram = ThresholdHistograms::build(&source);
    let unit_bin = |value: f64| {
        (value.clamp(0.0, 1.0) * (THRESHOLD_HISTOGRAM_BINS - 1) as f64).round() as usize
    };
    let hue_bin = |degrees: f64| {
        ((degrees.rem_euclid(360.0) / 360.0) * THRESHOLD_HISTOGRAM_BINS as f64).floor() as usize
            % THRESHOLD_HISTOGRAM_BINS
    };

    assert_eq!(histogram.visible_alpha, 1.75);
    assert_eq!(histogram.hsv[0].iter().sum::<f64>(), 1.5);
    assert_eq!(histogram.hsv[0][hue_bin(0.0)], 1.0);
    assert_eq!(histogram.hsv[0][hue_bin(120.0)], 0.5);
    assert_eq!(histogram.hsv[0][hue_bin(240.0)], 0.0);
    assert_eq!(histogram.hsv[1][unit_bin(0.0)], 0.25);
    assert_eq!(histogram.hsv[1][unit_bin(0.5)], 0.5);
    assert_eq!(histogram.hsv[1][unit_bin(1.0)], 1.0);
    assert_eq!(histogram.hsv[2][unit_bin(0.4)], 0.25);
    assert_eq!(histogram.hsv[2][unit_bin(0.75)], 0.5);
    assert_eq!(histogram.hsv[2][unit_bin(1.0)], 1.0);
}

#[test]
fn threshold_hue_histogram_rotates_at_seam_and_has_deterministic_mean() {
    let hue_bin = |degrees: f64| {
        ((degrees.rem_euclid(360.0) / 360.0) * THRESHOLD_HISTOGRAM_BINS as f64).floor() as usize
            % THRESHOLD_HISTOGRAM_BINS
    };
    let mut absolute = vec![0.0; THRESHOLD_HISTOGRAM_BINS];
    absolute[hue_bin(359.0)] = 2.0;
    absolute[hue_bin(1.0)] = 3.0;
    let relative = relative_hue_histogram(&absolute, 0.0);
    assert_eq!(relative[hue_bin(359.0)], 2.0);
    assert_eq!(relative[hue_bin(1.0)], 3.0);
    assert_eq!(relative.iter().sum::<f64>(), 5.0);

    let mut shifted_absolute = vec![0.0; THRESHOLD_HISTOGRAM_BINS];
    shifted_absolute[hue_bin(90.0)] = 4.0;
    shifted_absolute[hue_bin(10.0)] = 6.0;
    let shifted_relative = relative_hue_histogram(&shifted_absolute, 45.0);
    assert_eq!(shifted_relative[hue_bin(45.0)], 4.0);
    assert_eq!(shifted_relative[hue_bin(325.0)], 6.0);
    assert_eq!(shifted_relative.iter().sum::<f64>(), 10.0);

    let linear_hue = |hue| {
        let encoded = hsv_to_encoded([hue, 1.0, 1.0]);
        [
            srgb_to_linear(encoded[0] as f32),
            srgb_to_linear(encoded[1] as f32),
            srgb_to_linear(encoded[2] as f32),
            1.0,
        ]
    };
    let seam_source =
        threshiator::document::PixelImage::new(2, 1, vec![linear_hue(350.0), linear_hue(10.0)])
            .unwrap();
    let seam_mean = ThresholdHistograms::build(&seam_source).representative_hue_degrees;
    assert!(seam_mean < 1.0e-5 || (360.0 - seam_mean) < 1.0e-5);

    let grayscale =
        threshiator::document::PixelImage::new(1, 1, vec![[0.2, 0.2, 0.2, 1.0]]).unwrap();
    assert_eq!(
        ThresholdHistograms::build(&grayscale).representative_hue_degrees,
        0.0
    );

    let weighted_hue = |hue, saturation, alpha| {
        let encoded = hsv_to_encoded([hue, saturation, 1.0]);
        [
            srgb_to_linear(encoded[0] as f32),
            srgb_to_linear(encoded[1] as f32),
            srgb_to_linear(encoded[2] as f32),
            alpha,
        ]
    };
    let weighted_source = threshiator::document::PixelImage::new(
        2,
        1,
        vec![weighted_hue(0.0, 1.0, 1.0), weighted_hue(90.0, 0.5, 0.25)],
    )
    .unwrap();
    let weighted_mean = ThresholdHistograms::build(&weighted_source).representative_hue_degrees;
    let expected = 0.125_f64.atan2(1.0).to_degrees();
    assert!((weighted_mean - expected).abs() < 1.0e-4);
}

#[test]
fn threshold_hsv_link_presentation_keeps_saturation_and_value_distinct() {
    assert_eq!(
        threshold_histogram_series_mask(ThresholdEditTarget::Saturation),
        [false, true, false]
    );
    assert_eq!(
        threshold_histogram_series_mask(ThresholdEditTarget::LinkedSaturationValue),
        [false, true, true]
    );
    let encoded = hsv_to_encoded([210.0, 0.25, 0.8]);
    let source = threshiator::document::PixelImage::new(
        1,
        1,
        vec![[
            srgb_to_linear(encoded[0] as f32),
            srgb_to_linear(encoded[1] as f32),
            srgb_to_linear(encoded[2] as f32),
            1.0,
        ]],
    )
    .unwrap();
    let histogram = ThresholdHistograms::build(&source);
    assert_ne!(histogram.hsv[1], histogram.hsv[2]);
}

#[test]
fn threshold_hsv_strip_colors_cover_origin_mapping_and_bypass() {
    let assert_color = |actual: [f64; 3], expected: [f64; 3]| {
        for (actual, expected) in actual.into_iter().zip(expected) {
            assert!((actual - expected).abs() < 1.0e-9, "{actual} != {expected}");
        }
    };
    assert_color(
        hsv_input_strip_color(HsvHistogramComponent::Hue, 0.0, 30.0, 210.0),
        hsv_to_encoded([30.0, 1.0, 1.0]),
    );
    assert_color(
        hsv_input_strip_color(HsvHistogramComponent::Hue, 1.0, 30.0, 210.0),
        hsv_to_encoded([30.0, 1.0, 1.0]),
    );
    assert_color(
        hsv_input_strip_color(HsvHistogramComponent::Saturation, 0.5, 30.0, 210.0),
        hsv_to_encoded([210.0, 0.5, 1.0]),
    );
    assert_color(
        hsv_input_strip_color(HsvHistogramComponent::Value, 0.4, 30.0, 210.0),
        [0.4, 0.4, 0.4],
    );

    let mapped = ComponentQuantizer {
        enabled: true,
        boundaries: vec![0.5],
        outputs: vec![0.25, 0.75],
    };
    assert_color(
        hsv_output_strip_color(HsvHistogramComponent::Hue, 0.1, &mapped, 30.0, 210.0),
        hsv_to_encoded([120.0, 1.0, 1.0]),
    );
    let mut bypass = mapped;
    bypass.enabled = false;
    assert_color(
        hsv_output_strip_color(HsvHistogramComponent::Hue, 0.1, &bypass, 30.0, 210.0),
        hsv_to_encoded([66.0, 1.0, 1.0]),
    );
}

#[test]
fn threshold_histogram_build_honors_immediate_and_mid_scan_cancellation() {
    use std::cell::Cell;

    let source =
        threshiator::document::PixelImage::new(16_385, 1, vec![[0.25, 0.5, 0.75, 1.0]; 16_385])
            .unwrap();
    assert!(ThresholdHistograms::build_cancellable(&source, || false).is_none());

    let checks = Cell::new(0);
    assert!(
        ThresholdHistograms::build_cancellable(&source, || {
            checks.set(checks.get() + 1);
            checks.get() < 3
        })
        .is_none()
    );
    assert_eq!(checks.get(), 3);
}

#[test]
fn threshold_histogram_build_does_not_mutate_source_or_processing() {
    let source = threshiator::document::PixelImage::new(
        2,
        1,
        vec![[0.04, 0.2, 0.8, 0.3], [0.9, 0.1, 0.5, 1.0]],
    )
    .unwrap();
    let source_before = source.clone();
    let recipe = Recipe::thresholds_default();
    let result_before = process(&source, &recipe);

    let _ = ThresholdHistograms::build(&source);

    assert_eq!(source, source_before);
    assert_eq!(process(&source, &recipe), result_before);
}

#[test]
fn encoded_rgb_is_float_precise_and_bypassable() {
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
    assert!(recipe.threshold.set_hue_origin_degrees(390.0));
    assert_eq!(recipe.threshold.hsv_state.hue_origin_degrees, 30.0);
    let shifted = process(&known, &recipe);
    for (pixel, expected_hue) in shifted.pixels.iter().zip([345.0, 75.0, 165.0, 255.0]) {
        let actual = encoded_to_hsv([
            threshiator::processing::linear_to_srgb(pixel[0]) as f64,
            threshiator::processing::linear_to_srgb(pixel[1]) as f64,
            threshiator::processing::linear_to_srgb(pixel[2]) as f64,
        ])[0];
        let delta = (actual - expected_hue).abs();
        assert!(delta.min(360.0 - delta) < 1.0e-3);
    }
    recipe.threshold.hsv_state.hue = ComponentQuantizer::automatic_circular(8);
    let shifted_near_seam = threshiator::document::PixelImage::new(
        2,
        1,
        [29.0, 31.0]
            .map(|hue| {
                let rgb = hsv_to_encoded([hue, 1.0, 0.8]);
                [
                    srgb_to_linear(rgb[0] as f32),
                    srgb_to_linear(rgb[1] as f32),
                    srgb_to_linear(rgb[2] as f32),
                    1.0,
                ]
            })
            .to_vec(),
    )
    .unwrap();
    let shifted_seam = process(&shifted_near_seam, &recipe);
    for (pixel, expected_hue) in shifted_seam.pixels.iter().zip([7.5, 52.5]) {
        let actual = encoded_to_hsv([
            threshiator::processing::linear_to_srgb(pixel[0]) as f64,
            threshiator::processing::linear_to_srgb(pixel[1]) as f64,
            threshiator::processing::linear_to_srgb(pixel[2]) as f64,
        ])[0];
        let delta = (actual - expected_hue).abs();
        assert!(delta.min(360.0 - delta) < 1.0e-3);
    }
    assert_eq!(recipe.threshold.hsv_state.hue_origin_degrees, 30.0);
    recipe.threshold.hsv_state.locks[0] = true;
    assert!(!recipe.threshold.set_hue_origin_degrees(-30.0));
    assert_eq!(recipe.threshold.hsv_state.hue_origin_degrees, 30.0);
    let neutral = threshiator::document::PixelImage::new(1, 1, vec![[0.4, 0.4, 0.4, 0.8]]).unwrap();
    let neutral_out = process(&neutral, &recipe);
    for index in 0..3 {
        assert!((neutral_out.pixels[0][index] - neutral.pixels[0][index]).abs() < 1.0e-5);
    }
}

#[test]
fn threshold_automatic_baselines_follow_scalar_and_circular_formulas() {
    for bands in [2, 3, 8, 32] {
        let scalar = ComponentQuantizer::automatic_scalar(bands);
        let expected_boundaries = (1..bands)
            .map(|index| index as f32 / bands as f32)
            .collect::<Vec<_>>();
        let expected_outputs = (0..bands)
            .map(|index| index as f32 / (bands - 1) as f32)
            .collect::<Vec<_>>();
        assert_eq!(scalar.boundaries, expected_boundaries);
        assert_eq!(scalar.outputs, expected_outputs);
        assert_eq!(scalar.outputs[0], 0.0);
        assert_eq!(scalar.outputs[bands - 1], 1.0);

        let circular = ComponentQuantizer::automatic_circular(bands);
        let expected_outputs = (0..bands)
            .map(|index| (index as f32 + 0.5) / bands as f32)
            .collect::<Vec<_>>();
        assert_eq!(circular.boundaries, expected_boundaries);
        assert_eq!(circular.outputs, expected_outputs);
        assert!(circular.outputs[bands - 1] < 1.0);
        let seam_gap = circular.outputs[0] + 1.0 - circular.outputs[bands - 1];
        assert!((seam_gap - 1.0 / bands as f32).abs() < f32::EPSILON);
    }
}

#[test]
fn threshold_automatic_baselines_process_authoritative_endpoints() {
    let source = threshiator::document::PixelImage::new(
        3,
        1,
        vec![
            [0.0, 0.0, 0.0, 1.0],
            [1.0, 1.0, 1.0, 1.0],
            [1.0, 0.0, 0.0, 1.0],
        ],
    )
    .unwrap();
    let mut recipe = Recipe {
        active_method: Method::Thresholds,
        ..Recipe::default()
    };

    assert!(recipe.threshold.auto_map(ThresholdEditTarget::Red));
    let rgb = process(&source, &recipe);
    for (actual, expected) in rgb.pixels.iter().zip(source.pixels.iter()) {
        for channel in 0..4 {
            assert!((actual[channel] - expected[channel]).abs() < 1.0e-6);
        }
    }

    recipe.threshold.active_space = ThresholdSpace::Hsv;
    recipe.threshold.hsv_state.hue.enabled = false;
    assert!(recipe.threshold.auto_map(ThresholdEditTarget::Saturation));
    for component in [
        &recipe.threshold.hsv_state.saturation,
        &recipe.threshold.hsv_state.value,
    ] {
        assert_eq!(component.outputs.first(), Some(&0.0));
        assert_eq!(component.outputs.last(), Some(&1.0));
    }
    let hsv = process(&source, &recipe);
    for (actual, expected) in hsv.pixels.iter().zip(source.pixels.iter()) {
        for channel in 0..4 {
            assert!((actual[channel] - expected[channel]).abs() < 1.0e-5);
        }
    }
}

#[test]
fn threshold_auto_map_respects_link_locks_process_and_hue_independence() {
    let mut threshold = ThresholdState::default();
    threshold.rgb_state.components = [
        ComponentQuantizer::evenly_spaced(4),
        ComponentQuantizer::evenly_spaced(6),
        ComponentQuantizer::evenly_spaced(5),
    ];
    threshold.rgb_state.components[0].enabled = false;
    threshold.rgb_state.components[1].enabled = true;
    threshold.rgb_state.components[2].enabled = false;
    threshold.rgb_state.locks[2] = true;
    let locked_blue = threshold.rgb_state.components[2].clone();
    assert!(threshold.auto_map(ThresholdEditTarget::Red));
    let scalar = ComponentQuantizer::automatic_scalar(4);
    assert_eq!(
        threshold.rgb_state.components[0].boundaries,
        scalar.boundaries
    );
    assert_eq!(threshold.rgb_state.components[0].outputs, scalar.outputs);
    assert_eq!(
        threshold.rgb_state.components[1].boundaries,
        scalar.boundaries
    );
    assert_eq!(threshold.rgb_state.components[1].outputs, scalar.outputs);
    assert_eq!(threshold.rgb_state.components[2], locked_blue);
    assert!(!threshold.rgb_state.components[0].enabled);
    assert!(threshold.rgb_state.components[1].enabled);
    assert!(!threshold.rgb_state.components[2].enabled);
    assert!(!threshold.auto_map(ThresholdEditTarget::Red));
    threshold.rgb_state.locks[0] = true;
    assert!(!threshold.auto_map(ThresholdEditTarget::Red));

    threshold.active_space = ThresholdSpace::Hsv;
    threshold.hsv_state.saturation = ComponentQuantizer::evenly_spaced(7);
    threshold.hsv_state.value = ComponentQuantizer::evenly_spaced(5);
    threshold.hsv_state.saturation.enabled = false;
    threshold.hsv_state.value.enabled = true;
    threshold.hsv_state.locks[2] = true;
    let locked_value = threshold.hsv_state.value.clone();
    assert!(threshold.auto_map(ThresholdEditTarget::Saturation));
    let scalar = ComponentQuantizer::automatic_scalar(7);
    assert_eq!(threshold.hsv_state.saturation.boundaries, scalar.boundaries);
    assert_eq!(threshold.hsv_state.saturation.outputs, scalar.outputs);
    assert_eq!(threshold.hsv_state.value, locked_value);
    assert!(!threshold.hsv_state.saturation.enabled);
    assert!(threshold.hsv_state.value.enabled);

    threshold.hsv_state.hue = ComponentQuantizer::evenly_spaced(8);
    threshold.hsv_state.hue.outputs.reverse();
    threshold.hsv_state.hue_origin_degrees = 30.0;
    let saturation_before = threshold.hsv_state.saturation.clone();
    let value_before = threshold.hsv_state.value.clone();
    assert!(threshold.auto_map(ThresholdEditTarget::Hue));
    assert_eq!(
        threshold.hsv_state.hue,
        ComponentQuantizer::automatic_circular(8)
    );
    assert_eq!(threshold.hsv_state.hue_origin_degrees, 30.0);
    assert_eq!(threshold.hsv_state.saturation, saturation_before);
    assert_eq!(threshold.hsv_state.value, value_before);
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
        VoronoiMatching::Okhsl,
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
fn okhsl_matching_uses_a_seam_safe_cylinder_and_existing_influence_rule() {
    let linear = |hue_degrees: f64, saturation: f64, lightness: f64| {
        okhsl_to_linear([hue_degrees / 360.0, saturation, lightness])
            .unwrap()
            .map(|channel| channel as f32)
    };
    let output = |pixel: [f32; 3], sites: Vec<VoronoiSite>| {
        let source = threshiator::document::PixelImage::new(
            1,
            1,
            vec![[pixel[0], pixel[1], pixel[2], 0.63]],
        )
        .unwrap();
        let mut recipe = Recipe::default();
        recipe.voronoi.matching = VoronoiMatching::Okhsl;
        recipe.voronoi.sites = sites;
        process(&source, &recipe).pixels[0]
    };

    let seam = output(
        linear(359.0, 1.0, 0.5),
        vec![
            sample(1, linear(180.0, 1.0, 0.5), [0.9, 0.1, 0.1], 0.0),
            sample(2, linear(1.0, 1.0, 0.5), [0.1, 0.9, 0.1], 0.0),
        ],
    );
    assert_eq!(&seam[..3], &[0.1, 0.9, 0.1]);
    assert_eq!(seam[3], 0.63);

    // In the cylinder, the opposite-hue low-saturation site is closer (d²=.04) than
    // the same-hue higher-saturation site (d²=.09). Raw component Hue distance cannot
    // reproduce this assignment.
    let cylinder = output(
        linear(0.0, 0.1, 0.5),
        vec![
            sample(1, linear(180.0, 0.1, 0.5), [0.2, 0.3, 0.4], 0.0),
            sample(2, linear(0.0, 0.4, 0.5), [0.7, 0.8, 0.9], 0.0),
        ],
    );
    assert_eq!(&cylinder[..3], &[0.2, 0.3, 0.4]);

    let neutral = output(
        linear(240.0, 0.0, 0.5),
        vec![
            sample(1, linear(0.0, 0.0, 0.5), [0.1, 0.2, 0.3], 0.0),
            sample(2, linear(180.0, 0.0, 0.5), [0.8, 0.7, 0.6], 0.0),
        ],
    );
    assert_eq!(&neutral[..3], &[0.1, 0.2, 0.3]);

    let influenced = output(
        linear(0.0, 0.0, 0.5),
        vec![
            sample(1, linear(0.0, 0.0, 0.32), [0.9, 0.1, 0.1], 4.0),
            sample(2, linear(0.0, 0.0, 0.55), [0.1, 0.1, 0.9], 0.0),
        ],
    );
    assert_eq!(&influenced[..3], &[0.9, 0.1, 0.1]);
    let suppressed = output(
        linear(0.0, 0.0, 0.5),
        vec![
            sample(1, linear(0.0, 0.0, 0.32), [0.9, 0.1, 0.1], -4.0),
            sample(2, linear(0.0, 0.0, 0.55), [0.1, 0.1, 0.9], 0.0),
        ],
    );
    assert_eq!(&suppressed[..3], &[0.1, 0.1, 0.9]);
}

#[test]
fn project_v5_roundtrips_both_method_states_flags_locks_and_ids() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("current-v5.threshiator");
    let (bytes, decoded) = tiny_source();
    let mut recipe = Recipe::thresholds_default();
    recipe.threshold.rgb_state.link = LinkPolicy::Independent;
    recipe.threshold.rgb_state.components[2].enabled = false;
    recipe.threshold.rgb_state.locks = [false, true, false];
    recipe
        .threshold
        .set_rgb_component(0, ComponentQuantizer::evenly_spaced(8));
    recipe.threshold.active_space = ThresholdSpace::Hsv;
    recipe.threshold.hsv_state.hue_origin_degrees = 37.5;
    recipe.threshold.hsv_state.locks = [true, false, true];
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
fn project_v5_roundtrips_okhsl_matching_and_retains_hsv_serialization() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("okhsl-matching.threshiator");
    let (bytes, decoded) = tiny_source();
    let mut recipe = Recipe::default();
    recipe.voronoi.matching = VoronoiMatching::Okhsl;
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
    assert_eq!(reopened.recipe.voronoi.matching, VoronoiMatching::Okhsl);

    let hsv: VoronoiMatching = serde_json::from_str("\"Hsv\"").unwrap();
    assert_eq!(hsv, VoronoiMatching::Hsv);
}

#[test]
fn project_rejects_pre_release_versions_without_migration() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("pre-release-v4.threshiator");
    let (bytes, decoded) = tiny_source();
    let recipe = Recipe::thresholds_default();
    let mut recipe_value = serde_json::to_value(&recipe).unwrap();
    recipe_value["threshold"]["rgb_state"]
        .as_object_mut()
        .unwrap()
        .remove("locks");
    recipe_value["threshold"]["hsv_state"]
        .as_object_mut()
        .unwrap()
        .remove("locks");
    let manifest = serde_json::json!({
        "version": 4,
        "source_name": "legacy.png",
        "source_entry": "source/original",
        "source_interpretation": decoded.interpretation,
        "recipe": recipe_value,
        "export_defaults": ExportDefaults::default(),
    });
    write_project_fixture(&path, &manifest, &bytes);
    let error = project::open(&path).unwrap_err().to_string();
    assert!(error.contains("unsupported pre-release project version 4"));
    assert!(error.contains("only version 5"));

    let current_path = directory.path().join("current-missing-locks.threshiator");
    let mut current_manifest = manifest;
    current_manifest["version"] = serde_json::json!(5);
    write_project_fixture(&current_path, &current_manifest, &bytes);
    let error = format!("{:#}", project::open(&current_path).unwrap_err());
    assert!(error.contains("project version 5 manifest is malformed"));
    assert!(error.contains("locks"));
}

#[test]
fn project_v5_rejects_unsupported_export_defaults() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("unsupported-export.threshiator");
    let (bytes, decoded) = tiny_source();
    let manifest = serde_json::json!({
        "version": 5,
        "source_name": "fixture.png",
        "source_entry": "source/original",
        "source_interpretation": decoded.interpretation,
        "recipe": Recipe::default(),
        "export_defaults": {
            "format": "JPEG",
            "depth": "8-bit integer per channel"
        },
    });
    write_project_fixture(&path, &manifest, &bytes);
    let error = project::open(&path).unwrap_err().to_string();
    assert!(error.contains("invalid export defaults"));
    assert!(error.contains("JPEG"));
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
        "version": 5, "source_name": "fixture.png", "source_entry": "source/original",
        "source_interpretation": decoded.interpretation, "recipe": recipe_json,
        "export_defaults": ExportDefaults::default(),
    });
    write_project_fixture(&path, &manifest, &bytes);
    let error = project::open(&path).unwrap_err().to_string();
    assert!(error.contains("invalid Threshold state"));
    assert!(error.contains("strictly ordered"));
}

#[test]
fn project_current_schema_rejects_removed_color_groups() {
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
    let error = project::open(&path).unwrap_err().to_string();
    assert!(
        error.contains("project version 3 manifest is malformed")
            || error.contains("unsupported pre-release project version 3")
    );
}

#[test]
fn threshold_editor_targets_follow_space_and_link_semantics() {
    let mut threshold = Recipe::default().threshold;
    assert_eq!(
        threshold.edit_targets(),
        vec![
            ThresholdEditTarget::LinkedRgb,
            ThresholdEditTarget::Red,
            ThresholdEditTarget::Green,
            ThresholdEditTarget::Blue,
        ]
    );
    threshold.rgb_state.link = LinkPolicy::Independent;
    assert_eq!(
        threshold.edit_targets(),
        vec![
            ThresholdEditTarget::Red,
            ThresholdEditTarget::Green,
            ThresholdEditTarget::Blue,
        ]
    );
    assert_eq!(
        threshold.reconcile_edit_target(ThresholdEditTarget::Green),
        ThresholdEditTarget::Green
    );
    threshold.rgb_state.link = LinkPolicy::Linked;
    threshold.active_space = ThresholdSpace::Hsv;
    assert_eq!(
        threshold.edit_targets(),
        vec![
            ThresholdEditTarget::Hue,
            ThresholdEditTarget::LinkedSaturationValue,
            ThresholdEditTarget::Saturation,
            ThresholdEditTarget::Value,
        ]
    );
    assert_eq!(
        threshold.reconcile_edit_target(ThresholdEditTarget::Value),
        ThresholdEditTarget::Value
    );
    threshold.hsv_state.sv_link = LinkPolicy::Independent;
    assert_eq!(
        threshold.edit_targets(),
        vec![
            ThresholdEditTarget::Hue,
            ThresholdEditTarget::Saturation,
            ThresholdEditTarget::Value,
        ]
    );
    assert_eq!(
        threshold.reconcile_edit_target(ThresholdEditTarget::Value),
        ThresholdEditTarget::Value
    );
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

#[test]
fn threshold_band_split_is_exact_and_resize_respects_bounds() {
    let mut quantizer = ComponentQuantizer {
        enabled: true,
        boundaries: vec![0.2, 0.7],
        outputs: vec![0.1, 0.55, 0.9],
    };
    let before: Vec<_> = (0..=100)
        .map(|step| quantizer.quantize(step as f32 / 100.0))
        .collect();
    assert!(quantizer.split_band(1));
    let after: Vec<_> = (0..=100)
        .map(|step| quantizer.quantize(step as f32 / 100.0))
        .collect();
    assert_eq!(after, before);
    assert_eq!(quantizer.boundaries, vec![0.2, 0.45, 0.7]);
    assert_eq!(quantizer.outputs, vec![0.1, 0.55, 0.55, 0.9]);
    assert!(quantizer.resize_preserving_mapping(32, None, None));
    assert_eq!(quantizer.outputs.len(), 32);
    assert!(!quantizer.resize_preserving_mapping(40, None, None));
    assert!(quantizer.resize_preserving_mapping(2, None, None));
    assert_eq!(quantizer.outputs.len(), 2);
    assert!(!quantizer.resize_preserving_mapping(1, None, None));
}

#[test]
fn threshold_band_merge_is_deterministic_and_preserves_unrelated_boundaries() {
    let mut quantizer = ComponentQuantizer {
        enabled: true,
        boundaries: vec![0.2, 0.7, 0.9],
        outputs: vec![0.1, 0.4, 0.6, 0.95],
    };
    assert!(quantizer.remove_boundary(0));
    assert_eq!(quantizer.boundaries, vec![0.7, 0.9]);
    assert_eq!(quantizer.outputs, vec![0.4, 0.6, 0.95]);

    let mut least_error = ComponentQuantizer {
        enabled: true,
        boundaries: vec![0.25, 0.5, 0.75],
        outputs: vec![0.0, 0.8, 0.81, 1.0],
    };
    assert!(least_error.remove_least_error_boundary());
    assert_eq!(least_error.boundaries, vec![0.25, 0.75]);
    assert_eq!(least_error.outputs, vec![0.0, 0.8, 1.0]);
}

#[test]
fn threshold_link_and_sync_skip_locks_and_preserve_process_flags() {
    let mut threshold = ThresholdState::default();
    threshold.rgb_state.link = LinkPolicy::Linked;
    threshold.rgb_state.locks[1] = true;
    threshold.rgb_state.components[0].enabled = false;
    threshold.rgb_state.components[1].enabled = false;
    threshold.rgb_state.components[2].enabled = true;
    let edited = ComponentQuantizer::evenly_spaced(7);
    assert!(threshold.set_edit_quantizer(ThresholdEditTarget::Red, edited.clone()));
    assert_ne!(threshold.rgb_state.components[1].outputs.len(), 7);
    assert_eq!(threshold.rgb_state.components[2].outputs.len(), 7);
    assert!(!threshold.rgb_state.components[0].enabled);
    assert!(!threshold.rgb_state.components[1].enabled);
    assert!(threshold.rgb_state.components[2].enabled);

    threshold.rgb_state.link = LinkPolicy::Independent;
    threshold.rgb_state.components[2] = ComponentQuantizer::evenly_spaced(5);
    threshold.rgb_state.components[2].enabled = true;
    let (copied, skipped) = threshold.sync_from(ThresholdEditTarget::Red);
    assert_eq!((copied, skipped), (1, 1));
    assert_eq!(threshold.rgb_state.components[2].outputs, edited.outputs);
    assert!(threshold.rgb_state.components[2].enabled);

    threshold.active_space = ThresholdSpace::Hsv;
    threshold.hsv_state.sv_link = LinkPolicy::Linked;
    threshold.hsv_state.locks[2] = true;
    assert!(threshold.set_edit_quantizer(
        ThresholdEditTarget::Saturation,
        ComponentQuantizer::evenly_spaced(9),
    ));
    assert_ne!(threshold.hsv_state.value.outputs.len(), 9);
    assert_eq!(threshold.sync_from(ThresholdEditTarget::Hue), (0, 0));
}

#[test]
fn threshold_mapping_locks_block_edits_and_resets_but_not_process() {
    let mut threshold = ThresholdState::default();
    threshold.rgb_state.locks[0] = true;
    let original = threshold.rgb_state.components[0].clone();
    assert!(!threshold.set_edit_quantizer(
        ThresholdEditTarget::Red,
        ComponentQuantizer::evenly_spaced(8),
    ));
    assert!(!reset_component(&mut threshold, ThresholdEditTarget::Red));
    threshold.rgb_state.components[0].enabled = false;
    reset_active_space(&mut threshold);
    assert_eq!(
        threshold.rgb_state.components[0].boundaries,
        original.boundaries
    );
    assert!(!threshold.rgb_state.components[0].enabled);
    assert!(threshold.rgb_state.locks[0]);
    reset_method(&mut threshold);
    assert_eq!(
        threshold.rgb_state.components[0].boundaries,
        original.boundaries
    );
    assert!(threshold.rgb_state.locks[0]);
}
