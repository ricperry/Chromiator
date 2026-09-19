use crate::document::{Document, Recipe, SampleSize, VoronoiMatching, VoronoiSite, VoronoiState};

#[derive(Clone, Copy, Debug)]
pub struct StarterLook {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    recipe: StarterRecipe,
}

#[derive(Clone, Copy, Debug)]
enum StarterRecipe {
    Sites(VoronoiMatching, &'static [VoronoiSiteSpec]),
    /// Embed complete artist-authored presets without quantizing colors or losing settings.
    Saved(&'static str),
}

#[derive(Clone, Copy, Debug)]
struct VoronoiSiteSpec {
    source: &'static str,
    target: &'static str,
    influence: f64,
}

const INK_PAPER: &[VoronoiSiteSpec] = &[
    VoronoiSiteSpec {
        source: "#08090D",
        target: "#11131A",
        influence: 0.2,
    },
    VoronoiSiteSpec {
        source: "#48515B",
        target: "#40515C",
        influence: 0.0,
    },
    VoronoiSiteSpec {
        source: "#B9B2A5",
        target: "#C8BFA8",
        influence: 0.0,
    },
    VoronoiSiteSpec {
        source: "#FAF7EE",
        target: "#FFF4D6",
        influence: 0.1,
    },
];
const BLUEPRINT: &[VoronoiSiteSpec] = &[
    VoronoiSiteSpec {
        source: "#060B12",
        target: "#06192E",
        influence: 0.2,
    },
    VoronoiSiteSpec {
        source: "#263A50",
        target: "#0D3B66",
        influence: 0.0,
    },
    VoronoiSiteSpec {
        source: "#668DA0",
        target: "#298CB5",
        influence: 0.0,
    },
    VoronoiSiteSpec {
        source: "#C4D8DE",
        target: "#A8E2F5",
        influence: 0.0,
    },
    VoronoiSiteSpec {
        source: "#F6FAFA",
        target: "#EAFBFF",
        influence: 0.0,
    },
];
const NIGHT_NEON: &[VoronoiSiteSpec] = &[
    VoronoiSiteSpec {
        source: "#030309",
        target: "#090713",
        influence: 0.35,
    },
    VoronoiSiteSpec {
        source: "#24123D",
        target: "#5B2A86",
        influence: 0.0,
    },
    VoronoiSiteSpec {
        source: "#163F4A",
        target: "#00C9C8",
        influence: 0.0,
    },
    VoronoiSiteSpec {
        source: "#69364F",
        target: "#FF4FA3",
        influence: 0.0,
    },
    VoronoiSiteSpec {
        source: "#D8D2B8",
        target: "#F8F1C7",
        influence: -0.1,
    },
];

// Equal-influence neutral Sources partition by OKLab lightness, discarding hue.
// Chromatic Sources instead retain broad color families. All looks use the same
// recipe defaults unless a complete saved recipe is supplied below.
const GRAPHITE: &[VoronoiSiteSpec] = &[
    VoronoiSiteSpec { source: "#161616", target: "#171717", influence: 0.0 },
    VoronoiSiteSpec { source: "#505050", target: "#484848", influence: 0.0 },
    VoronoiSiteSpec { source: "#898989", target: "#898989", influence: 0.0 },
    VoronoiSiteSpec { source: "#BCBCBC", target: "#C5C5C5", influence: 0.0 },
    VoronoiSiteSpec { source: "#EEEEEE", target: "#F8F8F8", influence: 0.0 },
];
const SEPIA_PRESS: &[VoronoiSiteSpec] = &[
    VoronoiSiteSpec { source: "#222222", target: "#38291F", influence: 0.0 },
    VoronoiSiteSpec { source: "#737373", target: "#997044", influence: 0.0 },
    VoronoiSiteSpec { source: "#B2B2B2", target: "#CFB482", influence: 0.0 },
    VoronoiSiteSpec { source: "#EAEAEA", target: "#F5E8CB", influence: 0.0 },
];
const TEAL_TANGERINE: &[VoronoiSiteSpec] = &[
    VoronoiSiteSpec { source: "#19263E", target: "#152B43", influence: 0.1 },
    VoronoiSiteSpec { source: "#287D83", target: "#127F86", influence: 0.0 },
    VoronoiSiteSpec { source: "#8CCBC7", target: "#B5E2D9", influence: 0.0 },
    VoronoiSiteSpec { source: "#C66638", target: "#F18432", influence: 0.1 },
    VoronoiSiteSpec { source: "#F1DDB5", target: "#FFF0CF", influence: 0.0 },
];
const MOSS_CLAY: &[VoronoiSiteSpec] = &[
    VoronoiSiteSpec { source: "#18231D", target: "#202C24", influence: 0.1 },
    VoronoiSiteSpec { source: "#426D48", target: "#465F42", influence: 0.0 },
    VoronoiSiteSpec { source: "#9BB18B", target: "#A6B095", influence: 0.0 },
    VoronoiSiteSpec { source: "#B87358", target: "#AD6951", influence: 0.0 },
    VoronoiSiteSpec { source: "#E4DFC7", target: "#EAE4D1", influence: 0.0 },
];
const PRIMARY_PRINT: &[VoronoiSiteSpec] = &[
    VoronoiSiteSpec { source: "#161616", target: "#161616", influence: 0.0 },
    VoronoiSiteSpec { source: "#ECE8DB", target: "#FFF5DF", influence: 0.0 },
    VoronoiSiteSpec { source: "#C12B35", target: "#E3342F", influence: 0.0 },
    VoronoiSiteSpec { source: "#E0CE37", target: "#F6D52A", influence: 0.0 },
    VoronoiSiteSpec { source: "#3048AC", target: "#254DC3", influence: 0.0 },
];
const SOFT_PASTEL: &[VoronoiSiteSpec] = &[
    VoronoiSiteSpec { source: "#292332", target: "#625667", influence: -0.2 },
    VoronoiSiteSpec { source: "#A6678C", target: "#D5A7B8", influence: 0.0 },
    VoronoiSiteSpec { source: "#D49869", target: "#EBC2A7", influence: 0.0 },
    VoronoiSiteSpec { source: "#64A997", target: "#BDD5C7", influence: 0.0 },
    VoronoiSiteSpec { source: "#E9E3CD", target: "#F3EBD8", influence: 0.0 },
];
const CARBON_COPY: &[VoronoiSiteSpec] = &[
    VoronoiSiteSpec { source: "#303030", target: "#142945", influence: 0.0 },
    VoronoiSiteSpec { source: "#BDBDBD", target: "#E7EDF0", influence: 0.0 },
];
const TWO_COLOR_PRESS: &[VoronoiSiteSpec] = &[
    VoronoiSiteSpec { source: "#292A30", target: "#202022", influence: 0.05 },
    VoronoiSiteSpec { source: "#DFDDCE", target: "#F5ECD9", influence: 0.0 },
    VoronoiSiteSpec { source: "#B65143", target: "#D93632", influence: 0.1 },
];
const RISOGRAPH: &[VoronoiSiteSpec] = &[
    VoronoiSiteSpec { source: "#316D78", target: "#176D78", influence: 0.15 },
    VoronoiSiteSpec { source: "#BF6A69", target: "#EA7565", influence: 0.0 },
    VoronoiSiteSpec { source: "#E8DEBD", target: "#F5EACF", influence: 0.0 },
];

pub const STARTER_LOOKS: &[StarterLook] = &[
    StarterLook {
        id: "ink-paper",
        name: "Ink & Paper",
        description: "Charcoal ink, cool gray, and warm paper",
        recipe: StarterRecipe::Sites(VoronoiMatching::Perceptual, INK_PAPER),
    },
    StarterLook {
        id: "desert-dusk",
        name: "Desert Dusk",
        description: "Plum shadows, terracotta midtones, and sunlit sand",
        recipe: StarterRecipe::Saved(include_str!("../resources/presets/desert-dusk.json")),
    },
    StarterLook {
        id: "blueprint",
        name: "Blueprint",
        description: "Architectural navy, cyan linework, and pale highlights",
        recipe: StarterRecipe::Sites(VoronoiMatching::Perceptual, BLUEPRINT),
    },
    StarterLook {
        id: "arcade-four",
        name: "Arcade Four",
        description: "Punchy coral, aqua, violet, and gold on near-black",
        recipe: StarterRecipe::Saved(include_str!("../resources/presets/arcade-four.json")),
    },
    StarterLook {
        id: "night-neon",
        name: "Night Neon",
        description: "HSV-shaped neon violet, cyan, and pink with protected blacks",
        recipe: StarterRecipe::Sites(VoronoiMatching::Hsv, NIGHT_NEON),
    },
    StarterLook { id: "graphite", name: "Graphite", description: "Five neutral values for deliberate grayscale drawing", recipe: StarterRecipe::Sites(VoronoiMatching::Perceptual, GRAPHITE) },
    StarterLook { id: "sepia-press", name: "Sepia Press", description: "Four warm tonal steps from umber to antique cream", recipe: StarterRecipe::Sites(VoronoiMatching::Perceptual, SEPIA_PRESS) },
    StarterLook { id: "teal-and-tangerine", name: "Teal and Tangerine", description: "Complementary teal and orange with navy, aqua, and cream", recipe: StarterRecipe::Sites(VoronoiMatching::Perceptual, TEAL_TANGERINE) },
    StarterLook { id: "moss-and-clay", name: "Moss and Clay", description: "Muted forest, sage, terracotta, and bone", recipe: StarterRecipe::Sites(VoronoiMatching::Perceptual, MOSS_CLAY) },
    StarterLook { id: "primary-print", name: "Primary Print", description: "Hard-separated red, yellow, and blue with black and warm white", recipe: StarterRecipe::Sites(VoronoiMatching::Perceptual, PRIMARY_PRINT) },
    StarterLook { id: "soft-pastel", name: "Soft Pastel", description: "Lifted plum shadows, dusty pink, peach, mint, and cream", recipe: StarterRecipe::Sites(VoronoiMatching::Perceptual, SOFT_PASTEL) },
    StarterLook { id: "mimeograph", name: "Mimeograph", description: "Two-color violet ink and warm office paper; deliberately reductive", recipe: StarterRecipe::Saved(include_str!("../resources/presets/mimeograph.json")) },
    StarterLook { id: "photocopy", name: "Photocopy", description: "Near-binary black and white with dense copier shadows", recipe: StarterRecipe::Saved(include_str!("../resources/presets/photocopy.json")) },
    StarterLook { id: "carbon-copy", name: "Carbon Copy", description: "Two-color dark blue ink and cold pale paper", recipe: StarterRecipe::Sites(VoronoiMatching::Perceptual, CARBON_COPY) },
    StarterLook { id: "old-newsprint", name: "Old Newsprint", description: "Three compressed tones: charcoal, gray, and newsprint cream", recipe: StarterRecipe::Saved(include_str!("../resources/presets/old-newsprint.json")) },
    StarterLook { id: "two-color-press", name: "Two-Color Press", description: "Black and red spot ink on paper for broad graphic regions", recipe: StarterRecipe::Sites(VoronoiMatching::Perceptual, TWO_COLOR_PRESS) },
    StarterLook { id: "risograph", name: "Risograph", description: "Teal and coral spot inks on warm paper; no simulated halftones", recipe: StarterRecipe::Sites(VoronoiMatching::Perceptual, RISOGRAPH) },
    StarterLook { id: "smudged-graphite", name: "Smudged Graphite", description: "Smoothed grayscale drawing with soft tonal transitions", recipe: StarterRecipe::Saved(include_str!("../resources/presets/smudged-graphite.json")) },
    StarterLook { id: "watercolor", name: "Watercolor", description: "Smoothed color washes with thirteen sites and broad transitions", recipe: StarterRecipe::Saved(include_str!("../resources/presets/watercolor.json")) },
];

fn voronoi_state(matching: VoronoiMatching, specs: &[VoronoiSiteSpec]) -> VoronoiState {
    let sites = specs
        .iter()
        .enumerate()
        .map(|(index, spec)| {
            let source = crate::color::parse_hex(spec.source).expect("built-in Source hex");
            let target = crate::color::parse_hex(spec.target).expect("built-in Target hex");
            VoronoiSite {
                id: index as u64 + 1,
                order: index as u64 + 1,
                source_color: [source[0] as f32, source[1] as f32, source[2] as f32, 1.0],
                target_color: target.map(|channel| channel as f32),
                influence: spec.influence,
                locked: false,
                position: None,
                size: SampleSize::Point,
            }
        })
        .collect();
    VoronoiState {
        sites,
        next_site_id: specs.len() as u64 + 1,
        matching,
        ..VoronoiState::default()
    }
}

pub fn recipe_for_starter_look(look: StarterLook) -> Recipe {
    match look.recipe {
        StarterRecipe::Sites(matching, specs) => Recipe {
            voronoi: voronoi_state(matching, specs),
            ..Recipe::default()
        },
        StarterRecipe::Saved(json) => {
            let preset: crate::preset::Preset = serde_json::from_str(json).expect("valid embedded preset JSON");
            preset.processing.validate().expect("valid embedded preset recipe");
            assert_eq!(preset.name, look.name, "embedded preset name matches menu");
            preset.processing.recipe()
        }
    }
}

pub fn apply_starter_look(document: &mut Document, index: usize) -> bool {
    let Some(look) = STARTER_LOOKS.get(index).copied() else {
        return false;
    };
    let before = document.recipe.clone();
    document.recipe = recipe_for_starter_look(look);
    let changed = document.recipe != before;
    document.dirty |= changed;
    changed
}
