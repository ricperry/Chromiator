//! GTK-independent color-picker calculations and dialog-local history.

use crate::color::{
    ColorModel, DraftColor, hsl_to_encoded, hsv_to_encoded, linear_to_encoded, okhsl_to_linear,
};

/// Continuous picker gesture used to coalesce local draft changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerGesture {
    Wheel,
    Channel(usize),
}

/// Dialog-local draft history. It never mutates document history until the caller commits once.
#[derive(Default)]
pub struct PickerLocalHistory {
    pub undo: Vec<DraftColor>,
    pub redo: Vec<DraftColor>,
    active: Option<PickerGesture>,
}

impl PickerLocalHistory {
    /// Records a real draft change, coalescing repeated updates from one gesture.
    pub fn record(
        &mut self,
        before: DraftColor,
        after: DraftColor,
        gesture: Option<PickerGesture>,
    ) -> bool {
        if before == after {
            return false;
        }
        if gesture.is_none() || self.active != gesture {
            self.undo.push(before);
        }
        self.active = gesture;
        self.redo.clear();
        true
    }

    /// Ends the matching local gesture without touching the document session.
    pub fn finish(&mut self, gesture: PickerGesture) {
        if self.active == Some(gesture) {
            self.active = None;
        }
    }

    /// Applies one local undo or redo step and returns the replacement draft.
    pub fn step(&mut self, current: DraftColor, redo: bool) -> Option<DraftColor> {
        self.active = None;
        let target = if redo {
            self.redo.pop()
        } else {
            self.undo.pop()
        }?;
        if redo {
            self.undo.push(current);
        } else {
            self.redo.push(current);
        }
        Some(target)
    }
}

/// Evenly spaced values after `start`, including `target`, for bounded animation updates.
/// Retains an adjustment's full precision when its displayed rounded value is reparsed.
///
/// `None` delegates a genuinely different edit to GTK's normal input parser. Decimal
/// points and commas are accepted here only for recognizing unchanged numeric text;
/// parsing and validation of new input remain GTK's responsibility.
pub fn preserve_displayed_precision(text: &str, value: f64, digits: u32) -> Option<f64> {
    let entered = text.trim().replace(',', ".").parse::<f64>().ok()?;
    let displayed = format!("{value:.precision$}", precision = digits as usize)
        .parse::<f64>()
        .ok()?;
    (entered == displayed).then_some(value)
}

/// Evenly spaced values after `start`, including `target`, for bounded animation updates.
pub fn picker_lightness_sequence(start: f64, target: f64, updates: u32) -> Vec<f64> {
    (1..=updates)
        .map(|index| start + (target - start) * f64::from(index) / f64::from(updates))
        .collect()
}

/// Convert logical picker dimensions to at-least-one-pixel device dimensions.
pub fn picker_plane_physical_size(width: i32, height: i32, scale: i32) -> (i32, i32) {
    let scale = scale.max(1);
    (width.max(1) * scale, height.max(1) * scale)
}

/// Sample a circular hue/saturation plane or an RGB square at fixed Blue.
pub fn picker_plane_encoded_sample(
    model: ColorModel,
    fixed_axis: f64,
    x: f64,
    y: f64,
) -> Option<[f64; 3]> {
    if model == ColorModel::Rgb {
        return (x.abs() <= 1.0 + 1.0e-9 && y.abs() <= 1.0 + 1.0e-9).then(|| [
            ((x + 1.0) * 0.5).clamp(0.0, 1.0),
            ((1.0 - y) * 0.5).clamp(0.0, 1.0),
            fixed_axis.clamp(0.0, 1.0),
        ]);
    }
    let distance = x.hypot(y);
    if distance > 1.0 + 1.0e-9 {
        return None;
    }
    let distance = distance.min(1.0);
    let hue = y.atan2(x).to_degrees().rem_euclid(360.0);
    Some(match model {
        ColorModel::Rgb => unreachable!("RGB square handled before circular projection"),
        ColorModel::Hsv => hsv_to_encoded([hue, distance, fixed_axis.clamp(0.0, 1.0)]),
        ColorModel::Hsl => hsl_to_encoded([hue, distance, fixed_axis.clamp(0.0, 1.0)]),
        ColorModel::Okhsl => linear_to_encoded(
            okhsl_to_linear([hue / 360.0, distance, fixed_axis.clamp(0.0, 1.0)])
                .expect("the normalized OKHSL plane is completely inside bounded sRGB"),
        ),
    })
}

#[cfg(test)]
mod precision_tests {
    use super::*;

    #[test]
    fn unchanged_spin_text_preserves_bits_and_real_edits_use_the_normal_parser() {
        let value: f64 = 56.703123456789;
        for text in ["56.70", "56.7", "56,70"] {
            assert_eq!(
                preserve_displayed_precision(text, value, 2)
                    .unwrap()
                    .to_bits(),
                value.to_bits()
            );
        }
        for text in ["56.71", "invalid", "NaN"] {
            assert_eq!(preserve_displayed_precision(text, value, 2), None);
        }
    }

    #[test]
    fn focus_roundtrips_do_not_hide_the_real_lightness_undo_step() {
        let before = DraftColor::new([0.08, 0.3, 0.4]);
        let mut after = before;
        let mut channels = before.values(ColorModel::Okhsl);
        channels[2] = 0.45;
        after.set_values(ColorModel::Okhsl, channels);
        let mut history = PickerLocalHistory::default();
        assert!(history.record(before, after, Some(PickerGesture::Channel(2))));
        history.finish(PickerGesture::Channel(2));
        for channel in after.values(ColorModel::Okhsl) {
            let text = format!("{channel:.2}");
            assert_eq!(
                preserve_displayed_precision(&text, channel, 2),
                Some(channel)
            );
            assert!(!history.record(after, after, None));
        }
        assert_eq!(history.undo.len(), 1);
        assert_eq!(history.step(after, false), Some(before));
        assert_eq!(history.step(before, true), Some(after));
    }
}
