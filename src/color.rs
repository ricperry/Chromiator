#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorModel {
    /// Encoded sRGB channels, normalized internally and displayed as 0-255.
    Rgb,
    Hsv,
    Hsl,
    #[default]
    Okhsl,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaneKey {
    Left,
    Right,
    Up,
    Down,
    Home,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DraftColor {
    pub linear: [f64; 3],
    hue_hints: [f64; 3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PickerTransaction {
    pub draft: DraftColor,
    committed: bool,
}

impl PickerTransaction {
    pub fn new(original: [f32; 3]) -> Self {
        Self {
            draft: DraftColor::new(original),
            committed: false,
        }
    }

    pub fn cancel(self) -> Option<[f32; 3]> {
        None
    }

    pub fn commit(&mut self) -> Option<[f32; 3]> {
        if self.committed {
            None
        } else {
            self.committed = true;
            Some(self.draft.commit())
        }
    }
}

impl DraftColor {
    pub fn new(linear: [f32; 3]) -> Self {
        let linear = linear.map(f64::from);
        let encoded = linear_to_encoded(linear);
        let hsv = encoded_to_hsv(encoded);
        Self {
            linear,
            hue_hints: [hsv[0]; 3],
        }
    }

    pub fn values(self, model: ColorModel) -> [f64; 3] {
        match model {
            ColorModel::Rgb => self.encoded(),
            ColorModel::Hsv => {
                let mut values = encoded_to_hsv(self.encoded());
                if values[1] < 1e-9 {
                    values[0] = self.hue_hints[0];
                }
                values
            }
            ColorModel::Hsl => {
                let mut values = encoded_to_hsl(self.encoded());
                if values[1] < 1e-9 {
                    values[0] = self.hue_hints[1];
                }
                values
            }
            ColorModel::Okhsl => {
                let mut values = linear_to_okhsl(self.linear)
                    .expect("picker drafts remain inside bounded linear sRGB");
                values[0] *= 360.0;
                if values[1] < 1e-6 {
                    values[0] = self.hue_hints[2];
                }
                values
            }
        }
    }

    pub fn set_values(&mut self, model: ColorModel, values: [f64; 3]) {
        self.linear = match model {
            ColorModel::Rgb => encoded_to_linear(values.map(|value| value.clamp(0.0, 1.0))),
            ColorModel::Hsv => {
                self.hue_hints[0] = values[0].rem_euclid(360.0);
                encoded_to_linear(hsv_to_encoded(values))
            }
            ColorModel::Hsl => {
                self.hue_hints[1] = values[0].rem_euclid(360.0);
                encoded_to_linear(hsl_to_encoded(values))
            }
            ColorModel::Okhsl => {
                self.hue_hints[2] = values[0].rem_euclid(360.0);
                okhsl_to_linear([self.hue_hints[2] / 360.0, values[1], values[2]])
                    .expect("finite normalized OKHSL controls always produce bounded sRGB")
            }
        };
    }

    pub fn encoded(self) -> [f64; 3] {
        linear_to_encoded(self.linear)
    }

    pub fn commit(self) -> [f32; 3] {
        self.linear.map(|value| value as f32)
    }
}

pub fn encoded_to_linear(rgb: [f64; 3]) -> [f64; 3] {
    rgb.map(|value| {
        if value <= 0.040_45 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    })
}

/// Convert canonical linear sRGB to canonical encoded sRGB without narrowing through `f32`.
pub fn linear_to_encoded(rgb: [f64; 3]) -> [f64; 3] {
    rgb.map(|value| {
        if value <= 0.003_130_8 {
            12.92 * value
        } else {
            1.055 * value.powf(1.0 / 2.4) - 0.055
        }
    })
}

pub fn encoded_to_hsv(rgb: [f64; 3]) -> [f64; 3] {
    let max = rgb.into_iter().fold(f64::NEG_INFINITY, f64::max);
    let min = rgb.into_iter().fold(f64::INFINITY, f64::min);
    let delta = max - min;
    let hue = if delta < 1e-12 {
        0.0
    } else if max == rgb[0] {
        60.0 * ((rgb[1] - rgb[2]) / delta).rem_euclid(6.0)
    } else if max == rgb[1] {
        60.0 * ((rgb[2] - rgb[0]) / delta + 2.0)
    } else {
        60.0 * ((rgb[0] - rgb[1]) / delta + 4.0)
    };
    [
        hue.rem_euclid(360.0),
        if max <= 0.0 { 0.0 } else { delta / max },
        max,
    ]
}

pub fn hsv_to_encoded(hsv: [f64; 3]) -> [f64; 3] {
    let [h, s, v] = [
        hsv[0].rem_euclid(360.0),
        hsv[1].clamp(0.0, 1.0),
        hsv[2].clamp(0.0, 1.0),
    ];
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = v - c;
    let rgb = match (h / 60.0).floor() as u8 {
        0 => [c, x, 0.0],
        1 => [x, c, 0.0],
        2 => [0.0, c, x],
        3 => [0.0, x, c],
        4 => [x, 0.0, c],
        _ => [c, 0.0, x],
    };
    rgb.map(|value| value + m)
}

pub fn encoded_to_hsl(rgb: [f64; 3]) -> [f64; 3] {
    let hsv = encoded_to_hsv(rgb);
    let max = rgb.into_iter().fold(f64::NEG_INFINITY, f64::max);
    let min = rgb.into_iter().fold(f64::INFINITY, f64::min);
    let light = (max + min) / 2.0;
    let delta = max - min;
    [
        hsv[0],
        if delta < 1e-12 {
            0.0
        } else {
            delta / (1.0 - (2.0 * light - 1.0).abs())
        },
        light,
    ]
}

pub fn hsl_to_encoded(hsl: [f64; 3]) -> [f64; 3] {
    let [h, s, l] = [
        hsl[0].rem_euclid(360.0),
        hsl[1].clamp(0.0, 1.0),
        hsl[2].clamp(0.0, 1.0),
    ];
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = l - c / 2.0;
    let rgb = match (h / 60.0).floor() as u8 {
        0 => [c, x, 0.0],
        1 => [x, c, 0.0],
        2 => [0.0, c, x],
        3 => [0.0, x, c],
        4 => [x, 0.0, c],
        _ => [c, 0.0, x],
    };
    rgb.map(|value| value + m)
}

pub fn linear_to_oklab(rgb: [f64; 3]) -> [f64; 3] {
    let l = 0.412_221_470_8 * rgb[0] + 0.536_332_536_3 * rgb[1] + 0.051_445_992_9 * rgb[2];
    let m = 0.211_903_498_2 * rgb[0] + 0.680_699_545_1 * rgb[1] + 0.107_396_956_6 * rgb[2];
    let s = 0.088_302_461_9 * rgb[0] + 0.281_718_837_6 * rgb[1] + 0.629_978_700_5 * rgb[2];
    let (l, m, s) = (l.cbrt(), m.cbrt(), s.cbrt());
    [
        0.210_454_255_3 * l + 0.793_617_785 * m - 0.004_072_046_8 * s,
        1.977_998_495_1 * l - 2.428_592_205 * m + 0.450_593_709_9 * s,
        0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766 * s,
    ]
}

pub fn oklab_to_linear(lab: [f64; 3]) -> [f64; 3] {
    let l = lab[0] + 0.396_337_777_4 * lab[1] + 0.215_803_757_3 * lab[2];
    let m = lab[0] - 0.105_561_345_8 * lab[1] - 0.063_854_172_8 * lab[2];
    let s = lab[0] - 0.089_484_177_5 * lab[1] - 1.291_485_548 * lab[2];
    let (l, m, s) = (l.powi(3), m.powi(3), s.powi(3));
    [
        4.076_741_662_1 * l - 3.307_711_591_3 * m + 0.230_969_929_2 * s,
        -1.268_438_004_6 * l + 2.609_757_401_1 * m - 0.341_319_396_5 * s,
        -0.004_196_086_3 * l - 0.703_418_614_7 * m + 1.707_614_701 * s,
    ]
}

// The OKHSL conversion below is a Rust adaptation of Björn Ottosson's reference
// implementation. It maps the complete normalized OKHSL cylinder into bounded sRGB, so
// a picker never needs out-of-gamut holes or projection. See THIRD_PARTY_NOTICES.md.
// The canonical polynomial/one-step Halley approximation can land a few millionths
// outside a boundary at S=1. Clamp only that numerical residue; larger excursions fail.
const OKHSL_GAMUT_EPSILON: f64 = 2e-7;

#[derive(Clone, Copy)]
struct Cusp {
    l: f64,
    c: f64,
}

#[derive(Clone, Copy)]
struct ChromaBounds {
    c_0: f64,
    c_mid: f64,
    c_max: f64,
}

/// Allocation-local accelerator for a fixed-lightness OKHSL picker plane.
///
/// Canonical numeric conversion continues to use [`okhsl_to_linear`]. The visual plane may
/// sample many thousands of pixels at the same lightness, so this caches the expensive gamut
/// bounds around the hue circle and linearly interpolates them between sub-degree samples.
pub struct OkhslPlaneSampler {
    lightness: f64,
    l: f64,
    bounds: Vec<ChromaBounds>,
}

impl OkhslPlaneSampler {
    pub fn new(lightness: f64, hue_steps: usize) -> Self {
        let lightness = lightness.clamp(0.0, 1.0);
        let l = toe_inverse(lightness);
        let bounds = (0..hue_steps.max(360))
            .map(|index| {
                let radians = std::f64::consts::TAU * index as f64 / hue_steps.max(360) as f64;
                chroma_bounds(l, radians.cos(), radians.sin())
            })
            .collect();
        Self {
            lightness,
            l,
            bounds,
        }
    }

    pub fn encoded(&self, hue_degrees: f64, saturation: f64) -> [f64; 3] {
        if self.lightness <= f64::EPSILON {
            return [0.0; 3];
        }
        if (1.0 - self.lightness) <= f64::EPSILON {
            return [1.0; 3];
        }
        let position = hue_degrees.rem_euclid(360.0) / 360.0 * self.bounds.len() as f64;
        let lower = position.floor() as usize % self.bounds.len();
        let upper = (lower + 1) % self.bounds.len();
        let fraction = position.fract();
        let interpolate = |left: f64, right: f64| left + (right - left) * fraction;
        let bounds = ChromaBounds {
            c_0: interpolate(self.bounds[lower].c_0, self.bounds[upper].c_0),
            c_mid: interpolate(self.bounds[lower].c_mid, self.bounds[upper].c_mid),
            c_max: interpolate(self.bounds[lower].c_max, self.bounds[upper].c_max),
        };
        let radians = hue_degrees.to_radians();
        let chroma = okhsl_chroma(saturation.clamp(0.0, 1.0), bounds);
        linear_to_encoded(
            oklab_to_linear([self.l, chroma * radians.cos(), chroma * radians.sin()])
                .map(|channel| channel.clamp(0.0, 1.0)),
        )
    }
}

fn toe(value: f64) -> f64 {
    const K_1: f64 = 0.206;
    const K_2: f64 = 0.03;
    const K_3: f64 = (1.0 + K_1) / (1.0 + K_2);
    0.5 * (K_3 * value - K_1 + ((K_3 * value - K_1).powi(2) + 4.0 * K_2 * K_3 * value).sqrt())
}

fn toe_inverse(value: f64) -> f64 {
    const K_1: f64 = 0.206;
    const K_2: f64 = 0.03;
    const K_3: f64 = (1.0 + K_1) / (1.0 + K_2);
    (value * value + K_1 * value) / (K_3 * (value + K_2))
}

fn compute_max_saturation(a: f64, b: f64) -> f64 {
    let (k_0, k_1, k_2, k_3, k_4, w_l, w_m, w_s) = if -1.881_703_28 * a - 0.809_364_93 * b > 1.0 {
        (
            1.190_862_77,
            1.765_767_28,
            0.596_626_41,
            0.755_151_97,
            0.567_712_45,
            4.076_741_662_1,
            -3.307_711_591_3,
            0.230_969_929_2,
        )
    } else if 1.814_441_04 * a - 1.194_452_76 * b > 1.0 {
        (
            0.739_565_15,
            -0.459_544_04,
            0.082_854_27,
            0.125_410_7,
            0.145_032_04,
            -1.268_438_004_6,
            2.609_757_401_1,
            -0.341_319_396_5,
        )
    } else {
        (
            1.357_336_52,
            -0.009_157_99,
            -1.151_302_1,
            -0.505_596_06,
            0.006_921_67,
            -0.004_196_086_3,
            -0.703_418_614_7,
            1.707_614_701,
        )
    };
    let mut saturation = k_0 + k_1 * a + k_2 * b + k_3 * a * a + k_4 * a * b;
    let k_l = 0.396_337_777_4 * a + 0.215_803_757_3 * b;
    let k_m = -0.105_561_345_8 * a - 0.063_854_172_8 * b;
    let k_s = -0.089_484_177_5 * a - 1.291_485_548 * b;
    for _ in 0..3 {
        let l_ = 1.0 + saturation * k_l;
        let m_ = 1.0 + saturation * k_m;
        let s_ = 1.0 + saturation * k_s;
        let l = l_.powi(3);
        let m = m_.powi(3);
        let s = s_.powi(3);
        let l_d = 3.0 * k_l * l_.powi(2);
        let m_d = 3.0 * k_m * m_.powi(2);
        let s_d = 3.0 * k_s * s_.powi(2);
        let l_d2 = 6.0 * k_l * k_l * l_;
        let m_d2 = 6.0 * k_m * k_m * m_;
        let s_d2 = 6.0 * k_s * k_s * s_;
        let f = w_l * l + w_m * m + w_s * s;
        let f_1 = w_l * l_d + w_m * m_d + w_s * s_d;
        let f_2 = w_l * l_d2 + w_m * m_d2 + w_s * s_d2;
        saturation -= f * f_1 / (f_1 * f_1 - 0.5 * f * f_2);
    }
    saturation
}

fn find_cusp(a: f64, b: f64) -> Cusp {
    let saturation = compute_max_saturation(a, b);
    let rgb = oklab_to_linear([1.0, saturation * a, saturation * b]);
    let l = (1.0 / rgb.into_iter().fold(f64::NEG_INFINITY, f64::max)).cbrt();
    Cusp {
        l,
        c: l * saturation,
    }
}

fn halley_step(channel: f64, first: f64, second: f64) -> f64 {
    let denominator = first * first - 0.5 * channel * second;
    if denominator.abs() <= f64::EPSILON {
        f64::INFINITY
    } else {
        let u = first / denominator;
        if u >= 0.0 {
            -channel * u
        } else {
            f64::INFINITY
        }
    }
}

fn find_gamut_intersection(a: f64, b: f64, l_1: f64, c_1: f64, l_0: f64, cusp: Cusp) -> f64 {
    if (l_1 - l_0) * cusp.c - (cusp.l - l_0) * c_1 <= 0.0 {
        return cusp.c * l_0 / (c_1 * cusp.l + cusp.c * (l_0 - l_1));
    }
    let mut t = cusp.c * (l_0 - 1.0) / (c_1 * (cusp.l - 1.0) + cusp.c * (l_0 - l_1));
    let d_l = l_1 - l_0;
    let d_c = c_1;
    let k_l = 0.396_337_777_4 * a + 0.215_803_757_3 * b;
    let k_m = -0.105_561_345_8 * a - 0.063_854_172_8 * b;
    let k_s = -0.089_484_177_5 * a - 1.291_485_548 * b;
    let l_dt = d_l + d_c * k_l;
    let m_dt = d_l + d_c * k_m;
    let s_dt = d_l + d_c * k_s;
    // The reference permits repeating this block when boundary accuracy matters. Three
    // iterations prevent saturated picker-edge samples from falling just outside sRGB.
    for _ in 0..3 {
        let l = l_0 * (1.0 - t) + t * l_1;
        let c = t * c_1;
        let l_ = l + c * k_l;
        let m_ = l + c * k_m;
        let s_ = l + c * k_s;
        let l3 = l_.powi(3);
        let m3 = m_.powi(3);
        let s3 = s_.powi(3);
        let l_d = 3.0 * l_dt * l_.powi(2);
        let m_d = 3.0 * m_dt * m_.powi(2);
        let s_d = 3.0 * s_dt * s_.powi(2);
        let l_d2 = 6.0 * l_dt * l_dt * l_;
        let m_d2 = 6.0 * m_dt * m_dt * m_;
        let s_d2 = 6.0 * s_dt * s_dt * s_;
        let r = 4.076_741_662_1 * l3 - 3.307_711_591_3 * m3 + 0.230_969_929_2 * s3 - 1.0;
        let r_1 = 4.076_741_662_1 * l_d - 3.307_711_591_3 * m_d + 0.230_969_929_2 * s_d;
        let r_2 = 4.076_741_662_1 * l_d2 - 3.307_711_591_3 * m_d2 + 0.230_969_929_2 * s_d2;
        let g = -1.268_438_004_6 * l3 + 2.609_757_401_1 * m3 - 0.341_319_396_5 * s3 - 1.0;
        let g_1 = -1.268_438_004_6 * l_d + 2.609_757_401_1 * m_d - 0.341_319_396_5 * s_d;
        let g_2 = -1.268_438_004_6 * l_d2 + 2.609_757_401_1 * m_d2 - 0.341_319_396_5 * s_d2;
        let blue = -0.004_196_086_3 * l3 - 0.703_418_614_7 * m3 + 1.707_614_701 * s3 - 1.0;
        let blue_1 = -0.004_196_086_3 * l_d - 0.703_418_614_7 * m_d + 1.707_614_701 * s_d;
        let blue_2 = -0.004_196_086_3 * l_d2 - 0.703_418_614_7 * m_d2 + 1.707_614_701 * s_d2;
        t += halley_step(r, r_1, r_2)
            .min(halley_step(g, g_1, g_2))
            .min(halley_step(blue, blue_1, blue_2));
    }
    t
}

fn chroma_bounds(l: f64, a: f64, b: f64) -> ChromaBounds {
    let cusp = find_cusp(a, b);
    let mut c_max = find_gamut_intersection(a, b, l, 1.0, l, cusp);
    // The analytic cusp approximation can cross a different RGB boundary near
    // sector seams (notably saturated blue). Contract chroma at fixed L/hue
    // before deriving the saturation curve. Forward and inverse use this same
    // boundary, rather than hiding a substantial excursion by channel clipping.
    let is_bounded = |chroma: f64| {
        in_srgb_gamut(oklab_to_linear([l, chroma * a, chroma * b]))
    };
    if c_max.is_finite() && c_max > 0.0 && !is_bounded(c_max) {
        let mut lower = 0.0;
        let mut upper = c_max;
        for _ in 0..32 {
            let midpoint = (lower + upper) * 0.5;
            if is_bounded(midpoint) {
                lower = midpoint;
            } else {
                upper = midpoint;
            }
        }
        c_max = lower;
    }
    let s_max = cusp.c / cusp.l;
    let t_max = cusp.c / (1.0 - cusp.l);
    let s_mid = 0.115_169_93
        + 1.0
            / (7.447_789_7
                + 4.159_012_4 * b
                + a * (-2.195_573_47
                    + 1.751_984_01 * b
                    + a * (-2.137_049_48 - 10.023_010_43 * b
                        + a * (-4.248_945_61 + 5.387_708_19 * b + 4.698_910_13 * a))));
    let t_mid = 0.112_396_42
        + 1.0
            / (1.613_203_2 - 0.681_243_79 * b
                + a * (0.403_706_12
                    + 0.901_481_23 * b
                    + a * (-0.270_879_43
                        + 0.612_239_9 * b
                        + a * (0.002_992_15 - 0.453_995_68 * b - 0.146_618_72 * a))));
    let k = c_max / (l * s_max).min((1.0 - l) * t_max);
    let c_a = l * s_mid;
    let c_b = (1.0 - l) * t_mid;
    let c_mid = 0.9
        * k
        * (1.0 / (1.0 / c_a.powi(4) + 1.0 / c_b.powi(4)))
            .sqrt()
            .sqrt();
    let c_a = l * 0.4;
    let c_b = (1.0 - l) * 0.8;
    let c_0 = (1.0 / (1.0 / c_a.powi(2) + 1.0 / c_b.powi(2))).sqrt();
    ChromaBounds { c_0, c_mid, c_max }
}

fn okhsl_chroma(saturation: f64, bounds: ChromaBounds) -> f64 {
    const MID: f64 = 0.8;
    if saturation < MID {
        let t = saturation / MID;
        let k_1 = MID * bounds.c_0;
        let k_2 = 1.0 - k_1 / bounds.c_mid;
        t * k_1 / (1.0 - k_2 * t)
    } else {
        let t = (saturation - MID) / (1.0 - MID);
        let k_0 = bounds.c_mid;
        let k_1 = (1.0 - MID) * bounds.c_mid.powi(2) / (MID.powi(2) * bounds.c_0);
        let k_2 = 1.0 - k_1 / (bounds.c_max - bounds.c_mid);
        k_0 + t * k_1 / (1.0 - k_2 * t)
    }
}

fn okhsl_saturation(chroma: f64, bounds: ChromaBounds) -> f64 {
    const MID: f64 = 0.8;
    if chroma < bounds.c_mid {
        let k_1 = MID * bounds.c_0;
        let k_2 = 1.0 - k_1 / bounds.c_mid;
        MID * chroma / (k_1 + k_2 * chroma)
    } else {
        let k_0 = bounds.c_mid;
        let k_1 = (1.0 - MID) * bounds.c_mid.powi(2) / (MID.powi(2) * bounds.c_0);
        let k_2 = 1.0 - k_1 / (bounds.c_max - bounds.c_mid);
        let t = (chroma - k_0) / (k_1 + k_2 * (chroma - k_0));
        MID + (1.0 - MID) * t
    }
}

pub fn okhsl_to_linear(okhsl: [f64; 3]) -> Option<[f64; 3]> {
    if !okhsl.iter().all(|value| value.is_finite()) {
        return None;
    }
    let hue = okhsl[0].rem_euclid(1.0);
    let saturation = okhsl[1].clamp(0.0, 1.0);
    let lightness = okhsl[2].clamp(0.0, 1.0);
    if lightness <= f64::EPSILON {
        return Some([0.0; 3]);
    }
    if (1.0 - lightness) <= f64::EPSILON {
        return Some([1.0; 3]);
    }
    let radians = std::f64::consts::TAU * hue;
    let (a, b) = (radians.cos(), radians.sin());
    let l = toe_inverse(lightness);
    let chroma = okhsl_chroma(saturation, chroma_bounds(l, a, b));
    let at_chroma = |value: f64| oklab_to_linear([l, value * a, value * b]);
    let mut linear = at_chroma(chroma);
    if !linear.iter().all(|channel| channel.is_finite()) {
        return None;
    }
    // Near the blue seam, a bounded endpoint can still have an out-of-gamut
    // intermediate sample. Contract that sample along the same L/hue ray.
    if !in_srgb_gamut(linear) {
        let mut lower = 0.0;
        let mut upper = chroma;
        for _ in 0..32 {
            let midpoint = (lower + upper) * 0.5;
            if in_srgb_gamut(at_chroma(midpoint)) {
                lower = midpoint;
            } else {
                upper = midpoint;
            }
        }
        linear = at_chroma(lower);
    }
    for channel in &mut linear {
        if !channel.is_finite()
            || *channel < -OKHSL_GAMUT_EPSILON
            || *channel > 1.0 + OKHSL_GAMUT_EPSILON
        {
            return None;
        }
        *channel = channel.clamp(0.0, 1.0);
    }
    Some(linear)
}

pub fn linear_to_okhsl(linear: [f64; 3]) -> Option<[f64; 3]> {
    if !linear.iter().all(|value| value.is_finite()) || !in_srgb_gamut(linear) {
        return None;
    }
    let lab = linear_to_oklab(linear.map(|value| value.clamp(0.0, 1.0)));
    let chroma = lab[1].hypot(lab[2]);
    let lightness = toe(lab[0]).clamp(0.0, 1.0);
    if chroma <= 1e-7 || lightness <= f64::EPSILON || (1.0 - lightness) <= f64::EPSILON {
        return Some([0.0, 0.0, lightness]);
    }
    let a = lab[1] / chroma;
    let b = lab[2] / chroma;
    let hue = 0.5 + 0.5 * (-lab[2]).atan2(-lab[1]) / std::f64::consts::PI;
    let saturation = okhsl_saturation(chroma, chroma_bounds(lab[0], a, b));
    Some([hue.rem_euclid(1.0), saturation.clamp(0.0, 1.0), lightness])
}

pub fn linear_to_okhsl_cylinder(linear: [f64; 3]) -> Option<[f64; 3]> {
    let [hue, saturation, lightness] = linear_to_okhsl(linear)?;
    let radians = std::f64::consts::TAU * hue;
    Some([
        saturation * radians.cos(),
        saturation * radians.sin(),
        lightness,
    ])
}

pub fn in_srgb_gamut(linear: [f64; 3]) -> bool {
    linear
        .iter()
        .all(|value| (-1e-9..=1.0 + 1e-9).contains(value))
}

pub fn adjust_okhsl(okhsl: [f64; 3], key: PlaneKey, fine: bool) -> [f64; 3] {
    if key == PlaneKey::Home {
        return [okhsl[0].rem_euclid(360.0), 0.0, okhsl[2].clamp(0.0, 1.0)];
    }
    let hue_step = if fine { 0.1 } else { 1.0 };
    let saturation_step = if fine { 0.001 } else { 0.01 };
    let mut adjusted = okhsl;
    match key {
        PlaneKey::Left => adjusted[0] -= hue_step,
        PlaneKey::Right => adjusted[0] += hue_step,
        PlaneKey::Up => adjusted[1] += saturation_step,
        PlaneKey::Down => adjusted[1] -= saturation_step,
        PlaneKey::Home => unreachable!(),
    }
    adjusted[0] = adjusted[0].rem_euclid(360.0);
    adjusted[1] = adjusted[1].clamp(0.0, 1.0);
    adjusted[2] = adjusted[2].clamp(0.0, 1.0);
    adjusted
}

pub fn parse_hex(value: &str) -> Result<[f64; 3], &'static str> {
    let value = value.trim().strip_prefix('#').unwrap_or(value.trim());
    let expanded;
    let digits = if value.len() == 3 {
        expanded = value.chars().flat_map(|c| [c, c]).collect::<String>();
        expanded.as_str()
    } else {
        value
    };
    if digits.len() != 6 || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("Use #RGB or #RRGGBB");
    }
    let channel = |start| {
        u8::from_str_radix(&digits[start..start + 2], 16)
            .map(|v| v as f64 / 255.0)
            .map_err(|_| "Invalid hex color")
    };
    Ok(encoded_to_linear([channel(0)?, channel(2)?, channel(4)?]))
}

pub fn display_hex(linear: [f64; 3]) -> String {
    let encoded = linear_to_encoded(linear).map(|value| value.clamp(0.0, 1.0));
    format!(
        "#{:02X}{:02X}{:02X}",
        (encoded[0] * 255.0).round() as u8,
        (encoded[1] * 255.0).round() as u8,
        (encoded[2] * 255.0).round() as u8
    )
}
