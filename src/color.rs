use crate::processing::{linear_to_srgb, srgb_to_linear};
use crate::voronoi::linear_rgb_to_oklab;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorModel {
    #[default]
    Hsv,
    Hsl,
    Oklab,
}

pub const OKLAB_PLANE_SCALE: f64 = 0.4;

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
    pub hue_hint: f64,
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
        let encoded = linear.map(|value| linear_to_srgb(value) as f64);
        let hsv = encoded_to_hsv(encoded);
        Self {
            linear: linear.map(f64::from),
            hue_hint: hsv[0],
        }
    }

    pub fn values(self, model: ColorModel) -> [f64; 3] {
        match model {
            ColorModel::Hsv => {
                let mut values = encoded_to_hsv(self.encoded());
                if values[1] < 1e-9 {
                    values[0] = self.hue_hint;
                }
                values
            }
            ColorModel::Hsl => {
                let mut values = encoded_to_hsl(self.encoded());
                if values[1] < 1e-9 {
                    values[0] = self.hue_hint;
                }
                values
            }
            ColorModel::Oklab => linear_to_oklab(self.linear),
        }
    }

    pub fn set_values(&mut self, model: ColorModel, values: [f64; 3]) {
        self.linear = match model {
            ColorModel::Hsv => {
                self.hue_hint = values[0].rem_euclid(360.0);
                encoded_to_linear(hsv_to_encoded(values))
            }
            ColorModel::Hsl => {
                self.hue_hint = values[0].rem_euclid(360.0);
                encoded_to_linear(hsl_to_encoded(values))
            }
            ColorModel::Oklab => {
                let chroma = values[1].hypot(values[2]);
                if chroma > 1e-9 {
                    self.hue_hint = values[2].atan2(values[1]).to_degrees().rem_euclid(360.0);
                }
                oklab_to_linear(values)
            }
        };
    }

    pub fn encoded(self) -> [f64; 3] {
        self.linear.map(|value| linear_to_srgb(value as f32) as f64)
    }

    pub fn commit(self) -> [f32; 3] {
        self.linear.map(|value| value as f32)
    }
}

pub fn encoded_to_linear(rgb: [f64; 3]) -> [f64; 3] {
    rgb.map(|value| srgb_to_linear(value as f32) as f64)
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
    linear_rgb_to_oklab(rgb.map(|value| value as f32))
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

pub fn in_srgb_gamut(linear: [f64; 3]) -> bool {
    linear
        .iter()
        .all(|value| (-1e-9..=1.0 + 1e-9).contains(value))
}

pub fn oklab_polar(l: f64, hue: f64, chroma: f64) -> [f64; 3] {
    let radians = hue.to_radians();
    [l, chroma * radians.cos(), chroma * radians.sin()]
}

pub fn max_oklab_chroma(l: f64, hue: f64) -> f64 {
    let mut low = 0.0;
    let mut high = 0.5;
    for _ in 0..28 {
        let mid = (low + high) / 2.0;
        if in_srgb_gamut(oklab_to_linear(oklab_polar(l.clamp(0.0, 1.0), hue, mid))) {
            low = mid;
        } else {
            high = mid;
        }
    }
    low
}

pub fn oklab_plane(l: f64, x: f64, y: f64) -> [f64; 3] {
    [
        l.clamp(0.0, 1.0),
        x * OKLAB_PLANE_SCALE,
        y * OKLAB_PLANE_SCALE,
    ]
}

pub fn oklab_plane_coords(lab: [f64; 3]) -> [f64; 2] {
    [lab[1] / OKLAB_PLANE_SCALE, lab[2] / OKLAB_PLANE_SCALE]
}

pub fn project_oklab_to_gamut(lab: [f64; 3]) -> [f64; 3] {
    let l = if lab[0].is_finite() {
        lab[0].clamp(0.0, 1.0)
    } else {
        0.0
    };
    let a = if lab[1].is_finite() { lab[1] } else { 0.0 };
    let b = if lab[2].is_finite() { lab[2] } else { 0.0 };
    let requested = [l, a, b];
    if in_srgb_gamut(oklab_to_linear(requested)) {
        return requested;
    }
    let chroma = a.hypot(b);
    if chroma <= f64::EPSILON {
        return [l, 0.0, 0.0];
    }
    let hue = b.atan2(a).to_degrees().rem_euclid(360.0);
    oklab_polar(l, hue, chroma.min(max_oklab_chroma(l, hue)))
}

pub fn oklab_plane_projected(l: f64, x: f64, y: f64) -> [f64; 3] {
    project_oklab_to_gamut(oklab_plane(l, x, y))
}

pub fn adjust_oklab_plane(lab: [f64; 3], key: PlaneKey, fine: bool) -> [f64; 3] {
    if key == PlaneKey::Home {
        return [lab[0].clamp(0.0, 1.0), 0.0, 0.0];
    }
    let step = if fine { 0.0005 } else { 0.005 };
    let mut requested = lab;
    match key {
        PlaneKey::Left => requested[1] -= step,
        PlaneKey::Right => requested[1] += step,
        PlaneKey::Up => requested[2] += step,
        PlaneKey::Down => requested[2] -= step,
        PlaneKey::Home => unreachable!(),
    }
    project_oklab_to_gamut(requested)
}

pub fn oklab_wheel(l: f64, x: f64, y: f64) -> [f64; 3] {
    oklab_plane_projected(l, x, y)
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
    let encoded = linear.map(|value| linear_to_srgb(value as f32).clamp(0.0, 1.0));
    format!(
        "#{:02X}{:02X}{:02X}",
        (encoded[0] * 255.0).round() as u8,
        (encoded[1] * 255.0).round() as u8,
        (encoded[2] * 255.0).round() as u8
    )
}
