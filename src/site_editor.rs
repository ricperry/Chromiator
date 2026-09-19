//! Voronoi site inspector and canvas interaction presentation.
//!
//! Every persistent edit is submitted to `DocumentSession`; this module owns only GTK row
//! construction, selection presentation, and pointer or keyboard gesture translation.

use super::*;

pub(super) fn group_selection(ui: &Rc<Ui>, state: Rc<RefCell<State>>) {
    // Keep creation available independently of row selection, including an empty list.
    let add = gtk::Button::with_label("+");
    add.set_halign(gtk::Align::End);
    add.set_tooltip_text(Some("Add a detached Source color"));
    add.update_property(&[
        gtk::accessible::Property::Label("Add Source color"),
        gtk::accessible::Property::Description("Open the color picker to create a detached site. Cancel adds nothing."),
    ]);
    if let Some(list) = ui.voronoi_panel.child() {
        ui.voronoi_panel.set_child(None::<&gtk::Widget>);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
        content.append(&add);
        content.append(&list);
        ui.voronoi_panel.set_child(Some(&content));
    }
    {
        let weak = Rc::downgrade(ui);
        let state = state.clone();
        add.connect_clicked(move |_| {
            let Some(ui) = weak.upgrade() else { return; };
            let can_edit = {
                let current = state.borrow();
                current.session.document().is_some() && !current.jobs.is_busy()
            };
            if can_edit {
                present_color_picker(&ui, &state, ui.cli.color_model, PickerPurpose::NewSite);
            }
        });
    }
    let ui = ui.clone();
    ui.groups.clone().connect_row_selected(move |_, row| {
        let Some(row) = row else { return };
        let Ok(id) = row.widget_name().parse::<u64>() else {
            return;
        };
        let mut current = state.borrow_mut();
        if current.session.selection().selected == Some(id) {
            return;
        }
        current.session.select(Some(id), true);
        drop(current);
        ui.canvas.queue_draw();
        let ui = ui.clone();
        let state = state.clone();
        glib::idle_add_local_once(move || refresh_voronoi_ui(&ui, &state));
    });
}

fn schedule_voronoi(ui: &Ui, state: &Rc<RefCell<State>>, message: &str) {
    let state = state.borrow_mut();
    if state.jobs.is_busy() {
        ui.status
            .set_label("Finish or cancel the file operation before editing");
        return;
    }
    let Some(document) = state.session.document() else {
        return;
    };
    let dirty = document.dirty;
    let recipe = document.recipe.clone();
    if let Some(source) = state.preview_source.clone() {
        state.scheduler.schedule(source, recipe);
    }
    drop(state);
    ui.save.set_sensitive(dirty);
    ui.status.set_label(message);
    ui.canvas.queue_draw();
}

/// Sample the displayed image's color data, never the decorative canvas overlays.
/// Split draws Result on the left and Source on the right. The last published
/// floating-point Result is authoritative for what is currently visible.
fn visible_canvas_sample(
    state: &State,
    position: [f64; 2],
    canvas_x: f64,
    canvas_width: i32,
) -> Option<([f32; 4], Option<[f64; 2]>)> {
    let source_visible = state.mode == CompareMode::Source
        || (state.mode == CompareMode::Split
            && canvas_x >= f64::from(canvas_width) * state.divider)
        || state.result_pixbuf.is_none();
    let image = if source_visible {
        &state.session.document()?.source
    } else {
        state.result.as_ref()?
    };
    let color = chromiator::voronoi::sample_color(image, position, SampleSize::Point)?;
    Some((color, source_visible.then_some(position)))
}

fn add_site_from_artwork(
    ui: &Rc<Ui>, state: &Rc<RefCell<State>>, position: [f64; 2], canvas_x: f64,
) -> bool {
    let sample = visible_canvas_sample(&state.borrow(), position, canvas_x, ui.canvas.width());
    let Some((color, position)) = sample else { return false; };
    if submit_session_edit(
        ui,
        state,
        EditCommand::AddSiteColor { color, position },
        None,
        "Site added from artwork — updating preview…",
    ) {
        refresh_voronoi_ui(ui, state);
        ui.canvas.queue_draw();
        true
    } else {
        false
    }
}

fn site_color_button(color: [f32; 3], accessible_label: &str) -> gtk::Button {
    let swatch = gtk::DrawingArea::builder()
        .content_width(30)
        .content_height(20)
        .width_request(30)
        .height_request(20)
        .hexpand(false)
        .vexpand(false)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    swatch.set_draw_func(move |_, context, width, height| {
        let encoded = color.map(chromiator::processing::linear_to_srgb);
        context.set_source_rgb(encoded[0] as f64, encoded[1] as f64, encoded[2] as f64);
        context.rectangle(0.0, 0.0, width as f64, height as f64);
        let _ = context.fill();
    });
    let frame = gtk::Frame::builder().child(&swatch).build();
    frame.set_hexpand(false);
    frame.set_vexpand(false);
    frame.set_halign(gtk::Align::Center);
    frame.set_valign(gtk::Align::Center);
    let button = gtk::Button::builder()
        .child(&frame)
        .width_request(38)
        .height_request(30)
        .hexpand(false)
        .vexpand(false)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    button.add_css_class("flat");
    button.add_css_class("site-swatch");
    button.update_property(&[
        gtk::accessible::Property::Label(accessible_label),
        gtk::accessible::Property::Description("Open the color picker for this site"),
    ]);
    button
}

pub(super) fn refresh_voronoi_ui(ui: &Rc<Ui>, state: &Rc<RefCell<State>>) {
    ui.groups.unselect_all();
    while let Some(child) = ui.groups.first_child() {
        ui.groups.remove(&child);
    }
    *ui.audit_site_influence.borrow_mut() = None;
    *ui.audit_site_lock.borrow_mut() = None;
    *ui.audit_site_target.borrow_mut() = None;
    *ui.audit_site_source.borrow_mut() = None;
    *ui.audit_site_sample_size.borrow_mut() = None;
    *ui.audit_site_reattach.borrow_mut() = None;
    *ui.audit_site_expander.borrow_mut() = None;
    *ui.audit_other_site_expander.borrow_mut() = None;
    let Some((sites, coverage, selected_site, expanded_site)) = ({
        let state = state.borrow();
        state.session.document().map(|document| {
            (
                document.recipe.voronoi.sites.clone(),
                state.coverage.clone(),
                state.session.selection().selected,
                state.session.selection().expanded,
            )
        })
    }) else {
        return;
    };
    ui.voronoi_panel
        .set_label(Some(&format!("Color sites · {}", sites.len())));
    let visible = coverage.visible_total.max(1);
    for (index, site) in sites.iter().enumerate() {
        let site_id = site.id;
        let covered = coverage
            .site_counts
            .iter()
            .find(|(id, _)| *id == site.id)
            .map(|(_, count)| *count)
            .unwrap_or(0);
        let row = gtk::Expander::new(None);
        row.update_property(&[gtk::accessible::Property::Label(&format!("Site {} details", index + 1))]);
        row.set_expanded(expanded_site == Some(site_id));
        let title = gtk::Label::builder()
            .label(site_label(index))
            .xalign(0.0)
            .build();
        let subtitle = gtk::Label::builder()
            .label(format!(
                "{} · {:.1}%{}",
                if site.position.is_some() {
                    "Attached"
                } else {
                    "Detached"
                },
                covered as f64 * 100.0 / visible as f64,
                if site.locked { " · Locked" } else { "" }
            ))
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["dim-label", "caption"])
            .build();
        let row_labels = gtk::Box::new(gtk::Orientation::Vertical, 2);
        row_labels.set_hexpand(true);
        row_labels.append(&title);
        row_labels.append(&subtitle);
        subtitle.set_tooltip_text(Some("Winning-site coverage before blending, not the site's fractional contribution to blended colors."));
        let row_header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        row_header.append(&row_labels);
        row.set_label_widget(Some(&row_header));
        let list_row = gtk::ListBoxRow::new();
        list_row.update_property(&[gtk::accessible::Property::Label(&site_label(index))]);
        list_row.set_widget_name(&site.id.to_string());
        row.set_tooltip_text(Some(&format!(
            "{} details — Source, Target, coverage, and lock",
            accessible_site_label(index)
        )));
        let lock = gtk::ToggleButton::builder()
            .icon_name(if site.locked {
                "changes-prevent-symbolic"
            } else {
                "changes-allow-symbolic"
            })
            .active(site.locked)
            .tooltip_text(if site.locked {
                "Unlock this site"
            } else {
                "Lock this site"
            })
            .build();
        lock.add_css_class("flat");
        lock.set_valign(gtk::Align::Center);
        lock.set_vexpand(false);
        let lock_label = format!(
            "{} Site {}",
            if site.locked { "Unlock" } else { "Lock" },
            index + 1
        );
        lock.update_property(&[gtk::accessible::Property::Label(&lock_label)]);
        let source = site_color_button(
            [
                site.source_color[0],
                site.source_color[1],
                site.source_color[2],
            ],
            &format!("Edit Site {} Source color", index + 1),
        );
        source.set_sensitive(!site.locked);
        let target = site_color_button(
            site.target_color,
            &format!("Edit Site {} Target color", index + 1),
        );
        target.set_sensitive(!site.locked);
        source.set_tooltip_text(Some("Source color — click to edit"));
        target.set_tooltip_text(Some("Target color — click to edit"));
        let arrow_glyph = gtk::Label::builder()
            .label("→")
            .accessible_role(gtk::AccessibleRole::Presentation)
            .build();
        let arrow = gtk::Button::builder().child(&arrow_glyph).build();
        arrow.set_valign(gtk::Align::Center);
        let source_label = gtk::Label::builder()
            .label("Source")
            .css_classes(["caption", "dim-label"])
            .build();
        let target_label = gtk::Label::builder()
            .label("Target")
            .css_classes(["caption", "dim-label"])
            .build();
        let source_column = gtk::Box::new(gtk::Orientation::Vertical, 2);
        source_column.set_valign(gtk::Align::Center);
        source_column.set_vexpand(false);
        source_column.append(&source_label);
        source_column.append(&source);
        let target_column = gtk::Box::new(gtk::Orientation::Vertical, 2);
        target_column.set_valign(gtk::Align::Center);
        target_column.set_vexpand(false);
        target_column.append(&target_label);
        target_column.append(&target);
        let color_flow = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        color_flow.set_valign(gtk::Align::Center);
        color_flow.set_halign(gtk::Align::End);
        color_flow.set_hexpand(false);
        color_flow.set_vexpand(false);
        color_flow.append(&source_column);
        color_flow.append(&arrow);
        color_flow.append(&target_column);
        row_header.append(&color_flow);
        row_header.append(&lock);

        let influence = gtk::SpinButton::with_range(-4.0, 4.0, 0.1);
        influence.set_value(site.influence);
        influence.set_digits(1);
        influence.set_width_chars(4);
        influence.set_sensitive(!site.locked);
        influence.update_property(&[gtk::accessible::Property::Label(&format!(
            "Site {} Influence",
            index + 1
        ))]);
        influence.set_tooltip_text(Some("Adjust this site's competitive reach from -4 to 4"));
        let reattach = gtk::Button::with_label(if site.position.is_some() {
            "Resample…"
        } else {
            "Attach…"
        });
        reattach.set_sensitive(!site.locked);
        reattach.set_tooltip_text(Some(
            "Choose a source position; update Source while preserving Target",
        ));
        reattach.update_property(&[gtk::accessible::Property::Label(&format!(
            "{} source position for Site {}",
            if site.position.is_some() {
                "Resample"
            } else {
                "Attach"
            },
            index + 1
        ))]);
        let sizes = gtk::StringList::new(&["Point", "3×3", "5×5"]);
        let sample_size = gtk::DropDown::new(Some(sizes), None::<gtk::Expression>);
        sample_size.set_selected(match site.size {
            SampleSize::Point => 0,
            SampleSize::ThreeByThree => 1,
            SampleSize::FiveByFive => 2,
        });
        sample_size.set_sensitive(!site.locked && site.position.is_some());
        sample_size.set_tooltip_text(Some(
            "Sample authoritative full-resolution pixels around the source position",
        ));
        sample_size.update_property(&[gtk::accessible::Property::Label(&format!(
            "Sampling footprint for Site {}",
            index + 1
        ))]);
        let match_target = arrow.clone();
        match_target.set_sensitive(
            !site.locked && site.target_color != [site.source_color[0], site.source_color[1], site.source_color[2]],
        );
        match_target.set_tooltip_text(Some("Match target to source"));
        match_target.update_property(&[gtk::accessible::Property::Label(&format!(
            "Match target to source for Site {}", index + 1
        ))]);
        let delete = icon_button("user-trash-symbolic", "Delete Site…");
        delete.add_css_class("flat");
        delete.add_css_class("destructive-action");
        delete.set_halign(gtk::Align::End);
        delete.set_sensitive(!site.locked);
        delete.update_property(&[gtk::accessible::Property::Label(&format!(
            "Delete Site {}",
            index + 1
        ))]);

        let details = gtk::Grid::builder()
            .row_spacing(6)
            .column_spacing(8)
            .margin_top(6)
            .margin_bottom(8)
            .margin_start(12)
            .margin_end(12)
            .build();
        for (row_index, (label_text, control)) in [
            ("_Influence", influence.upcast_ref::<gtk::Widget>()),
            ("Source _position", reattach.upcast_ref::<gtk::Widget>()),
            ("_Footprint", sample_size.upcast_ref::<gtk::Widget>()),
        ]
            .into_iter()
            .enumerate()
        {
            let label = gtk::Label::builder()
                .label(label_text)
                .use_underline(true)
                .xalign(0.0)
                .hexpand(true)
                .build();
            label.set_mnemonic_widget(Some(control));
            control.update_relation(&[gtk::accessible::Relation::LabelledBy(&[label.upcast_ref()])]);
            details.attach(&label, 0, row_index as i32, 1, 1);
        }
        details.attach(&influence, 1, 0, 1, 1);
        details.attach(&reattach, 1, 1, 1, 1);
        details.attach(&sample_size, 1, 2, 1, 1);
        details.attach(&delete, 1, 3, 1, 1);
        row.set_child(Some(&details));
        list_row.set_child(Some(&row));
        ui.groups.append(&list_row);
        if selected_site == Some(site.id) {
            ui.groups.select_row(Some(&list_row));
            *ui.audit_site_influence.borrow_mut() = Some(influence.clone());
            *ui.audit_site_lock.borrow_mut() = Some(lock.clone());
            *ui.audit_site_source.borrow_mut() = Some(source.clone());
            *ui.audit_site_target.borrow_mut() = Some(target.clone());
            *ui.audit_site_sample_size.borrow_mut() = Some(sample_size.clone());
            *ui.audit_site_reattach.borrow_mut() = Some(reattach.clone());
            *ui.audit_site_expander.borrow_mut() = Some(row.clone());
        } else if ui.audit_other_site_expander.borrow().is_none() {
            *ui.audit_other_site_expander.borrow_mut() = Some(row.clone());
        }

        {
            let ui = ui.clone();
            let state = state.clone();
            row.connect_expanded_notify(move |row| {
                let mut current = state.borrow_mut();
                if row.is_expanded() {
                    current.session.select(Some(site_id), false);
                    current.session.set_expanded(Some(site_id));
                } else if current.session.selection().expanded == Some(site_id) {
                    current.session.set_expanded(None);
                } else {
                    return;
                }
                drop(current);
                let ui = ui.clone();
                let state = state.clone();
                glib::idle_add_local_once(move || {
                    refresh_voronoi_ui(&ui, &state);
                    ui.canvas.queue_draw();
                });
            });
        }

        {
            let ui = ui.clone();
            let state = state.clone();
            match_target.connect_clicked(move |_| {
                let color = {
                    let current = state.borrow();
                    current.session.document()
                        .and_then(|document| document.recipe.voronoi.site(site_id))
                        .map(|site| [site.source_color[0], site.source_color[1], site.source_color[2]])
                };
                let Some(color) = color else { return; };
                if submit_session_edit(
                    &ui, &state, EditCommand::SetTarget { site_id, color }, None,
                    "Target matched to Source - updating preview...",
                ) {
                    refresh_voronoi_ui(&ui, &state);
                }
            });
        }
        for (button, purpose) in [
            (source, PickerPurpose::Source),
            (target, PickerPurpose::Target),
        ] {
            let ui = ui.clone();
            let state = state.clone();
            button.connect_clicked(move |_| {
                {
                    let mut current = state.borrow_mut();
                    current.session.select(Some(site_id), true);
                }
                ui.canvas.queue_draw();
                present_color_picker(&ui, &state, ui.cli.color_model, purpose);
            });
        }
        {
            let ui = ui.clone();
            let state = state.clone();
            lock.connect_toggled(move |control| {
                if submit_session_edit(
                    &ui,
                    &state,
                    EditCommand::SetLocked {
                        site_id,
                        locked: control.is_active(),
                    },
                    None,
                    if control.is_active() {
                        "Site locked — its parameters are protected"
                    } else {
                        "Site unlocked — editing is available"
                    },
                ) {
                    {
                        let mut current = state.borrow_mut();
                        current.session.select(Some(site_id), true);
                    }
                    refresh_voronoi_ui(&ui, &state);
                    ui.canvas.queue_draw();
                }
            });
        }
        {
            let ui = ui.clone();
            let state = state.clone();
            influence.connect_value_changed(move |control| {
                if submit_session_edit(
                    &ui,
                    &state,
                    EditCommand::SetInfluence {
                        site_id,
                        influence: control.value(),
                    },
                    Some(EditGesture::Influence(site_id)),
                    "Influence changed — updating Coverage…",
                ) {
                    {
                        let mut current = state.borrow_mut();
                        current.session.select(Some(site_id), true);
                    }
                }
            });
        }
        install_document_spin_boundaries(&influence, state, EditGesture::Influence(site_id));
        {
            let ui = ui.clone();
            let state = state.clone();
            reattach.connect_clicked(move |_| {
                let locked = state
                    .borrow()
                    .session
                    .document()
                    .and_then(|document| document.recipe.voronoi.site(site_id))
                    .is_some_and(|site| site.locked);
                if locked {
                    ui.status
                        .set_label("Unlock this site before changing its Source position");
                    return;
                }
                let mut current = state.borrow_mut();
                current.session.select(Some(site_id), true);
                current.sampling_previous_mode = Some(current.mode);
                current.sampling = Some(SamplingState::AddSample);
                drop(current);
                ui.source_mode.set_active(true);
                ui.status.set_label(
                    "Click a visible source color to set Source; Target is preserved — Escape cancels",
                );
                ui.canvas.grab_focus();
            });
        }
        {
            let ui = ui.clone();
            let state = state.clone();
            sample_size.connect_selected_notify(move |control| {
                let size = match control.selected() {
                    0 => SampleSize::Point,
                    2 => SampleSize::FiveByFive,
                    _ => SampleSize::ThreeByThree,
                };
                submit_session_edit(
                    &ui,
                    &state,
                    EditCommand::SetSampleSize { site_id, size },
                    None,
                    "Footprint changed; Source updated, Target preserved — updating preview…",
                );
            });
        }
        {
            let ui = ui.clone();
            let state = state.clone();
            delete.connect_clicked(move |_| {
                let dialog = native_dialog::AlertDialog::builder()
                    .heading("Delete this site?")
                    .body("The site can be restored with document Undo.")
                    .build();
                dialog.add_response("cancel", "Cancel");
                dialog.add_response("delete", "Delete");
                dialog.set_response_appearance(
                    "delete",
                    native_dialog::ResponseAppearance::Destructive,
                );
                let ui = ui.clone();
                let state = state.clone();
                glib::spawn_future_local(async move {
                    if dialog.choose_future(Some(&ui.window)).await != "delete" {
                        return;
                    }
                    if !submit_session_edit(
                        &ui,
                        &state,
                        EditCommand::DeleteSite(site_id),
                        None,
                        "Site deleted — updating preview…",
                    ) {
                        ui.status.set_label("Unlock this site before deleting it");
                    }
                    refresh_voronoi_ui(&ui, &state);
                });
            });
        }
    }
}

pub(super) fn canvas_sampling(ui: &Rc<Ui>, state: Rc<RefCell<State>>) {
    let click = gtk::GestureClick::new();
    let ui_click = ui.clone();
    let state_click = state.clone();
    click.connect_pressed(move |gesture, _, x, y| {
        if gesture.current_button() != 1 {
            return;
        }
        if state_click.borrow().mode == CompareMode::Split
            && split_divider_hit(x, ui_click.canvas.width(), state_click.borrow().divider)
        {
            ui_click
                .divider
                .set_value(divider_from_canvas_x(x, ui_click.canvas.width()));
            ui_click.status.set_label(&format!(
                "Split divider · {:.0}%",
                ui_click.divider.value() * 100.0
            ));
            return;
        }
        let Some(position) = canvas_position(&ui_click.canvas, &state_click.borrow(), x, y) else {
            return;
        };
        let sampling = state_click.borrow().sampling;
        if sampling.is_none() {
            let action = {
                let current = state_click.borrow();
                current
                    .session
                    .document()
                    .map_or(CanvasSiteAction::Ignore, |_| {
                        canvas_site_action(site_at_canvas_point(&ui_click.canvas, &current, x, y))
                    })
            };
            if let CanvasSiteAction::Select(site_id) = action {
                let mut current = state_click.borrow_mut();
                current.session.select(Some(site_id), true);
                drop(current);
                refresh_voronoi_ui(&ui_click, &state_click);
                ui_click.canvas.queue_draw();
                return;
            }
            if action != CanvasSiteAction::Add {
                return;
            }
            if !add_site_from_artwork(&ui_click, &state_click, position, x) {
                ui_click
                    .status
                    .set_label("No visible image color could be sampled there");
            }
            return;
        }
        let sampling = sampling.expect("sampling was checked above");
        let (command, previous_mode) = {
            let mut current = state_click.borrow_mut();
            let Some((color, attachment)) = visible_canvas_sample(
                &current, position, x, ui_click.canvas.width(),
            ) else {
                ui_click.status.set_label("No visible image color could be sampled there");
                return;
            };
            let command = match sampling {
                SamplingState::AddColor => EditCommand::AddSiteColor { color, position: attachment },
                SamplingState::AddSample => {
                    let Some(site_id) = current.session.selection().selected else {
                        return;
                    };
                    if attachment.is_some() {
                        EditCommand::Reattach { site_id, position }
                    } else {
                        EditCommand::SetSource { site_id, color, position: None }
                    }
                }
            };
            current.sampling = None;
            (command, current.sampling_previous_mode.take())
        };
        if !submit_session_edit(
            &ui_click,
            &state_click,
            command,
            None,
            match sampling {
                SamplingState::AddColor => "Site added — updating preview…",
                SamplingState::AddSample => "Source sampled from visible image; Target preserved — updating preview…",
            },
        ) {
            ui_click
                .status
                .set_label("That area is fully transparent; choose a visible source color");
        }
        restore_view(&ui_click, previous_mode);
        refresh_voronoi_ui(&ui_click, &state_click);
        ui_click.canvas.queue_draw();
    });
    ui.canvas.add_controller(click);

    let drag_origin = Rc::new(RefCell::new(None::<[f64; 2]>));
    let drag_changed = Rc::new(Cell::new(false));
    let split_drag_origin = Rc::new(Cell::new(None::<f64>));
    let drag = gtk::GestureDrag::new();
    {
        let origin = drag_origin.clone();
        let changed = drag_changed.clone();
        let split_origin = split_drag_origin.clone();
        let ui = ui.clone();
        let state = state.clone();
        drag.connect_drag_begin(move |_, x, y| {
            if state.borrow().mode == CompareMode::Split
                && split_divider_hit(x, ui.canvas.width(), state.borrow().divider)
            {
                split_origin.set(Some(x));
                ui.divider
                    .set_value(divider_from_canvas_x(x, ui.canvas.width()));
                return;
            }
            if canvas_position(&ui.canvas, &state.borrow(), x, y).is_none() {
                return;
            }
            let nearest = {
                let current = state.borrow();
                site_at_canvas_point(&ui.canvas, &current, x, y).and_then(|site_id| {
                    current
                        .session
                        .document()?
                        .recipe
                        .voronoi
                        .site(site_id)
                        .and_then(|site| {
                            site.position
                                .map(|position| (site_id, position, site.locked))
                        })
                })
            };
            if let Some((site_id, position, locked)) = nearest {
                let mut state = state.borrow_mut();
                state.session.select(Some(site_id), true);
                if locked {
                    ui.status
                        .set_label("Unlock this site before moving its Source marker");
                } else {
                    *origin.borrow_mut() = Some(position);
                    changed.set(false);
                }
            }
        });
    }
    {
        let origin = drag_origin.clone();
        let split_origin = split_drag_origin.clone();
        let changed = drag_changed.clone();
        let ui = ui.clone();
        let state = state.clone();
        drag.connect_drag_update(move |_, dx, dy| {
            if let Some(start_x) = split_origin.get() {
                ui.divider
                    .set_value(divider_from_canvas_x(start_x + dx, ui.canvas.width()));
                ui.status.set_label(&format!(
                    "Split divider · {:.0}%",
                    ui.divider.value() * 100.0
                ));
                return;
            }
            let Some(start) = *origin.borrow() else {
                return;
            };
            let Some(pixbuf) = state.borrow().source_pixbuf.clone() else {
                return;
            };
            let scale = (ui.canvas.width() as f64 / pixbuf.width() as f64)
                .min(ui.canvas.height() as f64 / pixbuf.height() as f64);
            let position = [
                (start[0] + dx / (pixbuf.width() as f64 * scale)).clamp(0.0, 1.0),
                (start[1] + dy / (pixbuf.height() as f64 * scale)).clamp(0.0, 1.0),
            ];
            if resample_selected(&state, position, true) {
                changed.set(true);
                ui.canvas.queue_draw();
            }
        });
    }
    {
        let origin = drag_origin;
        let changed = drag_changed;
        let split_origin = split_drag_origin;
        let ui = ui.clone();
        let state = state.clone();
        drag.connect_drag_end(move |_, _, _| {
            if split_origin.take().is_some() {
                ui.status.set_label(&format!(
                    "Split divider set to {:.0}%",
                    ui.divider.value() * 100.0
                ));
                return;
            }
            if origin.borrow_mut().take().is_some() && changed.replace(false) {
                finish_site_drag(&state);
                sync_document_history_ui(&ui, &state);
                schedule_voronoi(&ui, &state, "Site moved; Target preserved — updating preview…");
                refresh_voronoi_ui(&ui, &state);
            }
        });
    }
    ui.canvas.add_controller(drag);

    let motion = gtk::EventControllerMotion::new();
    {
        let ui = ui.clone();
        let state = state.clone();
        motion.connect_motion(move |_, x, _| {
            let current = state.borrow();
            ui.canvas.set_cursor_from_name(
                (current.mode == CompareMode::Split
                    && split_divider_hit(x, ui.canvas.width(), current.divider))
                .then_some("col-resize"),
            );
        });
    }
    {
        let canvas = ui.canvas.clone();
        motion.connect_leave(move |_| canvas.set_cursor_from_name(None));
    }
    ui.canvas.add_controller(motion);

    let keys = gtk::EventControllerKey::new();
    let ui_key = ui.clone();
    let state_key = state;
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        if key == gtk::gdk::Key::Escape && state_key.borrow().sampling.is_some() {
            let previous = {
                let mut state = state_key.borrow_mut();
                state.sampling = None;
                state.sampling_previous_mode.take()
            };
            restore_view(&ui_key, previous);
            ui_key.status.set_label("Sampling cancelled");
            return glib::Propagation::Stop;
        }
        if state_key.borrow().mode == CompareMode::Split {
            let step = if modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK) {
                0.1
            } else {
                0.01
            };
            let value = match key {
                gtk::gdk::Key::Left => Some(ui_key.divider.value() - step),
                gtk::gdk::Key::Right => Some(ui_key.divider.value() + step),
                gtk::gdk::Key::Home => Some(0.0),
                gtk::gdk::Key::End => Some(1.0),
                _ => None,
            };
            if let Some(value) = value {
                ui_key.divider.set_value(value.clamp(0.0, 1.0));
                ui_key.status.set_label(&format!(
                    "Split divider · {:.0}%",
                    ui_key.divider.value() * 100.0
                ));
                return glib::Propagation::Stop;
            }
        }
        let delta = if modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK) {
            10.0
        } else {
            1.0
        };
        let (dx, dy) = match key {
            gtk::gdk::Key::Left => (-delta, 0.0),
            gtk::gdk::Key::Right => (delta, 0.0),
            gtk::gdk::Key::Up => (0.0, -delta),
            gtk::gdk::Key::Down => (0.0, delta),
            _ => return glib::Propagation::Proceed,
        };
        let locked = {
            let state = state_key.borrow();
            state
                .session
                .document()
                .and_then(|document| {
                    state
                        .session
                        .selection()
                        .selected
                        .and_then(|id| document.recipe.voronoi.site(id))
                })
                .is_some_and(|site| site.locked)
        };
        if locked {
            ui_key
                .status
                .set_label("Unlock this site before nudging its Source marker");
            return glib::Propagation::Stop;
        }
        let current = {
            let state = state_key.borrow();
            state.session.document().and_then(|document| {
                document
                    .recipe
                    .voronoi
                    .sites
                    .iter()
                    .find(|site| {
                        Some(site.id) == state.session.selection().selected && !site.locked
                    })
                    .and_then(|site| site.position)
            })
        };
        if let Some(position) = current {
            let dimensions = state_key
                .borrow()
                .session
                .document()
                .map(|document| (document.source.width, document.source.height))
                .unwrap_or((1, 1));
            let moved = [
                (position[0] + dx / dimensions.0.max(1) as f64).clamp(0.0, 1.0),
                (position[1] + dy / dimensions.1.max(1) as f64).clamp(0.0, 1.0),
            ];
            if resample_selected(&state_key, moved, false) {
                schedule_voronoi(
                    &ui_key,
                    &state_key,
                    "Site nudged; Target preserved — updating preview…",
                );
                sync_document_history_ui(&ui_key, &state_key);
                refresh_voronoi_ui(&ui_key, &state_key);
            }
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    ui.canvas.add_controller(keys);
}

fn restore_view(ui: &Ui, mode: Option<CompareMode>) {
    match mode {
        Some(CompareMode::Split) => ui.split_mode.set_active(true),
        Some(CompareMode::Source) => ui.source_mode.set_active(true),
        _ => ui.result_mode.set_active(true),
    }
}

/// Ends drag coalescing under one borrow, before any GTK refresh can re-enter state.
fn finish_site_drag(state: &RefCell<State>) {
    let mut current = state.borrow_mut();
    if let Some(site_id) = current.session.selection().selected {
        current
            .session
            .finish_gesture(EditGesture::Position(site_id));
    }
}

fn resample_selected(state: &Rc<RefCell<State>>, position: [f64; 2], coalesced: bool) -> bool {
    let mut state = state.borrow_mut();
    let sample_id = state.session.selection().selected;
    let Some(site_id) = sample_id else {
        return false;
    };
    state
        .session
        .edit(
            EditCommand::Reattach { site_id, position },
            coalesced.then_some(EditGesture::Position(site_id)),
        )
        .is_ok_and(|change| change.changed)
}

fn canvas_position(canvas: &gtk::DrawingArea, state: &State, x: f64, y: f64) -> Option<[f64; 2]> {
    let pixbuf = state.source_pixbuf.as_ref()?;
    let w = canvas.width() as f64;
    let h = canvas.height() as f64;
    let scale = (w / pixbuf.width() as f64).min(h / pixbuf.height() as f64);
    let draw_w = pixbuf.width() as f64 * scale;
    let draw_h = pixbuf.height() as f64 * scale;
    let left = (w - draw_w) / 2.0;
    let top = (h - draw_h) / 2.0;
    if x < left || y < top || x > left + draw_w || y > top + draw_h {
        return None;
    }
    Some([(x - left) / draw_w, (y - top) / draw_h])
}

fn site_at_canvas_point(canvas: &gtk::DrawingArea, state: &State, x: f64, y: f64) -> Option<u64> {
    let pixbuf = state.source_pixbuf.as_ref()?;
    let document = state.session.document()?;
    let width = canvas.width() as f64;
    let height = canvas.height() as f64;
    let scale = (width / pixbuf.width() as f64).min(height / pixbuf.height() as f64);
    let draw_width = pixbuf.width() as f64 * scale;
    let draw_height = pixbuf.height() as f64 * scale;
    let left = (width - draw_width) / 2.0;
    let top = (height - draw_height) / 2.0;
    document
        .recipe
        .voronoi
        .sites
        .iter()
        .filter_map(|site| {
            let position = site.position?;
            let marker_x = left + position[0] * draw_width;
            let marker_y = top + position[1] * draw_height;
            let dx = marker_x - x;
            let dy = marker_y - y;
            let distance2 = dx.powi(2) + dy.powi(2);
            marker_hit_test(dx, dy).then_some((distance2, site.id))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, id)| id)
}

pub(super) fn sync_string_list(model: &gtk::StringList, labels: &[String]) {
    let unchanged = model.n_items() as usize == labels.len()
        && labels.iter().enumerate().all(|(index, label)| {
            model
                .string(index as u32)
                .is_some_and(|current| current.as_str() == label)
        });
    if unchanged {
        return;
    }
    let additions: Vec<_> = labels.iter().map(String::as_str).collect();
    model.splice(0, model.n_items(), &additions);
}

pub(super) fn selected_output_color(state: &Rc<RefCell<State>>) -> Option<[f32; 3]> {
    let state = state.borrow();
    let site_id = state.session.selection().selected?;
    state
        .session
        .document()?
        .recipe
        .voronoi
        .sites
        .iter()
        .find(|site| site.id == site_id)
        .map(|site| site.target_color)
}

pub(super) fn selected_source_color(state: &Rc<RefCell<State>>) -> Option<[f32; 3]> {
    let state = state.borrow();
    let site_id = state.session.selection().selected?;
    state
        .session
        .document()?
        .recipe
        .voronoi
        .site(site_id)
        .map(|site| {
            [
                site.source_color[0],
                site.source_color[1],
                site.source_color[2],
            ]
        })
}

#[cfg(test)]
mod drag_tests {
    use super::*;

    /// Exercises the actual drag-completion helper without requiring a display server.
    #[test]
    fn finishing_drag_releases_state_and_separates_undo_transactions() {
        let state = Rc::new(RefCell::new(State::new()));
        state
            .borrow_mut()
            .session
            .load(example::spectrum_document().unwrap())
            .unwrap();
        state
            .borrow_mut()
            .session
            .edit(EditCommand::AddSiteAt([0.5, 0.5]), None)
            .unwrap();
        let before = state.borrow().session.document().unwrap().recipe.clone();
        let depth = state.borrow().session.undo_len();

        finish_site_drag(&state);
        assert_eq!(
            state.borrow().session.undo_len(),
            depth,
            "a click must not add history"
        );
        assert!(resample_selected(&state, [0.55, 0.5], true));
        assert!(resample_selected(&state, [0.6, 0.5], true));
        finish_site_drag(&state);
        assert!(state.try_borrow_mut().is_ok());
        assert_eq!(state.borrow().session.undo_len(), depth + 1);
        let first_drag = state.borrow().session.document().unwrap().recipe.clone();

        assert!(resample_selected(&state, [0.65, 0.5], true));
        finish_site_drag(&state);
        assert_eq!(state.borrow().session.undo_len(), depth + 2);
        assert_eq!(
            state.borrow_mut().session.undo().unwrap().recipe,
            first_drag
        );
        assert_eq!(state.borrow_mut().session.undo().unwrap().recipe, before);
        assert_eq!(
            state.borrow_mut().session.redo().unwrap().recipe,
            first_drag
        );

        let site_id = state.borrow().session.selection().selected.unwrap();
        state
            .borrow_mut()
            .session
            .edit(
                EditCommand::SetLocked {
                    site_id,
                    locked: true,
                },
                None,
            )
            .unwrap();
        let locked_depth = state.borrow().session.undo_len();
        assert!(!resample_selected(&state, [0.7, 0.5], true));
        finish_site_drag(&state);
        assert_eq!(state.borrow().session.undo_len(), locked_depth);
    }
}
