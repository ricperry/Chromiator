use chromiator::{document::PixelImage, preset::Preset, processing::process,
    starter_looks::{STARTER_LOOKS, recipe_for_starter_look}};

#[test]
fn palettes_roundtrip_and_reductive_looks_keep_their_ink_limits() {
    let counts = [4, 5, 5, 5, 5, 5, 4, 5, 5, 5, 5, 2, 2, 2, 3, 3, 3, 5, 13];
    assert_eq!(STARTER_LOOKS[..5].iter().map(|look| look.id).collect::<Vec<_>>(),
        ["ink-paper", "desert-dusk", "blueprint", "arcade-four", "night-neon"]);
    for (look, count) in STARTER_LOOKS.iter().zip(counts) {
        let recipe = recipe_for_starter_look(*look);
        assert_eq!(recipe.voronoi.sites.len(), count, "{}", look.name);
        let preset = Preset::new(look.name, Some(look.description.into()), &recipe).unwrap();
        let roundtrip: Preset = serde_json::from_str(&serde_json::to_string(&preset).unwrap()).unwrap();
        assert_eq!(roundtrip.processing.recipe(), recipe);
        let samples = PixelImage::new(count as u32, 1,
            recipe.voronoi.sites.iter().map(|s| s.source_color).collect()).unwrap();
        // Anchor checks isolate color mapping from intentionally authored smoothing.
        let mut mapping = recipe.clone();
        mapping.preprocessing.input_smoothing = 0.0;
        let output = process(&samples, &mapping);
        for (pixel, site) in output.pixels.iter().zip(&recipe.voronoi.sites) {
            assert_eq!(&pixel[..3], &site.target_color);
        }
    }
}

/// Reproducible artwork review, without modifying user fixtures.
#[test]
#[ignore = "Writes preset review images under target/validation/preset-library"]
fn render_preset_library() {
    use chromiator::export::{export, ExportFormat};
    let dir = std::path::Path::new("target/validation/preset-library");
    std::fs::create_dir_all(dir).unwrap();
    let source = chromiator::example::spectrum_document().unwrap().source;
    let mut sheet = image::RgbImage::new(960, (STARTER_LOOKS.len() + 1).div_ceil(3) as u32 * 180);
    let mut outputs = Vec::new();
    let mut report = String::from("# Preset library\n\nContact sheet: three columns, read left to right, top to bottom. Source first, then all presets in menu order. Each tile is 320 x 180. Soft transitions may produce more colors than sites.\n\n");
    for index in 0..=STARTER_LOOKS.len() {
        let (name, output) = if index == 0 { ("source", source.clone()) } else {
            let look = STARTER_LOOKS[index - 1];
            (look.id, process(&source, &recipe_for_starter_look(look)))
        };
        if index > 0 {
            assert!(outputs.iter().all(|previous| previous != &output), "duplicate output: {name}");
            let colors: std::collections::BTreeSet<_> = output.pixels.iter()
                .filter(|p| p[3] > 0.0).map(|p| [p[0].to_bits(), p[1].to_bits(), p[2].to_bits()]).collect();
            let recipe = recipe_for_starter_look(STARTER_LOOKS[index - 1]);
            if recipe.voronoi.transition.width() == 0.0 {
                assert_eq!(colors.len(), recipe.voronoi.sites.len(), "unused color: {name}");
            } else { assert!(colors.len() >= recipe.voronoi.sites.len(), "lost blended structure: {name}"); }
            report.push_str(&format!("{index}. {}: {} effective colors\n", STARTER_LOOKS[index - 1].name, colors.len()));
            outputs.push(output.clone());
        }
        let path = dir.join(format!("{name}.png"));
        export(&path, &output, ExportFormat::Png8).unwrap();
        let tile = image::imageops::resize(&image::open(path).unwrap().to_rgb8(), 320, 180, image::imageops::FilterType::Nearest);
        image::imageops::replace(&mut sheet, &tile, ((index % 3) * 320) as i64, ((index / 3) * 180) as i64);
    }
    sheet.save(dir.join("contact-sheet.png")).unwrap();
    std::fs::write(dir.join("RESULTS.md"), report).unwrap();
}
