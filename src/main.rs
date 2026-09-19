use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::thread;

use crossbeam_channel::{Receiver, Sender, unbounded};
use gdk_pixbuf::Pixbuf;
use gtk::gdk::prelude::GdkCairoContextExt;
use gtk::gdk::prelude::PaintableExt;
use gtk::prelude::*;
use chromiator::app_job::{DocumentKind, PreparedDocument, Work};
use chromiator::canvas::{
    CanvasSiteAction, accessible_site_label, canvas_site_action, divider_from_canvas_x,
    marker_hit_test, site_label, split_divider_hit,
};
use chromiator::cli::{Cli, CompareMode, SamplingState, apply_cli_recipe};
use chromiator::color::{
    ColorModel, DraftColor, OkhslPlaneSampler, PlaneKey, adjust_okhsl, display_hex, parse_hex,
};
use chromiator::components::ChromiatorMainShell;
use chromiator::document::{
    BlendSpace, Document, ExportDefaults, PixelImage, ProfileInterpretation, Recipe, SampleSize,
    VoronoiMatching,
};
use chromiator::export::{self, ExportFormat};
use chromiator::picker::{
    PickerGesture, PickerLocalHistory, picker_lightness_sequence, picker_plane_encoded_sample,
    picker_plane_physical_size,
};
use chromiator::preset::{Preset, PresetDiagnostic, PresetEntry, PresetStore};
use chromiator::processing::{
    Coverage, DisplayBuffer, bounded_preview, process_cancellable_with_progress_and_coverage,
    to_display_rgba8,
};
use chromiator::scheduler::{JobCoordinator, JobToken, PreviewScheduler, ProgressTracker};
use chromiator::session::{DocumentSession, EditCommand, EditGesture, SessionChange};
use chromiator::starter_looks::{STARTER_LOOKS, recipe_for_starter_look};
use chromiator::workflow::{
    OpenKind, SaveResolution, classify_open_path, ensure_project_extension,
    resolve_pending_after_save,
};
use chromiator::{example, project, raster};
use chromiator::{native_dialog, theme};

mod presentation;
mod shell_actions;
mod site_editor;
mod ui_audit;
mod transition_controls;
mod welcome;

use presentation::{
    PickerPurpose, canvas_draw, pixbuf, present_color_picker, schedule_cli_screenshot,
};
use shell_actions::{
    export_handler, mode_handler, poll, replacement_handler, save_handler, start_example_open,
};
use site_editor::{
    canvas_sampling, group_selection, refresh_voronoi_ui, selected_output_color,
    selected_source_color, sync_string_list,
};
use ui_audit::maybe_start_ui_audit;

const VORONOI_MATCHING_LABELS: &[&str] = &["Perceptual (OKLab)", "RGB (sRGB)", "HSV"];
const CREATIVE_FOCUS_CLASS: &str = "creative-focus";
const CANVAS_NATURAL_WIDTH: i32 = 320;
const CANVAS_NATURAL_HEIGHT: i32 = 240;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ContextualChrome {
    document_actions: bool,
    feedback_bar: bool,
    pipeline_metadata: bool,
}

fn contextual_chrome(has_document: bool, busy: bool) -> ContextualChrome {
    ContextualChrome {
        document_actions: has_document,
        feedback_bar: has_document || busy,
        pipeline_metadata: has_document,
    }
}

fn selected_user_preset_index(selected: usize, starter_count: usize) -> Option<usize> {
    selected.checked_sub(starter_count + 1)
}

fn visible_voronoi_matching_index(matching: VoronoiMatching) -> u32 {
    match matching {
        VoronoiMatching::Perceptual => 0,
        VoronoiMatching::Rgb => 1,
        VoronoiMatching::Hsv => 2,
        // Retain the dormant implementation for explicit developer fixtures without
        // presenting the parked experiment as a creator-facing choice.
        VoronoiMatching::Okhsl => gtk::INVALID_LIST_POSITION,
    }
}

fn visible_voronoi_matching_at(index: u32) -> VoronoiMatching {
    match index {
        1 => VoronoiMatching::Rgb,
        2 => VoronoiMatching::Hsv,
        _ => VoronoiMatching::Perceptual,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Replacement {
    OpenImage,
    OpenProject,
    Example,
}

struct State {
    session: DocumentSession,
    result: Option<PixelImage>,
    preview_source: Option<PixelImage>,
    coverage: Coverage,
    source_pixbuf: Option<Pixbuf>,
    result_pixbuf: Option<Pixbuf>,
    project_path: Option<PathBuf>,
    document_kind: DocumentKind,
    pending_replacement: Option<Replacement>,
    picker_visible: bool,
    picker_model: ColorModel,
    picker_lightness: Option<f64>,
    picker_lightness_updates: u32,
    picker_lightness_elapsed_ms: Option<f64>,
    picker_plane_render_max_us: u64,
    picker_plane_render_count: u32,
    mode: CompareMode,
    divider: f64,
    scheduler: PreviewScheduler,
    sender: Sender<Work>,
    receiver: Receiver<Work>,
    jobs: JobCoordinator,
    progress: ProgressTracker,
    sampling: Option<SamplingState>,
    sampling_previous_mode: Option<CompareMode>,
    preset_entries: Vec<PresetEntry>,
    preset_diagnostics: Vec<PresetDiagnostic>,
}

impl State {
    fn new() -> Self {
        let (sender, receiver) = unbounded();
        Self {
            session: DocumentSession::default(),
            result: None,
            preview_source: None,
            coverage: Coverage::default(),
            source_pixbuf: None,
            result_pixbuf: None,
            project_path: None,
            document_kind: DocumentKind::Welcome,
            pending_replacement: None,
            picker_visible: false,
            picker_model: ColorModel::Okhsl,
            picker_lightness: None,
            picker_lightness_updates: 0,
            picker_lightness_elapsed_ms: None,
            picker_plane_render_max_us: 0,
            picker_plane_render_count: 0,
            mode: CompareMode::Result,
            divider: 0.5,
            scheduler: PreviewScheduler::new(),
            sender,
            receiver,
            jobs: JobCoordinator::default(),
            progress: ProgressTracker::default(),
            sampling: None,
            sampling_previous_mode: None,
            preset_entries: Vec::new(),
            preset_diagnostics: Vec::new(),
        }
    }
}

struct Ui {
    transitions: transition_controls::TransitionControls,
    window: gtk::ApplicationWindow,
    inspector_scroll: gtk::ScrolledWindow,
    stack: gtk::Stack,
    canvas: gtk::DrawingArea,
    status: gtk::Label,
    pipeline_status: gtk::Label,
    progress: gtk::ProgressBar,
    cancel: gtk::Button,
    status_bar: gtk::Box,
    file_spacer: gtk::Separator,
    history_spacer: gtk::Separator,
    save: gtk::Button,
    save_as: gtk::Button,
    undo: gtk::Button,
    redo: gtk::Button,
    open: gtk::Button,
    empty_open: gtk::Button,
    empty_project: gtk::Button,
    empty_example: gtk::Button,
    recent_projects: gtk::Box,
    recent_clear: gtk::Button,
    recent_store: RefCell<welcome::RecentProjects>,
    export: gtk::Button,
    document_menu: gtk::MenuButton,
    hue: gtk::Adjustment,
    picker_dialog: RefCell<Option<gtk::Window>>,
    voronoi_matching: gtk::DropDown,
    smoothing: gtk::SpinButton,
    voronoi_panel: gtk::Expander,
    groups: gtk::ListBox,
    source_mode: gtk::ToggleButton,
    result_mode: gtk::ToggleButton,
    split_mode: gtk::ToggleButton,
    sidebar_button: gtk::ToggleButton,
    divider: gtk::Adjustment,
    preset_dropdown: gtk::DropDown,
    preset_model: gtk::StringList,
    preset_info: gtk::Label,
    preset_save: gtk::Button,
    preset_refresh: gtk::Button,
    preset_folder: gtk::Button,
    syncing: Cell<bool>,
    cli: Cli,
    snapshot_root: gtk::Widget,
    audit_started: Cell<bool>,
    audit_picker_model: RefCell<Option<gtk::DropDown>>,
    audit_picker_hex: RefCell<Option<gtk::Entry>>,
    audit_picker_channels: RefCell<Vec<gtk::Adjustment>>,
    audit_picker_undo: RefCell<Option<gtk::Button>>,
    audit_picker_redo: RefCell<Option<gtk::Button>>,
    audit_picker_cancel: RefCell<Option<gtk::Button>>,
    audit_picker_select: RefCell<Option<gtk::Button>>,
    audit_picker_plane: RefCell<Option<gtk::DrawingArea>>,
    audit_preset_name: RefCell<Option<gtk::Entry>>,
    audit_preset_cancel: RefCell<Option<gtk::Button>>,
    audit_preset_save: RefCell<Option<gtk::Button>>,
    audit_site_influence: RefCell<Option<gtk::SpinButton>>,
    audit_site_lock: RefCell<Option<gtk::ToggleButton>>,
    audit_site_target: RefCell<Option<gtk::Button>>,
    audit_site_source: RefCell<Option<gtk::Button>>,
    audit_site_sample_size: RefCell<Option<gtk::DropDown>>,
    audit_site_reattach: RefCell<Option<gtk::Button>>,
    audit_site_expander: RefCell<Option<gtk::Expander>>,
    audit_other_site_expander: RefCell<Option<gtk::Expander>>,
}

struct InspectorControls {
    transitions: transition_controls::TransitionControls,
    root: gtk::Box,
    hue: gtk::Adjustment,
    voronoi_matching: gtk::DropDown,
    smoothing: gtk::SpinButton,
    voronoi_panel: gtk::Expander,
    groups: gtk::ListBox,
    preset_dropdown: gtk::DropDown,
    preset_model: gtk::StringList,
    preset_info: gtk::Label,
    preset_save: gtk::Button,
    preset_refresh: gtk::Button,
    preset_folder: gtk::Button,
}

fn main() -> glib::ExitCode {
    let cli = Cli::parse();
    if let Some(message) = &cli.invalid_argument {
        eprintln!("chromiator: {message}");
        return glib::ExitCode::FAILURE;
    }
    if cli.screenshot.is_some() && std::env::var_os("GSK_RENDERER").is_none() {
        // Command-driven evidence is rendered deterministically before GTK starts.
        unsafe { std::env::set_var("GSK_RENDERER", "cairo") };
    }
    gio::resources_register_include!("chromiator.gresource")
        .expect("compiled Chromiator resources register");
    let app = gtk::Application::builder()
        .application_id("io.github.chromiator.Chromiator")
        .flags(application_flags(&cli))
        .build();
    let theme_bridge = Rc::new(RefCell::new(None));
    app.connect_startup({
        let theme_bridge = theme_bridge.clone();
        move |_| *theme_bridge.borrow_mut() = Some(theme::inherit_system_color_scheme())
    });
    app.connect_activate(move |app| {
        let _keep_theme_subscriptions = &theme_bridge;
        build(app, cli.clone())
    });
    app.run_with_args::<&str>(&[])
}

fn application_flags(cli: &Cli) -> gio::ApplicationFlags {
    if cli.screenshot.is_some() || cli.ui_audit_scenario.is_some() {
        gio::ApplicationFlags::NON_UNIQUE
    } else {
        gio::ApplicationFlags::empty()
    }
}

fn build(app: &gtk::Application, cli: Cli) {
    let state = Rc::new(RefCell::new(State::new()));
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Chromiator")
        .default_width(1180)
        .default_height(760)
        .build();
    install_creative_focus_style();
    let shell = ChromiatorMainShell::new();
    let header = shell.header();
    shell.remove(&header);
    window.set_titlebar(Some(&header));
    window.set_child(Some(&shell));
    if let Some((width, height)) = cli.window_size {
        window.set_default_size(width, height);
    }
    let open = shell.open_button();
    let save = shell.save_button();
    let save_as = icon_button(
        "document-save-as-symbolic",
        "Save project as (Ctrl+Shift+S)",
    );
    save_as.set_sensitive(false);
    let undo = shell.undo_button();
    undo.set_action_name(Some("app.creative-undo"));
    undo.set_sensitive(false);
    undo.update_property(&[
        gtk::accessible::Property::Label("Undo document change"),
        gtk::accessible::Property::Description(
            "Undo the most recent committed recipe or sampling change",
        ),
    ]);
    let redo = shell.redo_button();
    redo.set_action_name(Some("app.creative-redo"));
    redo.set_sensitive(false);
    redo.update_property(&[
        gtk::accessible::Property::Label("Redo document change"),
        gtk::accessible::Property::Description(
            "Redo the most recently undone recipe or sampling change",
        ),
    ]);
    let export = shell.export_button();
    let document_menu_model = gio::Menu::new();
    document_menu_model.append(Some("Open Project…"), Some("app.open-project"));
    document_menu_model.append(Some("Save Project As…"), Some("app.save-as"));
    document_menu_model.append(Some("Export Image…"), Some("app.export"));
    let document_menu = shell.document_menu();
    document_menu.set_menu_model(Some(&document_menu_model));
    document_menu.update_property(&[gtk::accessible::Property::Label("Document Menu")]);
    let sidebar_button = shell.sidebar_button();
    let file_spacer = shell.file_spacer();
    let history_spacer = shell.history_spacer();
    let canvas = shell.canvas();
    canvas.set_content_width(CANVAS_NATURAL_WIDTH);
    canvas.set_content_height(CANVAS_NATURAL_HEIGHT);
    canvas.add_css_class(CREATIVE_FOCUS_CLASS);
    canvas.update_property(&[gtk::accessible::Property::Label("Image comparison")]);
    canvas.set_accessible_role(gtk::AccessibleRole::Img);
    canvas.set_tooltip_text(Some("Processed image comparison surface"));
    // The Split divider is manipulated directly on the canvas. This adjustment remains the
    // single value owner for command-driven evidence and accessibility synchronization.
    let divider = gtk::Adjustment::new(0.5, 0.0, 1.0, 0.01, 0.1, 0.0);
    let result_mode = shell.result_mode();
    let split_mode = shell.split_mode();
    let source_mode = shell.source_mode();
    let smoothing = gtk::SpinButton::with_range(0.0, 10.0, 0.1);
    smoothing.set_numeric(true);
    smoothing.set_digits(2);
    smoothing.set_width_chars(4);
    smoothing.set_tooltip_text(Some(
        "Gaussian preprocessing before Voronoi mapping; 0 disables smoothing",
    ));
    smoothing.update_property(&[
        gtk::accessible::Property::Label("Smooth source amount from 0 to 10"),
        gtk::accessible::Property::Description(
            "Gaussian preprocessing before Voronoi mapping; zero disables smoothing",
        ),
    ]);
    let InspectorControls {
        transitions,
        root: inspector,
        hue,
        voronoi_matching,
        smoothing,
        voronoi_panel,
        groups,
        preset_dropdown,
        preset_model,
        preset_info,
        preset_save,
        preset_refresh,
        preset_folder,
    } = inspector(&smoothing);
    let inspector_scroll = shell.inspector_scroll();
    shell.inspector_content().append(&inspector);
    let empty_button = shell.welcome_open();
    let empty_project = shell.welcome_project();
    let empty_example = shell.welcome_example();
    let hero_texture = gtk::gdk::Texture::for_pixbuf(&embedded_example_pixbuf());
    let hero = gtk::Picture::for_paintable(&hero_texture);
    hero.set_content_fit(gtk::ContentFit::Cover);
    hero.set_can_shrink(true);
    hero.update_property(&[gtk::accessible::Property::Label(
        "Spectrum Breakpoint example artwork",
    )]);
    // The artwork covers its full allocation rather than fitting an aspect frame
    // inside it, which left side gutters when the available height was constrained.
    let banner = gtk::DrawingArea::builder()
        .content_height(320)
        .hexpand(true)
        .build();
    let banner_overlay = gtk::Overlay::new();
    banner_overlay.set_child(Some(&banner));
    hero.set_hexpand(true);
    hero.set_vexpand(true);
    banner_overlay.add_overlay(&hero);
    banner_overlay.set_measure_overlay(&hero, false);
    let window_controls = gtk::WindowControls::new(gtk::PackType::End);
    window_controls.set_halign(gtk::Align::End);
    window_controls.set_valign(gtk::Align::Start);
    window_controls.set_margin_top(8);
    window_controls.set_margin_end(8);
    window_controls.add_css_class("chromiator-splash-window-controls");
    banner_overlay.add_overlay(&window_controls);
    let banner_handle = gtk::WindowHandle::new();
    banner_handle.set_child(Some(&banner_overlay));
    shell.welcome_media().append(&banner_handle);
    let recent_projects = shell.welcome_recent();
    empty_project.bind_property("sensitive", &recent_projects, "sensitive").sync_create().build();
    let stack = shell.page_stack();
    stack.set_visible_child_name("empty");
    let progress = shell.progress();
    progress.update_property(&[
        gtk::accessible::Property::Label("Operation progress"),
        gtk::accessible::Property::Description(
            "Progress for the open, save, or export phase named in the adjacent status message",
        ),
    ]);
    let cancel = shell.cancel();
    let status = shell.status();
    let pipeline_status = shell.pipeline_status();
    pipeline_status.set_tooltip_text(Some(
        "Canonical pipeline: straight-alpha linear-sRGB RGBA f32; preview quantizes once for display",
    ));
    pipeline_status.update_property(&[
        gtk::accessible::Property::Label("Processing pipeline"),
        gtk::accessible::Property::Description(
            "Straight-alpha linear-sRGB RGBA floating point with one display quantization",
        ),
    ]);
    let status_box = shell.status_bar();
    let ui = Rc::new(Ui {
        transitions,
        window: window.clone(),
        inspector_scroll: inspector_scroll.clone(),
        stack,
        canvas: canvas.clone(),
        status,
        pipeline_status,
        progress,
        cancel,
        status_bar: status_box,
        file_spacer: file_spacer.clone(),
        history_spacer: history_spacer.clone(),
        save: save.clone(),
        save_as: save_as.clone(),
        undo: undo.clone(),
        redo: redo.clone(),
        open: open.clone(),
        empty_open: empty_button.clone(),
        empty_project: empty_project.clone(),
        empty_example: empty_example.clone(),
        recent_projects,
        recent_clear: shell.welcome_recent_clear(),
        recent_store: RefCell::new(welcome::RecentProjects::load()),
        export: export.clone(),
        document_menu: document_menu.clone(),
        hue,
        picker_dialog: RefCell::new(None),
        voronoi_matching,
        smoothing,
        voronoi_panel,
        groups,
        source_mode: source_mode.clone(),
        result_mode: result_mode.clone(),
        split_mode: split_mode.clone(),
        sidebar_button: sidebar_button.clone(),
        divider: divider.clone(),
        preset_dropdown,
        preset_model,
        preset_info,
        preset_save,
        preset_refresh,
        preset_folder,
        syncing: Cell::new(false),
        cli: cli.clone(),
        snapshot_root: shell.clone().upcast(),
        audit_started: Cell::new(false),
        audit_picker_model: RefCell::new(None),
        audit_picker_hex: RefCell::new(None),
        audit_picker_channels: RefCell::new(Vec::new()),
        audit_picker_undo: RefCell::new(None),
        audit_picker_redo: RefCell::new(None),
        audit_picker_cancel: RefCell::new(None),
        audit_picker_select: RefCell::new(None),
        audit_picker_plane: RefCell::new(None),
        audit_preset_name: RefCell::new(None),
        audit_preset_cancel: RefCell::new(None),
        audit_preset_save: RefCell::new(None),
        audit_site_influence: RefCell::new(None),
        audit_site_lock: RefCell::new(None),
        audit_site_target: RefCell::new(None),
        audit_site_source: RefCell::new(None),
        audit_site_sample_size: RefCell::new(None),
        audit_site_reattach: RefCell::new(None),
        audit_site_expander: RefCell::new(None),
        audit_other_site_expander: RefCell::new(None),
    });
    sync_contextual_chrome(&ui, &state);
    canvas_draw(&canvas, state.clone());
    recipe_controls(&ui, state.clone());
    preset_controls(&ui, state.clone());
    group_selection(&ui, state.clone());
    canvas_sampling(&ui, state.clone());
    replacement_handler(&open, Replacement::OpenImage, &ui, state.clone());
    replacement_handler(&empty_button, Replacement::OpenImage, &ui, state.clone());
    replacement_handler(&empty_project, Replacement::OpenProject, &ui, state.clone());
    replacement_handler(&empty_example, Replacement::Example, &ui, state.clone());
    welcome::connect(&ui, &state);
    save_handler(&save, &ui, state.clone(), false);
    save_handler(&save_as, &ui, state.clone(), true);
    export_handler(&export, &ui, state.clone());
    cancel_handler(&ui, state.clone());
    let s = state.clone();
    let c = canvas.clone();
    divider.connect_value_changed(move |v| {
        s.borrow_mut().divider = v.value();
        c.queue_draw();
    });
    mode_handler(&result_mode, CompareMode::Result, &canvas, state.clone());
    mode_handler(&split_mode, CompareMode::Split, &canvas, state.clone());
    mode_handler(&source_mode, CompareMode::Source, &canvas, state.clone());
    let inspector_toggle = inspector_scroll.clone();
    sidebar_button.connect_toggled(move |b| inspector_toggle.set_visible(b.is_active()));
    match cli.view.or_else(
        || match std::env::var("CHROMIATOR_AUTOMATION_MODE").as_deref() {
            Ok("split") => Some(CompareMode::Split),
            Ok("source") => Some(CompareMode::Source),
            _ => None,
        },
    ) {
        Some(CompareMode::Split) => split_mode.set_active(true),
        Some(CompareMode::Source) => source_mode.set_active(true),
        _ => result_mode.set_active(true),
    }
    if std::env::var_os("CHROMIATOR_AUTOMATION_NARROW").is_some() {
        window.set_default_size(720, 700);
        inspector_scroll.set_visible(false);
        sidebar_button.set_active(false);
    }
    if cli.window_size.is_some_and(|(width, _)| width <= 760) {
        inspector_scroll.set_visible(false);
        sidebar_button.set_active(false);
    }
    actions(app, &open, &empty_project, &save, &save_as, &export);
    creative_history_actions(app, &ui, &state);
    close_guard(&window, state.clone());
    poll(&ui, state.clone());
    let cli_has_input = cli.example
        || cli.open.is_some()
        || std::env::var_os("CHROMIATOR_AUTOMATION_IMAGE").is_some();
    if cli.example {
        start_example_open(&ui, &state);
    } else if let Some(path) = cli
        .open
        .clone()
        .or_else(|| std::env::var_os("CHROMIATOR_AUTOMATION_IMAGE").map(PathBuf::from))
    {
        let automation_hue = cli.hue.or_else(|| {
            std::env::var("CHROMIATOR_AUTOMATION_HUE")
                .ok()
                .and_then(|value| value.parse::<f32>().ok())
        });
        let automation_cli = cli.clone();
        let Some((sender, token)) = begin_job(&ui, &state, "Reading source file…") else {
            return;
        };
        thread::spawn(move || {
            let result = if classify_open_path(&path) == OpenKind::Project {
                project::open(&path)
                    .map(|document| (document, Some(path.clone()), DocumentKind::Project))
            } else {
                open_raster_job(&path, &token, &sender).map(|(bytes, decoded)| {
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
                })
            }
            .and_then(|(mut document, project_path, document_kind)| {
                if let Some(degrees) = automation_hue {
                    document.recipe.set_hue_degrees(degrees);
                }
                apply_cli_recipe(&automation_cli, &mut document.recipe);
                prepare_document(document, project_path, document_kind, &token, &sender)
            });
            let _ = sender.send(Work::Open(
                token.generation(),
                if classify_open_path(&path) == OpenKind::Project {
                    DocumentKind::Project
                } else {
                    DocumentKind::Image
                },
                Box::new(result.map_err(|error| format!("{error:#}"))),
            ));
        });
    }
    window.present();
    if cli.screenshot.is_some() && cli.window_size.is_some_and(|(width, _)| width <= 800) {
        inspector_scroll.set_visible(true);
    }
    if !cli_has_input && cli.screenshot.is_some() {
        schedule_cli_screenshot(&ui, &state);
    }
}

fn prepare_document(
    document: Document,
    path: Option<PathBuf>,
    document_kind: DocumentKind,
    token: &JobToken,
    sender: &Sender<Work>,
) -> anyhow::Result<PreparedDocument> {
    if !token.is_current() {
        anyhow::bail!("open cancelled")
    }
    let _ = sender.send(Work::Progress(
        token.generation(),
        0.50,
        "Building bounded preview…",
    ));
    let preview_source = bounded_preview(&document.source);
    if !token.is_current() {
        anyhow::bail!("open cancelled")
    }
    let (result, coverage) = process_cancellable_with_progress_and_coverage(
        &preview_source,
        &document.recipe,
        token.generation(),
        token.current(),
        |fraction| {
            let _ = sender.send(Work::Progress(
                token.generation(),
                0.55 + fraction * 0.30,
                "Processing preview rows…",
            ));
        },
    )
    .ok_or_else(|| anyhow::anyhow!("open cancelled"))?;
    let _ = sender.send(Work::Progress(
        token.generation(),
        0.88,
        "Preparing display buffers…",
    ));
    let source_display = to_display_rgba8(&preview_source);
    let result_display = to_display_rgba8(&result);
    if !token.is_current() {
        anyhow::bail!("open cancelled")
    }
    Ok((
        document,
        path,
        document_kind,
        preview_source,
        source_display,
        result_display,
        coverage,
    ))
}

fn open_raster_job(
    path: &std::path::Path,
    token: &JobToken,
    sender: &Sender<Work>,
) -> anyhow::Result<(std::sync::Arc<[u8]>, raster::DecodedRaster)> {
    let _ = sender.send(Work::Progress(
        token.generation(),
        0.03,
        "Reading source file…",
    ));
    let bytes: std::sync::Arc<[u8]> = std::fs::read(path)?.into();
    if !token.is_current() {
        anyhow::bail!("open cancelled")
    }
    let _ = sender.send(Work::Progress(
        token.generation(),
        0.18,
        "Decoding raster image…",
    ));
    let decoded = raster::decode(&bytes, Some(path))?;
    if !token.is_current() {
        anyhow::bail!("open cancelled")
    }
    let _ = sender.send(Work::Progress(token.generation(), 0.46, "Raster decoded"));
    Ok((bytes, decoded))
}

fn initialize_voronoi(document: &mut Document) {
    let proxy = bounded_preview(&document.source);
    document.recipe.voronoi = chromiator::voronoi::auto_initialize(&proxy, &document.source);
}

fn embedded_example_pixbuf() -> Pixbuf {
    Pixbuf::from_read(std::io::Cursor::new(example::SPECTRUM_BYTES))
        .expect("compiled Spectrum example is a valid raster")
}

fn embedded_example_document() -> anyhow::Result<Document> {
    example::spectrum_document()
}

fn begin_job(
    ui: &Ui,
    state: &Rc<RefCell<State>>,
    label: &'static str,
) -> Option<(Sender<Work>, JobToken)> {
    if ui.picker_dialog.borrow().is_some() {
        ui.status
            .set_label("Close the creative dialog before starting a file operation");
        return None;
    }
    let started = {
        let mut state = state.borrow_mut();
        let Some(token) = state.jobs.begin() else {
            drop(state);
            ui.status
                .set_label("Another file operation is already running");
            return None;
        };
        state.progress.start(token.generation());
        (state.sender.clone(), token)
    };
    ui.stack.set_sensitive(false);
    for button in [
        &ui.open,
        &ui.empty_open,
        &ui.empty_project,
        &ui.empty_example,
        &ui.save,
        &ui.save_as,
        &ui.export,
    ] {
        button.set_sensitive(false);
    }
    ui.progress.set_fraction(0.0);
    ui.progress.set_visible(true);
    ui.cancel.set_visible(true);
    ui.cancel.set_sensitive(true);
    ui.document_menu.set_sensitive(false);
    ui.status.set_label(label);
    sync_document_history_ui(ui, state);
    sync_contextual_chrome(ui, state);
    Some(started)
}

fn finish_job(ui: &Ui, state: &Rc<RefCell<State>>) {
    ui.stack.set_sensitive(true);
    ui.progress.set_visible(false);
    ui.cancel.set_visible(false);
    ui.open.set_sensitive(true);
    ui.empty_open.set_sensitive(true);
    ui.empty_project.set_sensitive(true);
    ui.empty_example.set_sensitive(true);
    let current = state.borrow();
    let has_document = current.session.document().is_some();
    let dirty = current
        .session
        .document()
        .is_some_and(|document| document.dirty);
    ui.save.set_sensitive(dirty);
    ui.save_as.set_sensitive(has_document);
    ui.export.set_sensitive(has_document);
    ui.document_menu.set_sensitive(has_document);
    drop(current);
    sync_document_history_ui(ui, state);
    sync_contextual_chrome(ui, state);
}

fn cancel_handler(ui: &Rc<Ui>, state: Rc<RefCell<State>>) {
    let ui = ui.clone();
    ui.cancel.clone().connect_clicked(move |_| {
        let mut state = state.borrow_mut();
        state.jobs.cancel();
        resolve_pending_after_save(&mut state.pending_replacement, SaveResolution::Cancelled);
        drop(state);
        ui.cancel.set_sensitive(false);
        ui.status.set_label("Cancelling after current codec phase…");
    });
}

fn icon_button(icon: &str, tip: &str) -> gtk::Button {
    let button = gtk::Button::builder()
        .icon_name(icon)
        .tooltip_text(tip)
        .build();
    button.update_property(&[gtk::accessible::Property::Label(tip)]);
    button
}

/// Region navigation supplements native Tab traversal without consuming editing keys.
fn install_workspace_accessibility(ui: &Rc<Ui>) {
    ui.open.update_property(&[gtk::accessible::Property::Label("Open image or project")]);
    ui.save.update_property(&[gtk::accessible::Property::Label("Save project")]);
    ui.export.update_property(&[gtk::accessible::Property::Label("Export image")]);
    ui.sidebar_button.update_property(&[gtk::accessible::Property::Label("Show adjustments")]);
    ui.progress.update_property(&[gtk::accessible::Property::Label("Image processing progress")]);
    ui.cancel.update_property(&[gtk::accessible::Property::Label("Cancel file operation")]);
    ui.groups.update_property(&[
        gtk::accessible::Property::Label("Color site list"),
        gtk::accessible::Property::Description("Use arrow keys to select a site and Tab to reach its controls. F6 moves between editing regions."),
    ]);
    ui.canvas.set_tooltip_text(Some("Artwork canvas. F6 switches editing regions; arrow keys move the selected Source marker."));
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(ui);
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        if key != gtk::gdk::Key::F6
            || modifiers.intersects(gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::ALT_MASK)
        {
            return glib::Propagation::Proceed;
        }
        let Some(ui) = weak.upgrade() else { return glib::Propagation::Proceed; };
        if !ui.canvas.is_mapped() { return glib::Propagation::Proceed; }
        let focus = gtk::prelude::GtkWindowExt::focus(&ui.window);
        let current = focus.as_ref().map_or(0, |widget| {
            if widget == ui.groups.upcast_ref::<gtk::Widget>() || widget.is_ancestor(&ui.groups) { 2 }
            else if widget.is_ancestor(&ui.inspector_scroll) { 1 }
            else { 0 }
        });
        let next = if !ui.inspector_scroll.is_visible() { 0 }
            else if modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK) { (current + 2) % 3 }
            else { (current + 1) % 3 };
        match next {
            1 => { ui.voronoi_matching.grab_focus(); }
            2 => {
                ui.voronoi_panel.set_expanded(true);
                if let Some(row) = ui.groups.selected_row() { row.grab_focus(); }
                else { ui.groups.child_focus(gtk::DirectionType::TabForward); }
            }
            _ => { ui.canvas.grab_focus(); }
        }
        glib::Propagation::Stop
    });
    ui.window.add_controller(keys);
}

fn install_creative_focus_style() {
    let provider = gtk::CssProvider::new();
    provider.load_from_resource("/io/github/chromiator/Chromiator/chromiator.css");
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().expect("GTK display is available"),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

fn sync_contextual_chrome(ui: &Ui, state: &Rc<RefCell<State>>) {
    let state = state.borrow();
    let chrome = contextual_chrome(state.session.document().is_some(), state.jobs.is_busy());
    if let Some(titlebar) = ui.window.titlebar() {
        titlebar.set_visible(state.session.document().is_some());
    }
    for widget in [
        ui.open.clone().upcast::<gtk::Widget>(),
        ui.save.clone().upcast(),
        ui.undo.clone().upcast(),
        ui.redo.clone().upcast(),
        ui.export.clone().upcast(),
        ui.sidebar_button.clone().upcast(),
        ui.document_menu.clone().upcast(),
        ui.file_spacer.clone().upcast(),
        ui.history_spacer.clone().upcast(),
    ] {
        widget.set_visible(chrome.document_actions);
    }
    ui.status_bar.set_visible(chrome.feedback_bar);
    ui.pipeline_status.set_visible(chrome.pipeline_metadata);
}

fn compact_section(title: &str, child: &impl IsA<gtk::Widget>, expanded: bool) -> gtk::Expander {
    let section = gtk::Expander::new(Some(title));
    section.update_property(&[gtk::accessible::Property::Label(title)]);
    section.set_expanded(expanded);
    section.set_child(Some(child));
    section.add_css_class("chromiator-inspector-group");
    section
}

fn inspector_control_row(title: &str, subtitle: &str, control: &impl IsA<gtk::Widget>) -> gtk::Box {
    let labels = gtk::Box::new(gtk::Orientation::Vertical, 2);
    labels.set_hexpand(true);
    let label = gtk::Label::builder().label(title).use_underline(true).xalign(0.0).build();
    label.set_mnemonic_widget(Some(control));
    control.as_ref().update_relation(&[gtk::accessible::Relation::LabelledBy(&[label.upcast_ref()])]);
    control.as_ref().set_tooltip_text(Some(subtitle));
    labels.append(&label);
    labels.append(
        &gtk::Label::builder()
            .label(subtitle)
            .xalign(0.0)
            .wrap(true)
            .css_classes(["dim-label", "caption"])
            .build(),
    );
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.add_css_class("chromiator-control-row");
    row.append(&labels);
    row.append(control);
    row
}

fn inspector(smoothing: &gtk::SpinButton) -> InspectorControls {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
    root.set_margin_top(12);
    root.set_margin_bottom(12);
    root.set_margin_start(12);
    root.set_margin_end(12);
    root.set_width_request(300);

    let matching = gtk::DropDown::from_strings(VORONOI_MATCHING_LABELS);
    matching.update_property(&[
        gtk::accessible::Property::Label("Voronoi color matching"),
        gtk::accessible::Property::Description(
            "Choose the three-dimensional color space used to assign pixels to color sites",
        ),
    ]);
    let mapping_group = gtk::Box::new(gtk::Orientation::Vertical, 4);
    mapping_group.add_css_class("chromiator-inspector-group");
    mapping_group.append(
        &gtk::Label::builder()
            .label("Color mapping")
            .xalign(0.0)
            .css_classes(["heading"])
            .build(),
    );
    mapping_group.append(&inspector_control_row(
        "_Matching",
        "Distance geometry for Source colors",
        &matching,
    ));
    mapping_group.append(&inspector_control_row(
        "_Smoothing",
        "Source preprocessing before color mapping",
        smoothing,
    ));
    root.append(&mapping_group);
    let transitions = transition_controls::TransitionControls::new(&mapping_group);

    let groups = gtk::ListBox::new();
    groups.set_selection_mode(gtk::SelectionMode::Single);
    groups.add_css_class("boxed-list");
    let sites_scroll = gtk::ScrolledWindow::builder()
        .child(&groups)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .min_content_height(220)
        .vexpand(true)
        .build();
    let voronoi_panel = compact_section("Color sites", &sites_scroll, true);
    voronoi_panel.set_vexpand(true);
    root.append(&voronoi_panel);

    let preset_model = gtk::StringList::new(&["Choose a preset…"]);
    let preset_dropdown = gtk::DropDown::new(Some(preset_model.clone()), gtk::Expression::NONE);
    preset_dropdown.set_hexpand(true);
    preset_dropdown.update_property(&[gtk::accessible::Property::Label("Preset")]);
    let preset_info = gtk::Label::builder()
        .label("Built-in and personal Voronoi recipes")
        .xalign(0.0)
        .wrap(true)
        .css_classes(["dim-label", "caption"])
        .build();
    let preset_save = gtk::Button::with_label("Save Preset…");
    let preset_refresh = icon_button("view-refresh-symbolic", "Refresh personal presets");
    let preset_folder = icon_button("folder-open-symbolic", "Open personal preset folder");
    let preset_actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    preset_actions.append(&preset_save);
    preset_actions.append(&preset_refresh);
    preset_actions.append(&preset_folder);
    let preset_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
    preset_box.append(&preset_dropdown);
    preset_box.append(&preset_info);
    preset_box.append(&preset_actions);
    root.append(&compact_section("Presets", &preset_box, true));

    InspectorControls {
        transitions,
        root,
        hue: gtk::Adjustment::new(0.0, -180.0, 180.0, 0.1, 1.0, 0.0),
        voronoi_matching: matching,
        smoothing: smoothing.clone(),
        voronoi_panel,
        groups,
        preset_dropdown,
        preset_model,
        preset_info,
        preset_save,
        preset_refresh,
        preset_folder,
    }
}

fn sync_document_history_ui(ui: &Ui, state: &Rc<RefCell<State>>) {
    let state = state.borrow();
    let available =
        state.session.document().is_some() && !state.jobs.is_busy() && !state.picker_visible;
    let can_undo = available && state.session.can_undo();
    let can_redo = available && state.session.can_redo();
    ui.undo.set_sensitive(can_undo);
    ui.redo.set_sensitive(can_redo);
    if let Some(app) = ui.window.application() {
        if let Some(action) = app
            .lookup_action("creative-undo")
            .and_then(|action| action.downcast::<gio::SimpleAction>().ok())
        {
            action.set_enabled(can_undo);
        }
        if let Some(action) = app
            .lookup_action("creative-redo")
            .and_then(|action| action.downcast::<gio::SimpleAction>().ok())
        {
            action.set_enabled(can_redo);
        }
    }
}

fn finish_coalesced_edit(state: &Rc<RefCell<State>>, edit: EditGesture) {
    state.borrow_mut().session.finish_gesture(edit);
}

fn install_document_spin_boundaries(
    control: &gtk::SpinButton,
    state: &Rc<RefCell<State>>,
    edit: EditGesture,
) {
    let click = gtk::GestureClick::new();
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    click.connect_pressed({
        let state = state.clone();
        move |_, _, _, _| finish_coalesced_edit(&state, edit)
    });
    click.connect_released({
        let state = state.clone();
        move |_, _, _, _| finish_coalesced_edit(&state, edit)
    });
    control.add_controller(click);

    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_released({
        let state = state.clone();
        move |_, _, _, _| finish_coalesced_edit(&state, edit)
    });
    control.add_controller(keys);

    control.connect_has_focus_notify({
        let state = state.clone();
        move |control| {
            if !control.has_focus() {
                finish_coalesced_edit(&state, edit);
            }
        }
    });
}

fn mark_document_saved(state: &mut State) {
    state.session.mark_saved();
}

/// Applies a validated session transition to presentation-only state.
fn apply_session_change(
    ui: &Ui,
    state: &Rc<RefCell<State>>,
    change: SessionChange,
    message: &str,
) -> bool {
    if !change.changed {
        return false;
    }
    {
        let current = state.borrow_mut();
        if change.preview_required
            && let Some(source) = current.preview_source.clone()
        {
            current.scheduler.schedule(source, change.recipe.clone());
        }
    }
    ui.syncing.set(true);
    ui.preset_dropdown.set_selected(0);
    ui.syncing.set(false);
    ui.save.set_sensitive(change.dirty);
    ui.status.set_label(message);
    update_preset_info(ui, state);
    sync_document_history_ui(ui, state);
    true
}

/// Admits one typed recipe edit through the document authority.
fn submit_session_edit(
    ui: &Ui,
    state: &Rc<RefCell<State>>,
    command: EditCommand,
    gesture: Option<EditGesture>,
    message: &str,
) -> bool {
    let change = {
        let mut current = state.borrow_mut();
        if current.jobs.is_busy() {
            ui.status
                .set_label("Finish or cancel the file operation before editing");
            return false;
        }
        match current.session.edit(command, gesture) {
            Ok(change) => change,
            Err(error) => {
                ui.status.set_label(&error.to_string());
                return false;
            }
        }
    };
    apply_session_change(ui, state, change, message)
}

fn update_preset_info(ui: &Ui, state: &Rc<RefCell<State>>) {
    let current = state.borrow();
    let count = STARTER_LOOKS.len() + current.preset_entries.len();
    let errors = current.preset_diagnostics.len();
    let summary = if errors == 0 {
        format!(
            "{count} preset{} available",
            if count == 1 { "" } else { "s" }
        )
    } else {
        format!(
            "{count} available · {errors} file error{}",
            if errors == 1 { "" } else { "s" }
        )
    };
    let selected = ui.preset_dropdown.selected() as usize;
    let description = if selected == 0 {
        "Choose a preset to apply immediately".into()
    } else if let Some(look) = STARTER_LOOKS.get(selected - 1) {
        format!("Built-in · {}", look.description)
    } else {
        current
            .preset_entries
            .get(selected_user_preset_index(selected, STARTER_LOOKS.len()).unwrap_or(usize::MAX))
            .and_then(|entry| entry.preset.description.clone())
            .unwrap_or_else(|| "User preset · no description".into())
    };
    ui.preset_info
        .set_label(&format!("{summary} · {description}"));
    ui.preset_dropdown
        .set_tooltip_text(Some(&format!("{summary} · {description}")));
}

fn unified_preset_labels(entries: &[PresetEntry]) -> Vec<String> {
    let mut labels = vec!["Presets…".to_owned()];
    labels.extend(STARTER_LOOKS.iter().map(|look| look.name.to_owned()));
    labels.extend(entries.iter().map(|entry| entry.preset.name.clone()));
    labels
}

fn install_preset_scan(
    ui: &Ui,
    state: &Rc<RefCell<State>>,
    scan: chromiator::preset::PresetScan,
    preferred_path: Option<&Path>,
    show_errors: bool,
) {
    let labels = unified_preset_labels(&scan.entries);
    let selected = preferred_path
        .and_then(|path| scan.entries.iter().position(|entry| entry.path == path))
        .map(|index| STARTER_LOOKS.len() + 1 + index);
    let diagnostics = scan.diagnostics.clone();
    {
        let mut current = state.borrow_mut();
        current.preset_entries = scan.entries;
        current.preset_diagnostics = scan.diagnostics;
    }
    ui.syncing.set(true);
    sync_string_list(&ui.preset_model, &labels);
    ui.preset_dropdown
        .set_selected(selected.map_or(0, |index| index as u32));
    ui.syncing.set(false);
    update_preset_info(ui, state);
    if show_errors && !diagnostics.is_empty() {
        let body = diagnostics
            .iter()
            .map(|item| {
                format!(
                    "{}: {}",
                    item.path.file_name().unwrap_or_default().to_string_lossy(),
                    item.reason
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        let dialog = native_dialog::AlertDialog::builder()
            .heading("Some presets could not be loaded")
            .body(body)
            .build();
        dialog.add_response("ok", "OK");
        dialog.present(Some(&ui.window));
    }
}

fn refresh_preset_ui(ui: &Ui, state: &Rc<RefCell<State>>, show_errors: bool) {
    let selected = ui.preset_dropdown.selected() as usize;
    let previous = state
        .borrow()
        .preset_entries
        .get(selected_user_preset_index(selected, STARTER_LOOKS.len()).unwrap_or(usize::MAX))
        .map(|entry| entry.path.clone());
    match PresetStore::system().scan() {
        Ok(scan) => install_preset_scan(ui, state, scan, previous.as_deref(), show_errors),
        Err(error) => {
            ui.status
                .set_label(&format!("Could not scan presets: {error:#}"));
        }
    }
}

fn apply_preset_selection(ui: &Rc<Ui>, state: &Rc<RefCell<State>>, selected: usize) {
    if selected == 0 {
        return;
    }
    let applied_selection = selected;
    let selected = selected - 1;
    let user_preset = if selected >= STARTER_LOOKS.len() {
        let cached = state
            .borrow()
            .preset_entries
            .get(selected - STARTER_LOOKS.len())
            .cloned();
        let Some(cached) = cached else { return };
        let scan = match PresetStore::system().scan() {
            Ok(scan) => scan,
            Err(error) => {
                ui.status.set_label(&format!(
                    "Could not rescan presets before selection: {error:#}"
                ));
                return;
            }
        };
        let fresh = scan
            .entries
            .iter()
            .find(|entry| entry.path == cached.path)
            .cloned();
        install_preset_scan(ui, state, scan, Some(&cached.path), false);
        let Some(fresh) = fresh else {
            ui.status
                .set_label("Selected user preset was removed or is no longer valid");
            return;
        };
        if fresh.preset.name != cached.preset.name {
            ui.status
                .set_label("Selected user preset changed identity; select it again");
            return;
        }
        Some(fresh)
    } else {
        None
    };
    let recipe = if selected < STARTER_LOOKS.len() {
        recipe_for_starter_look(STARTER_LOOKS[selected])
    } else if let Some(entry) = user_preset {
        entry.preset.recipe()
    } else {
        return;
    };
    if !submit_session_edit(
        ui,
        state,
        EditCommand::ReplaceRecipe(recipe),
        None,
        "Preset applied — updating preview…",
    ) {
        ui.status.set_label("Preset already matches this document");
        return;
    }
    {
        let mut current = state.borrow_mut();
        current.sampling = None;
        current.sampling_previous_mode = None;
    }
    ui.syncing.set(true);
    ui.preset_dropdown.set_selected(applied_selection as u32);
    ui.syncing.set(false);
    update_preset_info(ui, state);
    sync_recipe_controls(ui, state);
    refresh_voronoi_ui(ui, state);
}

fn creative_history_step(ui: &Rc<Ui>, state: &Rc<RefCell<State>>, redo: bool) {
    if state.borrow().picker_visible {
        return;
    }
    if state.borrow().jobs.is_busy() {
        ui.status
            .set_label("Finish or cancel the file operation before using document history");
        return;
    }
    let change = {
        let mut current = state.borrow_mut();
        let result = if redo {
            current.session.redo()
        } else {
            current.session.undo()
        };
        let Ok(change) = result else {
            ui.status.set_label(if redo {
                "Nothing to redo"
            } else {
                "Nothing to undo"
            });
            return;
        };
        if !change.changed {
            ui.status.set_label(if redo {
                "Nothing to redo"
            } else {
                "Nothing to undo"
            });
            return;
        }
        current.sampling = None;
        current.sampling_previous_mode = None;
        change
    };
    apply_session_change(
        ui,
        state,
        change,
        if redo {
            "Document change redone — updating preview…"
        } else {
            "Document change undone — updating preview…"
        },
    );
    ui.syncing.set(true);
    ui.preset_dropdown.set_selected(0);
    ui.syncing.set(false);
    sync_recipe_controls(ui, state);
    refresh_voronoi_ui(ui, state);
}

fn creative_history_actions(app: &gtk::Application, ui: &Rc<Ui>, state: &Rc<RefCell<State>>) {
    install_workspace_accessibility(ui);
    for (name, keys, redo) in [
        ("creative-undo", &["<primary>z"][..], false),
        ("creative-redo", &["<primary><shift>z"][..], true),
    ] {
        let action = gio::SimpleAction::new(name, None);
        action.set_enabled(false);
        let ui = ui.clone();
        let state = state.clone();
        action.connect_activate(move |_, _| creative_history_step(&ui, &state, redo));
        app.add_action(&action);
        app.set_accels_for_action(&format!("app.{name}"), keys);
    }
    sync_document_history_ui(ui, state);
}

fn present_preset_error(ui: &Ui, heading: &str, error: &anyhow::Error) {
    let dialog = native_dialog::AlertDialog::builder()
        .heading(heading)
        .body(format!("{error:#}"))
        .build();
    dialog.add_response("ok", "OK");
    dialog.present(Some(&ui.window));
}

fn present_save_preset(ui: &Rc<Ui>, state: &Rc<RefCell<State>>) {
    let Some(recipe) = state
        .borrow()
        .session
        .document()
        .map(|document| document.recipe.clone())
    else {
        ui.status
            .set_label("Open a document before saving a preset");
        return;
    };
    let name = gtk::Entry::builder()
        .placeholder_text("Preset name")
        .hexpand(true)
        .build();
    name.update_property(&[gtk::accessible::Property::Label("Preset name")]);
    let description = gtk::Entry::builder()
        .placeholder_text("Optional description")
        .hexpand(true)
        .build();
    description.update_property(&[gtk::accessible::Property::Label("Preset description")]);
    let form = gtk::Box::new(gtk::Orientation::Vertical, 12);
    form.set_margin_top(18);
    form.set_margin_bottom(18);
    form.set_margin_start(18);
    form.set_margin_end(18);
    form.append(&gtk::Label::builder().label("Name").xalign(0.0).build());
    form.append(&name);
    form.append(
        &gtk::Label::builder()
            .label("Description")
            .xalign(0.0)
            .build(),
    );
    form.append(&description);
    form.append(
        &gtk::Label::builder()
            .label("Saves processing settings only. Source artwork and image marker positions are omitted.")
            .wrap(true)
            .xalign(0.0)
            .css_classes(["dim-label"])
            .build(),
    );
    let header = gtk::HeaderBar::new();
    let cancel = gtk::Button::with_label("Cancel");
    let save = gtk::Button::with_label("Save");
    save.add_css_class("suggested-action");
    header.pack_start(&cancel);
    header.pack_end(&save);
    let dialog = gtk::Window::builder()
        .title("Save Current Preset")
        .default_width(440)
        .default_height(320)
        .transient_for(&ui.window)
        .modal(true)
        .destroy_with_parent(true)
        .child(&form)
        .build();
    dialog.set_titlebar(Some(&header));
    *ui.audit_preset_name.borrow_mut() = Some(name.clone());
    *ui.audit_preset_cancel.borrow_mut() = Some(cancel.clone());
    *ui.audit_preset_save.borrow_mut() = Some(save.clone());
    {
        let ui = ui.clone();
        dialog.connect_close_request(move |_| {
            *ui.audit_preset_name.borrow_mut() = None;
            *ui.audit_preset_cancel.borrow_mut() = None;
            *ui.audit_preset_save.borrow_mut() = None;
            glib::Propagation::Proceed
        });
    }
    {
        let dialog = dialog.clone();
        cancel.connect_clicked(move |_| {
            dialog.close();
        });
    }
    {
        let ui = ui.clone();
        let state = state.clone();
        let dialog = dialog.clone();
        let name_input = name.clone();
        let save_button = save.clone();
        let save_in_flight = Rc::new(Cell::new(false));
        save.connect_clicked(move |_| {
            if save_in_flight.replace(true) {
                return;
            }
            save_button.set_sensitive(false);
            let description = (!description.text().trim().is_empty())
                .then(|| description.text().trim().to_owned());
            let preset = match Preset::new(name_input.text().as_str(), description, &recipe) {
                Ok(preset) => preset,
                Err(error) => {
                    present_preset_error(&ui, "Preset name is not valid", &error);
                    save_in_flight.set(false);
                    save_button.set_sensitive(true);
                    return;
                }
            };
            let store = PresetStore::system();
            let duplicate = store.scan().ok().is_some_and(|scan| {
                scan.entries
                    .iter()
                    .any(|entry| entry.preset.name == preset.name)
            });
            let ui_save = ui.clone();
            let state_save = state.clone();
            let dialog_save = dialog.clone();
            let save_button = save_button.clone();
            let save_in_flight = save_in_flight.clone();
            glib::spawn_future_local(async move {
                if duplicate {
                    let confirm = native_dialog::AlertDialog::builder()
                        .heading("Replace existing preset?")
                        .body(format!("A preset named {:?} already exists.", preset.name))
                        .build();
                    confirm.add_response("cancel", "Cancel");
                    confirm.add_response("replace", "Replace");
                    confirm.set_response_appearance(
                        "replace",
                        native_dialog::ResponseAppearance::Destructive,
                    );
                    if confirm.choose_future(Some(&ui_save.window)).await != "replace" {
                        save_in_flight.set(false);
                        save_button.set_sensitive(true);
                        return;
                    }
                }
                match store.save(&preset, duplicate) {
                    Ok(_) => {
                        dialog_save.close();
                        refresh_preset_ui(&ui_save, &state_save, false);
                        ui_save.status.set_label("Preset saved");
                    }
                    Err(error) => {
                        present_preset_error(&ui_save, "Could not save preset", &error);
                        save_in_flight.set(false);
                        save_button.set_sensitive(true);
                    }
                }
            });
        });
    }
    dialog.present();
    name.grab_focus();
}

fn preset_controls(ui: &Rc<Ui>, state: Rc<RefCell<State>>) {
    refresh_preset_ui(ui, &state, false);
    {
        let ui = ui.clone();
        let state = state.clone();
        ui.preset_dropdown
            .clone()
            .connect_selected_notify(move |dropdown| {
                if ui.syncing.get() {
                    return;
                }
                let selected = dropdown.selected();
                if selected == gtk::INVALID_LIST_POSITION {
                    update_preset_info(&ui, &state);
                    return;
                }
                update_preset_info(&ui, &state);
                apply_preset_selection(&ui, &state, selected as usize);
            });
    }
    {
        let ui = ui.clone();
        let state = state.clone();
        ui.preset_refresh
            .clone()
            .connect_clicked(move |_| refresh_preset_ui(&ui, &state, true));
    }
    {
        let ui = ui.clone();
        let state = state.clone();
        ui.preset_save.clone().connect_clicked(move |_| {
            refresh_preset_ui(&ui, &state, false);
            present_save_preset(&ui, &state);
        });
    }
    {
        let ui = ui.clone();
        ui.preset_folder.clone().connect_clicked(move |_| {
            match PresetStore::system().prepare_directory() {
                Ok(path) => {
                    let file = gio::File::for_path(path);
                    if let Err(error) = gio::AppInfo::launch_default_for_uri(
                        &file.uri(),
                        None::<&gio::AppLaunchContext>,
                    ) {
                        ui.status
                            .set_label(&format!("Could not open preset folder: {error}"));
                    }
                }
                Err(error) => present_preset_error(&ui, "Could not prepare preset folder", &error),
            }
        });
    }
}

fn sync_recipe_controls(ui: &Ui, state: &Rc<RefCell<State>>) {
    let Some(recipe) = state
        .borrow()
        .session
        .document()
        .map(|document| document.recipe.clone())
    else {
        return;
    };
    ui.syncing.set(true);
    ui.smoothing
        .set_value(f64::from(recipe.preprocessing.input_smoothing));
    ui.hue.set_value(f64::from(recipe.hue_degrees()));
    ui.voronoi_matching
        .set_selected(visible_voronoi_matching_index(recipe.voronoi.matching));
    ui.transitions.sync(recipe.voronoi.transition);
    ui.syncing.set(false);
}

fn recipe_controls(ui: &Rc<Ui>, state: Rc<RefCell<State>>) {
    transition_controls::connect(ui, &state);
    {
        let ui = ui.clone();
        let state = state.clone();
        let smoothing = ui.smoothing.clone();
        let ui_handler = ui.clone();
        let state_handler = state.clone();
        smoothing.connect_value_changed(move |control| {
            if ui_handler.syncing.get() {
                return;
            }
            submit_session_edit(
                &ui_handler,
                &state_handler,
                EditCommand::SetSmoothing(control.value() as f32),
                Some(EditGesture::Smoothing),
                "Smoothing changed — updating preview…",
            );
        });
        install_document_spin_boundaries(&ui.smoothing, &state, EditGesture::Smoothing);
    }
    {
        let ui = ui.clone();
        let state = state.clone();
        ui.hue.clone().connect_value_changed(move |control| {
            if ui.syncing.get() {
                return;
            }
            submit_session_edit(
                &ui,
                &state,
                EditCommand::SetHue(control.value() as f32),
                None,
                "Hue changed — updating preview…",
            );
        });
    }
    {
        let ui = ui.clone();
        let state = state.clone();
        ui.voronoi_matching
            .clone()
            .connect_selected_notify(move |control| {
                if ui.syncing.get() || control.selected() == gtk::INVALID_LIST_POSITION {
                    return;
                }
                submit_session_edit(
                    &ui,
                    &state,
                    EditCommand::SetMatching(visible_voronoi_matching_at(control.selected())),
                    None,
                    "Color matching changed — updating preview…",
                );
            });
    }
}

fn error(ui: &Ui, heading: &str, body: &str) {
    eprintln!("{heading}: {body}");
    ui.status.set_label(body);
    let d = native_dialog::AlertDialog::builder()
        .heading(heading)
        .body(body)
        .build();
    d.add_response("close", "Close");
    d.present(Some(&ui.window));
}

fn actions(
    app: &gtk::Application,
    open: &gtk::Button,
    open_project: &gtk::Button,
    save: &gtk::Button,
    save_as: &gtk::Button,
    export: &gtk::Button,
) {
    for (name, keys, button) in [
        ("open", &["<primary>o"][..], open),
        ("open-project", &[][..], open_project),
        ("save", &["<primary>s"][..], save),
        ("save-as", &["<primary><shift>s"][..], save_as),
        ("export", &["<primary><shift>e"][..], export),
    ] {
        let a = gio::SimpleAction::new(name, None);
        let b = button.clone();
        a.connect_activate(move |_, _| {
            if b.is_sensitive() {
                b.emit_clicked();
            }
        });
        app.add_action(&a);
        app.set_accels_for_action(&format!("app.{name}"), keys);
    }
}

fn close_guard(window: &gtk::ApplicationWindow, state: Rc<RefCell<State>>) {
    let allow = Rc::new(Cell::new(false));
    let win = window.clone();
    window.connect_close_request(move |_| {
        if allow.get() || !state.borrow().session.document().is_some_and(|d| d.dirty) {
            return glib::Propagation::Proceed;
        }
        let d = native_dialog::AlertDialog::builder()
            .heading("Discard unsaved creative changes?")
            .body("View and divider changes are not saved, but recipe changes are unsaved.")
            .build();
        d.add_response("cancel", "Cancel");
        d.add_response("discard", "Discard Changes");
        d.set_response_appearance("discard", native_dialog::ResponseAppearance::Destructive);
        let a = allow.clone();
        let w = win.clone();
        glib::spawn_future_local(async move {
            if d.choose_future(Some(&w)).await == "discard" {
                a.set(true);
                w.close();
            }
        });
        glib::Propagation::Stop
    });
}

#[cfg(test)]
mod tests {
    use super::{
        CANVAS_NATURAL_HEIGHT, CANVAS_NATURAL_WIDTH, CREATIVE_FOCUS_CLASS, CanvasSiteAction, Cli,
        PickerGesture, PickerLocalHistory, VORONOI_MATCHING_LABELS, accessible_site_label,
        application_flags, canvas_site_action, contextual_chrome, divider_from_canvas_x,
        marker_hit_test, picker_lightness_sequence, picker_plane_encoded_sample,
        picker_plane_physical_size, selected_user_preset_index, site_label, split_divider_hit,
        unified_preset_labels, visible_voronoi_matching_at, visible_voronoi_matching_index,
    };
    use chromiator::{
        color::{ColorModel, DraftColor},
        document::VoronoiMatching,
    };

    #[test]
    fn creator_matching_menu_parks_okhsl_without_removing_its_mode() {
        assert_eq!(
            VORONOI_MATCHING_LABELS,
            ["Perceptual (OKLab)", "RGB (sRGB)", "HSV"]
        );
        assert_eq!(visible_voronoi_matching_at(0), VoronoiMatching::Perceptual);
        assert_eq!(visible_voronoi_matching_at(1), VoronoiMatching::Rgb);
        assert_eq!(visible_voronoi_matching_at(2), VoronoiMatching::Hsv);
        assert_eq!(
            visible_voronoi_matching_index(VoronoiMatching::Okhsl),
            gtk::INVALID_LIST_POSITION
        );
    }

    #[test]
    fn picker_lightness_sequence_is_dense_and_reaches_target() {
        let values = picker_lightness_sequence(0.1, 0.9, 120);
        assert_eq!(values.len(), 120);
        assert!(values.windows(2).all(|pair| pair[0] < pair[1]));
        assert!((values[119] - 0.9).abs() < f64::EPSILON);
    }

    #[test]
    fn picker_plane_uses_device_pixels_and_complete_model_disks() {
        assert_eq!(picker_plane_physical_size(300, 240, 1), (300, 240));
        assert_eq!(picker_plane_physical_size(300, 240, 2), (600, 480));
        assert!(picker_plane_encoded_sample(ColorModel::Hsv, 0.8, 0.0, 0.0).is_some());
        assert!(picker_plane_encoded_sample(ColorModel::Hsl, 0.5, 0.999, 0.0).is_some());
        assert!(picker_plane_encoded_sample(ColorModel::Okhsl, 0.5, 0.0, -0.999).is_some());
        assert!(picker_plane_encoded_sample(ColorModel::Hsv, 0.8, 1.01, 0.0).is_none());
        for model in [ColorModel::Hsv, ColorModel::Hsl, ColorModel::Okhsl] {
            let sample = picker_plane_encoded_sample(model, 0.55, 0.6, 0.7).unwrap();
            assert!(sample.iter().all(|channel| channel.is_finite()));
            assert!(sample.iter().all(|channel| (0.0..=1.0).contains(channel)));
        }
    }

    #[test]
    fn neutral_preset_selection_never_aliases_a_user_preset() {
        let starters = 8;
        assert_eq!(selected_user_preset_index(0, starters), None);
        assert_eq!(selected_user_preset_index(1, starters), None);
        assert_eq!(selected_user_preset_index(starters, starters), None);
        assert_eq!(selected_user_preset_index(starters + 1, starters), Some(0));
        assert_eq!(selected_user_preset_index(starters + 3, starters), Some(2));
    }

    #[test]
    fn contextual_chrome_hides_document_tools_on_welcome_but_shows_busy_feedback() {
        let welcome = contextual_chrome(false, false);
        assert!(!welcome.document_actions);
        assert!(!welcome.feedback_bar);
        assert!(!welcome.pipeline_metadata);

        let opening = contextual_chrome(false, true);
        assert!(!opening.document_actions);
        assert!(opening.feedback_bar);
        assert!(!opening.pipeline_metadata);

        let document = contextual_chrome(true, false);
        assert!(document.document_actions);
        assert!(document.feedback_bar);
        assert!(document.pipeline_metadata);
    }

    #[test]
    fn adaptive_canvas_keeps_a_small_natural_request_and_standard_focus_class() {
        assert_eq!((CANVAS_NATURAL_WIDTH, CANVAS_NATURAL_HEIGHT), (320, 240));
        assert_eq!(CREATIVE_FOCUS_CLASS, "creative-focus");
    }

    #[test]
    fn picker_local_history_coalesces_gestures_and_keeps_discrete_edits() {
        let a = DraftColor::new([0.1, 0.2, 0.3]);
        let mut b = a;
        b.linear[0] = 0.2;
        let mut c = b;
        c.linear[0] = 0.3;
        let mut d = c;
        d.linear[0] = 0.4;
        let mut history = PickerLocalHistory::default();
        assert!(history.record(a, b, Some(PickerGesture::Channel(0))));
        assert!(history.record(b, c, Some(PickerGesture::Channel(0))));
        assert_eq!(history.undo, vec![a]);
        history.finish(PickerGesture::Channel(0));
        assert_eq!(history.step(c, false), Some(a));
        assert_eq!(history.step(a, true), Some(c));
        assert!(history.record(c, b, None));
        assert_eq!(history.undo.last(), Some(&c));

        let mut separate = PickerLocalHistory::default();
        assert!(separate.record(a, b, Some(PickerGesture::Channel(0))));
        assert!(separate.record(b, c, Some(PickerGesture::Channel(0))));
        assert_eq!(separate.undo, vec![a], "key repeat stays in one gesture");
        separate.finish(PickerGesture::Channel(0));
        assert!(separate.record(c, d, Some(PickerGesture::Channel(0))));
        separate.finish(PickerGesture::Channel(0));
        assert_eq!(separate.undo, vec![a, c]);
        assert_eq!(separate.step(d, false), Some(c));
        assert_eq!(separate.step(c, false), Some(a));
    }

    #[test]
    fn screenshot_instances_are_non_unique() {
        let mut cli = Cli::default();
        assert_eq!(cli.color_model, ColorModel::Okhsl);
        assert!(application_flags(&cli).is_empty());
        cli.screenshot = Some("evidence.png".into());
        assert!(application_flags(&cli).contains(gio::ApplicationFlags::NON_UNIQUE));
    }

    #[test]
    fn canvas_click_selects_markers_or_adds_a_site() {
        assert_eq!(canvas_site_action(Some(7)), CanvasSiteAction::Select(7));
        assert_eq!(canvas_site_action(None), CanvasSiteAction::Add);
        assert!(marker_hit_test(12.0, 0.0));
        assert!(marker_hit_test(9.0, 9.0));
        assert!(!marker_hit_test(15.0, 0.0));
    }

    #[test]
    fn canvas_split_divider_uses_normalized_clamped_geometry() {
        assert!((divider_from_canvas_x(250.0, 1000) - 0.25).abs() < f64::EPSILON);
        assert_eq!(divider_from_canvas_x(-10.0, 1000), 0.0);
        assert_eq!(divider_from_canvas_x(1010.0, 1000), 1.0);
        assert_eq!(divider_from_canvas_x(10.0, 0), 0.5);
        assert!(split_divider_hit(500.0, 1000, 0.5));
        assert!(split_divider_hit(512.0, 1000, 0.5));
        assert!(!split_divider_hit(513.0, 1000, 0.5));
        assert!(!split_divider_hit(0.0, 0, 0.5));
    }

    #[test]
    fn site_list_uses_creator_labels() {
        assert_eq!(site_label(0), "Site 1");
        assert_eq!(site_label(11), "Site 12");
        assert_eq!(accessible_site_label(0), "Site 1");
    }

    #[test]
    fn unified_preset_labels_have_no_obsolete_mode_suffixes() {
        let user = chromiator::preset::PresetEntry {
            path: std::path::PathBuf::from("user.json"),
            preset: chromiator::preset::Preset::new(
                "My Look",
                None,
                &chromiator::document::Recipe::default(),
            )
            .unwrap(),
        };
        assert_eq!(
            unified_preset_labels(&[user]),
            [
                "Presets…",
                "Ink & Paper",
                "Desert Dusk",
                "Blueprint",
                "Arcade Four",
                "Night Neon",
                "Graphite",
                "Sepia Press",
                "Teal and Tangerine",
                "Moss and Clay",
                "Primary Print",
                "Soft Pastel",
                "Mimeograph",
                "Photocopy",
                "Carbon Copy",
                "Old Newsprint",
                "Two-Color Press",
                "Risograph",
                "Smudged Graphite",
                "Watercolor",
                "My Look",
            ]
        );
    }
}
