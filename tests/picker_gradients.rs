use chromiator::color::{ColorModel, DraftColor, okhsl_to_linear};

#[test]
fn saturated_blue_cusp_remains_in_gamut() {
    let rgb = okhsl_to_linear([
        0.7334778351057085,
        0.9999999965721178,
        0.390625,
    ])
    .expect("the blue cusp must support valid normalized OKHSL controls");
    assert!(rgb.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)));
}

#[test]
fn picker_channel_gradients_are_bounded_for_saturated_colors() {
    for original in [
        [0.0, 0.0, 0.21404114],
        [0.0, 0.0, 1.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [1.0, 0.0, 1.0],
        [0.0, 1.0, 1.0],
        [1.0, 1.0, 0.0],
    ] {
        let draft = DraftColor::new(original);
        let initial = draft.values(ColorModel::Okhsl);
        // Match the native gradient renderer, including both endpoints.
        for channel in 0..3 {
            for step in 0..=128 {
                let mut values = initial;
                values[channel] = f64::from(step) / 128.0
                    * if channel == 0 { 360.0 } else { 1.0 };
                let mut sample = draft;
                assert!(okhsl_to_linear([values[0].rem_euclid(360.0) / 360.0, values[1], values[2]]).is_some(),
                    "original={original:?} channel={channel} step={step} values={values:?}");
                sample.set_values(ColorModel::Okhsl, values);
                assert!(sample.linear.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)),
                    "original={original:?} channel={channel} step={step}");
            }
        }
        assert_eq!(draft.commit(), original, "drawing must not mutate the selected color");
    }
}
