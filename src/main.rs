use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::thread;
use std::time::Instant;

use adw::prelude::*;
use crossbeam_channel::{Receiver, Sender, unbounded};
use gdk_pixbuf::Pixbuf;
use gtk::gdk::prelude::GdkCairoContextExt;
use gtk::gdk::prelude::PaintableExt;
use gtk::gdk::prelude::TextureExtManual;
use threshiator::color::{
    ColorModel, DraftColor, PlaneKey, adjust_oklab_plane, display_hex, in_srgb_gamut, oklab_plane,
    oklab_plane_coords, oklab_plane_projected, oklab_to_linear, parse_hex,
};
use threshiator::document::{
    ComponentQuantizer, Document, ExportDefaults, LinkPolicy, Method, PixelImage,
    ProfileInterpretation, Recipe, SampleSize, ThresholdEditGesture, ThresholdEditTarget,
    ThresholdSpace, VoronoiMatching, clamp_threshold_boundary, threshold_display_value,
    threshold_normalized_value,
};
use threshiator::export::{self, ExportFormat};
use threshiator::preset::{Preset, PresetDiagnostic, PresetEntry, PresetStore, apply_to_document};
use threshiator::processing::{
    Coverage, DisplayBuffer, bounded_preview, process_cancellable_with_progress_and_coverage,
    to_display_rgba8,
};
use threshiator::scheduler::{JobCoordinator, JobToken, PreviewScheduler, ProgressTracker};
use threshiator::workflow::{
    OpenKind, SaveResolution, classify_open_path, ensure_project_extension,
    resolve_pending_after_save,
};
use threshiator::{example, project, raster};

#[derive(Clone, Copy, PartialEq, Eq)]
enum CompareMode {
    Result,
    Split,
    Source,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SamplingState {
    AddColor,
    AddSample,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ThresholdComponent {
    Red,
    Green,
    Blue,
    Hue,
    Saturation,
    Value,
}

#[derive(Clone, Default)]
struct Cli {
    open: Option<PathBuf>,
    example: bool,
    method: Option<Method>,
    view: Option<CompareMode>,
    window_size: Option<(i32, i32)>,
    select_group: Option<usize>,
    select_sample: Option<usize>,
    select_site: Option<usize>,
    lock_site: Option<usize>,
    sampling: Option<SamplingState>,
    hue: Option<f32>,
    show_color_picker: bool,
    color_model: ColorModel,
    picker_lightness: Option<f64>,
    threshold_space: Option<ThresholdSpace>,
    threshold_link: Option<LinkPolicy>,
    threshold_component: Option<ThresholdComponent>,
    threshold_bands: Option<usize>,
    threshold_bypass: Option<ThresholdComponent>,
    voronoi_matching: Option<VoronoiMatching>,
    threshold_advanced: bool,
    show_threshold_editor: bool,
    show_preset_dialog: bool,
    threshold_editor_target: Option<ThresholdEditTarget>,
    threshold_editor_handle: Option<ThresholdHandle>,
    screenshot: Option<PathBuf>,
    quit_after_screenshot: bool,
    ui_audit_scenario: Option<String>,
    ui_audit_log: Option<PathBuf>,
}

impl Cli {
    fn parse() -> Self {
        let mut cli = Self::default();
        let mut args = std::env::args().skip(1);
        while let Some(argument) = args.next() {
            match argument.as_str() {
                "--open" | "--project" => cli.open = args.next().map(PathBuf::from),
                "--example" => cli.example = true,
                "--method" => {
                    cli.method = args.next().and_then(|value| match value.as_str() {
                        "voronoi" => Some(Method::Voronoi),
                        "thresholds" => Some(Method::Thresholds),
                        _ => None,
                    })
                }
                "--view" => {
                    cli.view = args.next().and_then(|value| match value.as_str() {
                        "result" => Some(CompareMode::Result),
                        "split" => Some(CompareMode::Split),
                        "source" => Some(CompareMode::Source),
                        _ => None,
                    })
                }
                "--window-size" => {
                    cli.window_size = args.next().and_then(|value| {
                        value
                            .split_once('x')
                            .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
                    })
                }
                "--select-group" => {
                    let value = args.next().and_then(|value| value.parse().ok());
                    cli.select_group = value;
                    cli.select_site = value;
                }
                "--select-sample" => {
                    let value = args.next().and_then(|value| value.parse().ok());
                    cli.select_sample = value;
                    cli.select_site = value;
                }
                "--select-site" => {
                    cli.select_site = args.next().and_then(|value| value.parse().ok())
                }
                "--lock-site" => cli.lock_site = args.next().and_then(|value| value.parse().ok()),
                "--sampling-state" => {
                    cli.sampling = args.next().and_then(|value| match value.as_str() {
                        "add-color" => Some(SamplingState::AddColor),
                        "add-sample" => Some(SamplingState::AddSample),
                        _ => None,
                    })
                }
                "--hue" => cli.hue = args.next().and_then(|value| value.parse().ok()),
                "--threshold-space" => {
                    cli.threshold_space = args.next().and_then(|value| match value.as_str() {
                        "rgb" => Some(ThresholdSpace::Rgb),
                        "hsv" => Some(ThresholdSpace::Hsv),
                        _ => None,
                    })
                }
                "--threshold-link" => {
                    cli.threshold_link = args.next().and_then(|value| match value.as_str() {
                        "linked" => Some(LinkPolicy::Linked),
                        "independent" => Some(LinkPolicy::Independent),
                        _ => None,
                    })
                }
                "--threshold-component" => {
                    cli.threshold_component = args.next().and_then(parse_threshold_component)
                }
                "--threshold-bands" => {
                    cli.threshold_bands = args
                        .next()
                        .and_then(|value| value.parse::<usize>().ok())
                        .map(|value| value.clamp(2, 32))
                }
                "--threshold-bypass" => {
                    cli.threshold_bypass = args.next().and_then(parse_threshold_component)
                }
                "--voronoi-matching" => {
                    cli.voronoi_matching = args.next().and_then(|value| match value.as_str() {
                        "perceptual" => Some(VoronoiMatching::Perceptual),
                        "rgb" => Some(VoronoiMatching::Rgb),
                        "hsv" => Some(VoronoiMatching::Hsv),
                        _ => None,
                    })
                }
                "--threshold-advanced" => cli.threshold_advanced = true,
                "--show-threshold-editor" => cli.show_threshold_editor = true,
                "--show-preset-dialog" => cli.show_preset_dialog = true,
                "--threshold-editor-target" => {
                    cli.threshold_editor_target = args.next().and_then(parse_threshold_edit_target)
                }
                "--threshold-editor-handle" => {
                    cli.threshold_editor_handle = args.next().and_then(parse_threshold_handle)
                }
                "--show-color-picker" => cli.show_color_picker = true,
                "--picker-lightness" => {
                    cli.picker_lightness = args
                        .next()
                        .and_then(|value| value.parse::<f64>().ok())
                        .map(|value| value.clamp(0.0, 1.0))
                }
                "--color-model" => {
                    cli.color_model = args
                        .next()
                        .and_then(|value| match value.as_str() {
                            "hsv" => Some(ColorModel::Hsv),
                            "hsl" => Some(ColorModel::Hsl),
                            "oklab" => Some(ColorModel::Oklab),
                            _ => None,
                        })
                        .unwrap_or_default()
                }
                "--screenshot" => cli.screenshot = args.next().map(PathBuf::from),
                "--quit-after-screenshot" => cli.quit_after_screenshot = true,
                "--ui-audit-scenario" => cli.ui_audit_scenario = args.next(),
                "--ui-audit-log" => cli.ui_audit_log = args.next().map(PathBuf::from),
                _ => {}
            }
        }
        cli
    }
}

fn parse_threshold_edit_target(value: String) -> Option<ThresholdEditTarget> {
    match value.as_str() {
        "red" => Some(ThresholdEditTarget::Red),
        "green" => Some(ThresholdEditTarget::Green),
        "blue" => Some(ThresholdEditTarget::Blue),
        "rgb" | "linked-rgb" => Some(ThresholdEditTarget::LinkedRgb),
        "hue" => Some(ThresholdEditTarget::Hue),
        "saturation" => Some(ThresholdEditTarget::Saturation),
        "value" => Some(ThresholdEditTarget::Value),
        "sv" | "saturation-value" | "linked-sv" => Some(ThresholdEditTarget::LinkedSaturationValue),
        _ => None,
    }
}

fn parse_threshold_handle(value: String) -> Option<ThresholdHandle> {
    let (kind, index) = value.split_once(':')?;
    let index = index.parse::<usize>().ok()?.checked_sub(1)?;
    match kind {
        "boundary" => Some(ThresholdHandle::Boundary(index)),
        "output" => Some(ThresholdHandle::Output(index)),
        _ => None,
    }
}

fn threshold_handle_label(handle: ThresholdHandle) -> String {
    match handle {
        ThresholdHandle::Boundary(index) => format!("boundary:{}", index + 1),
        ThresholdHandle::Output(index) => format!("output:{}", index + 1),
    }
}

fn parse_threshold_component(value: String) -> Option<ThresholdComponent> {
    match value.as_str() {
        "red" => Some(ThresholdComponent::Red),
        "green" => Some(ThresholdComponent::Green),
        "blue" => Some(ThresholdComponent::Blue),
        "hue" => Some(ThresholdComponent::Hue),
        "saturation" => Some(ThresholdComponent::Saturation),
        "value" => Some(ThresholdComponent::Value),
        _ => None,
    }
}

fn apply_cli_recipe(cli: &Cli, recipe: &mut Recipe) {
    if let Some(space) = cli.threshold_space {
        recipe.threshold.active_space = space;
    }
    if let Some(link) = cli.threshold_link {
        match recipe.threshold.active_space {
            ThresholdSpace::Rgb => recipe.threshold.rgb_state.link = link,
            ThresholdSpace::Hsv => recipe.threshold.hsv_state.sv_link = link,
        }
    }
    if let (Some(component), Some(bands)) = (cli.threshold_component, cli.threshold_bands) {
        let edited = ComponentQuantizer::evenly_spaced(bands);
        match component {
            ThresholdComponent::Red => recipe.threshold.set_rgb_component(0, edited),
            ThresholdComponent::Green => recipe.threshold.set_rgb_component(1, edited),
            ThresholdComponent::Blue => recipe.threshold.set_rgb_component(2, edited),
            ThresholdComponent::Hue => recipe.threshold.set_hsv_component(0, edited),
            ThresholdComponent::Saturation => recipe.threshold.set_hsv_component(1, edited),
            ThresholdComponent::Value => recipe.threshold.set_hsv_component(2, edited),
        }
    }
    if let Some(component) = cli.threshold_bypass {
        match component {
            ThresholdComponent::Red => recipe.threshold.rgb_state.components[0].enabled = false,
            ThresholdComponent::Green => recipe.threshold.rgb_state.components[1].enabled = false,
            ThresholdComponent::Blue => recipe.threshold.rgb_state.components[2].enabled = false,
            ThresholdComponent::Hue => recipe.threshold.hsv_state.hue.enabled = false,
            ThresholdComponent::Saturation => recipe.threshold.hsv_state.saturation.enabled = false,
            ThresholdComponent::Value => recipe.threshold.hsv_state.value.enabled = false,
        }
    }
    if let Some(matching) = cli.voronoi_matching {
        recipe.voronoi.matching = matching;
    }
}

type PreparedDocument = (
    Document,
    Option<PathBuf>,
    DocumentKind,
    PixelImage,
    DisplayBuffer,
    DisplayBuffer,
    Coverage,
);
type OpenResult = Result<PreparedDocument, String>;

enum Work {
    Progress(u64, f64, &'static str),
    Open(u64, DocumentKind, Box<OpenResult>),
    Save(u64, Result<PathBuf, String>),
    Export(u64, Result<PathBuf, String>),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum DocumentKind {
    #[default]
    Welcome,
    Image,
    Example,
    Project,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Replacement {
    OpenImage,
    OpenProject,
    Example,
}

struct State {
    document: Option<Document>,
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
    threshold_editor_visible: bool,
    threshold_editor_target: Option<ThresholdEditTarget>,
    threshold_editor_handle: Option<ThresholdHandle>,
    mode: CompareMode,
    divider: f64,
    scheduler: PreviewScheduler,
    sender: Sender<Work>,
    receiver: Receiver<Work>,
    jobs: JobCoordinator,
    progress: ProgressTracker,
    selected_group: Option<u64>,
    selected_sample: Option<u64>,
    sampling: Option<SamplingState>,
    sampling_previous_mode: Option<CompareMode>,
    preset_entries: Vec<PresetEntry>,
    preset_diagnostics: Vec<PresetDiagnostic>,
}

impl State {
    fn new() -> Self {
        let (sender, receiver) = unbounded();
        Self {
            document: None,
            result: None,
            preview_source: None,
            coverage: Coverage::default(),
            source_pixbuf: None,
            result_pixbuf: None,
            project_path: None,
            document_kind: DocumentKind::Welcome,
            pending_replacement: None,
            picker_visible: false,
            picker_model: ColorModel::Hsv,
            picker_lightness: None,
            picker_lightness_updates: 0,
            picker_lightness_elapsed_ms: None,
            picker_plane_render_max_us: 0,
            picker_plane_render_count: 0,
            threshold_editor_visible: false,
            threshold_editor_target: None,
            threshold_editor_handle: None,
            mode: CompareMode::Result,
            divider: 0.5,
            scheduler: PreviewScheduler::new(),
            sender,
            receiver,
            jobs: JobCoordinator::default(),
            progress: ProgressTracker::default(),
            selected_group: None,
            selected_sample: None,
            sampling: None,
            sampling_previous_mode: None,
            preset_entries: Vec::new(),
            preset_diagnostics: Vec::new(),
        }
    }
}

struct Ui {
    window: adw::ApplicationWindow,
    inspector_split: adw::OverlaySplitView,
    stack: gtk::Stack,
    canvas: gtk::DrawingArea,
    status: gtk::Label,
    progress: gtk::ProgressBar,
    cancel: gtk::Button,
    save: gtk::Button,
    save_as: gtk::Button,
    open: gtk::Button,
    empty_open: gtk::Button,
    empty_project: gtk::Button,
    empty_example: gtk::Button,
    export: gtk::Button,
    hue: gtk::Adjustment,
    low: gtk::SpinButton,
    high: gtk::SpinButton,
    out_low: gtk::SpinButton,
    out_mid: gtk::SpinButton,
    out_high: gtk::SpinButton,
    threshold_space: gtk::DropDown,
    threshold_link: gtk::Switch,
    threshold_link_row: adw::ActionRow,
    threshold_process: Vec<gtk::Switch>,
    threshold_bands: Vec<gtk::SpinButton>,
    threshold_editor_button: adw::ActionRow,
    threshold_editor_refresh: RefCell<Option<Rc<dyn Fn()>>>,
    threshold_editor_dialog: RefCell<Option<adw::Dialog>>,
    picker_dialog: RefCell<Option<adw::Dialog>>,
    threshold_component_rows: Vec<adw::ActionRow>,
    voronoi_matching: gtk::DropDown,
    method_voronoi: gtk::ToggleButton,
    method_thresholds: gtk::ToggleButton,
    voronoi_panel: adw::PreferencesGroup,
    voronoi_list_panel: adw::PreferencesGroup,
    thresholds_panel: adw::PreferencesGroup,
    groups: gtk::ListBox,
    add_color: gtk::Button,
    add_sample: gtk::Button,
    source_mode: gtk::ToggleButton,
    result_mode: gtk::ToggleButton,
    split_mode: gtk::ToggleButton,
    sidebar_button: gtk::ToggleButton,
    divider: gtk::Scale,
    influence: gtk::SpinButton,
    sample_size: gtk::DropDown,
    delete_sample: gtk::Button,
    delete_group: gtk::Button,
    sample_info: adw::ActionRow,
    output_picker: gtk::Button,
    output_swatch: gtk::DrawingArea,
    source_picker: gtk::Button,
    source_swatch: gtk::DrawingArea,
    site_lock: gtk::Switch,
    preset_dropdown: gtk::DropDown,
    preset_model: gtk::StringList,
    preset_info: adw::ActionRow,
    preset_apply: gtk::Button,
    preset_save: gtk::Button,
    preset_refresh: gtk::Button,
    preset_folder: gtk::Button,
    sample_selector: gtk::DropDown,
    sample_selector_model: gtk::StringList,
    syncing: Cell<bool>,
    cli: Cli,
    snapshot_root: adw::ToolbarView,
    audit_started: Cell<bool>,
    audit_threshold_target: RefCell<Option<gtk::DropDown>>,
    audit_threshold_precise: RefCell<Option<gtk::SpinButton>>,
    audit_threshold_cancel: RefCell<Option<gtk::Button>>,
    audit_threshold_done: RefCell<Option<gtk::Button>>,
    audit_picker_model: RefCell<Option<gtk::DropDown>>,
    audit_picker_hex: RefCell<Option<gtk::Entry>>,
    audit_picker_channels: RefCell<Vec<gtk::Adjustment>>,
    audit_picker_cancel: RefCell<Option<gtk::Button>>,
    audit_picker_select: RefCell<Option<gtk::Button>>,
    audit_preset_name: RefCell<Option<gtk::Entry>>,
    audit_preset_cancel: RefCell<Option<gtk::Button>>,
    audit_preset_save: RefCell<Option<gtk::Button>>,
}

struct InspectorControls {
    root: gtk::ScrolledWindow,
    hue: gtk::Adjustment,
    low: gtk::SpinButton,
    high: gtk::SpinButton,
    out_low: gtk::SpinButton,
    out_mid: gtk::SpinButton,
    out_high: gtk::SpinButton,
    threshold_space: gtk::DropDown,
    threshold_link: gtk::Switch,
    threshold_link_row: adw::ActionRow,
    threshold_process: Vec<gtk::Switch>,
    threshold_bands: Vec<gtk::SpinButton>,
    threshold_editor_button: adw::ActionRow,
    threshold_component_rows: Vec<adw::ActionRow>,
    voronoi_matching: gtk::DropDown,
    method_voronoi: gtk::ToggleButton,
    method_thresholds: gtk::ToggleButton,
    voronoi_panel: adw::PreferencesGroup,
    voronoi_list_panel: adw::PreferencesGroup,
    thresholds_panel: adw::PreferencesGroup,
    groups: gtk::ListBox,
    add_color: gtk::Button,
    add_sample: gtk::Button,
    influence: gtk::SpinButton,
    sample_size: gtk::DropDown,
    delete_sample: gtk::Button,
    delete_group: gtk::Button,
    sample_info: adw::ActionRow,
    output_picker: gtk::Button,
    output_swatch: gtk::DrawingArea,
    source_picker: gtk::Button,
    source_swatch: gtk::DrawingArea,
    site_lock: gtk::Switch,
    preset_dropdown: gtk::DropDown,
    preset_model: gtk::StringList,
    preset_info: adw::ActionRow,
    preset_apply: gtk::Button,
    preset_save: gtk::Button,
    preset_refresh: gtk::Button,
    preset_folder: gtk::Button,
    sample_selector: gtk::DropDown,
    sample_selector_model: gtk::StringList,
}

fn main() -> glib::ExitCode {
    let cli = Cli::parse();
    if cli.screenshot.is_some() && std::env::var_os("GSK_RENDERER").is_none() {
        // Command-driven evidence is rendered deterministically before GTK starts.
        unsafe { std::env::set_var("GSK_RENDERER", "cairo") };
    }
    let app = adw::Application::builder()
        .application_id("io.github.threshiator.Threshiator")
        .flags(application_flags(&cli))
        .build();
    app.connect_activate(move |app| build(app, cli.clone()));
    app.run_with_args::<&str>(&[])
}

fn application_flags(cli: &Cli) -> gio::ApplicationFlags {
    if cli.screenshot.is_some() || cli.ui_audit_scenario.is_some() {
        gio::ApplicationFlags::NON_UNIQUE
    } else {
        gio::ApplicationFlags::empty()
    }
}

fn build(app: &adw::Application, cli: Cli) {
    let state = Rc::new(RefCell::new(State::new()));
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Threshiator")
        .default_width(1180)
        .default_height(760)
        .build();
    if let Some((width, height)) = cli.window_size {
        window.set_default_size(width, height);
    }
    let open = icon_button("document-open-symbolic", "Open image or project (Ctrl+O)");
    let save = icon_button("document-save-symbolic", "Save project (Ctrl+S)");
    save.set_sensitive(false);
    let save_as = icon_button(
        "document-save-as-symbolic",
        "Save project as (Ctrl+Shift+S)",
    );
    save_as.set_sensitive(false);
    let export = icon_button(
        "document-send-symbolic",
        "Export full-resolution image (Ctrl+Shift+E)",
    );
    export.add_css_class("suggested-action");
    export.set_sensitive(false);
    let sidebar_button = gtk::ToggleButton::builder()
        .icon_name("sidebar-show-symbolic")
        .tooltip_text("Show adjustments")
        .active(true)
        .build();
    let header = adw::HeaderBar::new();
    header.pack_start(&open);
    header.pack_start(&save);
    header.pack_start(&save_as);
    header.pack_end(&export);
    header.pack_end(&sidebar_button);
    header.set_title_widget(Some(&adw::WindowTitle::new(
        "Threshiator",
        "Precision posterization",
    )));

    let canvas = gtk::DrawingArea::builder()
        .hexpand(true)
        .vexpand(true)
        .content_width(640)
        .content_height(480)
        .focusable(true)
        .build();
    canvas.update_property(&[gtk::accessible::Property::Label("Image comparison")]);
    canvas.set_accessible_role(gtk::AccessibleRole::Img);
    canvas.set_tooltip_text(Some("Processed image comparison surface"));
    let divider = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.01);
    divider.set_value(0.5);
    divider.set_hexpand(true);
    divider.set_visible(false);
    divider.update_property(&[gtk::accessible::Property::Label("Comparison divider")]);
    let result_mode = gtk::ToggleButton::with_label("Result");
    let split_mode = gtk::ToggleButton::with_label("Split");
    let source_mode = gtk::ToggleButton::with_label("Source");
    split_mode.set_group(Some(&result_mode));
    source_mode.set_group(Some(&result_mode));
    result_mode.set_active(true);
    let modes = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    modes.add_css_class("linked");
    modes.set_halign(gtk::Align::Center);
    modes.append(&result_mode);
    modes.append(&split_mode);
    modes.append(&source_mode);
    let viewer = gtk::Box::new(gtk::Orientation::Vertical, 8);
    viewer.set_margin_top(12);
    viewer.set_margin_bottom(12);
    viewer.set_margin_start(12);
    viewer.set_margin_end(12);
    viewer.append(&modes);
    viewer.append(&canvas);
    viewer.append(&divider);

    let InspectorControls {
        root: inspector,
        hue,
        low,
        high,
        out_low,
        out_mid,
        out_high,
        threshold_space,
        threshold_link,
        threshold_link_row,
        threshold_process,
        threshold_bands,
        threshold_editor_button,
        threshold_component_rows,
        voronoi_matching,
        method_voronoi,
        method_thresholds,
        voronoi_panel,
        voronoi_list_panel,
        thresholds_panel,
        groups,
        add_color,
        add_sample,
        influence,
        sample_size,
        delete_sample,
        delete_group,
        sample_info,
        output_picker,
        output_swatch,
        source_picker,
        source_swatch,
        site_lock,
        preset_dropdown,
        preset_model,
        preset_info,
        preset_apply,
        preset_save,
        preset_refresh,
        preset_folder,
        sample_selector,
        sample_selector_model,
    } = inspector();
    let split = adw::OverlaySplitView::new();
    split.set_content(Some(&viewer));
    split.set_sidebar(Some(&inspector));
    split.set_min_sidebar_width(280.0);
    split.set_max_sidebar_width(360.0);
    split.set_sidebar_width_fraction(0.3);
    {
        let modes = modes.clone();
        split.connect_collapsed_notify(move |split| {
            modes.set_halign(if split.is_collapsed() {
                gtk::Align::End
            } else {
                gtk::Align::Center
            });
        });
    }
    let empty_button = gtk::Button::with_label("Open Image");
    empty_button.add_css_class("suggested-action");
    let empty_project = gtk::Button::with_label("Open Threshiator Project");
    let empty_example = gtk::Button::with_label("Try Spectrum Example");
    empty_example.add_css_class("flat");
    let welcome_actions = gtk::Box::new(gtk::Orientation::Vertical, 8);
    welcome_actions.set_halign(gtk::Align::Center);
    welcome_actions.append(&empty_button);
    welcome_actions.append(&empty_project);
    welcome_actions.append(&empty_example);
    let hero_texture = gtk::gdk::Texture::for_pixbuf(&embedded_example_pixbuf());
    let hero = gtk::Picture::for_paintable(&hero_texture);
    hero.set_content_fit(gtk::ContentFit::Cover);
    hero.set_size_request(560, 220);
    hero.set_can_shrink(true);
    hero.add_css_class("card");
    hero.update_property(&[gtk::accessible::Property::Label(
        "Spectrum Breakpoint example artwork",
    )]);
    let welcome_title = gtk::Label::builder()
        .label("Make color relationships visible")
        .css_classes(["title-1"])
        .wrap(true)
        .justify(gtk::Justification::Center)
        .build();
    let welcome_copy = gtk::Label::builder()
        .label("Threshiator is a precision posterization studio by Ric Perry. Begin with your own image, resume a saved project, or explore the included Spectrum example.")
        .wrap(true)
        .justify(gtk::Justification::Center)
        .max_width_chars(64)
        .css_classes(["dim-label"])
        .build();
    let welcome_box = gtk::Box::new(gtk::Orientation::Vertical, 16);
    welcome_box.set_halign(gtk::Align::Center);
    welcome_box.set_valign(gtk::Align::Center);
    welcome_box.set_margin_top(24);
    welcome_box.set_margin_bottom(24);
    welcome_box.set_margin_start(24);
    welcome_box.set_margin_end(24);
    welcome_box.append(&hero);
    welcome_box.append(&welcome_title);
    welcome_box.append(&welcome_copy);
    welcome_box.append(&welcome_actions);
    let empty = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .child(&welcome_box)
        .build();
    let stack = gtk::Stack::new();
    stack.add_named(&empty, Some("empty"));
    stack.add_named(&split, Some("document"));
    stack.set_visible_child_name("empty");
    let progress = gtk::ProgressBar::new();
    progress.set_size_request(180, -1);
    progress.set_visible(false);
    progress.update_property(&[
        gtk::accessible::Property::Label("Operation progress"),
        gtk::accessible::Property::Description(
            "Progress for the open, save, or export phase named in the adjacent status message",
        ),
    ]);
    let cancel = gtk::Button::with_label("Cancel");
    cancel.set_visible(false);
    let status = gtk::Label::builder()
        .label("Ready")
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    let status_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    status_box.set_margin_start(12);
    status_box.set_margin_end(12);
    status_box.set_margin_top(6);
    status_box.set_margin_bottom(6);
    status_box.append(&status);
    status_box.append(&progress);
    status_box.append(&cancel);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&stack);
    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    content.append(&status_box);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_css_class("background");
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&content));
    window.set_content(Some(&toolbar));
    let ui = Rc::new(Ui {
        window: window.clone(),
        inspector_split: split.clone(),
        stack,
        canvas: canvas.clone(),
        status,
        progress,
        cancel,
        save: save.clone(),
        save_as: save_as.clone(),
        open: open.clone(),
        empty_open: empty_button.clone(),
        empty_project: empty_project.clone(),
        empty_example: empty_example.clone(),
        export: export.clone(),
        hue,
        low,
        high,
        out_low,
        out_mid,
        out_high,
        threshold_space,
        threshold_link,
        threshold_link_row,
        threshold_process,
        threshold_bands,
        threshold_editor_button,
        threshold_editor_refresh: RefCell::new(None),
        threshold_editor_dialog: RefCell::new(None),
        picker_dialog: RefCell::new(None),
        threshold_component_rows,
        voronoi_matching,
        method_voronoi,
        method_thresholds,
        voronoi_panel,
        voronoi_list_panel,
        thresholds_panel,
        groups,
        add_color,
        add_sample,
        source_mode: source_mode.clone(),
        result_mode: result_mode.clone(),
        split_mode: split_mode.clone(),
        sidebar_button: sidebar_button.clone(),
        divider: divider.clone(),
        influence,
        sample_size,
        delete_sample,
        delete_group,
        sample_info,
        output_picker,
        output_swatch,
        source_picker,
        source_swatch,
        site_lock,
        preset_dropdown,
        preset_model,
        preset_info,
        preset_apply,
        preset_save,
        preset_refresh,
        preset_folder,
        sample_selector,
        sample_selector_model,
        syncing: Cell::new(false),
        cli: cli.clone(),
        snapshot_root: toolbar.clone(),
        audit_started: Cell::new(false),
        audit_threshold_target: RefCell::new(None),
        audit_threshold_precise: RefCell::new(None),
        audit_threshold_cancel: RefCell::new(None),
        audit_threshold_done: RefCell::new(None),
        audit_picker_model: RefCell::new(None),
        audit_picker_hex: RefCell::new(None),
        audit_picker_channels: RefCell::new(Vec::new()),
        audit_picker_cancel: RefCell::new(None),
        audit_picker_select: RefCell::new(None),
        audit_preset_name: RefCell::new(None),
        audit_preset_cancel: RefCell::new(None),
        audit_preset_save: RefCell::new(None),
    });
    canvas_draw(&canvas, state.clone());
    recipe_controls(&ui, state.clone());
    preset_controls(&ui, state.clone());
    group_selection(&ui, state.clone());
    canvas_sampling(&ui, state.clone());
    replacement_handler(&open, Replacement::OpenImage, &ui, state.clone());
    replacement_handler(&empty_button, Replacement::OpenImage, &ui, state.clone());
    replacement_handler(&empty_project, Replacement::OpenProject, &ui, state.clone());
    replacement_handler(&empty_example, Replacement::Example, &ui, state.clone());
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
    mode_handler(
        &result_mode,
        CompareMode::Result,
        &divider,
        &canvas,
        state.clone(),
    );
    mode_handler(
        &split_mode,
        CompareMode::Split,
        &divider,
        &canvas,
        state.clone(),
    );
    mode_handler(
        &source_mode,
        CompareMode::Source,
        &divider,
        &canvas,
        state.clone(),
    );
    let split_toggle = split.clone();
    sidebar_button.connect_toggled(move |b| split_toggle.set_show_sidebar(b.is_active()));
    let breakpoint = adw::Breakpoint::new(
        adw::BreakpointCondition::parse("max-width: 760px").expect("valid breakpoint"),
    );
    let collapsed = true.to_value();
    breakpoint.add_setter(&split, "collapsed", Some(&collapsed));
    window.add_breakpoint(breakpoint);
    match cli.view.or_else(
        || match std::env::var("THRESHIATOR_AUTOMATION_MODE").as_deref() {
            Ok("split") => Some(CompareMode::Split),
            Ok("source") => Some(CompareMode::Source),
            _ => None,
        },
    ) {
        Some(CompareMode::Split) => split_mode.set_active(true),
        Some(CompareMode::Source) => source_mode.set_active(true),
        _ => result_mode.set_active(true),
    }
    if std::env::var_os("THRESHIATOR_AUTOMATION_NARROW").is_some() {
        window.set_default_size(720, 700);
        split.set_show_sidebar(false);
        sidebar_button.set_active(false);
    }
    if cli.window_size.is_some_and(|(width, _)| width <= 760) {
        split.set_show_sidebar(false);
        sidebar_button.set_active(false);
    }
    actions(app, &open, &save, &save_as, &export);
    close_guard(&window, state.clone());
    poll(&ui, state.clone());
    let cli_has_input = cli.example
        || cli.open.is_some()
        || std::env::var_os("THRESHIATOR_AUTOMATION_IMAGE").is_some();
    if cli.example {
        start_example_open(&ui, &state);
    } else if let Some(path) = cli
        .open
        .clone()
        .or_else(|| std::env::var_os("THRESHIATOR_AUTOMATION_IMAGE").map(PathBuf::from))
    {
        let automation_hue = cli.hue.or_else(|| {
            std::env::var("THRESHIATOR_AUTOMATION_HUE")
                .ok()
                .and_then(|value| value.parse::<f32>().ok())
        });
        let automation_method = cli.method;
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
                if let Some(method) = automation_method {
                    document.recipe.active_method = method;
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
        split.set_show_sidebar(true);
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
    document.recipe.voronoi = threshiator::voronoi::auto_initialize(&proxy, &document.source);
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
    if ui.threshold_editor_dialog.borrow().is_some() || ui.picker_dialog.borrow().is_some() {
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
    ui.status.set_label(label);
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
    let state = state.borrow();
    let has_document = state.document.is_some();
    let dirty = state
        .document
        .as_ref()
        .is_some_and(|document| document.dirty);
    ui.save.set_sensitive(dirty);
    ui.save_as.set_sensitive(has_document);
    ui.export.set_sensitive(has_document);
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
    gtk::Button::builder()
        .icon_name(icon)
        .tooltip_text(tip)
        .build()
}

fn inspector() -> InspectorControls {
    let page = adw::PreferencesPage::new();
    let method = adw::PreferencesGroup::builder()
        .title("Method")
        .description("Choose how source colors become output colors")
        .build();
    let method_row = adw::ActionRow::new();
    let method_voronoi = gtk::ToggleButton::with_label("Voronoi");
    let method_thresholds = gtk::ToggleButton::with_label("Thresholds");
    method_thresholds.set_group(Some(&method_voronoi));
    method_voronoi.set_active(true);
    let segmented = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    segmented.add_css_class("linked");
    segmented.append(&method_voronoi);
    segmented.append(&method_thresholds);
    method_row.add_suffix(&segmented);
    method.add(&method_row);
    page.add(&method);
    let presets = adw::PreferencesGroup::builder()
        .title("Presets")
        .description("Save or apply reusable processing recipes")
        .build();
    let preset_model = gtk::StringList::new(&[]);
    let preset_dropdown = gtk::DropDown::new(Some(preset_model.clone()), None::<gtk::Expression>);
    preset_dropdown.set_hexpand(true);
    preset_dropdown.update_property(&[gtk::accessible::Property::Label("Preset selection")]);
    let preset_info = adw::ActionRow::builder()
        .title("Available presets")
        .subtitle("Refresh to scan the preset folder")
        .build();
    preset_info.add_suffix(&preset_dropdown);
    presets.add(&preset_info);
    let preset_actions = adw::ActionRow::new();
    let preset_apply = gtk::Button::with_label("Apply");
    let preset_save = gtk::Button::with_label("Save Current…");
    let preset_refresh = icon_button("view-refresh-symbolic", "Refresh presets");
    let preset_folder = icon_button("folder-open-symbolic", "Open Presets Folder");
    preset_refresh.update_property(&[gtk::accessible::Property::Label("Refresh presets")]);
    preset_folder.update_property(&[gtk::accessible::Property::Label("Open presets folder")]);
    preset_actions.add_prefix(&preset_apply);
    preset_actions.add_prefix(&preset_save);
    preset_actions.add_suffix(&preset_refresh);
    preset_actions.add_suffix(&preset_folder);
    presets.add(&preset_actions);
    page.add(&presets);
    let voronoi = adw::PreferencesGroup::builder()
        .title("Selected site")
        .description("Edit the Source center and the Target color applied to its cell")
        .build();
    let voronoi_list = adw::PreferencesGroup::builder()
        .title("Color sites")
        .description("Each independent site owns one Source center and one Target color")
        .build();
    let voronoi_matching = gtk::DropDown::new(
        Some(gtk::StringList::new(&[
            "Perceptual (OKLab)",
            "RGB (sRGB)",
            "HSV",
        ])),
        None::<gtk::Expression>,
    );
    let matching_row = adw::ActionRow::builder()
        .title("Color matching")
        .subtitle("Choose how source colors are compared")
        .build();
    matching_row.add_suffix(&voronoi_matching);
    voronoi_list.add(&matching_row);
    let groups = gtk::ListBox::new();
    groups.set_selection_mode(gtk::SelectionMode::Single);
    groups.update_property(&[gtk::accessible::Property::Label("Voronoi site list")]);
    let group_scroll = gtk::ScrolledWindow::builder()
        .child(&groups)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .min_content_height(168)
        .max_content_height(168)
        .propagate_natural_height(false)
        .build();
    voronoi_list.add(&group_scroll);
    page.add(&voronoi_list);
    let actions = adw::ActionRow::new();
    let add_color = gtk::Button::with_label("Add Site");
    let add_sample = gtk::Button::with_label("Reattach / Resample Source");
    actions.add_prefix(&add_color);
    actions.add_suffix(&add_sample);
    voronoi.add(&actions);
    let source_swatch = gtk::DrawingArea::builder()
        .content_width(42)
        .content_height(28)
        .build();
    let source_picker = gtk::Button::builder().child(&source_swatch).build();
    source_picker.update_property(&[gtk::accessible::Property::Label(
        "Selected site Source color",
    )]);
    let source_row = adw::ActionRow::builder()
        .title("Source")
        .subtitle("Changing Source resets Target and detaches the image marker")
        .build();
    source_row.add_suffix(&source_picker);
    voronoi.add(&source_row);
    let output_swatch = gtk::DrawingArea::builder()
        .content_width(42)
        .content_height(28)
        .build();
    let output_picker = gtk::Button::builder().child(&output_swatch).build();
    output_picker.update_property(&[gtk::accessible::Property::Label(
        "Selected site Target color",
    )]);
    let picker_row = adw::ActionRow::builder()
        .title("Target")
        .subtitle("Color applied to pixels assigned to this site")
        .build();
    picker_row.add_suffix(&output_picker);
    voronoi.add(&picker_row);
    let site_lock = gtk::Switch::new();
    site_lock.set_valign(gtk::Align::Center);
    site_lock.update_property(&[gtk::accessible::Property::Label("Lock selected site")]);
    let lock_row = adw::ActionRow::builder()
        .title("Lock site")
        .subtitle("Protect Source, Target, Influence, marker, and deletion")
        .build();
    lock_row.add_suffix(&site_lock);
    lock_row.set_activatable_widget(Some(&site_lock));
    voronoi.add(&lock_row);
    let sample_selector_model = gtk::StringList::new(&[]);
    let sample_selector =
        gtk::DropDown::new(Some(sample_selector_model.clone()), None::<gtk::Expression>);
    sample_selector.set_hexpand(false);
    sample_selector.set_halign(gtk::Align::End);
    sample_selector.set_width_request(112);
    sample_selector.set_tooltip_text(Some(
        "Select a Source sample; its ID and normalized position appear in Advanced",
    ));
    sample_selector.update_property(&[gtk::accessible::Property::Label("Source sample selector")]);
    let selector_row = adw::ActionRow::builder()
        .title("Source sample")
        .subtitle("Select a sample to edit or locate its marker")
        .build();
    selector_row.add_suffix(&sample_selector);
    voronoi.add(&selector_row);
    selector_row.set_visible(false);
    let advanced = adw::ExpanderRow::builder()
        .title("Site details")
        .subtitle("Influence, sampling footprint, and marker attachment")
        .build();
    let influence_adjustment = gtk::Adjustment::new(0.0, -4.0, 4.0, 0.1, 0.5, 0.0);
    let influence = gtk::SpinButton::new(Some(&influence_adjustment), 0.1, 2);
    influence.update_property(&[gtk::accessible::Property::Label("Selected site Influence")]);
    let influence_row = adw::ActionRow::builder()
        .title("Influence")
        .subtitle("Resolution-independent; neutral is 0")
        .build();
    influence_row.add_suffix(&influence);
    advanced.add_row(&influence_row);
    let sizes = gtk::StringList::new(&["Point", "3×3", "5×5"]);
    let sample_size = gtk::DropDown::new(Some(sizes), None::<gtk::Expression>);
    sample_size.update_property(&[gtk::accessible::Property::Label("Site sampling footprint")]);
    let size_row = adw::ActionRow::builder()
        .title("Sampling footprint")
        .subtitle("Resamples authoritative full-resolution pixels")
        .build();
    size_row.add_suffix(&sample_size);
    advanced.add_row(&size_row);
    let sample_info = adw::ActionRow::builder()
        .title("Marker")
        .subtitle("Select a site or marker")
        .build();
    advanced.add_row(&sample_info);
    let delete_row = adw::ActionRow::new();
    let delete_sample = gtk::Button::with_label("Delete Site…");
    let delete_group = gtk::Button::with_label("Delete Group…");
    delete_group.add_css_class("destructive-action");
    delete_row.add_prefix(&delete_sample);
    delete_row.add_suffix(&delete_group);
    delete_group.set_visible(false);
    advanced.add_row(&delete_row);
    voronoi.add(&advanced);
    page.add(&voronoi);
    let group = adw::PreferencesGroup::builder()
        .title("Threshold controls")
        .description(
            "Bands map encoded sRGB or HSV components; boundaries belong to the lower band.",
        )
        .build();
    let threshold_space = gtk::DropDown::new(
        Some(gtk::StringList::new(&["RGB", "HSV"])),
        None::<gtk::Expression>,
    );
    let space_row = adw::ActionRow::builder()
        .title("Working space")
        .subtitle("Switching restores that space's prior edits")
        .build();
    space_row.add_suffix(&threshold_space);
    group.add(&space_row);
    let threshold_link = gtk::Switch::new();
    threshold_link.set_active(true);
    threshold_link.set_valign(gtk::Align::Center);
    let threshold_link_row = adw::ActionRow::builder()
        .title("Link RGB")
        .subtitle("Copies future band edits; Process/Bypass remains independent")
        .build();
    threshold_link_row.add_suffix(&threshold_link);
    group.add(&threshold_link_row);
    let mut threshold_process = Vec::new();
    let mut threshold_bands = Vec::new();
    let mut threshold_component_rows = Vec::new();
    for title in ["Red", "Green", "Blue"] {
        let action = adw::ActionRow::builder()
            .title(title)
            .subtitle("Process • 3 Bands")
            .build();
        let process = gtk::Switch::new();
        process.set_active(true);
        process.set_valign(gtk::Align::Center);
        process.set_tooltip_text(Some("Process this component; off means Bypass"));
        let bands = gtk::SpinButton::with_range(2.0, 32.0, 1.0);
        bands.set_value(3.0);
        bands.set_numeric(true);
        bands.set_width_chars(3);
        bands.set_valign(gtk::Align::Center);
        bands.set_tooltip_text(Some("Bands"));
        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        controls.append(&process);
        controls.append(&bands);
        action.add_suffix(&controls);
        group.add(&action);
        threshold_process.push(process);
        threshold_bands.push(bands);
        threshold_component_rows.push(action);
    }
    let threshold_editor_button = adw::ActionRow::builder()
        .title("Edit mapping…")
        .activatable(true)
        .build();
    threshold_editor_button.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    threshold_editor_button.update_property(&[gtk::accessible::Property::Description(
        "Open the Boundary and Output transfer-curve editor",
    )]);
    group.add(&threshold_editor_button);
    // Retained as internal compatibility widgets for the old open-path sync; they are not
    // presented and no longer define the Threshold model.
    let legacy_spin = |value| {
        gtk::SpinButton::new(
            Some(&gtk::Adjustment::new(value, 0.0, 1.0, 0.001, 0.01, 0.0)),
            0.001,
            3,
        )
    };
    let low = legacy_spin(1.0 / 3.0);
    let high = legacy_spin(2.0 / 3.0);
    let out_low = legacy_spin(1.0 / 6.0);
    let out_mid = legacy_spin(0.5);
    let out_high = legacy_spin(5.0 / 6.0);
    page.add(&group);
    group.set_visible(false);
    let color = adw::PreferencesGroup::builder()
        .title("Color")
        .description("Circular hue in OKLCH; neutrals remain neutral.")
        .build();
    let hue = gtk::Adjustment::new(0.0, -180.0, 180.0, 0.1, 1.0, 0.0);
    let hue_row = adw::ActionRow::builder()
        .title("Hue rotation")
        .subtitle("Floating-point degrees")
        .build();
    let spin = gtk::SpinButton::new(Some(&hue), 0.1, 1);
    spin.update_property(&[gtk::accessible::Property::Label(
        "Hue rotation precise value in degrees",
    )]);
    hue_row.add_suffix(&spin);
    color.add(&hue_row);
    let scale = gtk::Scale::new(gtk::Orientation::Horizontal, Some(&hue));
    scale.update_property(&[gtk::accessible::Property::Label(
        "Hue rotation slider in degrees",
    )]);
    scale.set_draw_value(false);
    scale.set_hexpand(true);
    scale.set_margin_start(12);
    scale.set_margin_end(12);
    color.add(&scale);
    page.add(&color);
    let pipeline = adw::PreferencesGroup::builder().title("Pipeline").build();
    pipeline.add(
        &adw::ActionRow::builder()
            .title("Linear-sRGB RGBA f32")
            .subtitle("Straight alpha passes through; one display quantization")
            .build(),
    );
    page.add(&pipeline);
    let scroll = gtk::ScrolledWindow::builder()
        .child(&page)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    InspectorControls {
        root: scroll,
        hue,
        low,
        high,
        out_low,
        out_mid,
        out_high,
        threshold_space,
        threshold_link,
        threshold_link_row,
        threshold_process,
        threshold_bands,
        threshold_editor_button,
        threshold_component_rows,
        voronoi_matching,
        method_voronoi,
        method_thresholds,
        voronoi_panel: voronoi,
        voronoi_list_panel: voronoi_list,
        thresholds_panel: group,
        groups,
        add_color,
        add_sample,
        influence,
        sample_size,
        delete_sample,
        delete_group,
        sample_info,
        output_picker,
        output_swatch,
        source_picker,
        source_swatch,
        site_lock,
        preset_dropdown,
        preset_model,
        preset_info,
        preset_apply,
        preset_save,
        preset_refresh,
        preset_folder,
        sample_selector,
        sample_selector_model,
    }
}

fn threshold_quantizer(recipe: &Recipe, index: usize) -> ComponentQuantizer {
    match recipe.threshold.active_space {
        ThresholdSpace::Rgb => recipe.threshold.rgb_state.components[index].clone(),
        ThresholdSpace::Hsv => match index {
            0 => recipe.threshold.hsv_state.hue.clone(),
            1 => recipe.threshold.hsv_state.saturation.clone(),
            _ => recipe.threshold.hsv_state.value.clone(),
        },
    }
}

fn sync_threshold_ui(ui: &Ui, state: &Rc<RefCell<State>>) {
    let Some(recipe) = state
        .borrow()
        .document
        .as_ref()
        .map(|doc| doc.recipe.clone())
    else {
        return;
    };
    ui.syncing.set(true);
    let hsv = recipe.threshold.active_space == ThresholdSpace::Hsv;
    ui.threshold_space.set_selected(u32::from(hsv));
    let link = if hsv {
        recipe.threshold.hsv_state.sv_link
    } else {
        recipe.threshold.rgb_state.link
    };
    ui.threshold_link.set_active(link == LinkPolicy::Linked);
    ui.threshold_link_row.set_title(if hsv {
        "Link Saturation &amp; Value"
    } else {
        "Link RGB"
    });
    for (index, row) in ui.threshold_component_rows.iter().enumerate() {
        let name = if hsv {
            ["Hue", "Saturation", "Value"][index]
        } else {
            ["Red", "Green", "Blue"][index]
        };
        let quantizer = threshold_quantizer(&recipe, index);
        row.set_title(name);
        ui.threshold_process[index].set_active(quantizer.enabled);
        ui.threshold_bands[index].set_value(quantizer.outputs.len() as f64);
        row.set_subtitle(&format!(
            "{} • {} Bands{}",
            if quantizer.enabled {
                "Process"
            } else {
                "Bypass"
            },
            quantizer.outputs.len(),
            if hsv && index == 0 {
                " • circular degrees"
            } else {
                ""
            }
        ));
    }
    ui.voronoi_matching
        .set_selected(match recipe.voronoi.matching {
            VoronoiMatching::Perceptual => 0,
            VoronoiMatching::Rgb => 1,
            VoronoiMatching::Hsv => 2,
        });
    ui.syncing.set(false);
}

fn schedule_recipe_edit(ui: &Ui, state: &Rc<RefCell<State>>, message: &str) {
    let mut state = state.borrow_mut();
    if state.jobs.is_busy() {
        ui.status
            .set_label("Finish or cancel the file operation before editing");
        return;
    }
    let Some(document) = state.document.as_mut() else {
        return;
    };
    document.dirty = true;
    let recipe = document.recipe.clone();
    if let Some(source) = state.preview_source.clone() {
        state.scheduler.schedule(source, recipe);
    }
    drop(state);
    ui.save.set_sensitive(true);
    ui.status.set_label(message);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ThresholdHandle {
    Boundary(usize),
    Output(usize),
}

struct ThresholdDialogSession {
    snapshot: threshiator::document::ThresholdState,
    pre_dirty: bool,
    context: String,
    target: ThresholdEditTarget,
    draft: ComponentQuantizer,
    selected: ThresholdHandle,
    gesture: ThresholdEditGesture,
    completed_edits: usize,
    retain: bool,
}

fn document_context(document: &Document) -> String {
    format!(
        "{}:{}:{}x{}",
        document.source_name,
        document.source_bytes.len(),
        document.source.width,
        document.source.height
    )
}

fn update_threshold_precision_accessibility(
    spin: &gtk::SpinButton,
    target: ThresholdEditTarget,
    handle: ThresholdHandle,
    clamped: bool,
) {
    let (label, description) = threshold_precision_accessibility_text(target, handle, clamped);
    spin.update_property(&[
        gtk::accessible::Property::Label(&label),
        gtk::accessible::Property::Description(&description),
    ]);
}

fn threshold_precision_accessibility_text(
    target: ThresholdEditTarget,
    handle: ThresholdHandle,
    clamped: bool,
) -> (String, String) {
    let (kind, index) = match handle {
        ThresholdHandle::Boundary(index) => ("Boundary", index + 1),
        ThresholdHandle::Output(index) => ("Output", index + 1),
    };
    let units = if target.is_hue() {
        "Degrees, range 0 to 360."
    } else {
        "Normalized, range 0 to 1."
    };
    let ordering = if matches!(handle, ThresholdHandle::Boundary(_)) {
        " Must remain strictly ordered between adjacent Boundaries."
    } else {
        ""
    };
    let clamp_notice = if clamped {
        " The entered value was constrained to preserve Boundary order."
    } else {
        ""
    };
    (
        format!("{} {kind} {index} precise value", target.label()),
        format!("{units}{ordering}{clamp_notice}"),
    )
}

fn commit_threshold_dialog_edit(
    ui: &Ui,
    state: &Rc<RefCell<State>>,
    session: &Rc<RefCell<ThresholdDialogSession>>,
) -> bool {
    if state.borrow().jobs.is_busy() {
        ui.status
            .set_label("Finish or cancel the file operation before editing");
        return false;
    }
    let (target, edited, context) = {
        let session = session.borrow();
        (
            session.target,
            session.draft.clone(),
            session.context.clone(),
        )
    };
    if edited.validate(target.label()).is_err() {
        return false;
    }
    let changed = {
        let mut state = state.borrow_mut();
        let Some(document) = state.document.as_mut() else {
            return false;
        };
        if document_context(document) != context {
            return false;
        }
        let target = document.recipe.threshold.reconcile_edit_target(target);
        if document.recipe.threshold.edit_quantizer(target) == edited {
            false
        } else {
            document.recipe.threshold.set_edit_quantizer(target, edited);
            true
        }
    };
    if changed {
        session.borrow_mut().completed_edits += 1;
        schedule_recipe_edit(ui, state, "Threshold mapping changed — updating preview…");
    }
    changed
}

fn present_threshold_editor(ui: &Rc<Ui>, state: &Rc<RefCell<State>>) {
    if state.borrow().threshold_editor_visible {
        return;
    }
    let Some((threshold, pre_dirty, context)) = state.borrow().document.as_ref().map(|document| {
        (
            document.recipe.threshold.clone(),
            document.dirty,
            document_context(document),
        )
    }) else {
        return;
    };
    let requested = ui
        .cli
        .threshold_editor_target
        .unwrap_or_else(|| threshold.edit_targets()[0]);
    let target = threshold.reconcile_edit_target(requested);
    let draft = threshold.edit_quantizer(target);
    let requested_handle = ui
        .cli
        .threshold_editor_handle
        .unwrap_or(ThresholdHandle::Output(0));
    let selected = match requested_handle {
        ThresholdHandle::Boundary(index) if index < draft.boundaries.len() => requested_handle,
        ThresholdHandle::Output(index) if index < draft.outputs.len() => requested_handle,
        _ => ThresholdHandle::Output(0),
    };
    let session = Rc::new(RefCell::new(ThresholdDialogSession {
        snapshot: threshold.clone(),
        pre_dirty,
        context,
        target,
        draft,
        selected,
        gesture: ThresholdEditGesture::default(),
        completed_edits: 0,
        retain: false,
    }));

    let cancel = gtk::Button::with_label("Cancel");
    let done = gtk::Button::with_label("Done");
    *ui.audit_threshold_cancel.borrow_mut() = Some(cancel.clone());
    *ui.audit_threshold_done.borrow_mut() = Some(done.clone());
    done.add_css_class("suggested-action");
    let header = adw::HeaderBar::new();
    header.pack_start(&cancel);
    header.pack_end(&done);
    header.set_title_widget(Some(&adw::WindowTitle::new("Edit Threshold Mapping", "")));

    let edit_targets = threshold.edit_targets();
    let target_labels: Vec<_> = edit_targets.iter().map(|target| target.label()).collect();
    let target_model = gtk::StringList::new(&target_labels);
    let target_drop = gtk::DropDown::new(Some(target_model), None::<gtk::Expression>);
    *ui.audit_threshold_target.borrow_mut() = Some(target_drop.clone());
    target_drop.set_selected(
        edit_targets
            .iter()
            .position(|item| *item == target)
            .unwrap_or(0) as u32,
    );
    target_drop.set_hexpand(false);
    let target_row = adw::ActionRow::builder()
        .title("Component")
        .subtitle("Linked components share one mapping")
        .build();
    target_row.add_suffix(&target_drop);

    let instruction = gtk::Label::builder()
        .label("Drag vertical Boundary lines or horizontal Output handles. The preview updates when you release.")
        .wrap(true)
        .xalign(0.0)
        .css_classes(["dim-label"])
        .build();
    let plot = gtk::DrawingArea::builder()
        .content_width(640)
        .content_height(360)
        .height_request(300)
        .hexpand(true)
        .vexpand(true)
        .focusable(true)
        .build();
    plot.update_property(&[
        gtk::accessible::Property::Label("Threshold Input to Output transfer plot"),
        gtk::accessible::Property::Description(
            "Arrow keys select handles; Up and Down adjust the selected handle",
        ),
    ]);

    let selected_title = gtk::Label::builder().xalign(0.0).hexpand(true).build();
    let precise = gtk::SpinButton::new(
        Some(&gtk::Adjustment::new(0.0, 0.0, 1.0, 0.001, 0.01, 0.0)),
        0.001,
        3,
    );
    precise.set_numeric(true);
    precise.set_width_chars(9);
    *ui.audit_threshold_precise.borrow_mut() = Some(precise.clone());
    let precise_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    precise_row.append(&selected_title);
    precise_row.append(&precise);

    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    body.set_margin_top(18);
    body.set_margin_bottom(18);
    body.set_margin_start(18);
    body.set_margin_end(18);
    body.append(&target_row);
    body.append(&instruction);
    body.append(&plot);
    body.append(&precise_row);
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .child(&body)
        .build();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&scroll));
    let dialog = adw::Dialog::builder()
        .title("Edit Threshold Mapping")
        .content_width(720)
        .content_height(590)
        .can_close(false)
        .child(&toolbar)
        .build();
    *ui.threshold_editor_dialog.borrow_mut() = Some(dialog.clone());

    let syncing = Rc::new(Cell::new(false));
    let refresh: Rc<dyn Fn()> = Rc::new({
        let state = state.clone();
        let session = session.clone();
        let plot = plot.clone();
        let precise = precise.clone();
        let selected_title = selected_title.clone();
        let syncing = syncing.clone();
        move || {
            let Some(threshold) = state
                .borrow()
                .document
                .as_ref()
                .map(|document| document.recipe.threshold.clone())
            else {
                return;
            };
            syncing.set(true);
            let previous = session.borrow().target;
            let target = threshold.reconcile_edit_target(previous);
            {
                let mut session = session.borrow_mut();
                session.target = target;
                session.draft = threshold.edit_quantizer(target);
                let selected_valid = match session.selected {
                    ThresholdHandle::Boundary(index) => index < session.draft.boundaries.len(),
                    ThresholdHandle::Output(index) => index < session.draft.outputs.len(),
                };
                if !selected_valid {
                    session.selected = ThresholdHandle::Output(0);
                }
                let value = match session.selected {
                    ThresholdHandle::Boundary(index) => session.draft.boundaries[index],
                    ThresholdHandle::Output(index) => session.draft.outputs[index],
                };
                let kind = match session.selected {
                    ThresholdHandle::Boundary(index) => format!("Boundary {}", index + 1),
                    ThresholdHandle::Output(index) => format!("Output {}", index + 1),
                };
                let degrees = session.target.is_hue();
                selected_title.set_label(&format!(
                    "{kind} — {}",
                    if degrees { "degrees" } else { "0 to 1" }
                ));
                precise.set_range(0.0, if degrees { 360.0 } else { 1.0 });
                precise.set_increments(
                    if degrees { 0.1 } else { 0.001 },
                    if degrees { 1.0 } else { 0.01 },
                );
                precise.set_digits(if degrees { 3 } else { 6 });
                precise.set_value(threshold_display_value(value, degrees));
                update_threshold_precision_accessibility(
                    &precise,
                    session.target,
                    session.selected,
                    false,
                );
            }
            {
                let mut state = state.borrow_mut();
                state.threshold_editor_target = Some(target);
                state.threshold_editor_handle = Some(session.borrow().selected);
            }
            plot.queue_draw();
            syncing.set(false);
        }
    });
    *ui.threshold_editor_refresh.borrow_mut() = Some(refresh.clone());

    plot.set_draw_func({
        let session = session.clone();
        move |_, ctx, width, height| {
            let session = session.borrow();
            let left = 54.0;
            let right = f64::from(width) - 20.0;
            let top = 18.0;
            let bottom = f64::from(height) - 42.0;
            let pw = (right - left).max(1.0);
            let ph = (bottom - top).max(1.0);
            ctx.set_source_rgb(0.96, 0.96, 0.97);
            ctx.rectangle(left, top, pw, ph);
            let _ = ctx.fill();
            ctx.set_source_rgba(0.2, 0.2, 0.22, 0.25);
            ctx.set_line_width(1.0);
            for tick in 0..=4 {
                let x = left + pw * f64::from(tick) / 4.0;
                let y = bottom - ph * f64::from(tick) / 4.0;
                ctx.move_to(x, top);
                ctx.line_to(x, bottom);
                ctx.move_to(left, y);
                ctx.line_to(right, y);
            }
            let _ = ctx.stroke();

            for (index, boundary) in session.draft.boundaries.iter().enumerate() {
                let x = left + pw * f64::from(*boundary);
                let selected = session.selected == ThresholdHandle::Boundary(index);
                ctx.set_source_rgba(0.1, 0.42, 0.9, if selected { 1.0 } else { 0.55 });
                ctx.set_line_width(if selected { 4.0 } else { 2.0 });
                ctx.move_to(x, top);
                ctx.line_to(x, bottom);
                let _ = ctx.stroke();
                ctx.arc(
                    x,
                    bottom,
                    if selected { 7.0 } else { 5.0 },
                    0.0,
                    std::f64::consts::TAU,
                );
                let _ = ctx.fill();
            }
            let mut start = 0.0_f64;
            for (index, output) in session.draft.outputs.iter().enumerate() {
                let end = session
                    .draft
                    .boundaries
                    .get(index)
                    .map_or(1.0, |value| f64::from(*value));
                let y = bottom - ph * f64::from(*output);
                let selected = session.selected == ThresholdHandle::Output(index);
                ctx.set_source_rgb(0.82, 0.18, 0.22);
                ctx.set_line_width(if selected { 5.0 } else { 3.0 });
                ctx.move_to(left + pw * start, y);
                ctx.line_to(left + pw * end, y);
                let _ = ctx.stroke();
                let x = left + pw * (start + end) / 2.0;
                ctx.arc(
                    x,
                    y,
                    if selected { 8.0 } else { 6.0 },
                    0.0,
                    std::f64::consts::TAU,
                );
                let _ = ctx.fill();
                if let Some(boundary) = session.draft.boundaries.get(index) {
                    let x = left + pw * f64::from(*boundary);
                    if let Some(next) = session.draft.outputs.get(index + 1) {
                        let next_y = bottom - ph * f64::from(*next);
                        ctx.set_line_width(2.0);
                        ctx.move_to(x, y);
                        ctx.line_to(x, next_y);
                        let _ = ctx.stroke();
                    }
                }
                start = end;
            }
            ctx.set_source_rgb(0.18, 0.18, 0.2);
            ctx.set_font_size(13.0);
            ctx.move_to(left + pw / 2.0 - 16.0, f64::from(height) - 12.0);
            let _ = ctx.show_text("Input");
            let _ = ctx.save();
            ctx.translate(17.0, top + ph / 2.0 + 22.0);
            ctx.rotate(-std::f64::consts::FRAC_PI_2);
            ctx.move_to(0.0, 0.0);
            let _ = ctx.show_text("Output");
            let _ = ctx.restore();
        }
    });

    let update_precise: Rc<dyn Fn()> = Rc::new({
        let state = state.clone();
        let session = session.clone();
        let precise = precise.clone();
        let selected_title = selected_title.clone();
        let syncing = syncing.clone();
        let plot = plot.clone();
        move || {
            syncing.set(true);
            let session = session.borrow();
            let (kind, value) = match session.selected {
                ThresholdHandle::Boundary(index) => (
                    format!("Boundary {}", index + 1),
                    session.draft.boundaries[index],
                ),
                ThresholdHandle::Output(index) => (
                    format!("Output {}", index + 1),
                    session.draft.outputs[index],
                ),
            };
            let degrees = session.target.is_hue();
            selected_title.set_label(&format!(
                "{kind} — {}",
                if degrees { "degrees" } else { "0 to 1" }
            ));
            precise.set_value(threshold_display_value(value, degrees));
            update_threshold_precision_accessibility(
                &precise,
                session.target,
                session.selected,
                false,
            );
            state.borrow_mut().threshold_editor_handle = Some(session.selected);
            plot.queue_draw();
            syncing.set(false);
        }
    });

    target_drop.connect_selected_notify({
        let state = state.clone();
        let session = session.clone();
        let refresh = refresh.clone();
        let syncing = syncing.clone();
        move |drop| {
            if syncing.get() {
                return;
            }
            let Some(threshold) = state
                .borrow()
                .document
                .as_ref()
                .map(|d| d.recipe.threshold.clone())
            else {
                return;
            };
            let targets = threshold.edit_targets();
            if let Some(target) = targets.get(drop.selected() as usize).copied() {
                {
                    let mut session = session.borrow_mut();
                    session.target = target;
                    session.draft = threshold.edit_quantizer(target);
                }
                refresh();
            }
        }
    });

    precise.connect_value_changed({
        let ui = ui.clone();
        let state = state.clone();
        let session = session.clone();
        let syncing = syncing.clone();
        let plot = plot.clone();
        let update_precise = update_precise.clone();
        move |spin| {
            if syncing.get() {
                return;
            }
            let clamped = {
                let mut session = session.borrow_mut();
                let normalized = threshold_normalized_value(spin.value(), session.target.is_hue());
                match session.selected {
                    ThresholdHandle::Boundary(index) => {
                        let actual =
                            clamp_threshold_boundary(&session.draft.boundaries, index, normalized);
                        session.draft.boundaries[index] = actual;
                        (actual - normalized).abs() > f32::EPSILON
                    }
                    ThresholdHandle::Output(index) => {
                        let actual = normalized.clamp(0.0, 1.0);
                        session.draft.outputs[index] = actual;
                        (actual - normalized).abs() > f32::EPSILON
                    }
                }
            };
            if commit_threshold_dialog_edit(&ui, &state, &session) {
                plot.queue_draw();
            }
            update_precise();
            if clamped {
                let session = session.borrow();
                update_threshold_precision_accessibility(
                    spin,
                    session.target,
                    session.selected,
                    true,
                );
                ui.status
                    .set_label("Boundary constrained to preserve strict ordering");
            }
        }
    });

    let drag = gtk::GestureDrag::new();
    let drag_origin = Rc::new(Cell::new((0.0_f64, 0.0_f64)));
    drag.connect_drag_begin({
        let session = session.clone();
        let plot = plot.clone();
        let drag_origin = drag_origin.clone();
        let update_precise = update_precise.clone();
        move |_, x, y| {
            drag_origin.set((x, y));
            let width = f64::from(plot.width());
            let height = f64::from(plot.height());
            let left = 54.0;
            let right = width - 20.0;
            let top = 18.0;
            let bottom = height - 42.0;
            let pw = (right - left).max(1.0);
            let ph = (bottom - top).max(1.0);
            let mut nearest = (f64::MAX, ThresholdHandle::Output(0));
            {
                let draft = &session.borrow().draft;
                for (index, boundary) in draft.boundaries.iter().enumerate() {
                    let hx = left + pw * f64::from(*boundary);
                    let distance = (x - hx).abs();
                    if distance < nearest.0 {
                        nearest = (distance, ThresholdHandle::Boundary(index));
                    }
                }
                let mut start = 0.0;
                for (index, output) in draft.outputs.iter().enumerate() {
                    let end = draft
                        .boundaries
                        .get(index)
                        .map_or(1.0, |value| f64::from(*value));
                    let hx = left + pw * (start + end) / 2.0;
                    let hy = bottom - ph * f64::from(*output);
                    let distance = (x - hx).hypot(y - hy);
                    if distance < nearest.0 {
                        nearest = (distance, ThresholdHandle::Output(index));
                    }
                    start = end;
                }
            }
            session.borrow_mut().selected = nearest.1;
            update_precise();
            plot.grab_focus();
        }
    });
    drag.connect_drag_update({
        let session = session.clone();
        let plot = plot.clone();
        let drag_origin = drag_origin.clone();
        let update_precise = update_precise.clone();
        move |_, dx, dy| {
            let (origin_x, origin_y) = drag_origin.get();
            let x = origin_x + dx;
            let y = origin_y + dy;
            let left = 54.0;
            let right = f64::from(plot.width()) - 20.0;
            let top = 18.0;
            let bottom = f64::from(plot.height()) - 42.0;
            let pw = (right - left).max(1.0);
            let ph = (bottom - top).max(1.0);
            let mut session = session.borrow_mut();
            let before = session.draft.clone();
            match session.selected {
                ThresholdHandle::Boundary(index) => {
                    let candidate = ((x - left) / pw) as f32;
                    session.draft.boundaries[index] =
                        clamp_threshold_boundary(&session.draft.boundaries, index, candidate);
                }
                ThresholdHandle::Output(index) => {
                    session.draft.outputs[index] = ((bottom - y) / ph).clamp(0.0, 1.0) as f32;
                }
            }
            let changed = session.draft != before;
            session.gesture.motion(changed);
            drop(session);
            update_precise();
        }
    });
    drag.connect_drag_end({
        let ui = ui.clone();
        let state = state.clone();
        let session = session.clone();
        move |_, _, _| {
            if session.borrow_mut().gesture.complete() {
                commit_threshold_dialog_edit(&ui, &state, &session);
            }
        }
    });
    plot.add_controller(drag);

    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed({
        let ui = ui.clone();
        let state = state.clone();
        let session = session.clone();
        let update_precise = update_precise.clone();
        move |_, key, _, _| {
            use gtk::gdk::Key;
            let handled = match key {
                Key::Left | Key::Right => {
                    let mut session = session.borrow_mut();
                    let total_boundaries = session.draft.boundaries.len();
                    let total = total_boundaries + session.draft.outputs.len();
                    let current = match session.selected {
                        ThresholdHandle::Boundary(index) => index,
                        ThresholdHandle::Output(index) => total_boundaries + index,
                    };
                    let next = if key == Key::Right {
                        (current + 1) % total
                    } else {
                        (current + total - 1) % total
                    };
                    session.selected = if next < total_boundaries {
                        ThresholdHandle::Boundary(next)
                    } else {
                        ThresholdHandle::Output(next - total_boundaries)
                    };
                    drop(session);
                    update_precise();
                    true
                }
                Key::Up | Key::Down => {
                    let mut session_mut = session.borrow_mut();
                    let step = if session_mut.target.is_hue() {
                        1.0 / 360.0
                    } else {
                        0.001
                    } * if key == Key::Up { 1.0 } else { -1.0 };
                    match session_mut.selected {
                        ThresholdHandle::Boundary(index) => {
                            let candidate = session_mut.draft.boundaries[index] + step;
                            session_mut.draft.boundaries[index] = clamp_threshold_boundary(
                                &session_mut.draft.boundaries,
                                index,
                                candidate,
                            );
                        }
                        ThresholdHandle::Output(index) => {
                            session_mut.draft.outputs[index] =
                                (session_mut.draft.outputs[index] + step).clamp(0.0, 1.0)
                        }
                    }
                    drop(session_mut);
                    commit_threshold_dialog_edit(&ui, &state, &session);
                    update_precise();
                    true
                }
                _ => false,
            };
            handled.into()
        }
    });
    plot.add_controller(keys);

    let close_cancel: Rc<dyn Fn()> = Rc::new({
        let ui = ui.clone();
        let state = state.clone();
        let session = session.clone();
        let dialog = dialog.clone();
        move || {
            let (retain, edits, snapshot, pre_dirty, context) = {
                let session = session.borrow();
                (
                    session.retain,
                    session.completed_edits,
                    session.snapshot.clone(),
                    session.pre_dirty,
                    session.context.clone(),
                )
            };
            if !retain && edits > 0 {
                let mut state_mut = state.borrow_mut();
                if let Some(document) = state_mut
                    .document
                    .as_mut()
                    .filter(|document| document_context(document) == context)
                {
                    document.recipe.threshold = snapshot;
                    document.dirty = pre_dirty;
                    let recipe = document.recipe.clone();
                    if let Some(source) = state_mut.preview_source.clone() {
                        state_mut.scheduler.schedule(source, recipe);
                    }
                    ui.save.set_sensitive(pre_dirty);
                    ui.status
                        .set_label("Threshold edits cancelled — restoring preview…");
                }
            }
            dialog.force_close();
        }
    });
    dialog.connect_close_attempt({
        let close_cancel = close_cancel.clone();
        move |_| close_cancel()
    });
    cancel.connect_clicked({
        let close_cancel = close_cancel.clone();
        move |_| close_cancel()
    });
    done.connect_clicked({
        let session = session.clone();
        let dialog = dialog.clone();
        move |_| {
            session.borrow_mut().retain = true;
            dialog.force_close();
        }
    });
    dialog.connect_closed({
        let ui = ui.clone();
        let state = state.clone();
        move |_| {
            state.borrow_mut().threshold_editor_visible = false;
            *ui.threshold_editor_refresh.borrow_mut() = None;
            *ui.threshold_editor_dialog.borrow_mut() = None;
            *ui.audit_threshold_target.borrow_mut() = None;
            *ui.audit_threshold_precise.borrow_mut() = None;
            *ui.audit_threshold_cancel.borrow_mut() = None;
            *ui.audit_threshold_done.borrow_mut() = None;
        }
    });

    refresh();
    {
        let mut state = state.borrow_mut();
        state.threshold_editor_visible = true;
        state.threshold_editor_target = Some(session.borrow().target);
        state.threshold_editor_handle = Some(session.borrow().selected);
    }
    dialog.present(Some(&ui.window));
}

fn update_preset_info(ui: &Ui, state: &Rc<RefCell<State>>) {
    let current = state.borrow();
    let count = current.preset_entries.len();
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
    let description = current
        .preset_entries
        .get(ui.preset_dropdown.selected() as usize)
        .and_then(|entry| entry.preset.description.as_deref())
        .unwrap_or("Selected preset has no description");
    ui.preset_info
        .set_subtitle(&format!("{summary} · {description}"));
}

fn install_preset_scan(
    ui: &Ui,
    state: &Rc<RefCell<State>>,
    scan: threshiator::preset::PresetScan,
    preferred_path: Option<&Path>,
    show_errors: bool,
) {
    let labels: Vec<_> = scan
        .entries
        .iter()
        .map(|entry| entry.preset.name.clone())
        .collect();
    let selected = preferred_path
        .and_then(|path| scan.entries.iter().position(|entry| entry.path == path))
        .unwrap_or(0);
    let diagnostics = scan.diagnostics.clone();
    {
        let mut current = state.borrow_mut();
        current.preset_entries = scan.entries;
        current.preset_diagnostics = scan.diagnostics;
    }
    ui.syncing.set(true);
    sync_string_list(&ui.preset_model, &labels);
    if !labels.is_empty() {
        ui.preset_dropdown.set_selected(selected as u32);
    }
    ui.syncing.set(false);
    ui.preset_apply
        .set_sensitive(!labels.is_empty() && state.borrow().document.is_some());
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
        let dialog = adw::AlertDialog::builder()
            .heading("Some presets could not be loaded")
            .body(body)
            .build();
        dialog.add_response("ok", "OK");
        dialog.present(Some(&ui.window));
    }
}

fn refresh_preset_ui(ui: &Ui, state: &Rc<RefCell<State>>, show_errors: bool) {
    let previous = state
        .borrow()
        .preset_entries
        .get(ui.preset_dropdown.selected() as usize)
        .map(|entry| entry.path.clone());
    match PresetStore::system().scan() {
        Ok(scan) => install_preset_scan(ui, state, scan, previous.as_deref(), show_errors),
        Err(error) => {
            ui.preset_apply.set_sensitive(false);
            ui.status
                .set_label(&format!("Could not scan presets: {error:#}"));
        }
    }
}

fn present_preset_error(ui: &Ui, heading: &str, error: &anyhow::Error) {
    let dialog = adw::AlertDialog::builder()
        .heading(heading)
        .body(format!("{error:#}"))
        .build();
    dialog.add_response("ok", "OK");
    dialog.present(Some(&ui.window));
}

fn present_save_preset(ui: &Rc<Ui>, state: &Rc<RefCell<State>>) {
    let Some(recipe) = state
        .borrow()
        .document
        .as_ref()
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
    let header = adw::HeaderBar::new();
    let cancel = gtk::Button::with_label("Cancel");
    let save = gtk::Button::with_label("Save");
    save.add_css_class("suggested-action");
    header.pack_start(&cancel);
    header.pack_end(&save);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&form));
    let dialog = adw::Dialog::builder()
        .title("Save Current Preset")
        .content_width(440)
        .content_height(320)
        .child(&toolbar)
        .build();
    *ui.audit_preset_name.borrow_mut() = Some(name.clone());
    *ui.audit_preset_cancel.borrow_mut() = Some(cancel.clone());
    *ui.audit_preset_save.borrow_mut() = Some(save.clone());
    {
        let ui = ui.clone();
        dialog.connect_closed(move |_| {
            *ui.audit_preset_name.borrow_mut() = None;
            *ui.audit_preset_cancel.borrow_mut() = None;
            *ui.audit_preset_save.borrow_mut() = None;
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
                    let confirm = adw::AlertDialog::builder()
                        .heading("Replace existing preset?")
                        .body(format!("A preset named {:?} already exists.", preset.name))
                        .build();
                    confirm.add_response("cancel", "Cancel");
                    confirm.add_response("replace", "Replace");
                    confirm
                        .set_response_appearance("replace", adw::ResponseAppearance::Destructive);
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
    dialog.present(Some(&ui.window));
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
                let _ = dropdown;
                update_preset_info(&ui, &state);
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
        let state = state.clone();
        ui.preset_apply.clone().connect_clicked(move |_| {
            let selected_path = state
                .borrow()
                .preset_entries
                .get(ui.preset_dropdown.selected() as usize)
                .map(|entry| entry.path.clone());
            let Some(selected_path) = selected_path else {
                ui.status.set_label("Choose a valid preset to apply");
                return;
            };
            let scan = match PresetStore::system().scan() {
                Ok(scan) => scan,
                Err(error) => {
                    ui.status
                        .set_label(&format!("Could not rescan presets before Apply: {error:#}"));
                    return;
                }
            };
            let selected = scan
                .entries
                .iter()
                .find(|entry| entry.path == selected_path)
                .cloned();
            install_preset_scan(&ui, &state, scan, Some(&selected_path), false);
            let Some(entry) = selected else {
                ui.status.set_label(&format!(
                    "Preset {} is no longer available or valid; Refresh to review file errors",
                    selected_path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                ));
                return;
            };
            let changed = {
                let mut current = state.borrow_mut();
                let Some(document) = current.document.as_mut() else {
                    return;
                };
                match apply_to_document(document, &entry.preset) {
                    Ok(changed) => changed,
                    Err(error) => {
                        drop(current);
                        present_preset_error(&ui, "Could not apply preset", &error);
                        return;
                    }
                }
            };
            if !changed {
                ui.status.set_label("Preset already matches this document");
                return;
            }
            {
                let mut current = state.borrow_mut();
                current.sampling = None;
                current.sampling_previous_mode = None;
                let first = current
                    .document
                    .as_ref()
                    .and_then(|document| document.recipe.voronoi.sites.first())
                    .map(|site| site.id);
                current.selected_group = first;
                current.selected_sample = first;
            }
            ui.syncing.set(true);
            let recipe = state.borrow().document.as_ref().unwrap().recipe.clone();
            ui.method_voronoi
                .set_active(recipe.active_method == Method::Voronoi);
            ui.method_thresholds
                .set_active(recipe.active_method == Method::Thresholds);
            ui.voronoi_panel
                .set_visible(recipe.active_method == Method::Voronoi);
            ui.voronoi_list_panel
                .set_visible(recipe.active_method == Method::Voronoi);
            ui.thresholds_panel
                .set_visible(recipe.active_method == Method::Thresholds);
            ui.voronoi_matching
                .set_selected(match recipe.voronoi.matching {
                    VoronoiMatching::Perceptual => 0,
                    VoronoiMatching::Rgb => 1,
                    VoronoiMatching::Hsv => 2,
                });
            ui.hue.set_value(recipe.hue_degrees() as f64);
            ui.syncing.set(false);
            sync_threshold_ui(&ui, &state);
            refresh_voronoi_ui(&ui, &state);
            sync_selected_controls(&ui, &state);
            let status = match recipe.active_method {
                Method::Voronoi if !recipe.voronoi.sites.is_empty() => {
                    "Preset applied — sites detached; use Reattach / Resample Source as needed"
                }
                Method::Voronoi => "Preset applied — add a color site to begin",
                Method::Thresholds => "Threshold preset applied",
            };
            schedule_recipe_edit(&ui, &state, status);
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

fn recipe_controls(ui: &Rc<Ui>, state: Rc<RefCell<State>>) {
    let update_hue: Rc<dyn Fn()> = Rc::new({
        let ui = ui.clone();
        let state = state.clone();
        move || {
            if ui.syncing.get() {
                return;
            }
            if let Some(doc) = state.borrow_mut().document.as_mut() {
                doc.set_hue(ui.hue.value() as f32);
            }
            schedule_recipe_edit(&ui, &state, "Unsaved changes — updating preview…");
        }
    });
    let u = update_hue;
    ui.hue.connect_value_changed(move |_| u());

    {
        let ui = ui.clone();
        let state = state.clone();
        ui.threshold_space
            .clone()
            .connect_selected_notify(move |drop_down| {
                if ui.syncing.get() {
                    return;
                }
                if let Some(document) = state.borrow_mut().document.as_mut() {
                    document.recipe.threshold.active_space = if drop_down.selected() == 0 {
                        ThresholdSpace::Rgb
                    } else {
                        ThresholdSpace::Hsv
                    };
                }
                sync_threshold_ui(&ui, &state);
                let refresh = ui.threshold_editor_refresh.borrow().clone();
                if let Some(refresh) = refresh {
                    refresh();
                }
                schedule_recipe_edit(&ui, &state, "Working space changed — updating preview…");
            });
    }
    {
        let ui = ui.clone();
        let state = state.clone();
        ui.threshold_link
            .clone()
            .connect_active_notify(move |switch| {
                if ui.syncing.get() {
                    return;
                }
                if let Some(document) = state.borrow_mut().document.as_mut() {
                    let link = if switch.is_active() {
                        LinkPolicy::Linked
                    } else {
                        LinkPolicy::Independent
                    };
                    match document.recipe.threshold.active_space {
                        ThresholdSpace::Rgb => document.recipe.threshold.rgb_state.link = link,
                        ThresholdSpace::Hsv => document.recipe.threshold.hsv_state.sv_link = link,
                    }
                }
                let refresh = ui.threshold_editor_refresh.borrow().clone();
                if let Some(refresh) = refresh {
                    refresh();
                }
                schedule_recipe_edit(&ui, &state, "Link controls changed — updating preview…");
            });
    }
    for index in 0..3 {
        let ui_process = ui.clone();
        let state_process = state.clone();
        ui.threshold_process[index]
            .clone()
            .connect_active_notify(move |switch| {
                if ui_process.syncing.get() {
                    return;
                }
                if let Some(document) = state_process.borrow_mut().document.as_mut() {
                    match document.recipe.threshold.active_space {
                        ThresholdSpace::Rgb => {
                            document.recipe.threshold.rgb_state.components[index].enabled =
                                switch.is_active()
                        }
                        ThresholdSpace::Hsv => match index {
                            0 => {
                                document.recipe.threshold.hsv_state.hue.enabled = switch.is_active()
                            }
                            1 => {
                                document.recipe.threshold.hsv_state.saturation.enabled =
                                    switch.is_active()
                            }
                            _ => {
                                document.recipe.threshold.hsv_state.value.enabled =
                                    switch.is_active()
                            }
                        },
                    }
                }
                sync_threshold_ui(&ui_process, &state_process);
                schedule_recipe_edit(
                    &ui_process,
                    &state_process,
                    "Process/Bypass changed — updating preview…",
                );
            });
        let ui_bands = ui.clone();
        let state_bands = state.clone();
        ui.threshold_bands[index]
            .clone()
            .connect_value_changed(move |spin| {
                if ui_bands.syncing.get() {
                    return;
                }
                if let Some(document) = state_bands.borrow_mut().document.as_mut() {
                    let mut edited = ComponentQuantizer::evenly_spaced(spin.value() as usize);
                    edited.enabled = threshold_quantizer(&document.recipe, index).enabled;
                    match document.recipe.threshold.active_space {
                        ThresholdSpace::Rgb => {
                            document.recipe.threshold.set_rgb_component(index, edited)
                        }
                        ThresholdSpace::Hsv => {
                            document.recipe.threshold.set_hsv_component(index, edited)
                        }
                    }
                }
                sync_threshold_ui(&ui_bands, &state_bands);
                schedule_recipe_edit(&ui_bands, &state_bands, "Bands changed — updating preview…");
            });
    }
    {
        let ui_for_dialog = ui.clone();
        let state_for_dialog = state.clone();
        ui.threshold_editor_button.connect_activated(move |_| {
            present_threshold_editor(&ui_for_dialog, &state_for_dialog);
        });
    }
    {
        let ui = ui.clone();
        let state = state.clone();
        ui.voronoi_matching
            .clone()
            .connect_selected_notify(move |drop_down| {
                if ui.syncing.get() {
                    return;
                }
                if let Some(document) = state.borrow_mut().document.as_mut() {
                    document.recipe.voronoi.matching = match drop_down.selected() {
                        1 => VoronoiMatching::Rgb,
                        2 => VoronoiMatching::Hsv,
                        _ => VoronoiMatching::Perceptual,
                    };
                }
                schedule_recipe_edit(&ui, &state, "Color matching changed — updating preview…");
            });
    }

    for (button, method) in [
        (&ui.method_voronoi, Method::Voronoi),
        (&ui.method_thresholds, Method::Thresholds),
    ] {
        let ui = ui.clone();
        let state = state.clone();
        button.connect_toggled(move |button| {
            if ui.syncing.get() || !button.is_active() {
                return;
            }
            ui.voronoi_panel.set_visible(method == Method::Voronoi);
            ui.voronoi_list_panel.set_visible(method == Method::Voronoi);
            ui.thresholds_panel
                .set_visible(method == Method::Thresholds);
            let mut state = state.borrow_mut();
            let Some(document) = state.document.as_mut() else {
                return;
            };
            if document.recipe.active_method == method {
                return;
            }
            document.recipe.active_method = method;
            document.dirty = true;
            let recipe = document.recipe.clone();
            if let Some(source) = state.preview_source.clone() {
                state.scheduler.schedule(source, recipe);
            }
            drop(state);
            ui.save.set_sensitive(true);
            ui.status.set_label(match method {
                Method::Voronoi => "Color sites active — updating preview…",
                Method::Thresholds => "Thresholds active — updating preview…",
            });
        });
    }

    sampling_controls(ui, state.clone());
    voronoi_controls(ui, state);
}

fn sampling_controls(ui: &Rc<Ui>, state: Rc<RefCell<State>>) {
    for (button, sampling, message) in [
        (
            &ui.add_color,
            SamplingState::AddColor,
            "Click a visible source color to create a site — Escape cancels",
        ),
        (
            &ui.add_sample,
            SamplingState::AddSample,
            "Click a visible source color to reattach Source and reset Target — Escape cancels",
        ),
    ] {
        let ui = ui.clone();
        let state = state.clone();
        button.connect_clicked(move |_| {
            if sampling == SamplingState::AddSample {
                let selected = state.borrow().selected_sample;
                let locked = state
                    .borrow()
                    .document
                    .as_ref()
                    .and_then(|document| selected.and_then(|id| document.recipe.voronoi.site(id)))
                    .is_some_and(|site| site.locked);
                if selected.is_none() {
                    ui.status
                        .set_label("Select a site before reattaching its Source");
                    return;
                }
                if locked {
                    ui.status
                        .set_label("Unlock this site before reattaching its Source");
                    return;
                }
            }
            let mut state = state.borrow_mut();
            state.sampling_previous_mode = Some(state.mode);
            state.sampling = Some(sampling);
            drop(state);
            ui.source_mode.set_active(true);
            ui.status.set_label(message);
            ui.canvas.grab_focus();
        });
    }
}

fn group_selection(ui: &Rc<Ui>, state: Rc<RefCell<State>>) {
    let ui = ui.clone();
    ui.groups.clone().connect_row_selected(move |_, row| {
        let Some(row) = row else { return };
        let Ok(id) = row.widget_name().parse::<u64>() else {
            return;
        };
        let mut current = state.borrow_mut();
        current.selected_group = Some(id);
        current.selected_sample = Some(id);
        drop(current);
        sync_selected_controls(&ui, &state);
        ui.canvas.queue_draw();
    });
}

fn sync_selected_controls(ui: &Ui, state: &Rc<RefCell<State>>) {
    let site = {
        let state = state.borrow();
        state.document.as_ref().and_then(|document| {
            document
                .recipe
                .voronoi
                .sites
                .iter()
                .find(|site| Some(site.id) == state.selected_sample)
                .cloned()
        })
    };
    ui.syncing.set(true);
    sync_string_list(&ui.sample_selector_model, &[]);
    if site.is_some() {
        ui.output_swatch.queue_draw();
        ui.source_swatch.queue_draw();
    }
    if let Some(site) = site.as_ref() {
        ui.influence.set_value(site.influence);
        ui.sample_size.set_selected(match site.size {
            SampleSize::Point => 0,
            SampleSize::ThreeByThree => 1,
            SampleSize::FiveByFive => 2,
        });
        ui.site_lock.set_active(site.locked);
        let detail = site.position.map_or_else(
            || {
                format!(
                    "ID {} · detached from image · full-resolution linear Source",
                    site.id
                )
            },
            |position| {
                format!(
                    "ID {} · attached at {:.6}, {:.6} · alpha-weighted",
                    site.id, position[0], position[1]
                )
            },
        );
        ui.sample_info.set_subtitle(&detail);
        ui.sample_selector.set_tooltip_text(Some(&detail));
        ui.sample_selector
            .update_property(&[gtk::accessible::Property::Description(&detail)]);
    } else {
        ui.site_lock.set_active(false);
        let detail = "Select a site to inspect its ID and marker attachment";
        ui.sample_info.set_subtitle(detail);
        ui.sample_selector.set_tooltip_text(Some(detail));
        ui.sample_selector
            .update_property(&[gtk::accessible::Property::Description(detail)]);
    }
    let editable = site.as_ref().is_some_and(|site| !site.locked);
    for widget in [
        ui.output_picker.clone(),
        ui.source_picker.clone(),
        ui.delete_sample.clone(),
    ] {
        widget.set_sensitive(editable);
    }
    ui.influence.set_sensitive(editable);
    ui.sample_size
        .set_sensitive(editable && site.as_ref().is_some_and(|site| site.position.is_some()));
    ui.syncing.set(false);
}

fn schedule_voronoi(ui: &Ui, state: &Rc<RefCell<State>>, message: &str) {
    let mut state = state.borrow_mut();
    if state.jobs.is_busy() {
        ui.status
            .set_label("Finish or cancel the file operation before editing");
        return;
    }
    let Some(document) = state.document.as_mut() else {
        return;
    };
    document.dirty = true;
    let recipe = document.recipe.clone();
    if let Some(source) = state.preview_source.clone() {
        state.scheduler.schedule(source, recipe);
    }
    drop(state);
    ui.save.set_sensitive(true);
    ui.status.set_label(message);
    ui.canvas.queue_draw();
}

fn voronoi_controls(ui: &Rc<Ui>, state: Rc<RefCell<State>>) {
    {
        let state = state.clone();
        ui.output_swatch
            .set_draw_func(move |_, ctx, width, height| {
                let encoded = selected_output_color(&state)
                    .unwrap_or([0.25, 0.25, 0.25])
                    .map(threshiator::processing::linear_to_srgb);
                ctx.set_source_rgb(encoded[0] as f64, encoded[1] as f64, encoded[2] as f64);
                ctx.rectangle(0.0, 0.0, width as f64, height as f64);
                let _ = ctx.fill();
            });
    }
    {
        let state = state.clone();
        ui.source_swatch
            .set_draw_func(move |_, ctx, width, height| {
                let encoded = selected_source_color(&state)
                    .unwrap_or([0.25, 0.25, 0.25])
                    .map(threshiator::processing::linear_to_srgb);
                ctx.set_source_rgb(encoded[0] as f64, encoded[1] as f64, encoded[2] as f64);
                ctx.rectangle(0.0, 0.0, width as f64, height as f64);
                let _ = ctx.fill();
            });
    }
    {
        let ui = ui.clone();
        let state = state.clone();
        ui.sample_selector
            .clone()
            .connect_selected_notify(move |selector| {
                if ui.syncing.get() {
                    return;
                }
                let _ = selector.selected();
                sync_selected_controls(&ui, &state);
                ui.canvas.queue_draw();
            });
    }
    {
        let ui = ui.clone();
        let state = state.clone();
        let picker = ui.output_picker.clone();
        picker.connect_clicked(move |_| {
            present_color_picker(&ui, &state, ui.cli.color_model, PickerPurpose::Target);
        });
    }
    {
        let ui = ui.clone();
        let state = state.clone();
        ui.source_picker.clone().connect_clicked(move |_| {
            present_color_picker(&ui, &state, ui.cli.color_model, PickerPurpose::Source);
        });
    }
    {
        let ui = ui.clone();
        let state = state.clone();
        ui.site_lock.clone().connect_active_notify(move |control| {
            if ui.syncing.get() {
                return;
            }
            let site_id = state.borrow().selected_sample;
            let changed = state
                .borrow_mut()
                .document
                .as_mut()
                .is_some_and(|document| {
                    let changed = site_id.is_some_and(|id| {
                        document.recipe.voronoi.set_locked(id, control.is_active())
                    });
                    document.dirty |= changed;
                    changed
                });
            if changed {
                ui.save.set_sensitive(true);
                ui.status.set_label(if control.is_active() {
                    "Site locked — its parameters are protected"
                } else {
                    "Site unlocked — editing is available"
                });
                sync_selected_controls(&ui, &state);
                refresh_voronoi_ui(&ui, &state);
            }
        });
    }
    {
        let ui = ui.clone();
        let state = state.clone();
        ui.influence.clone().connect_value_changed(move |control| {
            if ui.syncing.get() {
                return;
            }
            let sample_id = state.borrow().selected_sample;
            let changed = state
                .borrow_mut()
                .document
                .as_mut()
                .is_some_and(|document| {
                    sample_id.is_some_and(|id| {
                        document.recipe.voronoi.set_influence(id, control.value())
                    })
                });
            if changed {
                schedule_voronoi(&ui, &state, "Influence changed — updating Coverage…");
            }
        });
    }
    {
        let ui = ui.clone();
        let state = state.clone();
        ui.sample_size
            .clone()
            .connect_selected_notify(move |control| {
                if ui.syncing.get() {
                    return;
                }
                let size = match control.selected() {
                    0 => SampleSize::Point,
                    2 => SampleSize::FiveByFive,
                    _ => SampleSize::ThreeByThree,
                };
                let sample_id = state.borrow().selected_sample;
                let changed = if let Some(document) = state.borrow_mut().document.as_mut() {
                    let source = document.source.clone();
                    let position = sample_id
                        .and_then(|id| document.recipe.voronoi.site(id))
                        .and_then(|site| (!site.locked).then_some(site.position).flatten());
                    if let (Some(id), Some(position)) = (sample_id, position)
                        && let Some(color) =
                            threshiator::voronoi::sample_color(&source, position, size)
                    {
                        document.recipe.voronoi.set_size(id, size, color)
                    } else {
                        false
                    }
                } else {
                    false
                };
                if changed {
                    schedule_voronoi(
                        &ui,
                        &state,
                        "Sampling footprint changed; Target reset — updating preview…",
                    );
                }
            });
    }
    for (button, group_delete) in [(&ui.delete_sample, false), (&ui.delete_group, true)] {
        let ui = ui.clone();
        let state = state.clone();
        button.connect_clicked(move |_| {
            let dialog = adw::AlertDialog::builder()
                .heading("Delete this site?")
                .body("This milestone has no undo history yet. This creative change cannot be undone.")
                .build();
            dialog.add_response("cancel", "Cancel"); dialog.add_response("delete", "Delete");
            dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
            let ui = ui.clone(); let state = state.clone();
            glib::spawn_future_local(async move {
                if dialog.choose_future(Some(&ui.window)).await != "delete" { return }
                let mut s = state.borrow_mut();
                let sample_id = s.selected_sample;
                let mut selection = sample_id;
                let mut deleted = false;
                if let Some(document) = s.document.as_mut() {
                    let _ = group_delete;
                    if sample_id.is_some_and(|id| document.recipe.voronoi.delete_site(id)) {
                        selection = document.recipe.voronoi.sites.first().map(|site| site.id);
                        deleted = true;
                    }
                }
                s.selected_group = selection;
                s.selected_sample = selection;
                drop(s);
                if deleted {
                    schedule_voronoi(&ui, &state, "Site deleted — updating preview…");
                } else {
                    ui.status.set_label("Unlock this site before deleting it");
                }
                refresh_voronoi_ui(&ui, &state);
                sync_selected_controls(&ui, &state);
            });
        });
    }
}

fn refresh_voronoi_ui(ui: &Ui, state: &Rc<RefCell<State>>) {
    while let Some(child) = ui.groups.first_child() {
        ui.groups.remove(&child);
    }
    let Some((sites, coverage, selected_site)) = ({
        let state = state.borrow();
        state.document.as_ref().map(|document| {
            (
                document.recipe.voronoi.sites.clone(),
                state.coverage.clone(),
                state.selected_group,
            )
        })
    }) else {
        return;
    };
    let visible = coverage.visible_total.max(1);
    for (index, site) in sites.iter().enumerate() {
        let covered = coverage
            .site_counts
            .iter()
            .find(|(id, _)| *id == site.id)
            .map(|(_, count)| *count)
            .unwrap_or(0);
        let row = gtk::ListBoxRow::new();
        row.set_widget_name(&site.id.to_string());
        let content = gtk::Box::new(gtk::Orientation::Vertical, 2);
        content.set_margin_top(6);
        content.set_margin_bottom(6);
        content.set_margin_start(8);
        content.set_margin_end(8);
        let title = gtk::Label::builder()
            .label(format!(
                "{}{}",
                site_label(index),
                if site.locked { "  🔒" } else { "" }
            ))
            .xalign(0.0)
            .build();
        let swatches = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        for (kind, color) in [
            (
                "Source",
                [
                    site.source_color[0],
                    site.source_color[1],
                    site.source_color[2],
                ],
            ),
            ("Target", site.target_color),
        ] {
            let swatch = gtk::DrawingArea::builder()
                .content_width(28)
                .content_height(18)
                .build();
            let accessible_label = format!("Site {} {kind} color swatch", index + 1);
            swatch.update_property(&[gtk::accessible::Property::Label(&accessible_label)]);
            swatch.set_draw_func(move |_, context, width, height| {
                let encoded = color.map(threshiator::processing::linear_to_srgb);
                context.set_source_rgb(encoded[0] as f64, encoded[1] as f64, encoded[2] as f64);
                context.rectangle(0.0, 0.0, width as f64, height as f64);
                let _ = context.fill();
            });
            let labeled = gtk::Box::new(gtk::Orientation::Vertical, 0);
            labeled.append(&swatch);
            labeled.append(
                &gtk::Label::builder()
                    .label(kind)
                    .css_classes(["caption"])
                    .build(),
            );
            swatches.append(&labeled);
        }
        let heading = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        heading.append(&swatches);
        heading.append(&title);
        let detail = gtk::Label::builder()
            .label(format!(
                "{} · Influence {:+.1} · {:.1}% coverage",
                if site.position.is_some() {
                    "Attached"
                } else {
                    "Detached"
                },
                site.influence,
                covered as f64 * 100.0 / visible as f64
            ))
            .xalign(0.0)
            .css_classes(["dim-label"])
            .build();
        content.append(&heading);
        content.append(&detail);
        row.set_child(Some(&content));
        ui.groups.append(&row);
        if selected_site == Some(site.id) {
            ui.groups.select_row(Some(&row));
        }
    }
}

fn canvas_sampling(ui: &Rc<Ui>, state: Rc<RefCell<State>>) {
    let click = gtk::GestureClick::new();
    let ui_click = ui.clone();
    let state_click = state.clone();
    click.connect_pressed(move |_, _, x, y| {
        let Some(position) = canvas_position(&ui_click.canvas, &state_click.borrow(), x, y) else {
            return;
        };
        let mut state = state_click.borrow_mut();
        let Some(sampling) = state.sampling else {
            let nearest = state.document.as_ref().and_then(|document| {
                document
                    .recipe
                    .voronoi
                    .sites
                    .iter()
                    .filter_map(|site| {
                        let marker = site.position?;
                        let d2 =
                            (marker[0] - position[0]).powi(2) + (marker[1] - position[1]).powi(2);
                        (d2 < 0.0025).then_some((d2, site.id))
                    })
                    .min_by(|a, b| a.0.total_cmp(&b.0))
            });
            if let Some((_, site_id)) = nearest {
                state.selected_group = Some(site_id);
                state.selected_sample = Some(site_id);
                drop(state);
                refresh_voronoi_ui(&ui_click, &state_click);
                sync_selected_controls(&ui_click, &state_click);
                ui_click.canvas.queue_draw();
            }
            return;
        };
        let selected_site = state.selected_sample;
        let Some(document) = state.document.as_mut() else {
            return;
        };
        let sample_size = match sampling {
            SamplingState::AddColor => SampleSize::ThreeByThree,
            SamplingState::AddSample => selected_site
                .and_then(|id| document.recipe.voronoi.site(id))
                .map(|site| site.size)
                .unwrap_or(SampleSize::ThreeByThree),
        };
        let Some(_color) =
            threshiator::voronoi::sample_color(&document.source, position, sample_size)
        else {
            drop(state);
            ui_click
                .status
                .set_label("That area is fully transparent; choose a visible source color");
            return;
        };
        let new_site = match sampling {
            SamplingState::AddColor => threshiator::voronoi::add_site_at(
                &mut document.recipe.voronoi,
                &document.source,
                position,
            ),
            SamplingState::AddSample => selected_site.filter(|id| {
                threshiator::voronoi::reattach_site(
                    &mut document.recipe.voronoi,
                    &document.source,
                    *id,
                    position,
                )
            }),
        };
        document.dirty = true;
        let recipe = document.recipe.clone();
        state.selected_group = new_site;
        state.selected_sample = new_site;
        state.sampling = None;
        let previous_mode = state.sampling_previous_mode.take();
        if let Some(source) = state.preview_source.clone() {
            state.scheduler.schedule(source, recipe);
        }
        drop(state);
        restore_view(&ui_click, previous_mode);
        refresh_voronoi_ui(&ui_click, &state_click);
        ui_click.save.set_sensitive(true);
        ui_click.status.set_label(match sampling {
            SamplingState::AddColor => "Site added — updating preview…",
            SamplingState::AddSample => "Source reattached; Target reset — updating preview…",
        });
        ui_click.canvas.queue_draw();
    });
    ui.canvas.add_controller(click);

    let drag_origin = Rc::new(RefCell::new(None::<[f64; 2]>));
    let drag = gtk::GestureDrag::new();
    {
        let origin = drag_origin.clone();
        let ui = ui.clone();
        let state = state.clone();
        drag.connect_drag_begin(move |_, x, y| {
            let Some(pointer) = canvas_position(&ui.canvas, &state.borrow(), x, y) else {
                return;
            };
            let nearest = state.borrow().document.as_ref().and_then(|document| {
                document
                    .recipe
                    .voronoi
                    .sites
                    .iter()
                    .filter_map(|site| {
                        let position = site.position?;
                        let d2 =
                            (position[0] - pointer[0]).powi(2) + (position[1] - pointer[1]).powi(2);
                        (d2 < 0.0025).then_some((d2, site.id, position, site.locked))
                    })
                    .min_by(|a, b| a.0.total_cmp(&b.0))
            });
            if let Some((_, site_id, position, locked)) = nearest {
                let mut state = state.borrow_mut();
                state.selected_group = Some(site_id);
                state.selected_sample = Some(site_id);
                if locked {
                    ui.status
                        .set_label("Unlock this site before moving its Source marker");
                } else {
                    *origin.borrow_mut() = Some(position);
                }
            }
        });
    }
    {
        let origin = drag_origin.clone();
        let ui = ui.clone();
        let state = state.clone();
        drag.connect_drag_update(move |_, dx, dy| {
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
            if resample_selected(&state, position) {
                ui.canvas.queue_draw();
            }
        });
    }
    {
        let origin = drag_origin;
        let ui = ui.clone();
        let state = state.clone();
        drag.connect_drag_end(move |_, _, _| {
            if origin.borrow_mut().take().is_some() {
                schedule_voronoi(&ui, &state, "Site moved; Target reset — updating preview…");
                refresh_voronoi_ui(&ui, &state);
                sync_selected_controls(&ui, &state);
            }
        });
    }
    ui.canvas.add_controller(drag);

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
                .document
                .as_ref()
                .and_then(|document| {
                    state
                        .selected_sample
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
            state.document.as_ref().and_then(|document| {
                document
                    .recipe
                    .voronoi
                    .sites
                    .iter()
                    .find(|site| Some(site.id) == state.selected_sample && !site.locked)
                    .and_then(|site| site.position)
            })
        };
        if let Some(position) = current {
            let dimensions = state_key
                .borrow()
                .document
                .as_ref()
                .map(|document| (document.source.width, document.source.height))
                .unwrap_or((1, 1));
            let moved = [
                (position[0] + dx / dimensions.0.max(1) as f64).clamp(0.0, 1.0),
                (position[1] + dy / dimensions.1.max(1) as f64).clamp(0.0, 1.0),
            ];
            if resample_selected(&state_key, moved) {
                schedule_voronoi(
                    &ui_key,
                    &state_key,
                    "Site nudged; Target reset — updating preview…",
                );
                refresh_voronoi_ui(&ui_key, &state_key);
                sync_selected_controls(&ui_key, &state_key);
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

fn resample_selected(state: &Rc<RefCell<State>>, position: [f64; 2]) -> bool {
    let mut state = state.borrow_mut();
    let sample_id = state.selected_sample;
    let Some(document) = state.document.as_mut() else {
        return false;
    };
    let Some(sample) = document
        .recipe
        .voronoi
        .sites
        .iter()
        .find(|site| Some(site.id) == sample_id && !site.locked)
        .cloned()
    else {
        return false;
    };
    let changed = threshiator::voronoi::reattach_site(
        &mut document.recipe.voronoi,
        &document.source,
        sample.id,
        position,
    );
    document.dirty |= changed;
    changed
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

fn show_voronoi_markers(method: Method) -> bool {
    method == Method::Voronoi
}

fn site_label(index: usize) -> String {
    format!("Site {}", index + 1)
}

fn sync_string_list(model: &gtk::StringList, labels: &[String]) {
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

fn selected_output_color(state: &Rc<RefCell<State>>) -> Option<[f32; 3]> {
    let state = state.borrow();
    let site_id = state.selected_sample?;
    state
        .document
        .as_ref()?
        .recipe
        .voronoi
        .sites
        .iter()
        .find(|site| site.id == site_id)
        .map(|site| site.target_color)
}

fn selected_source_color(state: &Rc<RefCell<State>>) -> Option<[f32; 3]> {
    let state = state.borrow();
    let site_id = state.selected_sample?;
    state
        .document
        .as_ref()?
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

#[derive(Clone, Copy)]
enum PickerPurpose {
    Source,
    Target,
}

#[derive(Clone)]
struct PickerControls {
    labels: [gtk::Label; 3],
    adjustments: [gtk::Adjustment; 3],
    spins: [gtk::SpinButton; 3],
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
    if matches!(controls.model.get(), ColorModel::Hsv | ColorModel::Hsl) {
        values[1] /= 100.0;
        values[2] /= 100.0;
    }
    values
}

fn picker_lightness_sequence(start: f64, target: f64, updates: u32) -> Vec<f64> {
    (1..=updates)
        .map(|index| start + (target - start) * index as f64 / updates as f64)
        .collect()
}

type PickerModelSpec = ([&'static str; 3], [(f64, f64); 3], [u32; 3]);

fn picker_model_spec(model: ColorModel) -> PickerModelSpec {
    match model {
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
        ColorModel::Oklab => (
            ["Lightness L", "Green–red a", "Blue–yellow b"],
            [(0.0, 1.0), (-0.5, 0.5), (-0.5, 0.5)],
            [6, 6, 6],
        ),
    }
}

fn configure_picker_model(controls: &PickerControls) {
    let model = controls.model.get();
    let (names, bounds, digits) = picker_model_spec(model);
    for index in 0..3 {
        controls.labels[index].set_label(names[index]);
        controls.adjustments[index].set_lower(bounds[index].0);
        controls.adjustments[index].set_upper(bounds[index].1);
        controls.adjustments[index].set_step_increment(if model == ColorModel::Oklab {
            0.000_001
        } else {
            0.1
        });
        controls.spins[index].set_digits(digits[index]);
        controls.spins[index].update_property(&[gtk::accessible::Property::Label(names[index])]);
    }
}

fn refresh_picker_outputs(controls: &PickerControls, draft: DraftColor) {
    let model = controls.model.get();
    let (_, _, digits) = picker_model_spec(model);
    let mut values = draft.values(model);
    if matches!(model, ColorModel::Hsv | ColorModel::Hsl) {
        values[1] *= 100.0;
        values[2] *= 100.0;
    }
    for index in 0..3 {
        controls.spins[index].update_property(&[gtk::accessible::Property::ValueText(&format!(
            "{:.prec$}",
            values[index],
            prec = digits[index] as usize
        ))]);
    }
    controls.hex.set_text(&display_hex(draft.linear));
    controls.wheel.queue_draw();
    controls.new_swatch.queue_draw();
}

fn sync_picker(controls: &PickerControls, draft: DraftColor) {
    controls.syncing.set(true);
    let model = controls.model.get();
    let mut values = draft.values(model);
    if matches!(model, ColorModel::Hsv | ColorModel::Hsl) {
        values[1] *= 100.0;
        values[2] *= 100.0;
    }
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
    match model {
        ColorModel::Hsv | ColorModel::Hsl => {
            current[0] = hue;
            current[1] = distance;
        }
        ColorModel::Oklab => {
            current = oklab_plane_projected(current[0], nx, ny);
        }
    }
    draft.borrow_mut().set_values(model, current);
    sync_picker(controls, *draft.borrow());
}

fn present_color_picker(
    ui: &Rc<Ui>,
    state: &Rc<RefCell<State>>,
    initial_model: ColorModel,
    purpose: PickerPurpose,
) {
    let original = match purpose {
        PickerPurpose::Source => selected_source_color(state),
        PickerPurpose::Target => selected_output_color(state),
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
        state.picker_lightness = (initial_model == ColorModel::Oklab)
            .then(|| draft.borrow().values(ColorModel::Oklab)[0]);
        state.picker_lightness_updates = 0;
        state.picker_lightness_elapsed_ms = None;
        state.picker_plane_render_max_us = 0;
        state.picker_plane_render_count = 0;
    }

    let dialog = adw::Dialog::builder()
        .title(match purpose {
            PickerPurpose::Source => "Choose Source Center",
            PickerPurpose::Target => "Choose Target Color",
        })
        .content_width(720)
        .content_height(600)
        .build();
    *ui.picker_dialog.borrow_mut() = Some(dialog.clone());
    let model_dropdown = gtk::DropDown::from_strings(&["HSV", "HSL", "OKLab"]);
    *ui.audit_picker_model.borrow_mut() = Some(model_dropdown.clone());
    model_dropdown.set_selected(match initial_model {
        ColorModel::Hsv => 0,
        ColorModel::Hsl => 1,
        ColorModel::Oklab => 2,
    });
    model_dropdown.update_property(&[gtk::accessible::Property::Label("Color model")]);
    let wheel = gtk::DrawingArea::builder()
        .content_width(300)
        .content_height(300)
        .focusable(true)
        .hexpand(true)
        .vexpand(true)
        .build();
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
        spin.set_width_chars(11);
        spin
    });
    let controls_box = gtk::Box::new(gtk::Orientation::Vertical, 12);
    controls_box.set_size_request(300, -1);
    controls_box.append(&model_dropdown);
    for index in 0..3 {
        let row = gtk::Box::new(gtk::Orientation::Vertical, 4);
        row.append(&labels[index]);
        let linked = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let scale = gtk::Scale::new(gtk::Orientation::Horizontal, Some(&adjustments[index]));
        scale.set_draw_value(false);
        scale.set_hexpand(true);
        linked.append(&scale);
        linked.append(&spins[index]);
        row.append(&linked);
        controls_box.append(&row);
    }
    let hex = gtk::Entry::builder().placeholder_text("#RRGGBB").build();
    *ui.audit_picker_hex.borrow_mut() = Some(hex.clone());
    hex.update_property(&[gtk::accessible::Property::Label("Hex color")]);
    controls_box.append(
        &gtk::Label::builder()
            .label("Hex (encoded sRGB)")
            .xalign(0.0)
            .build(),
    );
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
    original_box.append(&original_swatch);
    let new_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
    new_box.append(&gtk::Label::new(Some("New")));
    new_box.append(&new_swatch);
    swatches.append(&original_box);
    swatches.append(&new_box);
    controls_box.append(&swatches);
    let controls = PickerControls {
        labels,
        adjustments,
        spins,
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
    *ui.audit_picker_cancel.borrow_mut() = Some(cancel.clone());
    *ui.audit_picker_select.borrow_mut() = Some(select.clone());
    select.add_css_class("suggested-action");
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    buttons.append(&cancel);
    buttons.append(&select);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 16);
    content.set_margin_top(18);
    content.set_margin_bottom(18);
    content.set_margin_start(18);
    content.set_margin_end(18);
    content.append(
        &gtk::Label::builder()
            .label(match purpose {
                PickerPurpose::Source => "Choose Source Center",
                PickerPurpose::Target => "Choose Target Color",
            })
            .css_classes(["title-2"])
            .xalign(0.0)
            .build(),
    );
    content.append(
        &gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .vexpand(true)
            .child(&flow)
            .build(),
    );
    content.append(&buttons);
    dialog.set_child(Some(&content));

    {
        let draft = draft.clone();
        let controls = controls.clone();
        let state = state.clone();
        wheel.set_draw_func(move |_, ctx, width, height| {
            let started = std::time::Instant::now();
            draw_color_plane(ctx, width, height, *draft.borrow(), controls.model.get());
            let elapsed = started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64;
            let mut state = state.borrow_mut();
            state.picker_plane_render_count += 1;
            state.picker_plane_render_max_us = state.picker_plane_render_max_us.max(elapsed);
        });
    }
    {
        let encoded = original.map(threshiator::processing::linear_to_srgb);
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
        adjustment.connect_value_changed(move |_| {
            if controls.syncing.get() {
                return;
            }
            draft
                .borrow_mut()
                .set_values(controls.model.get(), picker_values(&controls));
            if index == 0 && controls.model.get() == ColorModel::Oklab {
                let mut state = state.borrow_mut();
                state.picker_lightness = Some(controls.adjustments[0].value());
                state.picker_lightness_updates += 1;
            }
            refresh_picker_outputs(&controls, *draft.borrow());
        });
    }
    {
        let controls = controls.clone();
        let draft = draft.clone();
        let state = state.clone();
        model_dropdown.connect_selected_notify(move |dropdown| {
            let selected = match dropdown.selected() {
                1 => ColorModel::Hsl,
                2 => ColorModel::Oklab,
                _ => ColorModel::Hsv,
            };
            controls.model.set(selected);
            {
                let mut state = state.borrow_mut();
                state.picker_model = selected;
                state.picker_lightness = (selected == ColorModel::Oklab)
                    .then(|| draft.borrow().values(ColorModel::Oklab)[0]);
            }
            configure_picker_model(&controls);
            sync_picker(&controls, *draft.borrow());
        });
    }
    {
        let controls = controls.clone();
        let draft = draft.clone();
        hex.connect_activate(move |entry| match parse_hex(entry.text().as_str()) {
            Ok(linear) => {
                draft.borrow_mut().linear = linear;
                entry.remove_css_class("error");
                sync_picker(&controls, *draft.borrow());
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
        click.connect_pressed(move |_, _, x, y| {
            update_draft_from_wheel(
                &controls,
                &draft,
                x,
                y,
                wheel.width() as f64,
                wheel.height() as f64,
            )
        });
    }
    wheel.add_controller(click);
    let drag = gtk::GestureDrag::new();
    {
        let controls = controls.clone();
        let draft = draft.clone();
        let wheel = wheel.clone();
        drag.connect_drag_update(move |gesture, dx, dy| {
            let Some((x, y)) = gesture.start_point() else {
                return;
            };
            update_draft_from_wheel(
                &controls,
                &draft,
                x + dx,
                y + dy,
                wheel.width() as f64,
                wheel.height() as f64,
            );
        });
    }
    wheel.add_controller(drag);
    let keys = gtk::EventControllerKey::new();
    {
        let controls = controls.clone();
        let draft = draft.clone();
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            let fine = modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK);
            let model = controls.model.get();
            let mut values = draft.borrow().values(model);
            if model == ColorModel::Oklab {
                let plane_key = match key {
                    gtk::gdk::Key::Left => PlaneKey::Left,
                    gtk::gdk::Key::Right => PlaneKey::Right,
                    gtk::gdk::Key::Up => PlaneKey::Up,
                    gtk::gdk::Key::Down => PlaneKey::Down,
                    gtk::gdk::Key::Home => PlaneKey::Home,
                    _ => return glib::Propagation::Proceed,
                };
                values = adjust_oklab_plane(values, plane_key, fine);
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
            draft.borrow_mut().set_values(model, values);
            sync_picker(&controls, *draft.borrow());
            controls.wheel.queue_draw();
            glib::Propagation::Stop
        });
    }
    wheel.add_controller(keys);
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
            let site_id = state.borrow().selected_sample;
            let selected = draft.borrow().commit();
            let changed = state
                .borrow_mut()
                .document
                .as_mut()
                .is_some_and(|document| {
                    let Some(id) = site_id else { return false };
                    match purpose {
                        PickerPurpose::Source => {
                            let alpha = document
                                .recipe
                                .voronoi
                                .site(id)
                                .map_or(1.0, |site| site.source_color[3]);
                            document.recipe.voronoi.set_source(
                                id,
                                [selected[0], selected[1], selected[2], alpha],
                                None,
                            )
                        }
                        PickerPurpose::Target => document.recipe.voronoi.set_target(id, selected),
                    }
                });
            dialog.close();
            sync_selected_controls(&ui, &state);
            if changed {
                schedule_voronoi(
                    &ui,
                    &state,
                    match purpose {
                        PickerPurpose::Source => {
                            "Source changed; Target reset and marker detached — updating preview…"
                        }
                        PickerPurpose::Target => "Target changed — updating preview…",
                    },
                );
            }
        });
    }
    {
        let state = state.clone();
        let ui = ui.clone();
        dialog.connect_closed(move |_| {
            state.borrow_mut().picker_visible = false;
            *ui.picker_dialog.borrow_mut() = None;
            *ui.audit_picker_model.borrow_mut() = None;
            *ui.audit_picker_hex.borrow_mut() = None;
            ui.audit_picker_channels.borrow_mut().clear();
            *ui.audit_picker_cancel.borrow_mut() = None;
            *ui.audit_picker_select.borrow_mut() = None;
        });
    }
    configure_picker_model(&controls);
    sync_picker(&controls, *draft.borrow());
    dialog.present(Some(&ui.window));
    if initial_model == ColorModel::Oklab
        && let Some(target) = ui.cli.picker_lightness
    {
        let values = Rc::new(picker_lightness_sequence(
            controls.adjustments[0].value(),
            target,
            120,
        ));
        let index = Rc::new(Cell::new(0usize));
        let adjustment = controls.adjustments[0].clone();
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

fn draw_color_plane(
    ctx: &gtk::cairo::Context,
    width: i32,
    height: i32,
    draft: DraftColor,
    model: ColorModel,
) {
    let size = width.min(height) as f64;
    let radius = size / 2.0;
    let left = (width as f64 - size) / 2.0;
    let top = (height as f64 - size) / 2.0;
    let steps = 42;
    let cell = size / steps as f64;
    let fixed = draft.values(model);
    for row in 0..steps {
        for column in 0..steps {
            let x = ((column as f64 + 0.5) * cell - radius) / radius;
            let y = ((row as f64 + 0.5) * cell - radius) / radius;
            if x.hypot(y) > 1.0 {
                continue;
            }
            let hue = y.atan2(x).to_degrees().rem_euclid(360.0);
            let distance = x.hypot(y);
            let linear = match model {
                ColorModel::Hsv => {
                    let mut values = fixed;
                    values[0] = hue;
                    values[1] = distance;
                    threshiator::color::encoded_to_linear(threshiator::color::hsv_to_encoded(
                        values,
                    ))
                }
                ColorModel::Hsl => {
                    let mut values = fixed;
                    values[0] = hue;
                    values[1] = distance;
                    threshiator::color::encoded_to_linear(threshiator::color::hsl_to_encoded(
                        values,
                    ))
                }
                ColorModel::Oklab => oklab_to_linear(oklab_plane(fixed[0], x, y)),
            };
            if in_srgb_gamut(linear) {
                let encoded =
                    linear.map(|value| threshiator::processing::linear_to_srgb(value as f32));
                ctx.set_source_rgb(encoded[0] as f64, encoded[1] as f64, encoded[2] as f64);
            } else {
                let hatch = (row + column) % 2 == 0;
                ctx.set_source_rgb(
                    if hatch { 0.32 } else { 0.22 },
                    if hatch { 0.32 } else { 0.22 },
                    if hatch { 0.32 } else { 0.22 },
                );
            }
            ctx.rectangle(
                left + column as f64 * cell,
                top + row as f64 * cell,
                cell + 1.0,
                cell + 1.0,
            );
            let _ = ctx.fill();
        }
    }
    let marker = match model {
        ColorModel::Hsv | ColorModel::Hsl => {
            let radians = fixed[0].to_radians();
            [radians.cos() * fixed[1], radians.sin() * fixed[1]]
        }
        ColorModel::Oklab => oklab_plane_coords(fixed),
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

fn canvas_draw(canvas: &gtk::DrawingArea, state: Rc<RefCell<State>>) {
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
            }
        }
        if let (Some(pixbuf), Some(document)) = (src, s.document.as_ref())
            && show_voronoi_markers(document.recipe.active_method)
        {
            let scale = (w as f64 / pixbuf.width() as f64).min(h as f64 / pixbuf.height() as f64);
            let left = (w as f64 - pixbuf.width() as f64 * scale) / 2.0;
            let top = (h as f64 - pixbuf.height() as f64 * scale) / 2.0;
            for (site_index, site) in document.recipe.voronoi.sites.iter().enumerate() {
                let Some(position) = site.position else {
                    continue;
                };
                let x = left + position[0] * pixbuf.width() as f64 * scale;
                let y = top + position[1] * pixbuf.height() as f64 * scale;
                let selected = s.selected_sample == Some(site.id);
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
                    .map(threshiator::processing::linear_to_srgb);
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

fn pixbuf(display: DisplayBuffer) -> Pixbuf {
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

fn schedule_cli_screenshot(ui: &Rc<Ui>, state: &Rc<RefCell<State>>) {
    let Some(path) = ui.cli.screenshot.clone() else {
        return;
    };
    let ui = ui.clone();
    let state = state.clone();
    let _ = std::fs::remove_file(&path);
    ui.window.queue_draw();
    glib::spawn_future_local(async move {
        let mut saved = false;
        for attempt in 0..5 {
            glib::timeout_future(std::time::Duration::from_millis(if attempt == 0 {
                750
            } else {
                400
            }))
            .await;
            let capture_widget: gtk::Widget = if state.borrow().picker_visible
                || state.borrow().threshold_editor_visible
                || ui.cli.show_preset_dialog
            {
                ui.window.clone().upcast()
            } else {
                ui.snapshot_root.clone().upcast()
            };
            let paintable = gtk::WidgetPaintable::new(Some(&capture_widget));
            let snapshot = gtk::Snapshot::new();
            paintable.snapshot(
                &snapshot,
                capture_widget.width() as f64,
                capture_widget.height() as f64,
            );
            let Some(node) = snapshot.to_node() else {
                continue;
            };
            let Some(renderer) = ui.window.renderer() else {
                continue;
            };
            let texture = renderer.render_texture(&node, None);
            let stride = texture.width() as usize * 4;
            let mut bytes = vec![0; stride * texture.height() as usize];
            texture.download(&mut bytes, stride);
            let black_fraction = bytes
                .chunks_exact(4)
                .filter(|pixel| pixel[0] < 3 && pixel[1] < 3 && pixel[2] < 3 && pixel[3] > 250)
                .count() as f64
                / (texture.width() as f64 * texture.height() as f64);
            let transparent_fraction = bytes.chunks_exact(4).filter(|pixel| pixel[3] < 250).count()
                as f64
                / (texture.width() as f64 * texture.height() as f64);
            let damaged_region = |x_end: usize, y_start: usize, y_end: usize| {
                let width = texture.width() as usize;
                let mut damaged = 0usize;
                let mut total = 0usize;
                for y in
                    y_start.min(texture.height() as usize)..y_end.min(texture.height() as usize)
                {
                    for x in 0..x_end.min(width) {
                        let pixel = &bytes[y * stride + x * 4..][..4];
                        damaged += usize::from(
                            pixel[3] < 250 || (pixel[0] < 3 && pixel[1] < 3 && pixel[2] < 3),
                        );
                        total += 1;
                    }
                }
                damaged as f64 / total.max(1) as f64
            };
            let header_damage = damaged_region(texture.width() as usize, 0, 48);
            let sidebar_damage = if texture.width() > 760 {
                damaged_region(
                    (texture.width() as f64 * 0.37) as usize,
                    48,
                    texture.height() as usize,
                )
            } else {
                0.0
            };
            // Threshold art can legitimately contain large pure-black regions. Structural
            // header/sidebar checks distinguish that content from an incomplete frame.
            if black_fraction <= 0.90
                && transparent_fraction <= 0.005
                && header_damage <= 0.10
                && sidebar_damage <= 0.10
            {
                saved = texture.save_to_png(&path).is_ok();
                break;
            }
            capture_widget.queue_draw();
        }
        let state = state.borrow();
        let method = state
            .document
            .as_ref()
            .map(|document| document.recipe.active_method);
        let threshold_metadata = state.document.as_ref().map(|document| {
            let threshold = &document.recipe.threshold;
            let components: Vec<_> = match threshold.active_space {
                ThresholdSpace::Rgb => threshold.rgb_state.components.iter().collect(),
                ThresholdSpace::Hsv => vec![
                    &threshold.hsv_state.hue,
                    &threshold.hsv_state.saturation,
                    &threshold.hsv_state.value,
                ],
            };
            serde_json::json!({
                "working_space": match threshold.active_space { ThresholdSpace::Rgb => "rgb", ThresholdSpace::Hsv => "hsv" },
                "link": match threshold.active_space {
                    ThresholdSpace::Rgb => threshold.rgb_state.link == LinkPolicy::Linked,
                    ThresholdSpace::Hsv => threshold.hsv_state.sv_link == LinkPolicy::Linked,
                },
                "components": components.iter().map(|component| serde_json::json!({
                    "enabled": component.enabled,
                    "bands": component.outputs.len(),
                    "boundaries": component.boundaries,
                    "outputs": component.outputs,
                })).collect::<Vec<_>>(),
                "hue_origin_degrees": threshold.hsv_state.hue_origin_degrees,
            })
        });
        let voronoi_matching =
            state
                .document
                .as_ref()
                .map(|document| match document.recipe.voronoi.matching {
                    VoronoiMatching::Perceptual => "perceptual",
                    VoronoiMatching::Rgb => "rgb",
                    VoronoiMatching::Hsv => "hsv",
                });
        let voronoi_sites = state.document.as_ref().map(|document| {
            document
                .recipe
                .voronoi
                .sites
                .iter()
                .map(|site| {
                    serde_json::json!({
                        "id": site.id,
                        "order": site.order,
                        "source": site.source_color,
                        "target": site.target_color,
                        "influence": site.influence,
                        "locked": site.locked,
                        "position": site.position,
                        "sample_size": format!("{:?}", site.size),
                    })
                })
                .collect::<Vec<_>>()
        });
        let metadata = serde_json::json!({
            "app_version": env!("CARGO_PKG_VERSION"),
            "source_path": ui.cli.open.as_ref().map(|path| path.display().to_string()),
            "requested_window_size": ui.cli.window_size.map(|(width, height)| serde_json::json!({"width": width, "height": height})),
            "actual_window_size": {"width": ui.snapshot_root.width(), "height": ui.snapshot_root.height()},
            "capture_success": saved,
            "screen": if state.document.is_some() { "document" } else { "welcome" },
            "document_kind": match state.document_kind { DocumentKind::Welcome => "none", DocumentKind::Image => "image", DocumentKind::Example => "example", DocumentKind::Project => "project" },
            "dirty": state.document.as_ref().is_some_and(|document| document.dirty),
            "example_embedded": state.document_kind == DocumentKind::Example,
            "project_path": state.project_path.as_ref().map(|path| path.display().to_string()),
            "threshold": threshold_metadata,
            "voronoi_matching": voronoi_matching,
            "voronoi_sites": voronoi_sites,
            "picker": {
                "visible": state.picker_visible,
                "model": match state.picker_model { ColorModel::Hsv => "hsv", ColorModel::Hsl => "hsl", ColorModel::Oklab => "oklab" },
                "lightness": state.picker_lightness,
                "lightness_updates": state.picker_lightness_updates,
                "lightness_sequence_elapsed_ms": state.picker_lightness_elapsed_ms,
                "plane_render_count": state.picker_plane_render_count,
                "plane_render_max_us": state.picker_plane_render_max_us,
            },
            "threshold_editor": {
                "visible": state.threshold_editor_visible,
                "target": state.threshold_editor_target.map(ThresholdEditTarget::label),
                "handle": state.threshold_editor_handle.map(threshold_handle_label),
            },
            "asserted": {
                "method": ui.cli.method.map(|method| match method { Method::Voronoi => "voronoi", Method::Thresholds => "thresholds" }),
                "view": ui.cli.view.map(|view| match view { CompareMode::Result => "result", CompareMode::Split => "split", CompareMode::Source => "source" }),
                "selected_group": ui.cli.select_group,
                "selected_sample": ui.cli.select_sample,
                "selected_site": ui.cli.select_site,
                "sampling_state": ui.cli.sampling.map(|sampling| match sampling { SamplingState::AddColor => "add-color", SamplingState::AddSample => "add-sample" }),
                "screen": if ui.cli.example || ui.cli.open.is_some() { "document" } else { "welcome" },
                "document_kind": if ui.cli.example { "example" } else if ui.cli.open.as_ref().is_some_and(|path| classify_open_path(path) == OpenKind::Project) { "project" } else if ui.cli.open.is_some() { "image" } else { "none" },
                "picker_visible": ui.cli.show_color_picker,
                "color_model": match ui.cli.color_model { ColorModel::Hsv => "hsv", ColorModel::Hsl => "hsl", ColorModel::Oklab => "oklab" },
                "picker_lightness": ui.cli.picker_lightness,
                "threshold_space": ui.cli.threshold_space.map(|space| match space { ThresholdSpace::Rgb => "rgb", ThresholdSpace::Hsv => "hsv" }),
                "threshold_link": ui.cli.threshold_link.map(|link| link == LinkPolicy::Linked),
                "threshold_component": ui.cli.threshold_component.map(|component| format!("{component:?}").to_lowercase()),
                "threshold_bands": ui.cli.threshold_bands,
                "threshold_bypass": ui.cli.threshold_bypass.map(|component| format!("{component:?}").to_lowercase()),
                "voronoi_matching": ui.cli.voronoi_matching.map(|matching| match matching { VoronoiMatching::Perceptual => "perceptual", VoronoiMatching::Rgb => "rgb", VoronoiMatching::Hsv => "hsv" }),
                "threshold_advanced": ui.cli.threshold_advanced,
                "threshold_editor_visible": ui.cli.show_threshold_editor,
                "preset_dialog_visible": ui.cli.show_preset_dialog,
                "threshold_editor_target": ui.cli.threshold_editor_target.map(ThresholdEditTarget::label),
                "threshold_editor_handle": ui.cli.threshold_editor_handle.map(threshold_handle_label),
            },
            "method": match method { Some(Method::Voronoi) => "voronoi", Some(Method::Thresholds) => "thresholds", None => "none" },
            "view": match state.mode { CompareMode::Result => "result", CompareMode::Split => "split", CompareMode::Source => "source" },
            "selected_group": state.selected_group,
            "selected_sample": state.selected_sample,
            "selected_site": state.selected_sample,
            "sampling_state": match state.sampling { Some(SamplingState::AddColor) => "add-color", Some(SamplingState::AddSample) => "add-sample", None => "none" },
        });
        let sidecar = PathBuf::from(format!("{}.json", path.display()));
        let _ = std::fs::write(sidecar, serde_json::to_vec_pretty(&metadata).unwrap());
        if !saved {
            ui.status.set_label("App-owned screenshot failed");
            eprintln!("app-owned screenshot rejected after five incomplete frames");
        }
        if ui.cli.quit_after_screenshot {
            ui.window.application().expect("application").quit();
        }
    });
}

fn mode_handler(
    button: &gtk::ToggleButton,
    mode: CompareMode,
    divider: &gtk::Scale,
    canvas: &gtk::DrawingArea,
    state: Rc<RefCell<State>>,
) {
    let d = divider.clone();
    let c = canvas.clone();
    button.connect_toggled(move |b| {
        if b.is_active() {
            state.borrow_mut().mode = mode;
            d.set_visible(mode == CompareMode::Split);
            c.queue_draw();
        }
    });
}

fn replacement_handler(
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
        .document
        .as_ref()
        .is_some_and(|document| document.dirty);
    if !dirty {
        execute_replacement(replacement, ui, state);
        return;
    }
    let dialog = adw::AlertDialog::builder()
        .heading("Save changes before replacing this document?")
        .body("Save continues only after the project is written successfully. Discard replaces without saving; Cancel keeps the current document.")
        .build();
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("save", "Save");
    dialog.add_response("discard", "Discard");
    dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
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
            filter.set_name(Some("Threshiator projects"));
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
            OpenKind::Project => "Open Threshiator Project",
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

fn start_example_open(ui: &Rc<Ui>, state: &Rc<RefCell<State>>) {
    let Some((sender, token)) = begin_job(ui, state, "Loading included Spectrum example…") else {
        return;
    };
    let cli = ui.cli.clone();
    thread::spawn(move || {
        let result = embedded_example_document().and_then(|mut document| {
            if let Some(method) = cli.method {
                document.recipe.active_method = method;
            }
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
            .document
            .as_ref()
            .is_some_and(|document| document.dirty),
    );
    ui.save_as.set_sensitive(state.document.is_some());
    ui.export.set_sensitive(state.document.is_some());
}

fn save_handler(button: &gtk::Button, ui: &Rc<Ui>, state: Rc<RefCell<State>>, force_as: bool) {
    let ui = ui.clone();
    button.connect_clicked(move |_| {
        if !force_as && let Some(path) = state.borrow().project_path.clone() {
            start_project_save(&ui, &state, path);
            return;
        }
        let dialog = gtk::FileDialog::builder()
            .title("Save Project As")
            .initial_name("Untitled.threshiator")
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
                    .document
                    .as_ref()
                    .is_some_and(|document| document.dirty);
                u.save.set_sensitive(dirty);
                u.save_as.set_sensitive(s.borrow().document.is_some());
            }
        });
    });
}

fn start_project_save(ui: &Rc<Ui>, state: &Rc<RefCell<State>>, path: PathBuf) {
    let (document, sender) = {
        let state = state.borrow();
        (state.document.clone(), state.sender.clone())
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

fn export_handler(button: &gtk::Button, ui: &Rc<Ui>, state: Rc<RefCell<State>>) {
    let ui = ui.clone();
    button.connect_clicked(move |_| {
        ui.export.set_sensitive(false);
        let choose = adw::AlertDialog::builder().heading("Choose export precision").body("PNG: encoded-sRGB integer. OpenEXR: linear-sRGB 32-bit float. Unsupported combinations are not substituted.").build();
        choose.add_response("cancel", "Cancel"); choose.add_response("p8", "PNG 8-bit"); choose.add_response("p16", "PNG 16-bit"); choose.add_response("exr", "OpenEXR 32f");
        let win = ui.window.clone(); let state = state.clone(); let ui = ui.clone();
        glib::spawn_future_local(async move {
            let (format, ext) = match choose.choose_future(Some(&win)).await.as_str() { "p8" => (ExportFormat::Png8, "png"), "p16" => (ExportFormat::Png16, "png"), "exr" => (ExportFormat::OpenExr32Float, "exr"), _ => { ui.export.set_sensitive(true); return } };
            let dialog = gtk::FileDialog::builder().title(format.description()).initial_name(format!("Threshiator-result.{ext}")).build();
            if let Ok(file) = dialog.save_future(Some(&win)).await && let Some(path) = file.path() {
                let (document, sender) = { let state = state.borrow(); (state.document.clone(), state.sender.clone()) };
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

fn poll(ui: &Rc<Ui>, state: Rc<RefCell<State>>) {
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
                            state.borrow_mut().document = None;
                            let (thresholds, outputs) = d.recipe.quantize();
                            ui.low.set_value(thresholds[0] as f64);
                            ui.high.set_value(thresholds[1] as f64);
                            ui.out_low.set_value(outputs[0] as f64);
                            ui.out_mid.set_value(outputs[1] as f64);
                            ui.out_high.set_value(outputs[2] as f64);
                            ui.hue.set_value(d.recipe.hue_degrees() as f64);
                            let profile = match d.interpretation.profile {
                                ProfileInterpretation::UntaggedAssumedSrgb => {
                                    "untagged; assumed sRGB"
                                }
                                ProfileInterpretation::EmbeddedProfileNotConverted => {
                                    "profile detected; not converted"
                                }
                            };
                            d.dirty = false;
                            let mut s = state.borrow_mut();
                            s.source_pixbuf = Some(pixbuf(source_display));
                            s.result_pixbuf = Some(pixbuf(result_display));
                            s.result = None;
                            s.preview_source = Some(preview_source);
                            s.coverage = coverage;
                            s.selected_group = d.recipe.voronoi.sites.first().map(|site| site.id);
                            s.selected_sample = s.selected_group;
                            let method = d.recipe.active_method;
                            s.document = Some(d);
                            s.project_path = path;
                            s.document_kind = document_kind;
                            drop(s);
                            ui.method_voronoi.set_active(method == Method::Voronoi);
                            ui.method_thresholds
                                .set_active(method == Method::Thresholds);
                            ui.voronoi_panel.set_visible(method == Method::Voronoi);
                            ui.voronoi_list_panel.set_visible(method == Method::Voronoi);
                            ui.thresholds_panel
                                .set_visible(method == Method::Thresholds);
                            {
                                let mut s = state.borrow_mut();
                                if let Some(index) = ui.cli.select_group {
                                    s.selected_group = s
                                        .document
                                        .as_ref()
                                        .and_then(|document| {
                                            document
                                                .recipe
                                                .voronoi
                                                .sites
                                                .get(index.saturating_sub(1))
                                        })
                                        .map(|site| site.id);
                                }
                                if let Some(index) = ui.cli.select_sample {
                                    s.selected_sample = s
                                        .document
                                        .as_ref()
                                        .and_then(|document| {
                                            document
                                                .recipe
                                                .voronoi
                                                .sites
                                                .get(index.saturating_sub(1))
                                        })
                                        .map(|site| site.id);
                                }
                                if let Some(index) = ui.cli.select_site {
                                    let site = s
                                        .document
                                        .as_ref()
                                        .and_then(|document| {
                                            document
                                                .recipe
                                                .voronoi
                                                .sites
                                                .get(index.saturating_sub(1))
                                        })
                                        .map(|site| site.id);
                                    s.selected_group = site;
                                    s.selected_sample = site;
                                }
                                if let Some(index) = ui.cli.lock_site
                                    && let Some(document) = s.document.as_mut()
                                    && let Some(site) = document
                                        .recipe
                                        .voronoi
                                        .sites
                                        .get_mut(index.saturating_sub(1))
                                {
                                    site.locked = true;
                                }
                                s.sampling = ui.cli.sampling;
                            }
                            if ui.cli.sampling.is_some() {
                                ui.source_mode.set_active(true);
                            }
                            refresh_voronoi_ui(&ui, &state);
                            refresh_preset_ui(&ui, &state, false);
                            sync_selected_controls(&ui, &state);
                            sync_threshold_ui(&ui, &state);
                            ui.stack.set_visible_child_name("document");
                            ui.save.set_sensitive(false);
                            ui.save_as.set_sensitive(true);
                            ui.export.set_sensitive(true);
                            match ui.cli.sampling {
                                Some(SamplingState::AddColor | SamplingState::AddSample) => ui.status.set_label("Click a visible source color to create a site — Escape cancels"),
                                None => ui.status.set_label(&format!("Ready — {profile}")),
                            }
                            ui.canvas.queue_draw();
                            if ui.cli.screenshot.is_some()
                                && ui.cli.window_size.is_some_and(|(width, _)| width <= 800)
                            {
                                ui.inspector_split.set_show_sidebar(true);
                            }
                            if ui.cli.show_color_picker {
                                present_color_picker(
                                    &ui,
                                    &state,
                                    ui.cli.color_model,
                                    PickerPurpose::Target,
                                );
                            }
                            if ui.cli.show_threshold_editor {
                                present_threshold_editor(&ui, &state);
                            }
                            if ui.cli.show_preset_dialog {
                                present_save_preset(&ui, &state);
                            }
                            schedule_cli_screenshot(&ui, &state);
                        }
                        Err(e) => {
                            let has_document = state.borrow().document.is_some();
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
                                if let Some(d) = s.document.as_mut() {
                                    d.dirty = false;
                                }
                                resolve_pending_after_save(
                                    &mut s.pending_replacement,
                                    SaveResolution::CurrentSuccess,
                                )
                            };
                            ui.save.set_sensitive(false);
                            ui.save_as.set_sensitive(true);
                            ui.status.set_label(&format!("Saved {}", path.display()));
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

#[derive(Clone)]
struct AuditLog {
    scenario: String,
    path: PathBuf,
    started: Instant,
    seq: Rc<Cell<u64>>,
    state: Weak<RefCell<State>>,
}

impl AuditLog {
    fn event(&self, phase: &str, control: &str, action: &str, value: serde_json::Value) {
        use std::io::Write;
        let seq = self.seq.get() + 1;
        self.seq.set(seq);
        let snapshot = self
            .state
            .upgrade()
            .map(|state| {
                let state = state.borrow();
                serde_json::json!({
                    "preview_generation": state.scheduler.current_generation(),
                    "dirty": state.document.as_ref().is_some_and(|document| document.dirty),
                    "dialog_state": {
                        "threshold": state.threshold_editor_visible,
                        "picker": state.picker_visible,
                    },
                    "job_busy": state.jobs.is_busy(),
                })
            })
            .unwrap_or(serde_json::Value::Null);
        let event = serde_json::json!({
            "seq": seq,
            "elapsed_ms": self.started.elapsed().as_millis(),
            "phase": phase,
            "scenario": self.scenario,
            "control": control,
            "action": action,
            "value": value,
            "callback_depth": 0,
            "state": snapshot,
        });
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            let _ = writeln!(file, "{event}");
        }
    }
}

type AuditStep = Box<dyn Fn() + 'static>;

fn maybe_start_ui_audit(ui: &Rc<Ui>, state: &Rc<RefCell<State>>) {
    let Some(scenario) = ui.cli.ui_audit_scenario.clone() else {
        return;
    };
    if ui.audit_started.get() || state.borrow().document.is_none() || state.borrow().jobs.is_busy()
    {
        return;
    }
    ui.audit_started.set(true);
    let path = ui
        .cli
        .ui_audit_log
        .clone()
        .unwrap_or_else(|| PathBuf::from("tests/artifacts/audit/ui-audit.jsonl"));
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::remove_file(&path);
    let log = AuditLog {
        scenario: scenario.clone(),
        path,
        started: Instant::now(),
        seq: Rc::new(Cell::new(0)),
        state: Rc::downgrade(state),
    };
    log.event("begin", "scenario", "start", serde_json::json!(scenario));
    let heartbeat = log.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
        heartbeat.event("heartbeat", "main-loop", "tick", serde_json::Value::Null);
        glib::ControlFlow::Continue
    });

    let mut steps: Vec<AuditStep> = Vec::new();
    let step = |control: &'static str,
                action: &'static str,
                value: serde_json::Value,
                operation: AuditStep| {
        let log = log.clone();
        Box::new(move || {
            log.event("begin", control, action, value.clone());
            operation();
            log.event("settled", control, action, value.clone());
        }) as AuditStep
    };

    match scenario.as_str() {
        "shell" | "narrow-core" => {
            for active in [false, true] {
                let button = ui.sidebar_button.clone();
                steps.push(step(
                    "sidebar",
                    "set-active",
                    serde_json::json!(active),
                    Box::new(move || button.set_active(active)),
                ));
            }
            for (control, operation) in [
                ("source", ui.source_mode.clone()),
                ("split", ui.split_mode.clone()),
                ("result", ui.result_mode.clone()),
            ] {
                steps.push(step(
                    "compare-mode",
                    "set-active",
                    serde_json::json!(control),
                    Box::new(move || operation.set_active(true)),
                ));
            }
            for value in [30.0, 210.0] {
                let adjustment = ui.hue.clone();
                steps.push(step(
                    "common-hue",
                    "set-value",
                    serde_json::json!(value),
                    Box::new(move || adjustment.set_value(value)),
                ));
            }
            for value in [0.2, 0.8] {
                let divider = ui.divider.clone();
                steps.push(step(
                    "comparison-divider",
                    "set-value",
                    serde_json::json!(value),
                    Box::new(move || divider.set_value(value)),
                ));
            }
            for button in [ui.method_thresholds.clone(), ui.method_voronoi.clone()] {
                steps.push(step(
                    "method",
                    "set-active",
                    serde_json::json!(true),
                    Box::new(move || button.set_active(true)),
                ));
            }
        }
        "threshold-inspector" => {
            let method = ui.method_thresholds.clone();
            steps.push(step(
                "method",
                "set-active",
                serde_json::json!("thresholds"),
                Box::new(move || method.set_active(true)),
            ));
            for selected in [0_u32, 1] {
                let drop = ui.threshold_space.clone();
                steps.push(step(
                    "working-space",
                    "set-selected",
                    serde_json::json!(selected),
                    Box::new(move || drop.set_selected(selected)),
                ));
                for active in [true, false] {
                    let switch = ui.threshold_link.clone();
                    steps.push(step(
                        "link",
                        "set-active",
                        serde_json::json!({"space": selected, "active": active}),
                        Box::new(move || switch.set_active(active)),
                    ));
                }
                for (index, control) in ui.threshold_process.iter().cloned().enumerate() {
                    for active in [false, true] {
                        let control = control.clone();
                        steps.push(step(
                            "process",
                            "set-active",
                            serde_json::json!({"space": selected, "component": index, "active": active}),
                            Box::new(move || control.set_active(active)),
                        ));
                    }
                }
                for value in [2.0, 8.0, 32.0] {
                    let control = ui.threshold_bands[0].clone();
                    steps.push(step(
                        "bands",
                        "set-value",
                        serde_json::json!({"space": selected, "value": value}),
                        Box::new(move || control.set_value(value)),
                    ));
                }
            }
        }
        "threshold-dialog" => {
            let method = ui.method_thresholds.clone();
            steps.push(step(
                "method",
                "set-active",
                serde_json::json!("thresholds"),
                Box::new(move || method.set_active(true)),
            ));
            let space = ui.threshold_space.clone();
            steps.push(step(
                "working-space",
                "set-selected",
                serde_json::json!("HSV"),
                Box::new(move || space.set_selected(1)),
            ));
            let link = ui.threshold_link.clone();
            steps.push(step(
                "link",
                "set-active",
                serde_json::json!(true),
                Box::new(move || link.set_active(true)),
            ));
            let ui_open = ui.clone();
            let state_open = state.clone();
            steps.push(step(
                "threshold-dialog",
                "open",
                serde_json::Value::Null,
                Box::new(move || present_threshold_editor(&ui_open, &state_open)),
            ));
            for index in 0..40 {
                let ui = ui.clone();
                let log_missing = log.clone();
                let selected = if index % 2 == 0 { 1 } else { 0 };
                steps.push(step(
                    "component",
                    "set-selected",
                    serde_json::json!(selected),
                    Box::new(move || {
                        if let Some(drop) = ui.audit_threshold_target.borrow().as_ref() {
                            drop.set_selected(selected);
                        } else {
                            log_missing.event(
                                "fail",
                                "component",
                                "missing-widget",
                                serde_json::Value::Null,
                            );
                        }
                    }),
                ));
            }
            for value in [0.2, 0.8] {
                let ui = ui.clone();
                let log_missing = log.clone();
                steps.push(step(
                    "precise-value",
                    "set-value",
                    serde_json::json!(value),
                    Box::new(move || {
                        if let Some(spin) = ui.audit_threshold_precise.borrow().as_ref() {
                            spin.set_value(value);
                        } else {
                            log_missing.event(
                                "fail",
                                "precise-value",
                                "missing-widget",
                                serde_json::Value::Null,
                            );
                        }
                    }),
                ));
            }
            let ui_done = ui.clone();
            let log_missing = log.clone();
            steps.push(step(
                "threshold-dialog",
                "done",
                serde_json::Value::Null,
                Box::new(move || {
                    let button = ui_done.audit_threshold_done.borrow().clone();
                    if let Some(button) = button {
                        button.emit_clicked();
                    } else {
                        log_missing.event(
                            "fail",
                            "threshold-dialog",
                            "missing-done",
                            serde_json::Value::Null,
                        );
                    }
                }),
            ));
            let ui_reopen = ui.clone();
            let state_reopen = state.clone();
            steps.push(step(
                "threshold-dialog",
                "reopen",
                serde_json::Value::Null,
                Box::new(move || present_threshold_editor(&ui_reopen, &state_reopen)),
            ));
            let cancel_snapshot = Rc::new(RefCell::new(None::<ComponentQuantizer>));
            let snapshot_store = cancel_snapshot.clone();
            let state_snapshot = state.clone();
            let log_snapshot = log.clone();
            steps.push(step(
                "threshold-dialog",
                "capture-before-cancel-edit",
                serde_json::Value::Null,
                Box::new(move || {
                    let quantizer = state_snapshot.borrow().document.as_ref().map(|document| {
                        let target = document
                            .recipe
                            .threshold
                            .reconcile_edit_target(ThresholdEditTarget::Hue);
                        document.recipe.threshold.edit_quantizer(target)
                    });
                    if quantizer.is_none() {
                        log_snapshot.event(
                            "fail",
                            "threshold-dialog",
                            "missing-document",
                            serde_json::Value::Null,
                        );
                    }
                    *snapshot_store.borrow_mut() = quantizer;
                }),
            ));
            let ui_edit = ui.clone();
            let log_missing = log.clone();
            steps.push(step(
                "precise-value",
                "edit-before-cancel",
                serde_json::json!(0.33),
                Box::new(move || {
                    if let Some(spin) = ui_edit.audit_threshold_precise.borrow().as_ref() {
                        spin.set_value(0.33);
                    } else {
                        log_missing.event(
                            "fail",
                            "precise-value",
                            "missing-widget",
                            serde_json::Value::Null,
                        );
                    }
                }),
            ));
            let ui_cancel = ui.clone();
            let log_missing = log.clone();
            steps.push(step(
                "threshold-dialog",
                "cancel",
                serde_json::Value::Null,
                Box::new(move || {
                    let button = ui_cancel.audit_threshold_cancel.borrow().clone();
                    if let Some(button) = button {
                        button.emit_clicked();
                    } else {
                        log_missing.event(
                            "fail",
                            "threshold-dialog",
                            "missing-cancel",
                            serde_json::Value::Null,
                        );
                    }
                }),
            ));
            let snapshot_check = cancel_snapshot.clone();
            let state_check = state.clone();
            let log_check = log.clone();
            steps.push(step(
                "threshold-dialog",
                "assert-cancel-rollback",
                serde_json::Value::Null,
                Box::new(move || {
                    let restored = state_check.borrow().document.as_ref().map(|document| {
                        let target = document
                            .recipe
                            .threshold
                            .reconcile_edit_target(ThresholdEditTarget::Hue);
                        document.recipe.threshold.edit_quantizer(target)
                    });
                    if restored != *snapshot_check.borrow() {
                        log_check.event(
                            "fail",
                            "threshold-dialog",
                            "cancel-rollback",
                            serde_json::json!(false),
                        );
                    }
                }),
            ));
            let ui_reopen = ui.clone();
            let state_reopen = state.clone();
            steps.push(step(
                "threshold-dialog",
                "reopen",
                serde_json::Value::Null,
                Box::new(move || present_threshold_editor(&ui_reopen, &state_reopen)),
            ));
            let ui_close = ui.clone();
            let log_missing = log.clone();
            steps.push(step(
                "threshold-dialog",
                "close",
                serde_json::Value::Null,
                Box::new(move || {
                    let dialog = ui_close.threshold_editor_dialog.borrow().clone();
                    if let Some(dialog) = dialog {
                        dialog.close();
                    } else {
                        log_missing.event(
                            "fail",
                            "threshold-dialog",
                            "missing-close-dialog",
                            serde_json::Value::Null,
                        );
                    }
                }),
            ));
        }
        "voronoi" => {
            let method = ui.method_voronoi.clone();
            steps.push(step(
                "method",
                "set-active",
                serde_json::json!("voronoi"),
                Box::new(move || method.set_active(true)),
            ));
            for selected in 0_u32..3 {
                let drop = ui.voronoi_matching.clone();
                steps.push(step(
                    "matching",
                    "set-selected",
                    serde_json::json!(selected),
                    Box::new(move || drop.set_selected(selected)),
                ));
            }
            for value in [0.1, 4.0] {
                let spin = ui.influence.clone();
                steps.push(step(
                    "influence",
                    "set-value",
                    serde_json::json!(value),
                    Box::new(move || spin.set_value(value)),
                ));
            }
            for selected in 0_u32..3 {
                let drop = ui.sample_size.clone();
                steps.push(step(
                    "sampling-footprint",
                    "set-selected",
                    serde_json::json!(selected),
                    Box::new(move || drop.set_selected(selected)),
                ));
            }
            for active in [true, false] {
                let lock = ui.site_lock.clone();
                steps.push(step(
                    "site-lock",
                    "set-active",
                    serde_json::json!(active),
                    Box::new(move || lock.set_active(active)),
                ));
            }
            let reattach = ui.add_sample.clone();
            steps.push(step(
                "reattach-source",
                "arm",
                serde_json::json!("canvas click remains manual"),
                Box::new(move || reattach.emit_clicked()),
            ));
            steps.push(step(
                "add-site",
                "skip",
                serde_json::json!("real canvas pointer/portal path requires manual coverage"),
                Box::new(|| {}),
            ));
        }
        "presets" => {
            let ui_open = ui.clone();
            let state_open = state.clone();
            steps.push(step(
                "preset-dialog",
                "open",
                serde_json::Value::Null,
                Box::new(move || present_save_preset(&ui_open, &state_open)),
            ));
            let ui_cancel = ui.clone();
            steps.push(step(
                "preset-dialog",
                "cancel",
                serde_json::Value::Null,
                Box::new(move || {
                    let button = ui_cancel.audit_preset_cancel.borrow().clone();
                    if let Some(button) = button {
                        button.emit_clicked();
                    }
                }),
            ));
            let ui_open = ui.clone();
            let state_open = state.clone();
            steps.push(step(
                "preset-dialog",
                "reopen",
                serde_json::Value::Null,
                Box::new(move || present_save_preset(&ui_open, &state_open)),
            ));
            let ui_name = ui.clone();
            steps.push(step(
                "preset-name",
                "set-text",
                serde_json::json!("Audit Look"),
                Box::new(move || {
                    if let Some(entry) = ui_name.audit_preset_name.borrow().as_ref() {
                        entry.set_text("Audit Look");
                    }
                }),
            ));
            let ui_save = ui.clone();
            steps.push(step(
                "preset-dialog",
                "save-repeat-guard",
                serde_json::Value::Null,
                Box::new(move || {
                    let button = ui_save.audit_preset_save.borrow().clone();
                    if let Some(button) = button {
                        button.emit_clicked();
                        button.emit_clicked();
                    }
                }),
            ));
            let refresh = ui.preset_refresh.clone();
            steps.push(step(
                "presets",
                "refresh",
                serde_json::Value::Null,
                Box::new(move || refresh.emit_clicked()),
            ));
            let method = ui.method_thresholds.clone();
            steps.push(step(
                "method",
                "change-before-apply",
                serde_json::json!("thresholds"),
                Box::new(move || method.set_active(true)),
            ));
            let generation_before = Rc::new(Cell::new(0_u64));
            let capture_state = state.clone();
            let capture_generation = generation_before.clone();
            steps.push(step(
                "presets",
                "capture-generation",
                serde_json::Value::Null,
                Box::new(move || {
                    capture_generation.set(capture_state.borrow().scheduler.current_generation());
                }),
            ));
            let apply = ui.preset_apply.clone();
            steps.push(step(
                "presets",
                "apply",
                serde_json::Value::Null,
                Box::new(move || apply.emit_clicked()),
            ));
            let state_check = state.clone();
            let generation_check = generation_before.clone();
            let log_check = log.clone();
            steps.push(step(
                "presets",
                "assert-one-generation",
                serde_json::Value::Null,
                Box::new(move || {
                    let before = generation_check.get();
                    let after = state_check.borrow().scheduler.current_generation();
                    if after != before + 1 {
                        log_check.event(
                            "fail",
                            "presets",
                            "apply-generation",
                            serde_json::json!({"before": before, "after": after}),
                        );
                    }
                    generation_check.set(after);
                }),
            ));
            let apply = ui.preset_apply.clone();
            steps.push(step(
                "presets",
                "apply-noop",
                serde_json::Value::Null,
                Box::new(move || apply.emit_clicked()),
            ));
            let state_check = state.clone();
            let generation_check = generation_before;
            let log_check = log.clone();
            steps.push(step(
                "presets",
                "assert-noop-generation",
                serde_json::Value::Null,
                Box::new(move || {
                    let expected = generation_check.get();
                    let actual = state_check.borrow().scheduler.current_generation();
                    if actual != expected {
                        log_check.event(
                            "fail",
                            "presets",
                            "noop-generation",
                            serde_json::json!({"expected": expected, "actual": actual}),
                        );
                    }
                }),
            ));
        }
        "picker" => {
            let ui_open = ui.clone();
            let state_open = state.clone();
            steps.push(step(
                "color-picker",
                "open",
                serde_json::Value::Null,
                Box::new(move || {
                    present_color_picker(
                        &ui_open,
                        &state_open,
                        ColorModel::Hsv,
                        PickerPurpose::Target,
                    )
                }),
            ));
            steps.push(step(
                "picker-model",
                "skip",
                serde_json::json!("RGB model is not present; tracked by THR-013"),
                Box::new(|| {}),
            ));
            for selected in 0_u32..3 {
                let ui_model = ui.clone();
                let log_missing = log.clone();
                steps.push(step(
                    "picker-model",
                    "set-selected",
                    serde_json::json!(selected),
                    Box::new(move || {
                        if let Some(drop) = ui_model.audit_picker_model.borrow().as_ref() {
                            drop.set_selected(selected);
                        } else {
                            log_missing.event(
                                "fail",
                                "picker-model",
                                "missing-widget",
                                serde_json::Value::Null,
                            );
                        }
                    }),
                ));
                let samples: [[f64; 2]; 3] = if selected == 2 {
                    [[0.2, 0.8], [-0.1, 0.1], [-0.1, 0.1]]
                } else {
                    [[20.0, 280.0], [0.2, 0.8], [0.25, 0.75]]
                };
                for (index, values) in samples.into_iter().enumerate() {
                    for value in values {
                        let ui = ui.clone();
                        let log_missing = log.clone();
                        steps.push(step(
                            "picker-channel",
                            "set-value",
                            serde_json::json!({"model": selected, "channel": index, "value": value}),
                            Box::new(move || {
                                if let Some(adjustment) = ui.audit_picker_channels.borrow().get(index) {
                                    adjustment.set_value(value);
                                } else {
                                    log_missing.event("fail", "picker-channel", "missing-widget", serde_json::json!(index));
                                }
                            }),
                        ));
                    }
                }
            }
            for value in ["#336699", "invalid", "#ff8800"] {
                let ui = ui.clone();
                let text = value.to_string();
                let log_missing = log.clone();
                steps.push(step(
                    "picker-hex",
                    "activate",
                    serde_json::json!(value),
                    Box::new(move || {
                        if let Some(entry) = ui.audit_picker_hex.borrow().as_ref() {
                            entry.set_text(&text);
                            entry.emit_by_name::<()>("activate", &[]);
                        } else {
                            log_missing.event(
                                "fail",
                                "picker-hex",
                                "missing-widget",
                                serde_json::Value::Null,
                            );
                        }
                    }),
                ));
            }
            let ui_cancel = ui.clone();
            let log_missing = log.clone();
            steps.push(step(
                "color-picker",
                "cancel",
                serde_json::Value::Null,
                Box::new(move || {
                    let button = ui_cancel.audit_picker_cancel.borrow().clone();
                    if let Some(button) = button {
                        button.emit_clicked();
                    } else {
                        log_missing.event(
                            "fail",
                            "color-picker",
                            "missing-cancel",
                            serde_json::Value::Null,
                        );
                    }
                }),
            ));
            let ui_reopen = ui.clone();
            let state_reopen = state.clone();
            steps.push(step(
                "color-picker",
                "reopen",
                serde_json::Value::Null,
                Box::new(move || {
                    present_color_picker(
                        &ui_reopen,
                        &state_reopen,
                        ColorModel::Hsl,
                        PickerPurpose::Target,
                    )
                }),
            ));
            let ui_select = ui.clone();
            let log_missing = log.clone();
            steps.push(step(
                "color-picker",
                "select",
                serde_json::Value::Null,
                Box::new(move || {
                    let button = ui_select.audit_picker_select.borrow().clone();
                    if let Some(button) = button {
                        button.emit_clicked();
                    } else {
                        log_missing.event(
                            "fail",
                            "color-picker",
                            "missing-select",
                            serde_json::Value::Null,
                        );
                    }
                }),
            ));
            let ui_reopen = ui.clone();
            let state_reopen = state.clone();
            steps.push(step(
                "color-picker",
                "reopen",
                serde_json::Value::Null,
                Box::new(move || {
                    present_color_picker(
                        &ui_reopen,
                        &state_reopen,
                        ColorModel::Oklab,
                        PickerPurpose::Target,
                    )
                }),
            ));
            let ui_close = ui.clone();
            let log_missing = log.clone();
            steps.push(step(
                "color-picker",
                "close",
                serde_json::Value::Null,
                Box::new(move || {
                    let dialog = ui_close.picker_dialog.borrow().clone();
                    if let Some(dialog) = dialog {
                        dialog.close();
                    } else {
                        log_missing.event(
                            "fail",
                            "color-picker",
                            "missing-close-dialog",
                            serde_json::Value::Null,
                        );
                    }
                }),
            ));
        }
        "io-workflow" => {
            let ui_dialogs = ui.clone();
            let state_dialogs = state.clone();
            steps.push(step(
                "io-conflict",
                "open-threshold-dialog",
                serde_json::Value::Null,
                Box::new(move || present_threshold_editor(&ui_dialogs, &state_dialogs)),
            ));
            let ui_dialogs = ui.clone();
            let state_dialogs = state.clone();
            steps.push(step(
                "io-conflict",
                "open-picker",
                serde_json::Value::Null,
                Box::new(move || {
                    present_color_picker(
                        &ui_dialogs,
                        &state_dialogs,
                        ColorModel::Hsv,
                        PickerPurpose::Target,
                    )
                }),
            ));
            let ui_conflict = ui.clone();
            let state_conflict = state.clone();
            let log_conflict = log.clone();
            steps.push(step(
                "io-conflict",
                "reject-with-creative-dialog",
                serde_json::Value::Null,
                Box::new(move || {
                    let both_open = ui_conflict.threshold_editor_dialog.borrow().is_some()
                        && ui_conflict.picker_dialog.borrow().is_some();
                    let rejected =
                        begin_job(&ui_conflict, &state_conflict, "Must reject").is_none();
                    let safe = rejected
                        && both_open
                        && ui_conflict.stack.is_sensitive()
                        && !state_conflict.borrow().jobs.is_busy();
                    if !safe {
                        log_conflict.event(
                            "fail",
                            "io-conflict",
                            "dialog-job-rejection",
                            serde_json::json!({"rejected": rejected, "both_open": both_open}),
                        );
                    }
                }),
            ));
            let ui_close = ui.clone();
            let log_close = log.clone();
            steps.push(step(
                "io-conflict",
                "close-dialogs",
                serde_json::Value::Null,
                Box::new(move || {
                    let threshold = ui_close.threshold_editor_dialog.borrow().clone();
                    let picker = ui_close.picker_dialog.borrow().clone();
                    if let (Some(threshold), Some(picker)) = (threshold, picker) {
                        picker.close();
                        threshold.close();
                    } else {
                        log_close.event(
                            "fail",
                            "io-conflict",
                            "missing-dialog",
                            serde_json::Value::Null,
                        );
                    }
                }),
            ));

            let active_token = Rc::new(RefCell::new(None::<JobToken>));
            let token_store = active_token.clone();
            let ui_lock = ui.clone();
            let state_lock = state.clone();
            let log_lock = log.clone();
            steps.push(step(
                "io-lock",
                "begin-and-assert",
                serde_json::Value::Null,
                Box::new(move || {
                    let dirty_before = state_lock.borrow().document.as_ref().is_some_and(|d| d.dirty);
                    let Some((_, token)) = begin_job(&ui_lock, &state_lock, "Audit I/O lock…") else {
                        log_lock.event("fail", "io-lock", "begin-rejected", serde_json::Value::Null);
                        return;
                    };
                    let locked = !ui_lock.stack.is_sensitive();
                    let cancel_reachable = ui_lock.cancel.is_visible() && ui_lock.cancel.is_sensitive();
                    let dirty_after = state_lock.borrow().document.as_ref().is_some_and(|d| d.dirty);
                    if !(locked && cancel_reachable && dirty_before == dirty_after) {
                        log_lock.event("fail", "io-lock", "creative-stack", serde_json::json!({"locked": locked, "cancel_reachable": cancel_reachable, "dirty_unchanged": dirty_before == dirty_after}));
                    }
                    *token_store.borrow_mut() = Some(token);
                }),
            ));
            let ui_cancel = ui.clone();
            steps.push(step(
                "io-lock",
                "cancel-click",
                serde_json::Value::Null,
                Box::new(move || ui_cancel.cancel.emit_clicked()),
            ));
            let token_finish = active_token.clone();
            let ui_finish = ui.clone();
            let state_finish = state.clone();
            let log_finish = log.clone();
            steps.push(step(
                "io-lock",
                "acknowledge-cancel-and-restore",
                serde_json::Value::Null,
                Box::new(move || {
                    let Some(token) = token_finish.borrow_mut().take() else {
                        log_finish.event(
                            "fail",
                            "io-lock",
                            "missing-token",
                            serde_json::Value::Null,
                        );
                        return;
                    };
                    if token.is_current()
                        || !state_finish
                            .borrow_mut()
                            .jobs
                            .acknowledge(token.generation())
                    {
                        log_finish.event(
                            "fail",
                            "io-lock",
                            "cancel-token",
                            serde_json::Value::Null,
                        );
                    }
                    finish_job(&ui_finish, &state_finish);
                    if !ui_finish.stack.is_sensitive() || ui_finish.cancel.is_visible() {
                        log_finish.event(
                            "fail",
                            "io-lock",
                            "restore-after-cancel",
                            serde_json::Value::Null,
                        );
                    }
                }),
            ));

            let ui_busy = ui.clone();
            let state_busy = state.clone();
            let log_busy = log.clone();
            steps.push(step(
                "io-lock",
                "second-job-rejection-and-finish",
                serde_json::Value::Null,
                Box::new(move || {
                    let Some((_, token)) = begin_job(&ui_busy, &state_busy, "First audit job")
                    else {
                        log_busy.event(
                            "fail",
                            "io-lock",
                            "first-job-rejected",
                            serde_json::Value::Null,
                        );
                        return;
                    };
                    let rejected = begin_job(&ui_busy, &state_busy, "Second audit job").is_none();
                    if !rejected || !state_busy.borrow_mut().jobs.acknowledge(token.generation()) {
                        log_busy.event(
                            "fail",
                            "io-lock",
                            "second-job-rejection",
                            serde_json::json!(rejected),
                        );
                    }
                    finish_job(&ui_busy, &state_busy);
                    if !ui_busy.stack.is_sensitive() {
                        log_busy.event(
                            "fail",
                            "io-lock",
                            "finish-restoration",
                            serde_json::Value::Null,
                        );
                    }
                }),
            ));
            steps.push(step(
                "portal-file-dialog",
                "skip",
                serde_json::json!("portal chooser requires manual desktop integration coverage"),
                Box::new(|| {}),
            ));
        }
        _ => {
            let log_fail = log.clone();
            steps.push(step(
                "scenario",
                "unknown",
                serde_json::json!("unknown scenario"),
                Box::new(move || {
                    log_fail.event("fail", "scenario", "unknown", serde_json::Value::Null)
                }),
            ));
        }
    }

    let steps = Rc::new(RefCell::new(steps.into_iter()));
    let window = ui.window.clone();
    let log_done = log.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(70), move || {
        if let Some(next) = steps.borrow_mut().next() {
            next();
            glib::ControlFlow::Continue
        } else {
            log_done.event("settled", "scenario", "complete", serde_json::Value::Null);
            if let Some(app) = window.application() {
                app.quit();
            }
            glib::ControlFlow::Break
        }
    });
}

fn error(ui: &Ui, heading: &str, body: &str) {
    ui.status.set_label(body);
    let d = adw::AlertDialog::builder()
        .heading(heading)
        .body(body)
        .build();
    d.add_response("close", "Close");
    d.present(Some(&ui.window));
}

fn actions(
    app: &adw::Application,
    open: &gtk::Button,
    save: &gtk::Button,
    save_as: &gtk::Button,
    export: &gtk::Button,
) {
    for (name, keys, button) in [
        ("open", &["<primary>o"][..], open),
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

fn close_guard(window: &adw::ApplicationWindow, state: Rc<RefCell<State>>) {
    let allow = Rc::new(Cell::new(false));
    let win = window.clone();
    window.connect_close_request(move |_| {
        if allow.get() || !state.borrow().document.as_ref().is_some_and(|d| d.dirty) {
            return glib::Propagation::Proceed;
        }
        let d = adw::AlertDialog::builder()
            .heading("Discard unsaved creative changes?")
            .body("View and divider changes are not saved, but recipe changes are unsaved.")
            .build();
        d.add_response("cancel", "Cancel");
        d.add_response("discard", "Discard Changes");
        d.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
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
        Cli, CompareMode, Method, application_flags, parse_threshold_edit_target,
        parse_threshold_handle, picker_lightness_sequence, show_voronoi_markers, site_label,
        threshold_precision_accessibility_text,
    };

    #[test]
    fn picker_lightness_sequence_is_dense_and_reaches_target() {
        let values = picker_lightness_sequence(0.1, 0.9, 120);
        assert_eq!(values.len(), 120);
        assert!(values.windows(2).all(|pair| pair[0] < pair[1]));
        assert!((values[119] - 0.9).abs() < f64::EPSILON);
    }

    #[test]
    fn screenshot_instances_are_non_unique() {
        let mut cli = Cli::default();
        assert!(application_flags(&cli).is_empty());
        cli.screenshot = Some("evidence.png".into());
        assert!(application_flags(&cli).contains(gio::ApplicationFlags::NON_UNIQUE));
    }

    #[test]
    fn site_list_uses_compact_creator_labels() {
        assert_eq!(site_label(0), "Site 1");
        assert_eq!(site_label(11), "Site 12");
    }

    #[test]
    fn voronoi_markers_follow_method_in_every_comparison_view() {
        for _view in [CompareMode::Result, CompareMode::Split, CompareMode::Source] {
            assert!(show_voronoi_markers(Method::Voronoi));
            assert!(!show_voronoi_markers(Method::Thresholds));
        }
    }

    #[test]
    fn threshold_editor_cli_targets_are_semantic() {
        assert_eq!(
            parse_threshold_edit_target("linked-sv".into())
                .unwrap()
                .label(),
            "Saturation & Value"
        );
        assert!(parse_threshold_edit_target("dropdown-index-2".into()).is_none());
        assert_eq!(
            parse_threshold_handle("boundary:1".into()),
            Some(super::ThresholdHandle::Boundary(0))
        );
        assert!(parse_threshold_handle("boundary:0".into()).is_none());
    }

    #[test]
    fn hue_output_accessibility_uses_degree_range_only() {
        let (_, description) = threshold_precision_accessibility_text(
            threshiator::document::ThresholdEditTarget::Hue,
            super::ThresholdHandle::Output(0),
            false,
        );
        assert!(description.contains("0 to 360"));
        assert!(!description.contains("0 to 1"));

        let (_, normalized) = threshold_precision_accessibility_text(
            threshiator::document::ThresholdEditTarget::Saturation,
            super::ThresholdHandle::Output(0),
            false,
        );
        assert!(normalized.contains("0 to 1"));
        assert!(!normalized.contains("0 to 360"));
    }
}
