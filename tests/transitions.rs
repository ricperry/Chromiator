//! Behavior-level transition regressions; rendering fixtures are opt-in artifacts.
use std::sync::atomic::{AtomicU64, Ordering};
use chromiator::color::{linear_to_oklab, oklab_to_linear};
use chromiator::document::{BlendSpace, PixelImage, Recipe, SampleSize, TransitionProfile, VoronoiMatching, VoronoiSite};
use chromiator::export::{self, ExportFormat};
use chromiator::preset::{Preset, PresetStore};
use chromiator::processing::{process, process_cancellable_with_progress_and_coverage, srgb_to_linear};
use chromiator::session::{DocumentSession, EditCommand, EditGesture};
use chromiator::{example, project};

fn recipe(points: &[[f32; 3]]) -> Recipe {
    let targets = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0; 3], [0.04, 0.015, 0.1]];
    let mut recipe = Recipe::default();
    recipe.steps.clear();
    recipe.voronoi.matching = VoronoiMatching::Rgb;
    recipe.voronoi.transition = TransitionProfile { blend_space: BlendSpace::LinearRgb, ..TransitionProfile::HARD.with_width(1.0) };
    recipe.voronoi.sites = points.iter().enumerate().map(|(i, p)| {
        let c = p.map(srgb_to_linear);
        VoronoiSite { id: i as u64 + 1, order: i as u64, source_color: [c[0], c[1], c[2], 1.0],
            target_color: targets[i % targets.len()], influence: 0.0, locked: false,
            position: None, size: SampleSize::Point }
    }).collect();
    recipe.voronoi.next_site_id = points.len() as u64 + 1;
    recipe
}

fn sample(recipe: &Recipe, encoded: [f32; 3]) -> [f32; 4] {
    let rgb = encoded.map(srgb_to_linear);
    let input = PixelImage::new(1, 1, vec![[rgb[0], rgb[1], rgb[2], 0.4]]).unwrap();
    process(&input, recipe).pixels[0]
}

fn near(actual: [f32; 4], expected: [f32; 4], tolerance: f32) {
    for (a, e) in actual.into_iter().zip(expected) { assert!((a - e).abs() <= tolerance, "{actual:?} != {expected:?}"); }
}

#[test]
fn two_sites_have_exact_centers_halfway_mixture_and_compact_support() {
    let mut r = recipe(&[[0.0; 3], [1.0, 0.0, 0.0]]);
    near(sample(&r, [0.0; 3]), [1.0, 0.0, 0.0, 0.4], 0.0);
    near(sample(&r, [1.0, 0.0, 0.0]), [0.0, 1.0, 0.0, 0.4], 0.0);
    near(sample(&r, [0.5, 0.0, 0.0]), [0.5, 0.5, 0.0, 0.4], 1.0e-6);
    r.voronoi.transition = r.voronoi.transition.with_width(0.25);
    near(sample(&r, [0.2, 0.0, 0.0]), [1.0, 0.0, 0.0, 0.4], 0.0);
}

#[test]
fn competitive_sites_mix_continuously_across_winner_change() {
    let r = recipe(&[[0.0; 3], [0.5, 0.0, 0.0], [1.0, 0.0, 0.0]]);
    let multisite = sample(&r, [0.25, 0.0, 0.0]);
    assert!(multisite[2] > 0.01);
    near(sample(&r, [0.25 - 1.0e-5, 0.0, 0.0]), sample(&r, [0.25 + 1.0e-5, 0.0, 0.0]), 1.0e-4);
}

#[test]
fn three_and_four_way_junctions_mix_without_a_contributor_cap() {
    for points in [vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]] {
        let r = recipe(&points);
        let level = if points.len() == 3 { 1.0 / 3.0 } else { 0.5 };
        near(sample(&r, [0.5; 3]), [level, level, level, 0.4], 1.0e-6);
        near(sample(&r, [0.5 - 1.0e-5, 0.5, 0.5]), sample(&r, [0.5 + 1.0e-5, 0.5, 0.5]), 1.0e-4);
    }
}

#[test]
fn coincident_sites_mix_only_when_width_is_positive() {
    let mut r = recipe(&[[0.2; 3]; 5]);
    let expected = r.voronoi.sites.iter().fold([0.0; 3], |sum, site| std::array::from_fn(|i| sum[i] + site.target_color[i] / 5.0));
    near(sample(&r, [0.2; 3]), [expected[0], expected[1], expected[2], 0.4], 1.0e-6);
    r.voronoi.transition = r.voronoi.transition.with_width(0.0);
    near(sample(&r, [0.2; 3]), [1.0, 0.0, 0.0, 0.4], 0.0);
}

#[test]
fn influence_moves_the_blended_boundary_without_changing_the_metric() {
    let mut r = recipe(&[[0.0; 3], [1.0, 0.0, 0.0]]);
    for influence in [-4.0_f64, 0.0, 4.0] {
        r.voronoi.sites[0].influence = influence;
        let boundary = (1.0 / (1.0 + 2.0_f64.powf(-influence * 0.5))) as f32;
        near(sample(&r, [boundary, 0.0, 0.0]), [0.5, 0.5, 0.0, 0.4], 1.0e-5);
    }
}

#[test]
fn all_matching_spaces_preserve_hard_pixels_coverage_and_alpha() {
    let mut r = recipe(&[[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0; 3]]);
    let input = PixelImage::new(4, 1, vec![[0.2, 0.3, 0.4, 0.4], [0.7, 0.1, 0.0, 1.0], [1.0, 0.5, 0.1, 0.0], [0.0; 4]]).unwrap();
    for matching in [VoronoiMatching::Perceptual, VoronoiMatching::Rgb, VoronoiMatching::Hsv, VoronoiMatching::Okhsl] {
        r.voronoi.matching = matching;
        r.voronoi.transition = TransitionProfile::HARD;
        let (hard, coverage) = process_cancellable_with_progress_and_coverage(&input, &r, 0, &AtomicU64::new(0), |_| {}).unwrap();
        for space in [BlendSpace::Oklab, BlendSpace::LinearRgb] {
                r.voronoi.transition = TransitionProfile { blend_space: space, ..TransitionProfile::HARD };
                assert_eq!(process(&input, &r), hard);
                r.voronoi.transition = r.voronoi.transition.with_width(1.0);
                let (soft, soft_coverage) = process_cancellable_with_progress_and_coverage(&input, &r, 0, &AtomicU64::new(0), |_| {}).unwrap();
                assert_eq!(soft_coverage, coverage);
                assert_eq!(soft.pixels[0][3], 0.4);
                assert_eq!(soft.pixels[2], [0.0; 4]);
                assert!(soft.pixels.iter().flatten().all(|v| v.is_finite() && (0.0..=1.0).contains(v)));
        }
    }
}

#[test]
fn oklab_mix_differs_from_linear_rgb_and_maps_saturated_mixtures_into_gamut() {
    let mut r = recipe(&[[0.0; 3], [1.0, 0.0, 0.0]]);
    let linear = sample(&r, [0.5, 0.0, 0.0]);
    r.voronoi.transition.blend_space = BlendSpace::Oklab;
    assert_ne!(sample(&r, [0.5, 0.0, 0.0]), linear);
    let colors = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 1.0], [0.0, 1.0, 1.0], [1.0, 1.0, 0.0]];
    let mut exercised_mapping = false;
    for a in colors { for b in colors {
        r.voronoi.sites[0].target_color = a;
        r.voronoi.sites[1].target_color = b;
        let aa = linear_to_oklab(a.map(f64::from));
        let bb = linear_to_oklab(b.map(f64::from));
        let raw = oklab_to_linear(std::array::from_fn(|i| (aa[i] + bb[i]) * 0.5));
        exercised_mapping |= raw.iter().any(|v| !(0.0..=1.0).contains(v));
        assert!(sample(&r, [0.5, 0.0, 0.0]).into_iter().all(|v| v.is_finite() && (0.0..=1.0).contains(&v)));
    }}
    assert!(exercised_mapping);
}

#[test]
fn current_hard_files_default_options_and_soft_profiles_round_trip() {
    let old = r#"{"start":0.5,"midpoint":0.5,"end":0.5}"#;
    let profile: TransitionProfile = serde_json::from_str(old).unwrap();
    assert_eq!(profile, TransitionProfile::HARD);
    assert_eq!(serde_json::to_value(profile).unwrap(), serde_json::from_str::<serde_json::Value>(old).unwrap());
    let mut document = example::spectrum_document().unwrap();
    document.recipe.voronoi.transition = TransitionProfile { blend_space: BlendSpace::LinearRgb, ..TransitionProfile::HARD.with_width(0.25) };
    for removed in ["Multisite", "SharedBorders"] {
        let mut obsolete = serde_json::to_value(document.recipe.voronoi.transition).unwrap();
        obsolete["blend_scope"] = serde_json::json!(removed);
        assert!(serde_json::from_value::<TransitionProfile>(obsolete).is_err());
    }
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("blended.chromiator");
    project::save(&path, &document).unwrap();
    let reopened = project::open(&path).unwrap();
    assert_eq!(reopened.recipe, document.recipe);
    let store = PresetStore::at(directory.path().join("presets"));
    let preset = Preset::new("Blended", None, &document.recipe).unwrap();
    store.save(&preset, false).unwrap();
    assert_eq!(store.scan().unwrap().entries[0].preset, preset);
}

#[test]
fn transition_edits_coalesce_undo_restore_savepoints_and_reject_invalid_widths() {
    let mut session = DocumentSession::default();
    session.load(example::spectrum_document().unwrap()).unwrap();
    session.edit(EditCommand::SetTransitionWidth(0.25), Some(EditGesture::TransitionWidth)).unwrap();
    session.edit(EditCommand::SetTransitionWidth(0.5), Some(EditGesture::TransitionWidth)).unwrap();
    assert_eq!(session.undo_len(), 1);
    session.finish_gesture(EditGesture::TransitionWidth);
    session.edit(EditCommand::SetBlendSpace(BlendSpace::LinearRgb), None).unwrap();
    let soft = session.document().unwrap().recipe.clone();
    session.mark_saved();
    session.undo().unwrap();
    assert!(session.is_dirty());
    session.redo().unwrap();
    assert!(!session.is_dirty());
    assert_eq!(session.document().unwrap().recipe, soft);
    for invalid in [f32::NAN, f32::INFINITY, -0.1, 1.1] { assert!(session.edit(EditCommand::SetTransitionWidth(invalid), None).is_err()); }
    assert_eq!(session.document().unwrap().recipe, soft);
    assert!(!session.edit(EditCommand::SetBlendSpace(BlendSpace::LinearRgb), None).unwrap().changed);
}

#[test]
fn preview_and_png_export_share_transition_pixels_and_cancelled_jobs_publish_nothing() {
    let r = recipe(&[[0.0; 3], [1.0, 0.0, 0.0]]);
    let source = PixelImage::new(64, 16, (0..1024).map(|i| [srgb_to_linear((i % 64) as f32 / 63.0), 0.0, 0.0, 1.0]).collect()).unwrap();
    let preview = process(&source, &r);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("gradient.png");
    export::export_recipe(&path, &source, &r, ExportFormat::Png16).unwrap();
    let decoded = image::open(path).unwrap().to_rgba16();
    for (actual, expected) in decoded.pixels().zip(preview.pixels.iter()) {
        for channel in 0..3 {
            let code = (chromiator::processing::linear_to_srgb(expected[channel]) * 65535.0).round();
            assert!((f32::from(actual[channel]) - code).abs() <= 1.0);
        }
    }
    let generation = AtomicU64::new(4);
    assert!(process_cancellable_with_progress_and_coverage(&source, &r, 4, &generation, |_| { generation.store(5, Ordering::Release); }).is_none());
}

#[test]
#[ignore = "Writes review images, project and timing evidence under target/validation/transitions-multisite"]
fn render_transition_evidence() {
    let directory = std::path::Path::new("target/validation/transitions-multisite");
    std::fs::create_dir_all(directory).unwrap();
    let source = PixelImage::new(256, 256, (0..65536).map(|i| [srgb_to_linear((i % 256) as f32 / 255.0), srgb_to_linear((i / 256) as f32 / 255.0), srgb_to_linear(0.5), 1.0]).collect()).unwrap();
    let mut r = recipe(&[[0.0, 0.0, 0.5], [1.0, 0.0, 0.5], [0.0, 1.0, 0.5], [1.0, 1.0, 0.5], [0.45, 0.6, 0.5]]);
    export::export(&directory.join("source.png"), &source, ExportFormat::Png8).unwrap();
    for space in [BlendSpace::Oklab, BlendSpace::LinearRgb] {
            for width in [0.0, 0.25, 1.0] {
                r.voronoi.transition = TransitionProfile { blend_space: space, ..TransitionProfile::HARD.with_width(width) };
                let start = std::time::Instant::now();
                let rendered = process(&source, &r);
                println!("{space:?} width={width} five sites 256x256: {:?}", start.elapsed());
                export::export(&directory.join(format!("{space:?}-{width}.png")), &rendered, ExportFormat::Png8).unwrap();
            }
    }
    let mut document = example::spectrum_document().unwrap();
    document.recipe = r;
    document.recipe.voronoi.transition = document.recipe.voronoi.transition.with_width(0.25);
    project::save(&directory.join("native-transitions.chromiator"), &document).unwrap();
}
