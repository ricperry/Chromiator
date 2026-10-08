use std::collections::BTreeSet;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use chromiator::color::hsv_to_encoded;
use chromiator::document::{
    BlendSpace, ComponentOperation, Document, ExportDefaults, PixelImage, Recipe,
    TransitionProfile, VoronoiMatching,
};
use chromiator::export::{self, ExportFormat};
use chromiator::preset::Preset;
use chromiator::processing::{CompiledVoronoi, bounded_preview, linear_to_srgb, process};
use chromiator::project;
use chromiator::raster;
use chromiator::starter_looks::{STARTER_LOOKS, StarterLook, recipe_for_starter_look};
use image::{ImageFormat, Rgba, RgbaImage};

const RELEASE_COMMIT: &str = "7f7f7353ff315c73636e5e3afc7a82ac9e7a37be";
const FIXTURES: &str = "tests/fixtures/release-0.2.0";

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURES)
}

fn fixture(name: &str) -> PathBuf {
    fixture_dir().join(name)
}

fn candidate_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("target/validation/stage-0/release-anchor-candidate")
}

fn read_fixture(name: &str) -> Vec<u8> {
    fs::read(fixture(name)).unwrap_or_else(|error| panic!("cannot read fixture {name}: {error}"))
}

fn encoded_u64(values: &[u64]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn encoded_f32(image: &PixelImage) -> Vec<u8> {
    image
        .pixels
        .iter()
        .flat_map(|pixel| pixel.iter().flat_map(|value| value.to_bits().to_le_bytes()))
        .collect()
}

fn winner_site_ids(source: &PixelImage, recipe: &Recipe) -> Vec<u64> {
    let compiled = CompiledVoronoi::compile(recipe).unwrap();
    let ids: Vec<_> = compiled.site_contexts().map(|site| site.id).collect();
    source
        .pixels
        .iter()
        .map(|pixel| {
            if pixel[3] == 0.0 {
                0
            } else {
                compiled.winner_index(*pixel).map_or(0, |index| ids[index])
            }
        })
        .collect()
}

fn hard_recipe(matching: VoronoiMatching) -> Recipe {
    let look = STARTER_LOOKS
        .iter()
        .copied()
        .find(|look| look.id == "desert-dusk")
        .expect("Desert Dusk is a release built-in");
    let mut recipe = recipe_for_starter_look(look);
    recipe.voronoi.matching = matching;
    recipe.voronoi.transition = recipe.voronoi.transition.with_width(0.0);
    recipe.preprocessing.input_smoothing = 0.0;
    recipe.set_hue_degrees(0.0);
    recipe
}

fn source_fixture() -> (Vec<u8>, PixelImage) {
    let bytes = read_fixture("stage0-scene.png");
    let image = raster::decode(&bytes, None).unwrap();
    (bytes, image.pixels)
}

fn embedded_preset_json(id: &str) -> Option<&'static str> {
    match id {
        "desert-dusk" => Some(include_str!("../resources/presets/desert-dusk.json")),
        "arcade-four" => Some(include_str!("../resources/presets/arcade-four.json")),
        "mimeograph" => Some(include_str!("../resources/presets/mimeograph.json")),
        "photocopy" => Some(include_str!("../resources/presets/photocopy.json")),
        "old-newsprint" => Some(include_str!("../resources/presets/old-newsprint.json")),
        "smudged-graphite" => Some(include_str!("../resources/presets/smudged-graphite.json")),
        "watercolor" => Some(include_str!("../resources/presets/watercolor.json")),
        _ => None,
    }
}

fn emitted_preset(look: StarterLook) -> Preset {
    if let Some(source_json) = embedded_preset_json(look.id) {
        serde_json::from_str(source_json).expect("release preset JSON is valid")
    } else {
        let recipe = recipe_for_starter_look(look);
        Preset::new(look.name, Some(look.description.to_owned()), &recipe)
            .expect("release Rust-defined preset is valid")
    }
}

fn scene_bytes() -> Vec<u8> {
    let mut pixels = RgbaImage::new(256, 128);
    for y in 0..128 {
        for x in 0..256 {
            let code = x as u8;
            let rgb = match y / 32 {
                0 => [code; 3],
                1 => [code, ((255 - x) as f64 * 0.72).round() as u8, code / 3],
                2 => {
                    let hue = x as f64 * 360.0 / 256.0;
                    let saturation = 0.08 + x as f64 / 255.0 * 0.92;
                    let value = 0.04 + (y % 32) as f64 / 31.0 * 0.96;
                    hsv_to_encoded([hue, saturation, value])
                        .map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8)
                }
                _ => flat_landscape(x, y - 96),
            };
            pixels.put_pixel(x, y, Rgba([rgb[0], rgb[1], rgb[2], code]));
        }
    }
    let mut cursor = Cursor::new(Vec::new());
    pixels.write_to(&mut cursor, ImageFormat::Png).unwrap();
    cursor.into_inner()
}

fn flat_landscape(x: u32, y: u32) -> [u8; 3] {
    let sky = [163, 200, 211];
    let grass = [92, 124, 70];
    let xx = x as i32;
    let yy = y as i32;
    let sun_x = xx - 218;
    let sun_y = yy - 10;
    if sun_x * sun_x + sun_y * sun_y <= 36 {
        return [247, 207, 88];
    }
    if (28..119).contains(&xx) && yy >= 4 + (xx - 74).abs() / 2 && yy < 25 {
        return [83, 91, 107];
    }
    if (65..164).contains(&xx) && yy >= 8 + (xx - 115).abs() / 2 && yy < 25 {
        return [115, 121, 125];
    }
    if (148..203).contains(&xx) && yy >= 7 + (xx - 175).abs() / 2 && yy < 20 {
        return [111, 57, 47];
    }
    if (154..197).contains(&xx) && (18..31).contains(&yy) {
        if (161..171).contains(&xx) && (21..27).contains(&yy) {
            return [93, 164, 177];
        }
        if (181..190).contains(&xx) && (24..31).contains(&yy) {
            return [71, 48, 42];
        }
        return [231, 200, 153];
    }
    if yy < 24 { sky } else { grass }
}

fn write_png(path: &Path, image: &PixelImage) {
    export::export(path, image, ExportFormat::Png8).unwrap();
}

fn write_winners(path: &Path, source: &PixelImage, recipe: &Recipe) {
    fs::write(path, encoded_u64(&winner_site_ids(source, recipe))).unwrap();
}

#[test]
#[ignore = "writes immutable Stage 0 anchors from the verified 0.2.0 processing source"]
fn generate_release_0_2_0_anchors() {
    let directory = candidate_dir();
    assert!(
        !directory.exists(),
        "refusing to overwrite release anchor candidate {}; move it aside to recapture",
        directory.display()
    );
    fs::create_dir_all(directory.join("builtins")).unwrap();
    let scene = scene_bytes();
    fs::write(directory.join("stage0-scene.png"), &scene).unwrap();
    let decoded = raster::decode(&scene, None).unwrap();
    let source = decoded.pixels;
    let base = hard_recipe(VoronoiMatching::Perceptual);

    let project_document = Document {
        source_name: "stage0-scene.png".into(),
        source_bytes: scene.clone().into(),
        source: source.clone(),
        interpretation: decoded.interpretation,
        recipe: recipe_for_starter_look(
            STARTER_LOOKS
                .iter()
                .copied()
                .find(|look| look.id == "desert-dusk")
                .unwrap(),
        ),
        export_defaults: ExportDefaults::default(),
        dirty: false,
    };
    project::save(
        &directory.join("desert-dusk-v6.chromiator"),
        &project_document,
    )
    .unwrap();

    for matching in [
        VoronoiMatching::Perceptual,
        VoronoiMatching::Rgb,
        VoronoiMatching::Hsv,
    ] {
        let recipe = hard_recipe(matching);
        let filename = match matching {
            VoronoiMatching::Perceptual => "winner-site-ids-oklab.u64le",
            VoronoiMatching::Rgb => "winner-site-ids-rgb.u64le",
            VoronoiMatching::Hsv => "winner-site-ids-hsv.u64le",
            VoronoiMatching::Okhsl => unreachable!("not in the released creator catalog"),
        };
        write_winners(&directory.join(filename), &source, &recipe);
    }

    let hard_render = process(&source, &base);
    fs::write(
        directory.join("desert-dusk-hard.f32le"),
        encoded_f32(&hard_render),
    )
    .unwrap();
    write_png(&directory.join("desert-dusk-hard.png"), &hard_render);

    let mut blended = base.clone();
    blended.voronoi.transition = blended.voronoi.transition.with_width(1.0);
    let blended_render = process(&source, &blended);
    fs::write(
        directory.join("desert-dusk-width-100.f32le"),
        encoded_f32(&blended_render),
    )
    .unwrap();
    write_png(
        &directory.join("desert-dusk-width-100.png"),
        &blended_render,
    );

    let mut source_edit = base.clone();
    source_edit.voronoi.sites[2].source_color = [0.08, 0.58, 0.10, 1.0];
    write_winners(
        &directory.join("winner-site-ids-source-edited.u64le"),
        &source,
        &source_edit,
    );
    let mut influence_edit = base.clone();
    influence_edit.voronoi.sites[0].influence = 4.0;
    write_winners(
        &directory.join("winner-site-ids-influence-edited.u64le"),
        &source,
        &influence_edit,
    );

    let mut ordered_hue = base.clone();
    let mut second = ordered_hue.steps[0].clone();
    ordered_hue.steps[0].operation = ComponentOperation::RotateHue { degrees: 37.0 };
    second.operation = ComponentOperation::RotateHue { degrees: -22.0 };
    ordered_hue.steps.push(second);
    fs::write(
        directory.join("desert-dusk-hue-ordered.f32le"),
        encoded_f32(&process(&source, &ordered_hue)),
    )
    .unwrap();
    let mut reversed_hue = base.clone();
    let mut second = reversed_hue.steps[0].clone();
    reversed_hue.steps[0].operation = ComponentOperation::RotateHue { degrees: -22.0 };
    second.operation = ComponentOperation::RotateHue { degrees: 37.0 };
    reversed_hue.steps.push(second);
    fs::write(
        directory.join("desert-dusk-hue-reversed.f32le"),
        encoded_f32(&process(&source, &reversed_hue)),
    )
    .unwrap();

    let mut index = Vec::new();
    for look in STARTER_LOOKS.iter().copied() {
        let preset = emitted_preset(look);
        let filename = format!("builtins/{}.json", look.id);
        fs::write(
            directory.join(&filename),
            serde_json::to_vec_pretty(&preset).unwrap(),
        )
        .unwrap();
        index.push(serde_json::json!({
            "id": look.id,
            "name": look.name,
            "source": if embedded_preset_json(look.id).is_some() {
                format!("resources/presets/{}.json", look.id)
            } else {
                "src/starter_looks.rs".to_owned()
            },
            "serialized": filename,
        }));
    }
    fs::write(
        directory.join("builtins/index.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "release_commit": RELEASE_COMMIT,
            "preset_version": 3,
            "presets": index,
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        directory.join("STAGE0-RESULTS.md"),
        format!(
            "# Stage 0 release anchor candidate\n\nGenerated by the ignored regression generator with release commit `{RELEASE_COMMIT}` as the intended source baseline. Before treating this candidate as release evidence, verify the processing, raster, project, preset, and starter-look sources against that commit and record any differences.\n\nDimensions: 256×128. Rows 0–31 are a neutral ramp, 32–63 a channel ramp, 64–95 an HSV hue/value sweep, and 96–127 a small flat landscape with boundaries. Every x coordinate supplies one of the 256 alpha codes; alpha-zero pixels include nonblack RGB.\n\nThe candidate directory is review-only. The ignored generator refuses to overwrite it. After review, copy the bounded fixture set to `tests/fixtures/release-0.2.0` without clobbering unrelated files.\n"
        ),
    )
    .unwrap();
}

#[test]
fn frozen_release_builtin_presets_match_all_nineteen_definitions() {
    assert_eq!(STARTER_LOOKS.len(), 19);
    let index: serde_json::Value =
        serde_json::from_slice(&read_fixture("builtins/index.json")).unwrap();
    assert_eq!(index["release_commit"], RELEASE_COMMIT);
    assert_eq!(index["preset_version"], 3);
    let records = index["presets"].as_array().unwrap();
    assert_eq!(records.len(), STARTER_LOOKS.len());

    for (look, record) in STARTER_LOOKS.iter().copied().zip(records) {
        assert_eq!(record["id"], look.id);
        assert_eq!(record["name"], look.name);
        let expected = fs::read(fixture(&format!("builtins/{}.json", look.id))).unwrap();
        let frozen: Preset = serde_json::from_slice(&expected).unwrap();
        assert_eq!(
            recipe_for_starter_look(look),
            frozen.processing.recipe(),
            "applied recipe changed: {}",
            look.id
        );
        let actual = serde_json::to_vec_pretty(&emitted_preset(look)).unwrap();
        assert_eq!(actual, expected, "released preset changed: {}", look.id);
    }
}

#[test]
fn desert_dusk_hard_and_blended_pixels_match_release_goldens() {
    let (source_bytes, source) = source_fixture();
    let hard_recipe = hard_recipe(VoronoiMatching::Perceptual);
    let hard = process(&source, &hard_recipe);
    assert_eq!(encoded_f32(&hard), read_fixture("desert-dusk-hard.f32le"));
    assert_eq!(
        winner_site_ids(&source, &hard_recipe),
        decode_u64(&read_fixture("winner-site-ids-oklab.u64le"))
    );

    let preview = process(&bounded_preview(&source), &hard_recipe);
    assert_eq!(encoded_f32(&preview), encoded_f32(&hard));
    let temporary = tempfile::tempdir().unwrap();
    let exported = temporary.path().join("export.png");
    export::export_recipe(&exported, &source, &hard_recipe, ExportFormat::Png8).unwrap();
    assert_eq!(
        fs::read(exported).unwrap(),
        read_fixture("desert-dusk-hard.png")
    );

    let source_png = image::load_from_memory(&source_bytes).unwrap().to_rgba8();
    let hard_png = image::load_from_memory(&read_fixture("desert-dusk-hard.png"))
        .unwrap()
        .to_rgba8();
    assert_eq!(hard_png.dimensions(), (256, 128));
    let target_codes: BTreeSet<[u8; 3]> = hard_recipe
        .voronoi
        .sites
        .iter()
        .map(|site| {
            site.target_color
                .map(|value| (linear_to_srgb(value.clamp(0.0, 1.0)) * 255.0).round() as u8)
        })
        .collect();
    let actual_codes: BTreeSet<[u8; 3]> = hard_png
        .pixels()
        .filter(|pixel| pixel[3] != 0)
        .map(|pixel| [pixel[0], pixel[1], pixel[2]])
        .collect();
    assert_eq!(
        actual_codes, target_codes,
        "hard width must use all five Target codes"
    );
    for (source_pixel, output_pixel) in source_png.pixels().zip(hard_png.pixels()) {
        assert_eq!(source_pixel[3], output_pixel[3], "PNG8 alpha changed");
        if output_pixel[3] != 0 {
            assert!(target_codes.contains(&[output_pixel[0], output_pixel[1], output_pixel[2]]));
        }
    }
    assert!(
        source_png
            .pixels()
            .any(|pixel| pixel[3] == 0 && [pixel[0], pixel[1], pixel[2]] != [0, 0, 0])
    );

    let mut blended_recipe = hard_recipe.clone();
    blended_recipe.voronoi.transition = TransitionProfile::HARD.with_width(1.0);
    let blended = process(&source, &blended_recipe);
    assert_eq!(
        encoded_f32(&blended),
        read_fixture("desert-dusk-width-100.f32le")
    );
    assert_eq!(
        winner_site_ids(&source, &blended_recipe),
        winner_site_ids(&source, &hard_recipe),
        "transition width must not change winner assignment"
    );
    let mut linear_blend_recipe = blended_recipe.clone();
    linear_blend_recipe.voronoi.transition.blend_space = BlendSpace::LinearRgb;
    let linear_blend = process(&source, &linear_blend_recipe);
    assert_eq!(
        winner_site_ids(&source, &linear_blend_recipe),
        winner_site_ids(&source, &blended_recipe),
        "blend-space choice must not change winner assignment"
    );
    assert_ne!(encoded_f32(&linear_blend), encoded_f32(&blended));
    let blended_png = image::load_from_memory(&read_fixture("desert-dusk-width-100.png"))
        .unwrap()
        .to_rgba8();
    let blended_codes: BTreeSet<[u8; 3]> = blended_png
        .pixels()
        .filter(|pixel| pixel[3] != 0)
        .map(|pixel| [pixel[0], pixel[1], pixel[2]])
        .collect();
    assert!(blended_codes.len() > target_codes.len());
}

#[test]
fn target_edits_preserve_full_winner_maps_source_and_influence_edits_move_them() {
    let (_, source) = source_fixture();
    for (matching, frozen) in [
        (VoronoiMatching::Perceptual, "winner-site-ids-oklab.u64le"),
        (VoronoiMatching::Rgb, "winner-site-ids-rgb.u64le"),
        (VoronoiMatching::Hsv, "winner-site-ids-hsv.u64le"),
    ] {
        let baseline = winner_site_ids(&source, &hard_recipe(matching));
        assert_eq!(encoded_u64(&baseline), read_fixture(frozen));
        let mut target_edit = hard_recipe(matching);
        target_edit.voronoi.sites[0].target_color = [0.7, 0.1, 0.6];
        assert_eq!(winner_site_ids(&source, &target_edit), baseline);
    }
    assert_ne!(
        winner_site_ids(&source, &hard_recipe(VoronoiMatching::Rgb)),
        winner_site_ids(&source, &hard_recipe(VoronoiMatching::Hsv)),
        "the fixture must distinguish RGB and HSV matching"
    );

    let baseline = winner_site_ids(&source, &hard_recipe(VoronoiMatching::Perceptual));
    let mut source_edit = hard_recipe(VoronoiMatching::Perceptual);
    source_edit.voronoi.sites[2].source_color = [0.08, 0.58, 0.10, 1.0];
    let source_map = winner_site_ids(&source, &source_edit);
    assert_eq!(
        encoded_u64(&source_map),
        read_fixture("winner-site-ids-source-edited.u64le")
    );
    assert!(source_map.iter().zip(&baseline).any(|(a, b)| a != b));

    let mut influence_edit = hard_recipe(VoronoiMatching::Perceptual);
    influence_edit.voronoi.sites[0].influence = 4.0;
    let influence_map = winner_site_ids(&source, &influence_edit);
    assert_eq!(
        encoded_u64(&influence_map),
        read_fixture("winner-site-ids-influence-edited.u64le")
    );
    assert!(influence_map.iter().zip(&baseline).any(|(a, b)| a != b));
}

#[test]
fn frozen_project_reopens_source_recipe_and_render_exactly() {
    let document = project::open(&fixture("desert-dusk-v6.chromiator")).unwrap();
    let source_bytes = read_fixture("stage0-scene.png");
    let decoded_source = raster::decode(&source_bytes, None).unwrap().pixels;
    assert_eq!(document.source_bytes.as_ref(), source_bytes);
    assert_eq!(encoded_f32(&document.source), encoded_f32(&decoded_source));
    assert_eq!(document.source_name, "stage0-scene.png");
    assert_eq!(
        document.recipe,
        recipe_for_starter_look(
            STARTER_LOOKS
                .iter()
                .copied()
                .find(|look| look.id == "desert-dusk")
                .unwrap()
        )
    );
    assert_eq!(
        encoded_f32(&process(&document.source, &document.recipe)),
        read_fixture("desert-dusk-width-100.f32le")
    );

    let temporary = tempfile::tempdir().unwrap();
    let round_trip = temporary.path().join("round-trip.chromiator");
    project::save(&round_trip, &document).unwrap();
    let reopened = project::open(&round_trip).unwrap();
    assert_eq!(reopened.source_bytes.as_ref(), source_bytes);
    assert_eq!(encoded_f32(&reopened.source), encoded_f32(&decoded_source));
    assert_eq!(reopened.recipe, document.recipe);
    assert_eq!(
        encoded_f32(&process(&reopened.source, &reopened.recipe)),
        read_fixture("desert-dusk-width-100.f32le")
    );
}

#[test]
fn smoothing_hue_order_and_empty_sites_keep_their_released_limits() {
    let (_, source) = source_fixture();
    let base = hard_recipe(VoronoiMatching::Perceptual);

    let mut smoothed = base.clone();
    smoothed.preprocessing.input_smoothing = 1.0;
    let smoothed_render = process(&source, &smoothed);
    assert!(
        smoothed_render
            .pixels
            .iter()
            .zip(source.pixels.iter())
            .any(|(after, before)| after[3].to_bits() != before[3].to_bits()),
        "smoothing can change alpha"
    );

    let mut ordered = base.clone();
    let mut second = ordered.steps[0].clone();
    ordered.steps[0].operation = ComponentOperation::RotateHue { degrees: 37.0 };
    second.operation = ComponentOperation::RotateHue { degrees: -22.0 };
    ordered.steps.push(second);
    assert_eq!(
        encoded_f32(&process(&source, &ordered)),
        read_fixture("desert-dusk-hue-ordered.f32le")
    );
    let mut reversed = base.clone();
    let mut second = reversed.steps[0].clone();
    reversed.steps[0].operation = ComponentOperation::RotateHue { degrees: -22.0 };
    second.operation = ComponentOperation::RotateHue { degrees: 37.0 };
    reversed.steps.push(second);
    assert_eq!(
        encoded_f32(&process(&source, &reversed)),
        read_fixture("desert-dusk-hue-reversed.f32le")
    );
    assert_ne!(
        encoded_f32(&process(&source, &ordered)),
        encoded_f32(&process(&source, &reversed))
    );

    let mut empty = base;
    empty.voronoi.sites.clear();
    let pass_through = process(&source, &empty);
    for (input, output) in source.pixels.iter().zip(pass_through.pixels.iter()) {
        if input[3] == 0.0 {
            assert_eq!(*output, [0.0; 4]);
        } else {
            assert_eq!(*input, *output);
        }
    }
}

fn decode_u64(bytes: &[u8]) -> Vec<u64> {
    bytes
        .chunks_exact(8)
        .map(|chunk| u64::from_le_bytes(chunk.try_into().unwrap()))
        .collect()
}
