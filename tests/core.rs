use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::{Duration, Instant};

use image::{ImageBuffer, ImageFormat, Rgba};
use chromiator::color::{
    ColorModel, DraftColor, PickerTransaction, PlaneKey, adjust_okhsl, encoded_to_hsl,
    encoded_to_hsv, encoded_to_linear, hsl_to_encoded, hsv_to_encoded, in_srgb_gamut,
    linear_to_encoded, linear_to_okhsl, linear_to_okhsl_cylinder, linear_to_oklab, okhsl_to_linear,
    oklab_to_linear, parse_hex,
};
use chromiator::document::{
    ComponentOperation, ComponentSet, ConversionBack, Document, ExportDefaults, PassThroughPolicy,
    Preprocessing, ProcessingStep, ProfileInterpretation, Recipe, SampleSize, TransitionProfile,
    VoronoiMatching, VoronoiSite, VoronoiState, WorkingColorSpace,
};
use chromiator::export::{self, ExportFormat};
use chromiator::preset::{Preset, PresetStore, apply_to_document, validate_name};
use chromiator::processing::{
    CompiledVoronoi, PREVIEW_MAX_DIMENSION, bounded_preview, gaussian_blur_cancellable, process,
    process_cancellable_with_progress_and_coverage, rotate_oklch, srgb_to_linear, to_display_rgba8,
};
use chromiator::project;
use chromiator::raster;
use chromiator::scheduler::{JobCoordinator, LatestGeneration, PreviewScheduler, ProgressTracker};
use chromiator::starter_looks::{STARTER_LOOKS, apply_starter_look, recipe_for_starter_look};
use chromiator::voronoi::{auto_initialize, linear_rgb_to_oklab, reattach_site, sample_color};
use chromiator::workflow::{
    OpenKind, ReplacementDecision, SaveResolution, classify_open_path, ensure_project_extension,
    replacement_decision, resolve_pending_after_save,
};
use zip::write::SimpleFileOptions;

#[test]
fn gaussian_smoothing_zero_constant_impulse_alpha_and_cancellation() {
    let generation = AtomicU64::new(1);
    let source =
        chromiator::document::PixelImage::new(5, 5, vec![[0.2, 0.4, 0.6, 1.0]; 25]).unwrap();
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
    let impulse = chromiator::document::PixelImage::new(5, 5, impulse).unwrap();
    let blurred = gaussian_blur_cancellable(&impulse, 1.0, 1, &generation, |_| {}).unwrap();
    assert!((blurred.pixels[11][3] - blurred.pixels[13][3]).abs() < 1.0e-6);
    assert!((blurred.pixels[7][3] - blurred.pixels[17][3]).abs() < 1.0e-6);
    for pixel in blurred.pixels.iter().filter(|pixel| pixel[3] > 1.0e-8) {
        assert!((pixel[0] - 1.0).abs() < 1.0e-5);
        assert_eq!(pixel[1], 0.0);
        assert_eq!(pixel[2], 0.0);
    }
    let transparent =
        chromiator::document::PixelImage::new(1, 1, vec![[1.0, 1.0, 1.0, 0.0]]).unwrap();
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
fn built_in_looks_are_voronoi_only_valid_distinct_and_idempotent() {
    assert_eq!(STARTER_LOOKS.len(), 5);
    let mut ids = std::collections::BTreeSet::new();
    let mut names = std::collections::BTreeSet::new();
    let source = chromiator::example::spectrum_document().unwrap().source;
    for (index, look) in STARTER_LOOKS.iter().copied().enumerate() {
        assert!(ids.insert(look.id), "duplicate built-in id {}", look.id);
        assert!(
            names.insert(look.name),
            "duplicate built-in name {}",
            look.name
        );
        let recipe = recipe_for_starter_look(look);
        recipe.validate().unwrap();
        assert!(recipe.voronoi.sites.len() >= 4);
        assert!(
            recipe
                .voronoi
                .sites
                .iter()
                .all(|site| site.position.is_none())
        );
        assert!(
            process(&source, &recipe)
                .pixels
                .windows(2)
                .any(|pair| pair[0] != pair[1])
        );

        let mut document = chromiator::example::spectrum_document().unwrap();
        let identity = (
            document.source_name.clone(),
            document.source_bytes.clone(),
            document.source.clone(),
            document.interpretation.clone(),
            document.export_defaults.clone(),
        );
        assert!(apply_starter_look(&mut document, index));
        assert!(document.dirty);
        assert_eq!(document.recipe, recipe);
        assert_eq!(
            (
                document.source_name.clone(),
                document.source_bytes.clone(),
                document.source.clone(),
                document.interpretation.clone(),
                document.export_defaults.clone(),
            ),
            identity
        );
        assert!(!apply_starter_look(&mut document, index));
    }
}

#[test]
fn preprocessing_and_transition_profiles_validate_at_recipe_and_render_boundaries() {
    for valid in [0.0, 0.75, 10.0] {
        Preprocessing {
            input_smoothing: valid,
        }
        .validate()
        .unwrap();
    }
    for invalid in [-0.01, 10.01, f32::NAN, f32::INFINITY] {
        assert!(
            Preprocessing {
                input_smoothing: invalid,
            }
            .validate()
            .is_err()
        );
    }
    TransitionProfile::HARD.validate().unwrap();
    for invalid in [
        TransitionProfile {
            start: f32::NAN,
            midpoint: 0.5,
            end: 0.5,
            ..TransitionProfile::HARD
        },
        TransitionProfile {
            start: -0.1,
            midpoint: 0.5,
            end: 0.5,
            ..TransitionProfile::HARD
        },
        TransitionProfile {
            start: 0.6,
            midpoint: 0.5,
            end: 0.7,
            ..TransitionProfile::HARD
        },
        TransitionProfile {
            start: 0.2,
            midpoint: 0.5,
            end: 0.7,
            ..TransitionProfile::HARD
        },
    ] {
        assert!(invalid.validate().is_err());
    }

    let source = chromiator::document::PixelImage::new(1, 1, vec![[0.2, 0.3, 0.4, 1.0]]).unwrap();
    let mut recipe = Recipe::default();
    recipe.voronoi.transition = TransitionProfile {
        start: 0.2,
        midpoint: 0.5,
        end: 0.7,
        ..TransitionProfile::HARD
    };
    assert!(
        recipe
            .validate()
            .unwrap_err()
            .contains("symmetric")
    );
    assert!(std::panic::catch_unwind(|| process(&source, &recipe)).is_err());
}

#[test]
fn smoothing_changes_authoritative_voronoi_classification() {
    let source = chromiator::document::PixelImage::new(
        3,
        1,
        vec![
            [0.0, 0.0, 0.0, 1.0],
            [1.0, 1.0, 1.0, 1.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    )
    .unwrap();
    let mut recipe = Recipe::default();
    recipe.voronoi.sites = vec![
        sample(1, [0.0; 3], [0.9, 0.1, 0.1], 0.0),
        sample(2, [1.0; 3], [0.1, 0.1, 0.9], 0.0),
    ];
    recipe.voronoi.next_site_id = 3;
    let unsmoothed = process(&source, &recipe);
    recipe.preprocessing.input_smoothing = 1.0;
    let smoothed = process(&source, &recipe);
    assert_ne!(unsmoothed, smoothed);
    assert_eq!(&unsmoothed.pixels[1][..3], &[0.1, 0.1, 0.9]);
    assert!(
        unsmoothed
            .pixels
            .iter()
            .zip(smoothed.pixels.iter())
            .any(|(before, after)| before != after)
    );
}

#[test]
fn ordered_hue_steps_execute_individually_after_voronoi_mapping() {
    let source = chromiator::document::PixelImage::new(1, 1, vec![[0.2, 0.3, 0.4, 0.35]]).unwrap();
    let target = [0.95, 0.08, 0.02];
    let mut recipe = Recipe::default();
    recipe.voronoi.sites = vec![sample(1, [0.2, 0.3, 0.4], target, 0.0)];
    recipe.voronoi.next_site_id = 2;
    let step = |degrees| ProcessingStep {
        working_space: WorkingColorSpace::Oklch,
        components: ComponentSet::Hue,
        operation: ComponentOperation::RotateHue { degrees },
        pass_through: PassThroughPolicy::PreserveUnselectedComponents,
        alpha: chromiator::document::AlphaPolicy::PassThroughStraight,
        conversion_back: ConversionBack::KeepLinearSrgbF32,
    };
    recipe.steps = vec![step(143.0), step(-97.0)];
    recipe.validate().unwrap();
    let output = process(&source, &recipe).pixels[0];
    let expected = rotate_oklch(rotate_oklch(target, 143.0), -97.0);
    assert_eq!(
        output.map(f32::to_bits),
        [expected[0], expected[1], expected[2], 0.35].map(f32::to_bits)
    );

    let mut collapsed = recipe.clone();
    collapsed.steps = vec![step(46.0)];
    assert_ne!(
        output[..3]
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>(),
        process(&source, &collapsed).pixels[0][..3]
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>(),
        "per-step gamut clipping must not be replaced by one net rotation"
    );
}

#[test]
fn refactor_baseline_preserves_exact_voronoi_rgba_and_coverage() {
    let source = chromiator::document::PixelImage::new(
        4,
        2,
        vec![
            [0.01, 0.02, 0.03, 0.0],
            [0.02, 0.04, 0.08, 0.25],
            [0.11, 0.32, 0.71, 0.75],
            [0.72, 0.18, 0.06, 1.0],
            [0.33, 0.33, 0.33, 1.0],
            [0.87, 0.91, 0.95, 0.5],
            [0.2, 0.7, 0.15, 1.0],
            [0.8, 0.25, 0.65, 1.0],
        ],
    )
    .unwrap();
    let step = |degrees| ProcessingStep {
        working_space: WorkingColorSpace::Oklch,
        components: ComponentSet::Hue,
        operation: ComponentOperation::RotateHue { degrees },
        pass_through: PassThroughPolicy::PreserveUnselectedComponents,
        alpha: chromiator::document::AlphaPolicy::PassThroughStraight,
        conversion_back: ConversionBack::KeepLinearSrgbF32,
    };
    let mut actual = String::new();
    for matching in [
        VoronoiMatching::Perceptual,
        VoronoiMatching::Okhsl,
        VoronoiMatching::Rgb,
        VoronoiMatching::Hsv,
    ] {
        for smoothing in [0.0_f32, 1.25] {
            let mut recipe = Recipe::default();
            recipe.preprocessing.input_smoothing = smoothing;
            recipe.voronoi.matching = matching;
            recipe.voronoi.sites = vec![
                sample(1, [0.03, 0.03, 0.04], [0.08, 0.12, 0.22], -0.25),
                sample(2, [0.7, 0.16, 0.05], [0.92, 0.35, 0.08], 0.5),
                sample(3, [0.18, 0.68, 0.14], [0.12, 0.82, 0.28], 0.0),
                sample(4, [0.82, 0.28, 0.7], [0.75, 0.18, 0.85], -0.5),
            ];
            recipe.voronoi.next_site_id = 5;
            recipe.steps = vec![step(143.0), step(-97.0)];
            let generation = AtomicU64::new(0);
            let (output, coverage) = process_cancellable_with_progress_and_coverage(
                &source,
                &recipe,
                0,
                &generation,
                |_| {},
            )
            .unwrap();
            let bits = output
                .pixels
                .iter()
                .map(|pixel| pixel.map(f32::to_bits))
                .collect::<Vec<_>>();
            actual.push_str(&format!(
                "matching={matching:?} smoothing={smoothing:?} coverage={:?} visible={} bits={bits:?}\n",
                coverage.site_counts, coverage.visible_total
            ));
        }
    }
    assert_eq!(
        actual,
        include_str!("fixtures/refactor-voronoi-baseline.txt")
    );
}

#[test]
fn compiled_hard_partition_retains_context_stable_winner_and_alpha() {
    let mut recipe = Recipe::default();
    let mut later = sample(2, [0.5, 0.5, 0.5], [0.8, 0.7, 0.6], 0.0);
    later.order = 9;
    let mut earlier = sample(1, [0.5, 0.5, 0.5], [0.1, 0.2, 0.3], 0.0);
    earlier.order = 3;
    recipe.voronoi.sites = vec![later, earlier];
    recipe.voronoi.next_site_id = 3;
    let compiled = CompiledVoronoi::compile(&recipe).unwrap();
    let contexts = compiled.site_contexts().cloned().collect::<Vec<_>>();
    assert_eq!(contexts[0].source_linear, [0.5, 0.5, 0.5]);
    assert_eq!(contexts[1].target_linear, [0.1, 0.2, 0.3]);
    let pixel = [0.5, 0.5, 0.5, 0.37];
    let winner = compiled.winner_index(pixel).unwrap();
    assert_eq!(
        contexts[winner].id, 1,
        "ties use persisted order, then site ID"
    );
    assert_eq!(compiled.resolve_hard(pixel, winner), [0.1, 0.2, 0.3, 0.37]);
    let source = chromiator::document::PixelImage::new(1, 1, vec![pixel]).unwrap();
    assert_eq!(process(&source, &recipe).pixels[0], [0.1, 0.2, 0.3, 0.37]);
}

#[test]
fn hue_rotation_retains_fractional_precision_wraps_and_preserves_neutrals() {
    let source = chromiator::document::PixelImage::new(1, 1, vec![[0.2, 0.3, 0.4, 0.6]]).unwrap();
    let render = |target, degrees| {
        let mut recipe = Recipe::default();
        recipe.voronoi.sites = vec![sample(1, [0.2, 0.3, 0.4], target, 0.0)];
        recipe.voronoi.next_site_id = 2;
        recipe.set_hue_degrees(degrees);
        process(&source, &recipe).pixels[0]
    };
    let fractional = render([0.9, 0.2, 0.05], 12.345_67);
    let rounded = render([0.9, 0.2, 0.05], 12.0);
    assert_ne!(fractional.map(f32::to_bits), rounded.map(f32::to_bits));
    let original = render([0.35, 0.55, 0.75], 0.0);
    let wrapped = render([0.35, 0.55, 0.75], 360.0);
    for channel in 0..3 {
        assert!((original[channel] - wrapped[channel]).abs() < 2e-6);
    }
    let neutral = render([0.4, 0.4, 0.4], 173.25);
    for channel in neutral.iter().take(3) {
        assert!((*channel - 0.4).abs() < 2e-6);
    }
    assert_eq!(neutral[3], 0.6);
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
            .ends_with("chromiator/presets")
    );
}

#[test]
fn preset_store_reports_conflicting_folder_and_nonfinite_steps() {
    let root = tempfile::tempdir().unwrap();
    let conflict = root.path().join("presets");
    fs::write(&conflict, b"not a directory").unwrap();
    assert!(PresetStore::at(&conflict).scan().is_err());
    let mut recipe = preset_recipe();
    recipe.steps[0].operation =
        chromiator::document::ComponentOperation::RotateHue { degrees: f32::NAN };
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

#[test]
fn preset_v3_roundtrips_complete_recipe_and_v2_is_rejected_without_rewrite() {
    let root = tempfile::tempdir().unwrap();
    let store = PresetStore::at(root.path());
    let mut recipe = preset_recipe();
    recipe.preprocessing.input_smoothing = 1.25;
    recipe.voronoi.matching = VoronoiMatching::Hsv;
    recipe.steps = vec![
        ProcessingStep {
            working_space: WorkingColorSpace::Oklch,
            components: ComponentSet::Hue,
            operation: ComponentOperation::RotateHue { degrees: 143.0 },
            pass_through: PassThroughPolicy::PreserveUnselectedComponents,
            alpha: chromiator::document::AlphaPolicy::PassThroughStraight,
            conversion_back: ConversionBack::KeepLinearSrgbF32,
        },
        ProcessingStep {
            working_space: WorkingColorSpace::Oklch,
            components: ComponentSet::Hue,
            operation: ComponentOperation::RotateHue { degrees: -97.0 },
            pass_through: PassThroughPolicy::PreserveUnselectedComponents,
            alpha: chromiator::document::AlphaPolicy::PassThroughStraight,
            conversion_back: ConversionBack::KeepLinearSrgbF32,
        },
    ];
    let path = store
        .save(&Preset::new("Version three", None, &recipe).unwrap(), false)
        .unwrap();
    let reopened = store.scan().unwrap().entries.remove(0).preset;
    assert_eq!(reopened.version, 3);
    assert_eq!(reopened.recipe(), recipe_for_detached_sites(&recipe));

    let mut legacy: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    legacy["version"] = serde_json::json!(2);
    let legacy_path = root.path().join("legacy.json");
    let legacy_bytes = serde_json::to_vec_pretty(&legacy).unwrap();
    fs::write(&legacy_path, &legacy_bytes).unwrap();
    let scan = store.scan().unwrap();
    assert!(scan.diagnostics.iter().any(|diagnostic| {
        diagnostic.path == legacy_path && diagnostic.reason.contains("preset version 2")
    }));
    assert_eq!(fs::read(&legacy_path).unwrap(), legacy_bytes);
}

fn recipe_for_detached_sites(recipe: &Recipe) -> Recipe {
    let mut detached = recipe.clone();
    for site in &mut detached.voronoi.sites {
        site.position = None;
    }
    detached
}

#[test]
fn project_v6_roundtrips_and_v5_is_rejected_before_new_schema_decoding() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("current.chromiator");
    let (bytes, decoded) = tiny_source();
    let mut recipe = preset_recipe();
    recipe.preprocessing.input_smoothing = 0.75;
    recipe.voronoi.matching = VoronoiMatching::Okhsl;
    let document = Document {
        source_name: "fixture.png".into(),
        source_bytes: bytes.clone().into(),
        source: decoded.pixels.clone(),
        interpretation: decoded.interpretation.clone(),
        recipe: recipe.clone(),
        export_defaults: ExportDefaults::default(),
        dirty: true,
    };
    project::save(&path, &document).unwrap();
    let reopened = project::open(&path).unwrap();
    assert_eq!(reopened.recipe, recipe);
    assert_eq!(reopened.source_bytes.as_ref(), bytes.as_slice());
    assert!(!reopened.dirty);

    let legacy = directory.path().join("legacy-v5.chromiator");
    let manifest = serde_json::json!({
        "version": 5,
        "source_name": "fixture.png",
        "source_entry": "source/original",
        "source_interpretation": decoded.interpretation,
        "recipe": "deliberately not the version-six schema",
        "export_defaults": ExportDefaults::default(),
    });
    write_project_fixture(&legacy, &manifest, &bytes);
    let before = fs::read(&legacy).unwrap();
    let error = project::open(&legacy).unwrap_err().to_string();
    assert!(error.contains("project version 5"), "{error}");
    assert!(error.contains("only version 6"), "{error}");
    assert_eq!(fs::read(&legacy).unwrap(), before);
}

#[test]
fn project_v6_rejects_invalid_export_defaults_and_unknown_recipe_fields() {
    let directory = tempfile::tempdir().unwrap();
    let (bytes, decoded) = tiny_source();
    let invalid_export = directory.path().join("unsupported-export.chromiator");
    let export_manifest = serde_json::json!({
        "version": 6,
        "source_name": "fixture.png",
        "source_entry": "source/original",
        "source_interpretation": decoded.interpretation,
        "recipe": Recipe::default(),
        "export_defaults": { "format": "JPEG", "depth": "8-bit integer per channel" },
    });
    write_project_fixture(&invalid_export, &export_manifest, &bytes);
    let error = project::open(&invalid_export).unwrap_err().to_string();
    assert!(error.contains("invalid export defaults"), "{error}");
    assert!(error.contains("JPEG"), "{error}");

    let malformed = directory.path().join("unknown-recipe-field.chromiator");
    let mut recipe = serde_json::to_value(Recipe::default()).unwrap();
    recipe["voronoi"]["matchng"] = serde_json::json!("Rgb");
    let malformed_manifest = serde_json::json!({
        "version": 6,
        "source_name": "fixture.png",
        "source_entry": "source/original",
        "source_interpretation": decoded.interpretation,
        "recipe": recipe,
        "export_defaults": ExportDefaults::default(),
    });
    write_project_fixture(&malformed, &malformed_manifest, &bytes);
    let error = format!("{:#}", project::open(&malformed).unwrap_err());
    assert!(
        error.contains("project version 6 manifest is malformed"),
        "{error}"
    );
    assert!(error.contains("matchng"), "{error}");
}

#[test]
fn persisted_transition_profiles_are_required_strict_and_symmetric() {
    let mut variants = Vec::new();
    let base = serde_json::to_value(Recipe::default()).unwrap();

    let mut absent = base.clone();
    absent["voronoi"]
        .as_object_mut()
        .unwrap()
        .remove("transition");
    variants.push(("absent", absent));

    let mut missing_component = base.clone();
    missing_component["voronoi"]["transition"]
        .as_object_mut()
        .unwrap()
        .remove("midpoint");
    variants.push(("missing-midpoint", missing_component));

    let mut unknown = base.clone();
    unknown["voronoi"]["transition"]["curve"] = serde_json::json!("linear");
    variants.push(("unknown-component", unknown));

    let mut unsupported = base;
    unsupported["voronoi"]["transition"] = serde_json::json!({
        "start": 0.2, "midpoint": 0.5, "end": 0.7
    });
    variants.push(("unsupported-asymmetric", unsupported));

    let project_root = tempfile::tempdir().unwrap();
    let (bytes, decoded) = tiny_source();
    for (name, recipe) in &variants {
        let path = project_root.path().join(format!("{name}.chromiator"));
        let manifest = serde_json::json!({
            "version": 6,
            "source_name": "fixture.png",
            "source_entry": "source/original",
            "source_interpretation": decoded.interpretation,
            "recipe": recipe,
            "export_defaults": ExportDefaults::default(),
        });
        write_project_fixture(&path, &manifest, &bytes);
        let before = fs::read(&path).unwrap();
        assert!(
            project::open(&path).is_err(),
            "project variant {name} was accepted"
        );
        assert_eq!(
            fs::read(&path).unwrap(),
            before,
            "project {name} was rewritten"
        );
    }

    let preset_root = tempfile::tempdir().unwrap();
    let store = PresetStore::at(preset_root.path());
    let mut preset_bytes = Vec::new();
    for (index, (name, recipe)) in variants.into_iter().enumerate() {
        let value = serde_json::json!({
            "version": 3,
            "name": format!("Invalid {index}"),
            "processing": {
                "preprocessing": recipe["preprocessing"].clone(),
                "voronoi": recipe["voronoi"].clone(),
                "steps": recipe["steps"].clone(),
            }
        });
        let path = preset_root.path().join(format!("{name}.json"));
        let original = serde_json::to_vec_pretty(&value).unwrap();
        fs::write(&path, &original).unwrap();
        preset_bytes.push((path, original));
    }
    let scan = store.scan().unwrap();
    assert!(scan.entries.is_empty());
    assert_eq!(scan.diagnostics.len(), 4);
    for (path, original) in preset_bytes {
        assert_eq!(fs::read(path).unwrap(), original);
    }
    for diagnostic in scan.diagnostics {
        let before = fs::read(&diagnostic.path).unwrap();
        assert!(!before.is_empty());
        assert!(
            diagnostic.reason.contains("transition")
                || diagnostic.reason.contains("midpoint")
                || diagnostic.reason.contains("curve")
                || diagnostic.reason.contains("supports only the hard"),
            "{}: {}",
            diagnostic.path.display(),
            diagnostic.reason
        );
    }
}

fn hue_recipe(degrees: f32) -> Recipe {
    let mut recipe = Recipe::default();
    recipe.set_hue_degrees(degrees);
    recipe
}

#[test]
fn stale_preview_cannot_win() {
    let scheduler = PreviewScheduler::new();
    let source =
        chromiator::document::PixelImage::new(256, 256, vec![[0.4, 0.5, 0.6, 1.0]; 65536])
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
        chromiator::document::PixelImage::new(512, 32, vec![[0.4, 0.4, 0.4, 1.0]; 16384]).unwrap();
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
        chromiator::document::PixelImage::new(4, 4, vec![[0.2, 0.4, 0.6, 1.0]; 16]).unwrap();
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
        chromiator::document::PixelImage::new(2000, 2, vec![[0.1, 0.2, 0.3, 1.0]; 4000]).unwrap();
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
    let image = chromiator::document::PixelImage::new(
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
    let image = chromiator::document::PixelImage::new(1, 1, vec![[1.25, 0.123_456_7, -0.1, 0.75]])
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
fn site_mutations_preserve_target_and_lock_protects_every_parameter() {
    let mut state = VoronoiState {
        sites: vec![sample(1, [0.1, 0.2, 0.3], [0.8, 0.7, 0.6], 1.0)],
        next_site_id: 2,
        matching: VoronoiMatching::Perceptual,
        transition: TransitionProfile::HARD,
    };
    let replacement = [0.25, 0.5, 0.75, 0.4];
    assert!(state.set_source(1, replacement, None));
    assert_eq!(state.sites[0].source_color, replacement);
    assert_eq!(state.sites[0].target_color, [0.8, 0.7, 0.6]);
    assert_eq!(state.sites[0].position, None);
    let resampled = [0.3, 0.2, 0.1, 0.7];
    assert!(state.set_size(1, SampleSize::FiveByFive, resampled));
    assert_eq!(state.sites[0].source_color, resampled);
    assert_eq!(state.sites[0].target_color, [0.8, 0.7, 0.6]);
    assert!(state.set_target(1, [resampled[0], resampled[1], resampled[2]]));
    assert_eq!(state.sites[0].target_color, resampled[..3]);
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
    let source = chromiator::document::PixelImage::new(
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
    let source = chromiator::document::PixelImage::new(3, 2, pixels).unwrap();
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
        chromiator::document::PixelImage::new(1, 1, vec![[0.8, 0.2, 0.1, 0.0]]).unwrap();
    assert_eq!(process(&transparent, &recipe).pixels[0], [0.0; 4]);
}

#[test]
fn hsv_full_resolution_export_matches_authoritative_processing_and_alpha() {
    let rgb = hsv_to_encoded([359.0, 1.0, 0.8]).map(|c| srgb_to_linear(c as f32));
    let source =
        chromiator::document::PixelImage::new(2001, 1, vec![[rgb[0], rgb[1], rgb[2], 0.37]; 2001])
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
    let image = chromiator::document::PixelImage::new(
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
    let image = chromiator::document::PixelImage::new(5, 5, pixels.clone()).unwrap();
    let sampled = sample_color(&image, [0.5, 0.5], SampleSize::FiveByFive).unwrap();
    let alpha: f32 = pixels.iter().map(|pixel| pixel[3]).sum();
    let expected_red: f32 = pixels.iter().map(|pixel| pixel[0] * pixel[3]).sum::<f32>() / alpha;
    assert!((sampled[0] - expected_red).abs() < 1.0e-6);
    assert!((sampled[1] - 0.25).abs() < 1.0e-6);
    assert!((sampled[2] - 0.75).abs() < 1.0e-6);
}

#[test]
fn reattach_uses_the_sites_persisted_sampling_footprint_and_preserves_target() {
    let pixels = (0..25)
        .map(|index| [index as f32 / 24.0, 0.25, 0.75, 1.0])
        .collect();
    let image = chromiator::document::PixelImage::new(5, 5, pixels).unwrap();
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
        assert_eq!(site.target_color, [1.0, 0.0, 0.0]);
    }
}

#[test]
fn auto_sites_are_deterministic_representative_and_ignore_rare_noise() {
    let mut pixels = vec![[0.1, 0.1, 0.1, 1.0]];
    pixels.extend(vec![[0.8, 0.1, 0.1, 1.0]; 40]);
    pixels.extend(vec![[0.1, 0.8, 0.1, 1.0]; 40]);
    pixels.extend(vec![[0.1, 0.1, 0.8, 1.0]; 40]);
    pixels.push([1.0, 1.0, 1.0, 0.0]);
    let image = chromiator::document::PixelImage::new(122, 1, pixels).unwrap();
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
    let proxy = chromiator::document::PixelImage::new(1600, 1600, pixels).unwrap();
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
    let source = chromiator::document::PixelImage::new(
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
    let source = chromiator::document::PixelImage::new(
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
    let source = chromiator::document::PixelImage::new(3200, 1, pixels).unwrap();
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
    let source = chromiator::document::PixelImage::new(
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
            transition: TransitionProfile::HARD,
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
fn oklab_reference_fixture_and_preview_full_classification_agree() {
    let red = linear_rgb_to_oklab([1.0, 0.0, 0.0]);
    assert!((red[0] - 0.627_955_36).abs() < 1e-7);
    assert!((red[1] - 0.224_863_06).abs() < 1e-7);
    let source =
        chromiator::document::PixelImage::new(4, 1, vec![[0.1, 0.2, 0.3, 1.0]; 4]).unwrap();
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
    let source = chromiator::document::PixelImage::new(
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
    let source = chromiator::document::PixelImage::new(3200, 2, pixels).unwrap();
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
        chromiator::document::PixelImage::new(1, 1, vec![[pixel[0], pixel[1], pixel[2], 1.0]])
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
    let source = chromiator::document::PixelImage::new(3200, 1, pixels).unwrap();
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
        classify_open_path(Path::new("work.CHROMIATOR")),
        OpenKind::Project
    );
    assert_eq!(classify_open_path(Path::new("photo.png")), OpenKind::Image);
    assert_eq!(
        ensure_project_extension(PathBuf::from("work")),
        PathBuf::from("work.chromiator")
    );
    assert_eq!(
        ensure_project_extension(PathBuf::from("work.ChRoMiAtOr")),
        PathBuf::from("work.ChRoMiAtOr")
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
        .map(|channel| chromiator::processing::linear_to_srgb(channel as f32));
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
        chromiator::document::PixelImage::new(3, 1, vec![[0.2, 0.3, 0.4, 1.0]; 3]).unwrap();
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
fn voronoi_matching_modes_are_deterministic_and_hsv_fades_hue_for_neutrals() {
    let source = chromiator::document::PixelImage::new(1, 1, vec![[0.5, 0.5, 0.5, 0.22]]).unwrap();
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
        chromiator::document::PixelImage::new(1, 1, vec![[blue[0], blue[1], blue[2], 0.6]])
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
        chromiator::document::PixelImage::new(1, 1, vec![[seam[0], seam[1], seam[2], 1.0]])
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
        let source = chromiator::document::PixelImage::new(
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
