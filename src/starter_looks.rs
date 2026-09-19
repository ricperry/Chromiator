use crate::document::{Document, Recipe, SampleSize, VoronoiMatching, VoronoiSite, VoronoiState};

#[derive(Clone, Copy, Debug)]
pub struct StarterLook {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    matching: VoronoiMatching,
    sites: &'static [VoronoiSiteSpec],
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
const DESERT_DUSK: &[VoronoiSiteSpec] = &[
    VoronoiSiteSpec {
        source: "#16101D",
        target: "#241B2F",
        influence: 0.15,
    },
    VoronoiSiteSpec {
        source: "#633A4B",
        target: "#70405D",
        influence: 0.0,
    },
    VoronoiSiteSpec {
        source: "#A36A4F",
        target: "#D27B58",
        influence: 0.0,
    },
    VoronoiSiteSpec {
        source: "#D7B16A",
        target: "#F0C775",
        influence: 0.0,
    },
    VoronoiSiteSpec {
        source: "#F3E8D1",
        target: "#FFF0CE",
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
const ARCADE_FOUR: &[VoronoiSiteSpec] = &[
    VoronoiSiteSpec {
        source: "#0B0710",
        target: "#15121C",
        influence: 0.15,
    },
    VoronoiSiteSpec {
        source: "#8A263D",
        target: "#FF4F69",
        influence: 0.0,
    },
    VoronoiSiteSpec {
        source: "#287C72",
        target: "#36D6C0",
        influence: 0.0,
    },
    VoronoiSiteSpec {
        source: "#3F357D",
        target: "#7868E6",
        influence: 0.0,
    },
    VoronoiSiteSpec {
        source: "#D6B66A",
        target: "#FFD166",
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

pub const STARTER_LOOKS: &[StarterLook] = &[
    StarterLook {
        id: "ink-paper",
        name: "Ink & Paper",
        description: "Charcoal ink, cool gray, and warm paper",
        matching: VoronoiMatching::Perceptual,
        sites: INK_PAPER,
    },
    StarterLook {
        id: "desert-dusk",
        name: "Desert Dusk",
        description: "Plum shadows, terracotta midtones, and sunlit sand",
        matching: VoronoiMatching::Perceptual,
        sites: DESERT_DUSK,
    },
    StarterLook {
        id: "blueprint",
        name: "Blueprint",
        description: "Architectural navy, cyan linework, and pale highlights",
        matching: VoronoiMatching::Perceptual,
        sites: BLUEPRINT,
    },
    StarterLook {
        id: "arcade-four",
        name: "Arcade Four",
        description: "Punchy coral, aqua, violet, and gold on near-black",
        matching: VoronoiMatching::Perceptual,
        sites: ARCADE_FOUR,
    },
    StarterLook {
        id: "night-neon",
        name: "Night Neon",
        description: "HSV-shaped neon violet, cyan, and pink with protected blacks",
        matching: VoronoiMatching::Hsv,
        sites: NIGHT_NEON,
    },
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
    Recipe {
        voronoi: voronoi_state(look.matching, look.sites),
        ..Recipe::default()
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
