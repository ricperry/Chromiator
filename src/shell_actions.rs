//! GTK shell actions and asynchronous file-job presentation.
//!
//! Background work communicates through `app_job::Work`; widgets only start jobs and render
//! admitted results on the main loop.

use super::*;

pub(super) fn mode_handler(
    button: &gtk::ToggleButton,
    mode: CompareMode,
    canvas: &gtk::DrawingArea,
    state: Rc<RefCell<State>>,
) {
    let c = canvas.clone();
    button.connect_toggled(move |b| {
        if b.is_active() {
            state.borrow_mut().mode = mode;
            c.set_cursor_from_name(None);
            c.set_tooltip_text(Some(if mode == CompareMode::Split {
                "Result is on the left; Source is on the right. Click the visible image color to add a site. Drag the divider or use Left/Right to move the split."
            } else {
                "Click an image color to add a site: Source samples attach to the image; Result samples are detached."
            }));
            c.queue_draw();
        }
    });
}

pub(super) fn replacement_handler(
    button: &gtk::Button,
    replacement: Replacement,
    ui: &Rc<Ui>,
    state: Rc<RefCell<State>>,
) {
    let ui = ui.clone();
    button.connect_clicked(move |_| request_replacement(replacement, &ui, &state));
}

fn request_replacement(replacement: Replacement, ui: &Rc<Ui>, state: &Rc<RefCell<State>>) {
    let dirty = state
        .borrow()
        .session
        .document()
        .is_some_and(|document| document.dirty);
    if !dirty {
        execute_replacement(replacement, ui, state);
        return;
    }
    let dialog = native_dialog::AlertDialog::builder()
        .heading("Save changes before replacing this document?")
        .body("Save continues only after the project is written successfully. Discard replaces without saving; Cancel keeps the current document.")
        .build();
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("save", "Save");
    dialog.add_response("discard", "Discard");
    dialog.set_response_appearance("discard", native_dialog::ResponseAppearance::Destructive);
    let ui = ui.clone();
    let state = state.clone();
    glib::spawn_future_local(async move {
        match dialog.choose_future(Some(&ui.window)).await.as_str() {
            "save" => {
                state.borrow_mut().pending_replacement = Some(replacement);
                ui.save.emit_clicked();
            }
            "discard" => execute_replacement(replacement, &ui, &state),
            _ => {}
        }
    });
}

fn execute_replacement(replacement: Replacement, ui: &Rc<Ui>, state: &Rc<RefCell<State>>) {
    match replacement {
        Replacement::OpenImage => present_open_dialog(ui, state, OpenKind::Image),
        Replacement::OpenProject => present_open_dialog(ui, state, OpenKind::Project),
        Replacement::Example => start_example_open(ui, state),
    }
}

fn open_filter(kind: OpenKind) -> gtk::FileFilter {
    let filter = gtk::FileFilter::new();
    match kind {
        OpenKind::Image => {
            filter.set_name(Some("Supported raster images"));
            for mime in [
                "image/png",
                "image/jpeg",
                "image/tiff",
                "image/webp",
                "image/bmp",
                "image/gif",
            ] {
                filter.add_mime_type(mime);
            }
        }
        OpenKind::Project => {
            filter.set_name(Some("Chromiator projects"));
            filter.add_pattern("*.[Tt][Hh][Rr][Ee][Ss][Hh][Ii][Aa][Tt][Oo][Rr]");
        }
    }
    filter
}

fn present_open_dialog(ui: &Rc<Ui>, state: &Rc<RefCell<State>>, kind: OpenKind) {
    let filter = open_filter(kind);
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let dialog = gtk::FileDialog::builder()
        .title(match kind {
            OpenKind::Image => "Open Image",
            OpenKind::Project => "Open Chromiator Project",
        })
        .filters(&filters)
        .default_filter(&filter)
        .build();
    let win = ui.window.clone();
    let state = state.clone();
    let ui = ui.clone();
    for button in [
        &ui.open,
        &ui.empty_open,
        &ui.empty_project,
        &ui.empty_example,
    ] {
        button.set_sensitive(false);
    }
    glib::spawn_future_local(async move {
        if let Some(path) = dialog
            .open_future(Some(&win))
            .await
            .ok()
            .and_then(|file| file.path())
        {
            start_path_open(&ui, &state, path, kind);
        } else {
            finish_job_controls(&ui, &state);
        }
    });
}

fn start_path_open(ui: &Rc<Ui>, state: &Rc<RefCell<State>>, path: PathBuf, kind: OpenKind) {
    let document_kind = match kind {
        OpenKind::Image => DocumentKind::Image,
        OpenKind::Project => DocumentKind::Project,
    };
    let label = match kind {
        OpenKind::Image => "Reading image file…",
        OpenKind::Project => "Reading project archive…",
    };
    let Some((sender, token)) = begin_job(ui, state, label) else {
        return;
    };
    thread::spawn(move || {
        let result = match kind {
            OpenKind::Project => project::open(&path)
                .map(|document| (document, Some(path.clone()), DocumentKind::Project)),
            OpenKind::Image => open_raster_job(&path, &token, &sender).map(|(bytes, decoded)| {
                let mut document = Document {
                    source_name: path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    source_bytes: bytes,
                    source: decoded.pixels,
                    interpretation: decoded.interpretation,
                    recipe: Recipe::default(),
                    export_defaults: ExportDefaults::default(),
                    dirty: false,
                };
                initialize_voronoi(&mut document);
                (document, None, DocumentKind::Image)
            }),
        }
        .and_then(|(document, project_path, document_kind)| {
            prepare_document(document, project_path, document_kind, &token, &sender)
        });
        let _ = sender.send(Work::Open(
            token.generation(),
            document_kind,
            Box::new(result.map_err(|error| format!("{error:#}"))),
        ));
    });
}

/// Startup-only recent-project action; reuse the existing cancellable open job.
pub(super) fn open_recent_project(ui: &Rc<Ui>, state: &Rc<RefCell<State>>, path: PathBuf) {
    if !ui.empty_project.is_sensitive() || state.borrow().session.document().is_some() {
        return;
    }
    start_path_open(ui, state, path, OpenKind::Project);
}

pub(super) fn start_example_open(ui: &Rc<Ui>, state: &Rc<RefCell<State>>) {
    let Some((sender, token)) = begin_job(ui, state, "Loading included Spectrum example…") else {
        return;
    };
    let cli = ui.cli.clone();
    thread::spawn(move || {
        let result = embedded_example_document().and_then(|mut document| {
            if let Some(degrees) = cli.hue {
                document.recipe.set_hue_degrees(degrees);
            }
            apply_cli_recipe(&cli, &mut document.recipe);
            prepare_document(document, None, DocumentKind::Example, &token, &sender)
        });
        let _ = sender.send(Work::Open(
            token.generation(),
            DocumentKind::Example,
            Box::new(result.map_err(|error| format!("{error:#}"))),
        ));
    });
}

fn finish_job_controls(ui: &Ui, state: &Rc<RefCell<State>>) {
    for button in [
        &ui.open,
        &ui.empty_open,
        &ui.empty_project,
        &ui.empty_example,
    ] {
        button.set_sensitive(true);
    }
    let state = state.borrow();
    ui.save.set_sensitive(
        state
            .session
            .document()
            .is_some_and(|document| document.dirty),
    );
    ui.save_as.set_sensitive(state.session.document().is_some());
    ui.export.set_sensitive(state.session.document().is_some());
    ui.document_menu
        .set_sensitive(state.session.document().is_some());
}

pub(super) fn save_handler(
    button: &gtk::Button,
    ui: &Rc<Ui>,
    state: Rc<RefCell<State>>,
    force_as: bool,
) {
    let ui = ui.clone();
    button.connect_clicked(move |_| {
        // Drop the read guard before begin_job needs mutable application state.
        let existing_path = state.borrow().project_path.clone();
        if !force_as && let Some(path) = existing_path {
            start_project_save(&ui, &state, path);
            return;
        }
        let dialog = gtk::FileDialog::builder()
            .title("Save Project As")
            .initial_name("Untitled.chromiator")
            .default_filter(&open_filter(OpenKind::Project))
            .build();
        let win = ui.window.clone();
        let s = state.clone();
        let u = ui.clone();
        ui.save.set_sensitive(false);
        ui.save_as.set_sensitive(false);
        glib::spawn_future_local(async move {
            let path = dialog
                .save_future(Some(&win))
                .await
                .ok()
                .and_then(|file| file.path());
            if let Some(path) = path {
                start_project_save(&u, &s, ensure_project_extension(path));
            } else {
                s.borrow_mut().pending_replacement = None;
                let dirty = s
                    .borrow()
                    .session
                    .document()
                    .is_some_and(|document| document.dirty);
                u.save.set_sensitive(dirty);
                u.save_as
                    .set_sensitive(s.borrow().session.document().is_some());
            }
        });
    });
}

fn start_project_save(ui: &Rc<Ui>, state: &Rc<RefCell<State>>, path: PathBuf) {
    let (document, sender) = {
        let state = state.borrow();
        (state.session.document_cloned(), state.sender.clone())
    };
    if let Some(document) = document {
        let Some((_, token)) = begin_job(ui, state, "Serializing project manifest…") else {
            return;
        };
        thread::spawn(move || {
            let generation = token.generation();
            let result = project::save_cancellable(&path, &document, &token, |fraction, label| {
                let _ = sender.send(Work::Progress(generation, fraction, label));
            })
            .map(|_| path)
            .map_err(|error| format!("{error:#}"));
            let _ = sender.send(Work::Save(generation, result));
        });
    }
}

pub(super) fn export_handler(button: &gtk::Button, ui: &Rc<Ui>, state: Rc<RefCell<State>>) {
    let ui = ui.clone();
    button.connect_clicked(move |_| {
        ui.export.set_sensitive(false);
        let choose = native_dialog::AlertDialog::builder().heading("Choose export precision").body("PNG: encoded-sRGB integer. OpenEXR: linear-sRGB 32-bit float. Unsupported combinations are not substituted.").build();
        choose.add_response("cancel", "Cancel"); choose.add_response("p8", "PNG 8-bit"); choose.add_response("p16", "PNG 16-bit"); choose.add_response("exr", "OpenEXR 32f");
        let win = ui.window.clone(); let state = state.clone(); let ui = ui.clone();
        glib::spawn_future_local(async move {
            let (format, ext) = match choose.choose_future(Some(&win)).await.as_str() { "p8" => (ExportFormat::Png8, "png"), "p16" => (ExportFormat::Png16, "png"), "exr" => (ExportFormat::OpenExr32Float, "exr"), _ => { ui.export.set_sensitive(true); return } };
            let dialog = gtk::FileDialog::builder().title(format.description()).initial_name(format!("Chromiator-result.{ext}")).build();
            if let Ok(file) = dialog.save_future(Some(&win)).await && let Some(path) = file.path() {
                let (document, sender) = { let state = state.borrow(); (state.session.document_cloned(), state.sender.clone()) };
                if let Some(document) = document {
                    let Some((_, token)) = begin_job(&ui, &state, "Processing full-resolution rows…") else {
                        return;
                    };
                    thread::spawn(move || { let generation = token.generation(); let result = export::export_recipe_cancellable(&path, &document.source, &document.recipe, format, &token, |fraction, label| { let _ = sender.send(Work::Progress(generation, fraction, label)); }).map(|_| path).map_err(|error| format!("{error:#}")); let _ = sender.send(Work::Export(generation, result)); });
                }
            } else { ui.export.set_sensitive(true); }
        });
    });
}

pub(super) fn poll(ui: &Rc<Ui>, state: Rc<RefCell<State>>) {
    let ui = ui.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(30), move || {
        let preview = { state.borrow().scheduler.try_latest() };
        if let Some(preview) = preview {
            let mut s = state.borrow_mut();
            s.result_pixbuf = Some(pixbuf(preview.display));
            s.result = Some(preview.image);
            s.coverage = preview.coverage;
            drop(s);
            refresh_voronoi_ui(&ui, &state);
            ui.canvas.queue_draw();
            ui.status.set_label("Unsaved changes — preview current");
        }
        let work: Vec<_> = { state.borrow().receiver.try_iter().collect() };
        for w in work {
            match w {
                Work::Progress(generation, fraction, label) => {
                    let accepted = {
                        let mut state = state.borrow_mut();
                        state.jobs.is_current(generation)
                            && state.progress.accept(generation, fraction)
                    };
                    if accepted {
                        ui.progress.set_fraction(fraction);
                        ui.status.set_label(label);
                    }
                }
                Work::Open(generation, attempted_kind, result) => {
                    let current = state.borrow().jobs.is_current(generation);
                    if !state.borrow_mut().jobs.acknowledge(generation) {
                        continue;
                    }
                    finish_job(&ui, &state);
                    if !current {
                        resolve_pending_after_save(
                            &mut state.borrow_mut().pending_replacement,
                            SaveResolution::Cancelled,
                        );
                        ui.status.set_label("Cancelled");
                        continue;
                    }
                    match *result {
                        Ok((
                            mut d,
                            path,
                            document_kind,
                            preview_source,
                            source_display,
                            result_display,
                            coverage,
                        )) => {
                            // Keep the inspector and reopened recipe synchronized without
                            // treating initialization as a creative edit.
                            ui.syncing.set(true);
                            ui.hue.set_value(d.recipe.hue_degrees() as f64);
                            ui.syncing.set(false);
                            let profile = match d.interpretation.profile {
                                ProfileInterpretation::UntaggedAssumedSrgb => {
                                    "untagged; assumed sRGB"
                                }
                                ProfileInterpretation::EmbeddedProfileConvertedToSrgb => {
                                    "embedded profile converted to sRGB"
                                }
                            };
                            let pipeline_profile = match d.interpretation.profile {
                                ProfileInterpretation::UntaggedAssumedSrgb => "sRGB assumed",
                                ProfileInterpretation::EmbeddedProfileConvertedToSrgb => {
                                    "ICC → sRGB"
                                }
                            };
                            d.dirty = false;
                            if let Some(index) = ui.cli.lock_site
                                && let Some(site) =
                                    d.recipe.voronoi.sites.get_mut(index.saturating_sub(1))
                            {
                                site.locked = true;
                            }
                            let mut s = state.borrow_mut();
                            s.source_pixbuf = Some(pixbuf(source_display));
                            s.result_pixbuf = Some(pixbuf(result_display));
                            s.result = None;
                            s.preview_source = Some(preview_source);
                            s.coverage = coverage;
                            if let Err(error) = s.session.load(d) {
                                drop(s);
                                self::error(&ui, "Could not open document", &error.to_string());
                                continue;
                            }
                            s.project_path = path.clone();
                            s.document_kind = document_kind;
                            drop(s);
                            if let Some(path) = path {
                                welcome::remember(&ui, &state, &path);
                            }
                            ui.voronoi_panel.set_visible(true);
                            ui.pipeline_status.set_label(&format!(
                                "32-bit float · Linear sRGB · {pipeline_profile}"
                            ));
                            ui.pipeline_status.set_tooltip_text(Some(&format!(
                                "Straight-alpha linear-sRGB RGBA f32; {profile}; preview quantizes once for display"
                            )));
                            {
                                let mut s = state.borrow_mut();
                                if let Some(index) = ui.cli.select_site {
                                    let site = s
                                        .session
                                        .document()
                                        .and_then(|document| {
                                            document
                                                .recipe
                                                .voronoi
                                                .sites
                                                .get(index.saturating_sub(1))
                                        })
                                        .map(|site| site.id);
                                    s.session.select(site, false);
                                    s.session.set_expanded(site);
                                }
                                s.sampling = ui.cli.sampling;
                            }
                            if ui.cli.sampling.is_some() {
                                ui.source_mode.set_active(true);
                            }
                            refresh_voronoi_ui(&ui, &state);
                            refresh_preset_ui(&ui, &state, false);
                            sync_recipe_controls(&ui, &state);
                            ui.stack.set_visible_child_name("document");
                            ui.save.set_sensitive(false);
                            ui.save_as.set_sensitive(true);
                            ui.export.set_sensitive(true);
                            ui.document_menu.set_sensitive(true);
                            sync_contextual_chrome(&ui, &state);
                            sync_document_history_ui(&ui, &state);
                            match ui.cli.sampling {
                                Some(SamplingState::AddColor) => ui.status.set_label("Click a visible image color to create a site — Escape cancels"),
                                Some(SamplingState::AddSample) => ui.status.set_label("Click a visible source color to reattach Source and reset Target — Escape cancels"),
                                None => ui.status.set_label("Ready"),
                            }
                            ui.canvas.queue_draw();
                            if ui.cli.screenshot.is_some()
                                && ui.cli.window_size.is_some_and(|(width, _)| width <= 800)
                            {
                                ui.inspector_scroll.set_visible(true);
                            }
                            if ui.cli.show_color_picker {
                                present_color_picker(
                                    &ui,
                                    &state,
                                    ui.cli.color_model,
                                    PickerPurpose::Target,
                                );
                            }
                            if ui.cli.show_preset_dialog {
                                present_save_preset(&ui, &state);
                            }
                            schedule_cli_screenshot(&ui, &state);
                        }
                        Err(e) => {
                            let has_document = state.borrow().session.document().is_some();
                            ui.save_as.set_sensitive(has_document);
                            ui.export.set_sensitive(has_document);
                            let heading = match attempted_kind {
                                DocumentKind::Project => "Could not open project",
                                DocumentKind::Example => "Could not load included example",
                                _ => "Could not open image",
                            };
                            error(&ui, heading, &e);
                        }
                    }
                }
                Work::Save(generation, result) => {
                    let current = state.borrow().jobs.is_current(generation);
                    if !state.borrow_mut().jobs.acknowledge(generation) {
                        continue;
                    }
                    finish_job(&ui, &state);
                    if !current {
                        resolve_pending_after_save(
                            &mut state.borrow_mut().pending_replacement,
                            SaveResolution::Cancelled,
                        );
                        ui.status.set_label("Cancelled");
                        continue;
                    }
                    match result {
                        Ok(path) => {
                            let replacement = {
                                let mut s = state.borrow_mut();
                                s.project_path = Some(path.clone());
                                mark_document_saved(&mut s);
                                resolve_pending_after_save(
                                    &mut s.pending_replacement,
                                    SaveResolution::CurrentSuccess,
                                )
                            };
                            ui.save.set_sensitive(false);
                            ui.save_as.set_sensitive(true);
                            sync_document_history_ui(&ui, &state);
                            ui.status.set_label(&format!("Saved {}", path.display()));
                            welcome::remember(&ui, &state, &path);
                            if let Some(replacement) = replacement {
                                execute_replacement(replacement, &ui, &state);
                            }
                        }
                        Err(e) => {
                            resolve_pending_after_save(
                                &mut state.borrow_mut().pending_replacement,
                                SaveResolution::Failed,
                            );
                            error(&ui, "Save failed", &e)
                        }
                    }
                }
                Work::Export(generation, result) => {
                    let current = state.borrow().jobs.is_current(generation);
                    if !state.borrow_mut().jobs.acknowledge(generation) {
                        continue;
                    }
                    finish_job(&ui, &state);
                    if !current {
                        ui.status.set_label("Cancelled");
                        continue;
                    }
                    match result {
                        Ok(path) => ui.status.set_label(&format!("Exported {}", path.display())),
                        Err(e) => error(&ui, "Export failed", &e),
                    }
                }
            }
        }
        maybe_start_ui_audit(&ui, &state);
        glib::ControlFlow::Continue
    });
}
