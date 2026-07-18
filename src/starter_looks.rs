use crate::document::{
    ComponentQuantizer, Document, LinkPolicy, Method, Recipe, RgbThresholdState, SampleSize,
    ThresholdEditTarget, ThresholdEncoding, ThresholdSpace, ThresholdState, VoronoiMatching,
    VoronoiSite, VoronoiState,
};

#[derive(Clone, Copy, Debug)]
pub struct StarterLook {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub method: Method,
    definition: StarterDefinition,
}

#[derive(Clone, Copy, Debug)]
enum StarterDefinition {
    ThresholdRgb {
        linked: bool,
        channels: [(&'static [u8], &'static [u8]); 3],
    },
    ThresholdHsv(ThresholdHsvLook),
    Voronoi {
        matching: VoronoiMatching,
        sites: &'static [VoronoiSiteSpec],
    },
}

#[derive(Clone, Copy, Debug)]
enum ThresholdHsvLook {
    HuePoster,
    NeonShadows,
    PastelBands,
}

#[derive(Clone, Copy, Debug)]
struct VoronoiSiteSpec {
    source: &'static str,
    target: &'static str,
    influence: f64,
}

const COMIC_BOUNDARIES: &[u8] = &[85, 170];
const COMIC_OUTPUTS: &[u8] = &[0, 128, 255];
const DUOTONE_BOUNDARIES: &[u8] = &[128];
const DUOTONE_ZERO: &[u8] = &[0, 0];
const DUOTONE_BLUE: &[u8] = &[0, 255];
const VINTAGE_BOUNDARIES: &[u8] = &[64, 128, 192];
const VINTAGE_RED: &[u8] = &[112, 176, 240, 255];
const VINTAGE_GREEN: &[u8] = &[80, 144, 208, 224];
const VINTAGE_BLUE: &[u8] = &[48, 112, 176, 192];
const NOIR_OUTPUTS: &[u8] = &[0, 85, 170, 255];
const POP_RED: &[u8] = &[255, 0];
const POP_CYAN: &[u8] = &[0, 255];

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
        id: "comic-book",
        name: "Comic Book",
        description: "High contrast with a limited classic comic palette",
        method: Method::Thresholds,
        definition: StarterDefinition::ThresholdRgb {
            linked: true,
            channels: [(COMIC_BOUNDARIES, COMIC_OUTPUTS); 3],
        },
    },
    StarterLook {
        id: "duotone-blue",
        name: "Duotone Blue",
        description: "Deep blacks with blue highlights",
        method: Method::Thresholds,
        definition: StarterDefinition::ThresholdRgb {
            linked: false,
            channels: [
                (DUOTONE_BOUNDARIES, DUOTONE_ZERO),
                (DUOTONE_BOUNDARIES, DUOTONE_ZERO),
                (DUOTONE_BOUNDARIES, DUOTONE_BLUE),
            ],
        },
    },
    StarterLook {
        id: "vintage-photo",
        name: "Vintage Photo",
        description: "A warm, sepia-toned photographic palette",
        method: Method::Thresholds,
        definition: StarterDefinition::ThresholdRgb {
            linked: false,
            channels: [
                (VINTAGE_BOUNDARIES, VINTAGE_RED),
                (VINTAGE_BOUNDARIES, VINTAGE_GREEN),
                (VINTAGE_BOUNDARIES, VINTAGE_BLUE),
            ],
        },
    },
    StarterLook {
        id: "noir",
        name: "Noir",
        description: "Dramatic grayscale with deep shadows",
        method: Method::Thresholds,
        definition: StarterDefinition::ThresholdRgb {
            linked: true,
            channels: [(VINTAGE_BOUNDARIES, NOIR_OUTPUTS); 3],
        },
    },
    StarterLook {
        id: "pop-art",
        name: "Pop Art",
        description: "Vibrant, inverted red and cyan posterization",
        method: Method::Thresholds,
        definition: StarterDefinition::ThresholdRgb {
            linked: false,
            channels: [
                (DUOTONE_BOUNDARIES, POP_RED),
                (DUOTONE_BOUNDARIES, POP_CYAN),
                (DUOTONE_BOUNDARIES, POP_CYAN),
            ],
        },
    },
    StarterLook {
        id: "hue-poster",
        name: "Hue Poster",
        description: "Six bold hue regions with stepped saturation and value",
        method: Method::Thresholds,
        definition: StarterDefinition::ThresholdHsv(ThresholdHsvLook::HuePoster),
    },
    StarterLook {
        id: "neon-shadows",
        name: "Neon Shadows",
        description: "Electric hues emerging from deep, graphic shadows",
        method: Method::Thresholds,
        definition: StarterDefinition::ThresholdHsv(ThresholdHsvLook::NeonShadows),
    },
    StarterLook {
        id: "pastel-bands",
        name: "Pastel Bands",
        description: "Soft chroma and bright value steps for a screen-print feel",
        method: Method::Thresholds,
        definition: StarterDefinition::ThresholdHsv(ThresholdHsvLook::PastelBands),
    },
    StarterLook {
        id: "ink-paper",
        name: "Ink & Paper",
        description: "Charcoal ink, cool gray, and warm paper",
        method: Method::Voronoi,
        definition: StarterDefinition::Voronoi {
            matching: VoronoiMatching::Perceptual,
            sites: INK_PAPER,
        },
    },
    StarterLook {
        id: "desert-dusk",
        name: "Desert Dusk",
        description: "Plum shadows, terracotta midtones, and sunlit sand",
        method: Method::Voronoi,
        definition: StarterDefinition::Voronoi {
            matching: VoronoiMatching::Perceptual,
            sites: DESERT_DUSK,
        },
    },
    StarterLook {
        id: "blueprint",
        name: "Blueprint",
        description: "Architectural navy, cyan linework, and pale highlights",
        method: Method::Voronoi,
        definition: StarterDefinition::Voronoi {
            matching: VoronoiMatching::Perceptual,
            sites: BLUEPRINT,
        },
    },
    StarterLook {
        id: "arcade-four",
        name: "Arcade Four",
        description: "Punchy coral, aqua, violet, and gold on near-black",
        method: Method::Voronoi,
        definition: StarterDefinition::Voronoi {
            matching: VoronoiMatching::Perceptual,
            sites: ARCADE_FOUR,
        },
    },
    StarterLook {
        id: "night-neon",
        name: "Night Neon",
        description: "HSV-shaped neon violet, cyan, and pink with protected blacks",
        method: Method::Voronoi,
        definition: StarterDefinition::Voronoi {
            matching: VoronoiMatching::Hsv,
            sites: NIGHT_NEON,
        },
    },
];

fn normalized(values: &[u8]) -> Vec<f32> {
    values
        .iter()
        .map(|value| f32::from(*value) / 255.0)
        .collect()
}

fn quantizer(boundaries: &[u8], outputs: &[u8]) -> ComponentQuantizer {
    ComponentQuantizer {
        enabled: true,
        boundaries: normalized(boundaries),
        outputs: normalized(outputs),
    }
}

pub fn rgb_state(look: StarterLook) -> RgbThresholdState {
    let StarterDefinition::ThresholdRgb { linked, channels } = look.definition else {
        panic!("{} is not an RGB Threshold preset", look.name);
    };
    RgbThresholdState {
        link: if linked {
            LinkPolicy::Linked
        } else {
            LinkPolicy::Independent
        },
        encoding: ThresholdEncoding::EncodedSrgb,
        locks: [false; 3],
        components: channels.map(|(boundaries, outputs)| quantizer(boundaries, outputs)),
    }
}

fn hue_quantizer(boundaries: &[u16], outputs: &[u16]) -> ComponentQuantizer {
    ComponentQuantizer {
        enabled: true,
        boundaries: boundaries
            .iter()
            .map(|value| *value as f32 / 360.0)
            .collect(),
        outputs: outputs.iter().map(|value| *value as f32 / 360.0).collect(),
    }
}

fn apply_threshold_hsv(threshold: &mut ThresholdState, look: ThresholdHsvLook) {
    threshold.active_space = ThresholdSpace::Hsv;
    threshold.hsv_state.sv_link = LinkPolicy::Independent;
    let (hue_boundaries, hue_outputs, s_boundaries, s_outputs, v_boundaries, v_outputs) = match look
    {
        ThresholdHsvLook::HuePoster => (
            &[45, 105, 165, 225, 285][..],
            &[15, 75, 135, 195, 255, 315][..],
            &[96, 180][..],
            &[64, 176, 242][..],
            &[64, 150, 220][..],
            &[36, 112, 196, 244][..],
        ),
        ThresholdHsvLook::NeonShadows => (
            &[90, 210, 300][..],
            &[330, 285, 185, 45][..],
            &[80, 170][..],
            &[160, 230, 255][..],
            &[55, 120, 200][..],
            &[12, 48, 190, 255][..],
        ),
        ThresholdHsvLook::PastelBands => (
            &[60, 180, 300][..],
            &[30, 150, 270, 330][..],
            &[85, 170][..],
            &[45, 95, 145][..],
            &[80, 160, 225][..],
            &[96, 176, 224, 250][..],
        ),
    };
    threshold.hsv_state.hue = hue_quantizer(hue_boundaries, hue_outputs);
    threshold.hsv_state.saturation = quantizer(s_boundaries, s_outputs);
    threshold.hsv_state.value = quantizer(v_boundaries, v_outputs);
}

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
    }
}

pub fn recipe_for_starter_look(look: StarterLook) -> Recipe {
    let mut recipe = Recipe {
        active_method: look.method,
        ..Recipe::default()
    };
    recipe.threshold.input_smoothing = 0.0;
    match look.definition {
        StarterDefinition::ThresholdRgb { .. } => {
            recipe.threshold = ThresholdState::default();
            recipe.threshold.rgb_state = rgb_state(look);
        }
        StarterDefinition::ThresholdHsv(hsv) => {
            recipe.threshold = ThresholdState::default();
            apply_threshold_hsv(&mut recipe.threshold, hsv);
        }
        StarterDefinition::Voronoi { matching, sites } => {
            recipe.voronoi = voronoi_state(matching, sites);
        }
    }
    recipe
}

pub fn apply_starter_look(document: &mut Document, index: usize) -> bool {
    let Some(look) = STARTER_LOOKS.get(index).copied() else {
        return false;
    };
    let before = document.recipe.clone();
    let built_in = recipe_for_starter_look(look);
    document.recipe.active_method = built_in.active_method;
    match look.method {
        Method::Thresholds => document.recipe.threshold = built_in.threshold,
        Method::Voronoi => document.recipe.voronoi = built_in.voronoi,
    }
    let changed = document.recipe != before;
    document.dirty |= changed;
    changed
}

pub fn reset_component(threshold: &mut ThresholdState, requested: ThresholdEditTarget) -> bool {
    let target = requested;
    if threshold.is_locked(target) {
        return false;
    }
    let before = threshold.clone();
    threshold.set_edit_quantizer(target, ComponentQuantizer::evenly_spaced(3));
    *threshold != before
}

pub fn reset_active_space(threshold: &mut ThresholdState) -> bool {
    let before = threshold.clone();
    let defaults = ThresholdState::default();
    match threshold.active_space {
        ThresholdSpace::Rgb => {
            threshold.rgb_state.link = defaults.rgb_state.link;
            threshold.rgb_state.encoding = defaults.rgb_state.encoding;
            for index in 0..3 {
                if !threshold.rgb_state.locks[index] {
                    threshold.rgb_state.components[index] =
                        defaults.rgb_state.components[index].clone();
                }
            }
        }
        ThresholdSpace::Hsv => {
            threshold.hsv_state.sv_link = defaults.hsv_state.sv_link;
            threshold.hsv_state.hue_origin_degrees = defaults.hsv_state.hue_origin_degrees;
            if !threshold.hsv_state.locks[0] {
                threshold.hsv_state.hue = defaults.hsv_state.hue;
            }
            if !threshold.hsv_state.locks[1] {
                threshold.hsv_state.saturation = defaults.hsv_state.saturation;
            }
            if !threshold.hsv_state.locks[2] {
                threshold.hsv_state.value = defaults.hsv_state.value;
            }
        }
    }
    *threshold != before
}

pub fn reset_method(threshold: &mut ThresholdState) -> bool {
    let before = threshold.clone();
    let defaults = ThresholdState::default();
    threshold.active_space = defaults.active_space;
    threshold.rgb_state.link = defaults.rgb_state.link;
    threshold.rgb_state.encoding = defaults.rgb_state.encoding;
    threshold.hsv_state.sv_link = defaults.hsv_state.sv_link;
    threshold.hsv_state.hue_origin_degrees = defaults.hsv_state.hue_origin_degrees;
    for index in 0..3 {
        if !threshold.rgb_state.locks[index] {
            threshold.rgb_state.components[index] = defaults.rgb_state.components[index].clone();
        }
    }
    if !threshold.hsv_state.locks[0] {
        threshold.hsv_state.hue = defaults.hsv_state.hue;
    }
    if !threshold.hsv_state.locks[1] {
        threshold.hsv_state.saturation = defaults.hsv_state.saturation;
    }
    if !threshold.hsv_state.locks[2] {
        threshold.hsv_state.value = defaults.hsv_state.value;
    }
    threshold.alpha_policy = defaults.alpha_policy;
    threshold.input_smoothing = 0.0;
    *threshold != before
}
