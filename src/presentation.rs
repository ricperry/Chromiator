//! GTK canvas, color-picker dialog, and screenshot presentation helpers.
//!
//! Creative state remains in `DocumentSession`; this module owns only widget state, drawing,
//! dialog-local drafts, and display-buffer conversion.

use super::*;

#[derive(Clone, Copy)]
pub(super) enum PickerPurpose {
    Source,
    Target,
    NewSite,
}

#[derive(Clone)]
struct PickerControls {
    labels: [gtk::Label; 3],
    adjustments: [gtk::Adjustment; 3],
    spins: [gtk::SpinButton; 3],
    scales: [gtk::Scale; 3],
    channel_bars: [gtk::DrawingArea; 3],
    wheel: gtk::DrawingArea,
    hex: gtk::Entry,
    new_swatch: gtk::DrawingArea,
    syncing: Rc<Cell<bool>>,
    model: Rc<Cell<ColorModel>>,
}

fn picker_values(controls: &PickerControls) -> [f64; 3] {
    let mut values = controls
        .adjustments
        .clone()
        .map(|adjustment| adjustment.value());
    if controls.model.get() == ColorModel::Rgb {
        values = values.map(|value| value / 255.0);
    } else {
        values[1] /= 100.0;
        values[2] /= 100.0;
    }
    values
}

type PickerModelSpec = ([&'static str; 3], [(f64, f64); 3], [u32; 3]);

fn picker_model_spec(model: ColorModel) -> PickerModelSpec {
    match model {
        ColorModel::Rgb => (
            ["Red", "Green", "Blue"],
            [(0.0, 255.0); 3],
            [0; 3],
        ),
        ColorModel::Hsv => (
            ["Hue", "Saturation", "Value"],
            [(0.0, 360.0), (0.0, 100.0), (0.0, 100.0)],
            [2, 2, 2],
        ),
        ColorModel::Hsl => (
            ["Hue", "Saturation", "Lightness"],
            [(0.0, 360.0), (0.0, 100.0), (0.0, 100.0)],
            [2, 2, 2],
        ),
        ColorModel::Okhsl => (
            ["Hue", "Saturation", "Lightness"],
            [(0.0, 360.0), (0.0, 100.0), (0.0, 100.0)],
            [2, 2, 2],
        ),
    }
}

fn configure_picker_model(controls: &PickerControls) {
    // Changing bounds can clamp adjustments; that must not author a color edit.
    let was_syncing = controls.syncing.replace(true);
    let model = controls.model.get();
    let (names, bounds, digits) = picker_model_spec(model);
    for index in 0..3 {
        let unit = if model == ColorModel::Rgb { "0-255" } else if index == 0 { "degrees" } else { "%" };
        controls.labels[index].set_label(&format!("{} ({unit})", names[index]));
        controls.adjustments[index].set_lower(bounds[index].0);
        controls.adjustments[index].set_upper(bounds[index].1);
        controls.adjustments[index].set_step_increment(if model == ColorModel::Rgb { 1.0 } else { 0.1 });
        controls.spins[index].set_digits(digits[index]);
        controls.spins[index].update_property(&[gtk::accessible::Property::Label(names[index])]);
        controls.scales[index].update_property(&[
            gtk::accessible::Property::Label(&format!("{} slider", names[index])),
            gtk::accessible::Property::Description(
                "The gradient previews this channel with the other two channels unchanged. Arrow keys adjust the value.",
            ),
        ]);
    }
    let description = if model == ColorModel::Rgb {
        "RGB square: Red increases left to right, Green bottom to top. Blue is set by its slider. Arrow keys adjust Red and Green; Shift makes fine adjustments; Home sets both to zero."
    } else {
        "Arrow keys adjust the color, Shift makes fine adjustments, Home returns to neutral"
    };
    controls.wheel.set_tooltip_text(Some(description));
    controls.wheel.update_property(&[gtk::accessible::Property::Description(description)]);
    controls.syncing.set(was_syncing);
}

fn picker_display_values(draft: DraftColor, model: ColorModel) -> [f64; 3] {
    let values = draft.values(model);
    if model == ColorModel::Rgb {
        values.map(|value| value * 255.0)
    } else {
        [values[0], values[1] * 100.0, values[2] * 100.0]
    }
}

fn refresh_picker_outputs(controls: &PickerControls, draft: DraftColor) {
    let model = controls.model.get();
    let (_, _, digits) = picker_model_spec(model);
    let values = picker_display_values(draft, model);
    for index in 0..3 {
        controls.spins[index].update_property(&[gtk::accessible::Property::ValueText(&format!(
            "{:.prec$}",
            values[index],
            prec = digits[index] as usize
        ))]);
    }
    controls.hex.set_text(&display_hex(draft.linear));
    controls.wheel.queue_draw();
    for bar in &controls.channel_bars {
        bar.queue_draw();
    }
    controls.new_swatch.queue_draw();
}

fn sync_picker(controls: &PickerControls, draft: DraftColor) {
    controls.syncing.set(true);
    let model = controls.model.get();
    let values = picker_display_values(draft, model);
    for (adjustment, value) in controls.adjustments.iter().zip(values) {
        adjustment.set_value(value);
    }
    refresh_picker_outputs(controls, draft);
    controls.syncing.set(false);
}

fn update_draft_from_wheel(
    controls: &PickerControls,
    draft: &Rc<RefCell<DraftColor>>,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) {
    let radius = width.min(height) / 2.0;
    let nx = (x - width / 2.0) / radius;
    let ny = (y - height / 2.0) / radius;
    let distance = nx.hypot(ny).clamp(0.0, 1.0);
    let hue = ny.atan2(nx).to_degrees().rem_euclid(360.0);
    let model = controls.model.get();
    let mut current = draft.borrow().values(model);
    if model == ColorModel::Rgb {
        current[0] = ((nx + 1.0) * 0.5).clamp(0.0, 1.0);
        current[1] = ((1.0 - ny) * 0.5).clamp(0.0, 1.0);
    } else {
        if distance > 1e-9 {
            current[0] = hue;
        }
        current[1] = distance;
    }
    draft.borrow_mut().set_values(model, current);
    sync_picker(controls, *draft.borrow());
}

pub(super) fn present_color_picker(
    ui: &Rc<Ui>,
    state: &Rc<RefCell<State>>,
    initial_model: ColorModel,
    purpose: PickerPurpose,
) {
    if let Some(dialog) = ui.picker_dialog.borrow().as_ref() {
        dialog.present();
        return;
    }
    let original = match purpose {
        PickerPurpose::Source => selected_source_color(state),
        PickerPurpose::Target => selected_output_color(state),
        PickerPurpose::NewSite => state.borrow().session.document()
            .map(|_| [0.214_041_14; 3]),
    };
    let Some(original) = original else {
        return;
    };
    let draft = Rc::new(RefCell::new(DraftColor::new(original)));
    let model = Rc::new(Cell::new(initial_model));
    let syncing = Rc::new(Cell::new(false));
    {
        let mut state = state.borrow_mut();
        state.picker_visible = true;
        state.picker_model = initial_model;
        state.picker_lightness = (initial_model == ColorModel::Okhsl)
            .then(|| draft.borrow().values(ColorModel::Okhsl)[2]);
        state.picker_lightness_updates = 0;
        state.picker_lightness_elapsed_ms = None;
        state.picker_plane_render_max_us = 0;
        state.picker_plane_render_count = 0;
    }

    let picker_title = match purpose {
        PickerPurpose::Source | PickerPurpose::NewSite => "Pick a source color",
        PickerPurpose::Target => "Pick a target color",
    };
    let dialog = gtk::Window::builder()
        .title(picker_title)
        .transient_for(&ui.window)
        .modal(true)
        .destroy_with_parent(true)
        .default_width(800)
        .default_height(540)
        .build();
    *ui.picker_dialog.borrow_mut() = Some(dialog.clone());
    let model_dropdown = gtk::DropDown::from_strings(&["HSV", "HSL", "OKHSL", "RGB"]);
    *ui.audit_picker_model.borrow_mut() = Some(model_dropdown.clone());
    model_dropdown.set_selected(match initial_model {
        ColorModel::Hsv => 0,
        ColorModel::Hsl => 1,
        ColorModel::Okhsl => 2,
        ColorModel::Rgb => 3,
    });
    model_dropdown.update_property(&[gtk::accessible::Property::Label("Color model")]);
    let wheel = gtk::DrawingArea::builder()
        .content_width(250)
        .content_height(250)
        .focusable(true)
        .hexpand(true)
        .vexpand(false)
        .valign(gtk::Align::Start)
        .build();
    wheel.add_css_class(CREATIVE_FOCUS_CLASS);
    *ui.audit_picker_plane.borrow_mut() = Some(wheel.clone());
    wheel.update_property(&[
        gtk::accessible::Property::Label("Color plane"),
        gtk::accessible::Property::Description(
            "Arrow keys adjust the color, Shift makes fine adjustments, Home returns to neutral",
        ),
    ]);
    let labels = std::array::from_fn(|_| gtk::Label::builder().xalign(0.0).build());
    let adjustments =
        std::array::from_fn(|_| gtk::Adjustment::new(0.0, -0.5, 360.0, 0.1, 1.0, 0.0));
    *ui.audit_picker_channels.borrow_mut() = adjustments.to_vec();
    let spins = std::array::from_fn(|index| {
        let spin = gtk::SpinButton::new(Some(&adjustments[index]), 0.1, 2);
        spin.set_width_chars(7);
        spin.add_css_class("picker-channel-value");
        // Focus changes must not turn rounded display text into an authored color edit.
        spin.connect_input(|spin| {
            chromiator::picker::preserve_displayed_precision(
                spin.text().as_str(),
                spin.value(),
                spin.digits(),
            )
            .map(Ok)
        });
        spin
    });
    let scales = std::array::from_fn(|index| {
        let scale = gtk::Scale::new(gtk::Orientation::Horizontal, Some(&adjustments[index]));
        scale.set_draw_value(false);
        scale.set_hexpand(true);
        scale.add_css_class("picker-channel-scale");
        scale
    });
    let channel_bars = std::array::from_fn(|index| {
        let bar = gtk::DrawingArea::builder()
            .content_width(180)
            .content_height(40)
            .hexpand(true)
            .can_target(false)
            .build();
        let draft = draft.clone();
        let model = model.clone();
        bar.set_draw_func(move |_, ctx, width, height| {
            // Match the native scale's horizontal padding and trough height.
            // Sampling uses the same color conversion as authored picker edits.
            let x = 8.0;
            let y = (f64::from(height) - 28.0) * 0.5;
            let w = (f64::from(width) - 16.0).max(1.0);
            let gradient = gtk::cairo::LinearGradient::new(x, 0.0, x + w, 0.0);
            let original = *draft.borrow();
            let model = model.get();
            let original_values = original.values(model);
            for step in 0..=128 {
                let fraction = f64::from(step) / 128.0;
                let mut values = original_values;
                values[index] = fraction * if model != ColorModel::Rgb && index == 0 { 360.0 } else { 1.0 };
                let mut sample = original;
                sample.set_values(model, values);
                let rgb = sample.encoded();
                gradient.add_color_stop_rgb(fraction, rgb[0], rgb[1], rgb[2]);
            }
            let radius = 5.0;
            ctx.new_sub_path();
            ctx.arc(x + w - radius, y + radius, radius, -std::f64::consts::FRAC_PI_2, 0.0);
            ctx.arc(x + w - radius, y + 28.0 - radius, radius, 0.0, std::f64::consts::FRAC_PI_2);
            ctx.arc(x + radius, y + 28.0 - radius, radius, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
            ctx.arc(x + radius, y + radius, radius, std::f64::consts::PI, 3.0 * std::f64::consts::FRAC_PI_2);
            ctx.close_path();
            let _ = ctx.set_source(&gradient);
            let _ = ctx.fill();
        });
        bar
    });
    let controls_box = gtk::Box::new(gtk::Orientation::Vertical, 12);
    controls_box.set_width_request(360);
    let model_label = gtk::Label::builder().label("_Color model").use_underline(true).xalign(0.0).build();
    model_label.set_mnemonic_widget(Some(&model_dropdown));
    model_dropdown.update_relation(&[gtk::accessible::Relation::LabelledBy(&[model_label.upcast_ref()])]);
    controls_box.append(&model_label);
    controls_box.append(&model_dropdown);
    for index in 0..3 {
        labels[index].set_mnemonic_widget(Some(&spins[index]));
        spins[index].update_relation(&[gtk::accessible::Relation::LabelledBy(&[labels[index].upcast_ref()])]);
        let row = gtk::Box::new(gtk::Orientation::Vertical, 4);
        row.append(&labels[index]);
        let linked = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let slider = gtk::Overlay::new();
        slider.set_hexpand(true);
        slider.set_child(Some(&channel_bars[index]));
        slider.add_overlay(&scales[index]);
        slider.set_measure_overlay(&scales[index], true);
        linked.append(&slider);
        linked.append(&spins[index]);
        row.append(&linked);
        controls_box.append(&row);
    }
    let hex = gtk::Entry::builder().placeholder_text("#RRGGBB").build();
    *ui.audit_picker_hex.borrow_mut() = Some(hex.clone());
    hex.update_property(&[gtk::accessible::Property::Label("Hex color")]);
    let hex_label = gtk::Label::builder().label("_Hex (encoded sRGB)").use_underline(true).xalign(0.0).build();
    hex_label.set_mnemonic_widget(Some(&hex));
    hex.update_relation(&[gtk::accessible::Relation::LabelledBy(&[hex_label.upcast_ref()])]);
    controls_box.append(&hex_label);
    controls_box.append(&hex);
    let original_swatch = gtk::DrawingArea::builder()
        .content_width(120)
        .content_height(48)
        .build();
    let new_swatch = gtk::DrawingArea::builder()
        .content_width(120)
        .content_height(48)
        .build();
    let swatches = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let original_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
    original_box.append(&gtk::Label::new(Some("Original")));
    original_box.append(&gtk::Frame::builder().child(&original_swatch).build());
    let new_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
    new_box.append(&gtk::Label::new(Some("New")));
    new_box.append(&gtk::Frame::builder().child(&new_swatch).build());
    swatches.append(&original_box);
    swatches.append(&new_box);
    controls_box.append(&swatches);
    let controls = PickerControls {
        labels,
        adjustments,
        spins,
        scales,
        channel_bars,
        wheel: wheel.clone(),
        hex: hex.clone(),
        new_swatch: new_swatch.clone(),
        syncing,
        model,
    };

    let flow = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(false)
        .column_spacing(20)
        .row_spacing(20)
        .min_children_per_line(1)
        .max_children_per_line(2)
        .build();
    flow.insert(&wheel, -1);
    flow.insert(&controls_box, -1);
    let cancel = gtk::Button::with_label("Cancel");
    let select = gtk::Button::with_label("Select");
    let local_undo = icon_button("edit-undo-symbolic", "Undo color edit (Ctrl+Z)");
    let local_redo = icon_button("edit-redo-symbolic", "Redo color edit (Ctrl+Shift+Z)");
    local_undo.set_sensitive(false);
    local_redo.set_sensitive(false);
    local_undo.update_property(&[gtk::accessible::Property::Label("Undo color edit")]);
    local_redo.update_property(&[gtk::accessible::Property::Label("Redo color edit")]);
    *ui.audit_picker_cancel.borrow_mut() = Some(cancel.clone());
    *ui.audit_picker_select.borrow_mut() = Some(select.clone());
    *ui.audit_picker_undo.borrow_mut() = Some(local_undo.clone());
    *ui.audit_picker_redo.borrow_mut() = Some(local_redo.clone());
    select.add_css_class("suggested-action");
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    buttons.append(&cancel);
    buttons.append(&select);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 16);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);
    content.append(
        &gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .vexpand(true)
            .child(&flow)
            .build(),
    );
    content.append(&buttons);
    let header = gtk::HeaderBar::new();
    header.pack_start(&local_undo);
    header.pack_start(&local_redo);
    header.set_title_widget(Some(&gtk::Label::new(Some(picker_title))));
    dialog.set_titlebar(Some(&header));
    dialog.set_child(Some(&content));
    let picker_history = Rc::new(RefCell::new(PickerLocalHistory::default()));
    let sync_picker_history: Rc<dyn Fn()> = Rc::new({
        let picker_history = picker_history.clone();
        let local_undo = local_undo.clone();
        let local_redo = local_redo.clone();
        move || {
            let history = picker_history.borrow();
            local_undo.set_sensitive(!history.undo.is_empty());
            local_redo.set_sensitive(!history.redo.is_empty());
        }
    });

    {
        let draft = draft.clone();
        let controls = controls.clone();
        let state = state.clone();
        let plane_cache = RefCell::new(None);
        wheel.set_draw_func(move |area, ctx, width, height| {
            let started = std::time::Instant::now();
            draw_color_plane(
                ctx,
                width,
                height,
                area.scale_factor(),
                *draft.borrow(),
                controls.model.get(),
                &plane_cache,
            );
            let elapsed = started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64;
            let mut state = state.borrow_mut();
            state.picker_plane_render_count += 1;
            state.picker_plane_render_max_us = state.picker_plane_render_max_us.max(elapsed);
        });
    }
    {
        let encoded = original.map(chromiator::processing::linear_to_srgb);
        original_swatch.set_draw_func(move |_, ctx, width, height| {
            ctx.set_source_rgb(encoded[0] as f64, encoded[1] as f64, encoded[2] as f64);
            ctx.rectangle(0.0, 0.0, width as f64, height as f64);
            let _ = ctx.fill();
        });
    }
    {
        let draft = draft.clone();
        new_swatch.set_draw_func(move |_, ctx, width, height| {
            let encoded = draft.borrow().encoded();
            ctx.set_source_rgb(encoded[0], encoded[1], encoded[2]);
            ctx.rectangle(0.0, 0.0, width as f64, height as f64);
            let _ = ctx.fill();
        });
    }
    for (index, adjustment) in controls.adjustments.iter().enumerate() {
        let controls = controls.clone();
        let draft = draft.clone();
        let state = state.clone();
        let picker_history = picker_history.clone();
        let sync_picker_history = sync_picker_history.clone();
        adjustment.connect_value_changed(move |_| {
            if controls.syncing.get() {
                return;
            }
            let before = *draft.borrow();
            draft
                .borrow_mut()
                .set_values(controls.model.get(), picker_values(&controls));
            if index == 2 && controls.model.get() == ColorModel::Okhsl {
                let mut state = state.borrow_mut();
                state.picker_lightness = Some(controls.adjustments[2].value() / 100.0);
                state.picker_lightness_updates += 1;
            }
            picker_history.borrow_mut().record(
                before,
                *draft.borrow(),
                Some(PickerGesture::Channel(index)),
            );
            sync_picker_history();
            refresh_picker_outputs(&controls, *draft.borrow());
        });
    }
    for index in 0..3 {
        for widget in [
            controls.spins[index].clone().upcast::<gtk::Widget>(),
            controls.scales[index].clone().upcast::<gtk::Widget>(),
        ] {
            let click = gtk::GestureClick::new();
            click.set_propagation_phase(gtk::PropagationPhase::Capture);
            click.connect_pressed({
                let picker_history = picker_history.clone();
                move |_, _, _, _| {
                    picker_history
                        .borrow_mut()
                        .finish(PickerGesture::Channel(index));
                }
            });
            click.connect_released({
                let picker_history = picker_history.clone();
                move |_, _, _, _| {
                    picker_history
                        .borrow_mut()
                        .finish(PickerGesture::Channel(index));
                }
            });
            widget.add_controller(click);

            let keys = gtk::EventControllerKey::new();
            keys.set_propagation_phase(gtk::PropagationPhase::Capture);
            keys.connect_key_released({
                let picker_history = picker_history.clone();
                move |_, _, _, _| {
                    picker_history
                        .borrow_mut()
                        .finish(PickerGesture::Channel(index));
                }
            });
            widget.add_controller(keys);

            let picker_history = picker_history.clone();
            widget.connect_has_focus_notify(move |widget| {
                if !widget.has_focus() {
                    picker_history
                        .borrow_mut()
                        .finish(PickerGesture::Channel(index));
                }
            });
        }
    }
    {
        let controls = controls.clone();
        let draft = draft.clone();
        let state = state.clone();
        model_dropdown.connect_selected_notify(move |dropdown| {
            let selected = match dropdown.selected() {
                1 => ColorModel::Hsl,
                2 => ColorModel::Okhsl,
                3 => ColorModel::Rgb,
                _ => ColorModel::Hsv,
            };
            controls.model.set(selected);
            {
                let mut state = state.borrow_mut();
                state.picker_model = selected;
                state.picker_lightness = (selected == ColorModel::Okhsl)
                    .then(|| draft.borrow().values(ColorModel::Okhsl)[2]);
            }
            configure_picker_model(&controls);
            sync_picker(&controls, *draft.borrow());
        });
    }
    {
        let controls = controls.clone();
        let draft = draft.clone();
        let picker_history = picker_history.clone();
        let sync_picker_history = sync_picker_history.clone();
        hex.connect_activate(move |entry| match parse_hex(entry.text().as_str()) {
            Ok(linear) => {
                let before = *draft.borrow();
                draft.borrow_mut().linear = linear;
                entry.remove_css_class("error");
                sync_picker(&controls, *draft.borrow());
                picker_history
                    .borrow_mut()
                    .record(before, *draft.borrow(), None);
                sync_picker_history();
            }
            Err(message) => {
                entry.add_css_class("error");
                entry.set_tooltip_text(Some(message));
            }
        });
    }
    let click = gtk::GestureClick::new();
    {
        let controls = controls.clone();
        let draft = draft.clone();
        let wheel = wheel.clone();
        let picker_history = picker_history.clone();
        let sync_picker_history = sync_picker_history.clone();
        click.connect_pressed(move |_, _, x, y| {
            let before = *draft.borrow();
            update_draft_from_wheel(
                &controls,
                &draft,
                x,
                y,
                wheel.width() as f64,
                wheel.height() as f64,
            );
            picker_history
                .borrow_mut()
                .record(before, *draft.borrow(), Some(PickerGesture::Wheel));
            sync_picker_history();
        });
    }
    {
        let picker_history = picker_history.clone();
        click.connect_released(move |_, _, _, _| {
            picker_history.borrow_mut().finish(PickerGesture::Wheel);
        });
    }
    wheel.add_controller(click);
    let drag = gtk::GestureDrag::new();
    {
        let controls = controls.clone();
        let draft = draft.clone();
        let wheel = wheel.clone();
        let picker_history = picker_history.clone();
        let sync_picker_history = sync_picker_history.clone();
        drag.connect_drag_update(move |gesture, dx, dy| {
            let Some((x, y)) = gesture.start_point() else {
                return;
            };
            let before = *draft.borrow();
            update_draft_from_wheel(
                &controls,
                &draft,
                x + dx,
                y + dy,
                wheel.width() as f64,
                wheel.height() as f64,
            );
            picker_history
                .borrow_mut()
                .record(before, *draft.borrow(), Some(PickerGesture::Wheel));
            sync_picker_history();
        });
    }
    {
        let picker_history = picker_history.clone();
        drag.connect_drag_end(move |_, _, _| {
            picker_history.borrow_mut().finish(PickerGesture::Wheel);
        });
    }
    wheel.add_controller(drag);
    let keys = gtk::EventControllerKey::new();
    {
        let controls = controls.clone();
        let draft = draft.clone();
        let picker_history = picker_history.clone();
        let sync_picker_history = sync_picker_history.clone();
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            let fine = modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK);
            let model = controls.model.get();
            let mut values = draft.borrow().values(model);
            if model == ColorModel::Rgb {
                let step = if fine { 0.1 / 255.0 } else { 1.0 / 255.0 };
                match key {
                    gtk::gdk::Key::Left => values[0] = (values[0] - step).max(0.0),
                    gtk::gdk::Key::Right => values[0] = (values[0] + step).min(1.0),
                    gtk::gdk::Key::Up => values[1] = (values[1] + step).min(1.0),
                    gtk::gdk::Key::Down => values[1] = (values[1] - step).max(0.0),
                    gtk::gdk::Key::Home => { values[0] = 0.0; values[1] = 0.0; }
                    _ => return glib::Propagation::Proceed,
                }
            } else if model == ColorModel::Okhsl {
                let plane_key = match key {
                    gtk::gdk::Key::Left => PlaneKey::Left,
                    gtk::gdk::Key::Right => PlaneKey::Right,
                    gtk::gdk::Key::Up => PlaneKey::Up,
                    gtk::gdk::Key::Down => PlaneKey::Down,
                    gtk::gdk::Key::Home => PlaneKey::Home,
                    _ => return glib::Propagation::Proceed,
                };
                values = adjust_okhsl(values, plane_key, fine);
            } else {
                let step = if fine { 0.1 } else { 1.0 };
                match key {
                    gtk::gdk::Key::Left => values[0] -= step,
                    gtk::gdk::Key::Right => values[0] += step,
                    gtk::gdk::Key::Up => values[1] = (values[1] + step / 100.0).min(1.0),
                    gtk::gdk::Key::Down => values[1] = (values[1] - step / 100.0).max(0.0),
                    gtk::gdk::Key::Home => values[1] = 0.0,
                    _ => return glib::Propagation::Proceed,
                }
            }
            let before = *draft.borrow();
            draft.borrow_mut().set_values(model, values);
            picker_history
                .borrow_mut()
                .record(before, *draft.borrow(), None);
            sync_picker_history();
            sync_picker(&controls, *draft.borrow());
            controls.wheel.queue_draw();
            glib::Propagation::Stop
        });
    }
    wheel.add_controller(keys);
    let picker_history_step: Rc<dyn Fn(bool)> = Rc::new({
        let draft = draft.clone();
        let picker_history = picker_history.clone();
        let controls = controls.clone();
        let sync_picker_history = sync_picker_history.clone();
        move |redo| {
            let current = *draft.borrow();
            let Some(target) = picker_history.borrow_mut().step(current, redo) else {
                return;
            };
            *draft.borrow_mut() = target;
            sync_picker(&controls, target);
            sync_picker_history();
        }
    });
    local_undo.connect_clicked({
        let picker_history_step = picker_history_step.clone();
        move |_| picker_history_step(false)
    });
    local_redo.connect_clicked({
        let picker_history_step = picker_history_step.clone();
        move |_| picker_history_step(true)
    });
    {
        let dialog = dialog.clone();
        cancel.connect_clicked(move |_| {
            dialog.close();
        });
    }
    {
        let ui = ui.clone();
        let state = state.clone();
        let draft = draft.clone();
        let dialog = dialog.clone();
        let committed = Rc::new(Cell::new(false));
        select.connect_clicked(move |_| {
            if committed.replace(true) {
                return;
            }
            let site_id = state.borrow().session.selection().selected;
            let selected = draft.borrow().commit();
            let command = match (purpose, site_id) {
                (PickerPurpose::NewSite, _) => EditCommand::AddSiteColor {
                    color: [selected[0], selected[1], selected[2], 1.0],
                    position: None,
                },
                (PickerPurpose::Source, Some(site_id)) => {
                    let alpha = state
                        .borrow()
                        .session
                        .document()
                        .and_then(|document| document.recipe.voronoi.site(site_id))
                        .map_or(1.0, |site| site.source_color[3]);
                    EditCommand::SetSource {
                        site_id,
                        color: [selected[0], selected[1], selected[2], alpha],
                        position: None,
                    }
                }
                (PickerPurpose::Target, Some(site_id)) => EditCommand::SetTarget {
                    site_id,
                    color: selected,
                },
                _ => {
                    dialog.close();
                    return;
                }
            };
            let changed = submit_session_edit(
                &ui,
                &state,
                command,
                None,
                match purpose {
                    PickerPurpose::Source => {
                        "Source changed; Target preserved and marker detached — updating preview…"
                    }
                    PickerPurpose::Target => "Target changed — updating preview…",
                    PickerPurpose::NewSite => "Detached Source color added — updating preview…",
                },
            );
            dialog.close();
            refresh_voronoi_ui(&ui, &state);
            let _ = changed;
        });
    }
    {
        let state = state.clone();
        let ui = ui.clone();
        dialog.connect_close_request(move |_| {
            state.borrow_mut().picker_visible = false;
            *ui.picker_dialog.borrow_mut() = None;
            *ui.audit_picker_model.borrow_mut() = None;
            *ui.audit_picker_hex.borrow_mut() = None;
            ui.audit_picker_channels.borrow_mut().clear();
            *ui.audit_picker_undo.borrow_mut() = None;
            *ui.audit_picker_redo.borrow_mut() = None;
            *ui.audit_picker_cancel.borrow_mut() = None;
            *ui.audit_picker_select.borrow_mut() = None;
            *ui.audit_picker_plane.borrow_mut() = None;
            sync_document_history_ui(&ui, &state);
            let focus = ui.groups.clone();
            glib::idle_add_local_once(move || {
                focus.grab_focus();
            });
            glib::Propagation::Proceed
        });
    }
    let escape = gtk::EventControllerKey::new();
    escape.set_propagation_phase(gtk::PropagationPhase::Capture);
    escape.connect_key_pressed({
        let dialog = dialog.clone();
        let picker_history_step = picker_history_step.clone();
        move |_, key, _, modifiers| {
            if key == gtk::gdk::Key::Escape {
                dialog.close();
                glib::Propagation::Stop
            } else if key == gtk::gdk::Key::z
                && modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK)
            {
                picker_history_step(modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK));
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        }
    });
    dialog.add_controller(escape);
    configure_picker_model(&controls);
    sync_picker(&controls, *draft.borrow());
    sync_document_history_ui(ui, state);
    dialog.present();
    if initial_model == ColorModel::Okhsl
        && let Some(target) = ui.cli.picker_lightness
    {
        let values = Rc::new(picker_lightness_sequence(
            controls.adjustments[2].value(),
            target * 100.0,
            120,
        ));
        let index = Rc::new(Cell::new(0usize));
        let adjustment = controls.adjustments[2].clone();
        let state = state.clone();
        let started = std::time::Instant::now();
        glib::timeout_add_local(std::time::Duration::from_millis(2), move || {
            let current = index.get();
            adjustment.set_value(values[current]);
            index.set(current + 1);
            if current + 1 == values.len() {
                state.borrow_mut().picker_lightness_elapsed_ms =
                    Some(started.elapsed().as_secs_f64() * 1000.0);
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PickerPlaneKey {
    model: ColorModel,
    width: i32,
    height: i32,
    scale: i32,
    fixed_axis: u64,
}

struct PickerPlaneCache {
    key: PickerPlaneKey,
    surface: gtk::cairo::ImageSurface,
}

fn picker_plane_raster_size(model: ColorModel, width: i32, height: i32, scale: i32) -> (i32, i32) {
    let (width, height) = picker_plane_physical_size(width, height, scale);
    if model != ColorModel::Okhsl || width.max(height) <= 96 {
        return (width, height);
    }
    let ratio = 96.0 / f64::from(width.max(height));
    (
        (f64::from(width) * ratio).round().max(1.0) as i32,
        (f64::from(height) * ratio).round().max(1.0) as i32,
    )
}

fn render_picker_plane(
    model: ColorModel,
    fixed_axis: f64,
    width: i32,
    height: i32,
    scale: i32,
) -> gtk::cairo::ImageSurface {
    let (pixel_width, pixel_height) = picker_plane_raster_size(model, width, height, scale);
    let stride = gtk::cairo::Format::Rgb24
        .stride_for_width(pixel_width as u32)
        .expect("picker plane stride");
    let mut data = vec![0_u8; stride as usize * pixel_height as usize];
    let size = pixel_width.min(pixel_height) as f64;
    let radius = size / 2.0;
    let center_x = pixel_width as f64 / 2.0;
    let center_y = pixel_height as f64 / 2.0;
    let okhsl_sampler =
        (model == ColorModel::Okhsl).then(|| OkhslPlaneSampler::new(fixed_axis, 720));
    for row in 0..pixel_height {
        for column in 0..pixel_width {
            let x = (column as f64 + 0.5 - center_x) / radius;
            let y = (row as f64 + 0.5 - center_y) / radius;
            let distance = x.hypot(y);
            let sample_distance = distance.min(1.0);
            let (sample_x, sample_y) = if model == ColorModel::Rgb {
                (x.clamp(-1.0, 1.0), y.clamp(-1.0, 1.0))
            } else if distance > 1.0 {
                (x / distance, y / distance)
            } else {
                (x, y)
            };
            let encoded = if let Some(sampler) = &okhsl_sampler {
                sampler.encoded(sample_y.atan2(sample_x).to_degrees(), sample_distance)
            } else {
                picker_plane_encoded_sample(model, fixed_axis, sample_x, sample_y)
                    .expect("projected inside picker plane")
            };
            let [red, green, blue] =
                encoded.map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u32);
            let native = ((red << 16) | (green << 8) | blue).to_ne_bytes();
            let offset = row as usize * stride as usize + column as usize * 4;
            data[offset..offset + 4].copy_from_slice(&native);
        }
    }
    gtk::cairo::ImageSurface::create_for_data(
        data,
        gtk::cairo::Format::Rgb24,
        pixel_width,
        pixel_height,
        stride,
    )
    .expect("picker plane surface")
}

fn draw_color_plane(
    ctx: &gtk::cairo::Context,
    width: i32,
    height: i32,
    scale: i32,
    draft: DraftColor,
    model: ColorModel,
    cache: &RefCell<Option<PickerPlaneCache>>,
) {
    let size = width.min(height) as f64;
    let radius = size / 2.0;
    let left = (width as f64 - size) / 2.0;
    let top = (height as f64 - size) / 2.0;
    let fixed = draft.values(model);
    let key = PickerPlaneKey {
        model,
        width,
        height,
        scale: scale.max(1),
        fixed_axis: fixed[2].to_bits(),
    };
    let surface = {
        let mut cache = cache.borrow_mut();
        if cache.as_ref().is_none_or(|entry| entry.key != key) {
            *cache = Some(PickerPlaneCache {
                key,
                surface: render_picker_plane(model, fixed[2], width, height, scale),
            });
        }
        cache
            .as_ref()
            .expect("picker plane cache populated")
            .surface
            .clone()
    };
    let _ = ctx.save();
    if model == ColorModel::Rgb {
        ctx.rectangle(left, top, size, size);
    } else {
        ctx.arc(
            width as f64 / 2.0,
            height as f64 / 2.0,
            radius,
            0.0,
            std::f64::consts::TAU,
        );
    }
    ctx.clip();
    ctx.scale(
        f64::from(width) / f64::from(surface.width()),
        f64::from(height) / f64::from(surface.height()),
    );
    let _ = ctx.set_source_surface(&surface, 0.0, 0.0);
    let _ = ctx.paint();
    let _ = ctx.restore();
    let radians = fixed[0].to_radians();
    let marker = if model == ColorModel::Rgb {
        [2.0 * fixed[0] - 1.0, 1.0 - 2.0 * fixed[1]]
    } else {
        [radians.cos() * fixed[1], radians.sin() * fixed[1]]
    };
    let x = left + radius + marker[0].clamp(-1.0, 1.0) * radius;
    let y = top + radius + marker[1].clamp(-1.0, 1.0) * radius;
    ctx.arc(x, y, 6.0, 0.0, std::f64::consts::TAU);
    ctx.set_source_rgb(1.0, 1.0, 1.0);
    ctx.set_line_width(3.0);
    let _ = ctx.stroke_preserve();
    ctx.set_source_rgb(0.0, 0.0, 0.0);
    ctx.set_line_width(1.0);
    let _ = ctx.stroke();
}

pub(super) fn canvas_draw(canvas: &gtk::DrawingArea, state: Rc<RefCell<State>>) {
    canvas.set_draw_func(move |_, ctx, w, h| {
        ctx.set_source_rgb(0.07, 0.07, 0.08);
        let _ = ctx.paint();
        let s = state.borrow();
        let src = s.source_pixbuf.as_ref();
        let out = s.result_pixbuf.as_ref();
        match s.mode {
            CompareMode::Result => draw(ctx, out.or(src), w, h),
            CompareMode::Source => draw(ctx, src, w, h),
            CompareMode::Split => {
                draw(ctx, out.or(src), w, h);
                let x = w as f64 * s.divider;
                let _ = ctx.save();
                ctx.rectangle(x, 0.0, w as f64 - x, h as f64);
                ctx.clip();
                draw(ctx, src, w, h);
                let _ = ctx.restore();
                ctx.set_source_rgba(1.0, 1.0, 1.0, 0.9);
                ctx.set_line_width(2.0);
                ctx.move_to(x, 0.0);
                ctx.line_to(x, h as f64);
                let _ = ctx.stroke();
                ctx.arc(x, h as f64 / 2.0, 9.0, 0.0, std::f64::consts::TAU);
                ctx.set_source_rgba(0.12, 0.12, 0.14, 0.92);
                let _ = ctx.fill_preserve();
                ctx.set_source_rgba(1.0, 1.0, 1.0, 0.95);
                ctx.set_line_width(2.0);
                let _ = ctx.stroke();
            }
        }
        if let (Some(pixbuf), Some(document)) = (src, s.session.document()) {
            let scale = (w as f64 / pixbuf.width() as f64).min(h as f64 / pixbuf.height() as f64);
            let left = (w as f64 - pixbuf.width() as f64 * scale) / 2.0;
            let top = (h as f64 - pixbuf.height() as f64 * scale) / 2.0;
            for (site_index, site) in document.recipe.voronoi.sites.iter().enumerate() {
                let Some(position) = site.position else {
                    continue;
                };
                let x = left + position[0] * pixbuf.width() as f64 * scale;
                let y = top + position[1] * pixbuf.height() as f64 * scale;
                let selected = s.session.selection().selected == Some(site.id);
                if selected {
                    ctx.new_path();
                    ctx.arc(x, y, 12.0, 0.0, std::f64::consts::TAU);
                    ctx.set_source_rgba(0.20, 0.55, 1.0, 0.98);
                    ctx.set_line_width(4.0);
                    let _ = ctx.stroke();
                }
                ctx.new_path();
                ctx.arc(x, y, 8.0, 0.0, std::f64::consts::TAU);
                ctx.set_source_rgba(0.05, 0.05, 0.05, 0.9);
                ctx.set_line_width(4.0);
                let _ = ctx.stroke_preserve();
                let encoded = site
                    .target_color
                    .map(chromiator::processing::linear_to_srgb);
                ctx.set_source_rgb(encoded[0] as f64, encoded[1] as f64, encoded[2] as f64);
                let _ = ctx.fill_preserve();
                ctx.set_source_rgba(1.0, 1.0, 1.0, 0.98);
                ctx.set_line_width(2.0);
                let _ = ctx.stroke();
                ctx.set_font_size(10.0);
                ctx.set_source_rgba(0.0, 0.0, 0.0, 0.95);
                ctx.move_to(x - 3.0, y + 3.5);
                let _ = ctx.show_text(&(site_index + 1).to_string());
                ctx.new_path();
                if site.locked {
                    ctx.set_source_rgba(1.0, 1.0, 1.0, 0.95);
                    ctx.move_to(x + 9.0, y - 8.0);
                    let _ = ctx.show_text("L");
                    ctx.new_path();
                }
            }
        }
    });
}

fn draw(ctx: &gtk::cairo::Context, pix: Option<&Pixbuf>, w: i32, h: i32) {
    let Some(p) = pix else { return };
    let scale = (w as f64 / p.width() as f64).min(h as f64 / p.height() as f64);
    let _ = ctx.save();
    ctx.translate(
        (w as f64 - p.width() as f64 * scale) / 2.0,
        (h as f64 - p.height() as f64 * scale) / 2.0,
    );
    ctx.scale(scale, scale);
    ctx.set_source_pixbuf(p, 0.0, 0.0);
    let _ = ctx.paint();
    let _ = ctx.restore();
}

pub(super) fn pixbuf(display: DisplayBuffer) -> Pixbuf {
    Pixbuf::from_bytes(
        &glib::Bytes::from_owned(display.bytes),
        gdk_pixbuf::Colorspace::Rgb,
        true,
        8,
        display.width as i32,
        display.height as i32,
        display.width as i32 * 4,
    )
}

pub(super) fn schedule_cli_screenshot(ui: &Rc<Ui>, state: &Rc<RefCell<State>>) {
    let Some(path) = ui.cli.screenshot.clone() else {
        return;
    };
    let ui = ui.clone();
    let state = state.clone();
    let _ = std::fs::remove_file(&path);
    ui.window.queue_draw();
    glib::spawn_future_local(async move {
        glib::timeout_future(std::time::Duration::from_millis(750)).await;
        let capture_widget: gtk::Widget = if state.borrow().picker_visible {
            ui.picker_dialog.borrow().as_ref().map_or_else(
                || ui.window.clone().upcast(),
                |dialog| dialog.clone().upcast(),
            )
        } else {
            ui.snapshot_root.clone()
        };
        let paintable = gtk::WidgetPaintable::new(Some(&capture_widget));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(
            &snapshot,
            capture_widget.width() as f64,
            capture_widget.height() as f64,
        );
        let saved = snapshot
            .to_node()
            .and_then(|node| {
                ui.window
                    .renderer()
                    .map(|renderer| renderer.render_texture(&node, None))
            })
            .is_some_and(|texture| texture.save_to_png(&path).is_ok());
        let current = state.borrow();
        let metadata = serde_json::json!({
            "app_version": env!("CARGO_PKG_VERSION"),
            "capture_success": saved,
            "screen": if current.session.document().is_some() { "document" } else { "welcome" },
            "requested_window_size": ui.cli.window_size.map(|(width, height)| serde_json::json!({"width": width, "height": height})),
            "actual_window_size": {"width": capture_widget.width(), "height": capture_widget.height()},
            "view": match current.mode { CompareMode::Result => "result", CompareMode::Split => "split", CompareMode::Source => "source" },
            "preprocessing": current.session.document().map(|document| &document.recipe.preprocessing),
            "voronoi": current.session.document().map(|document| &document.recipe.voronoi),
            "ordered_steps": current.session.document().map(|document| &document.recipe.steps),
            "picker_visible": current.picker_visible,
        });
        drop(current);
        let sidecar = PathBuf::from(format!("{}.json", path.display()));
        let _ = std::fs::write(sidecar, serde_json::to_vec_pretty(&metadata).unwrap());
        if !saved {
            ui.status.set_label("App-owned screenshot failed");
            eprintln!("app-owned screenshot capture failed");
        }
        if ui.cli.quit_after_screenshot {
            ui.window.application().expect("application").quit();
        }
    });
}
