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
    ColorModel, DraftColor, OkhslPlaneSampler, PlaneKey, adjust_okhsl, display_hex,
    linear_to_encoded, okhsl_to_linear, parse_hex,
};
use threshiator::document::{
    ComponentQuantizer, Document, ExportDefaults, LinkPolicy, Method, PixelImage,
    ProfileInterpretation, Recipe, SampleSize, ThresholdEditGesture, ThresholdEditTarget,
    ThresholdSpace, ThresholdState, VoronoiMatching, clamp_threshold_boundary,
    threshold_display_value, threshold_normalized_value,
};
use threshiator::export::{self, ExportFormat};
use threshiator::histogram::{
    HsvHistogramComponent, ThresholdHistograms, hsv_input_strip_color, hsv_output_strip_color,
    relative_hue_histogram, threshold_histogram_series_mask,
};
use threshiator::preset::{Preset, PresetDiagnostic, PresetEntry, PresetStore, apply_to_document};
use threshiator::processing::{
    Coverage, DisplayBuffer, bounded_preview, process_cancellable_with_progress_and_coverage,
    to_display_rgba8,
};
use threshiator::scheduler::{JobCoordinator, JobToken, PreviewScheduler, ProgressTracker};
use threshiator::starter_looks::{
    STARTER_LOOKS, apply_starter_look, reset_active_space, reset_component, reset_method,
};
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

const VORONOI_MATCHING_LABELS: &[&str] = &["Perceptual (OKLab)", "RGB (sRGB)", "HSV"];
const CREATIVE_FOCUS_CLASS: &str = "creative-focus";
const MEDIUM_BREAKPOINT: i32 = 1100;
const NARROW_BREAKPOINT: i32 = 760;
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
    threshold_lock: Option<ThresholdComponent>,
    threshold_auto: Option<ThresholdComponent>,
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
                "--threshold-lock" => {
                    cli.threshold_lock = args.next().and_then(parse_threshold_component)
                }
                "--threshold-auto" => {
                    cli.threshold_auto = args.next().and_then(parse_threshold_component)
                }
                "--voronoi-matching" => {
                    cli.voronoi_matching = args.next().and_then(|value| match value.as_str() {
                        "perceptual" => Some(VoronoiMatching::Perceptual),
                        "okhsl" => Some(VoronoiMatching::Okhsl),
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
                            "okhsl" => Some(ColorModel::Okhsl),
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

fn threshold_component_index(
    space: ThresholdSpace,
    component: ThresholdComponent,
) -> Option<usize> {
    match (space, component) {
        (ThresholdSpace::Rgb, ThresholdComponent::Red)
        | (ThresholdSpace::Hsv, ThresholdComponent::Hue) => Some(0),
        (ThresholdSpace::Rgb, ThresholdComponent::Green)
        | (ThresholdSpace::Hsv, ThresholdComponent::Saturation) => Some(1),
        (ThresholdSpace::Rgb, ThresholdComponent::Blue)
        | (ThresholdSpace::Hsv, ThresholdComponent::Value) => Some(2),
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
        };
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
    if let Some(component) = cli.threshold_lock {
        let target = match component {
            ThresholdComponent::Red => ThresholdEditTarget::Red,
            ThresholdComponent::Green => ThresholdEditTarget::Green,
            ThresholdComponent::Blue => ThresholdEditTarget::Blue,
            ThresholdComponent::Hue => ThresholdEditTarget::Hue,
            ThresholdComponent::Saturation => ThresholdEditTarget::Saturation,
            ThresholdComponent::Value => ThresholdEditTarget::Value,
        };
        recipe.threshold.set_locked(target, true);
    }
    if let Some(component) = cli.threshold_auto {
        let target = match component {
            ThresholdComponent::Red => ThresholdEditTarget::Red,
            ThresholdComponent::Green => ThresholdEditTarget::Green,
            ThresholdComponent::Blue => ThresholdEditTarget::Blue,
            ThresholdComponent::Hue => ThresholdEditTarget::Hue,
            ThresholdComponent::Saturation => ThresholdEditTarget::Saturation,
            ThresholdComponent::Value => ThresholdEditTarget::Value,
        };
        recipe.threshold.auto_map(target);
    }
    if let Some(matching) = cli.voronoi_matching {
        recipe.voronoi.matching = matching;
    }
}

type PreparedDocument = (
    Document,
    Option<PathBuf>,
    DocumentKind,
    std::sync::Arc<ThresholdHistograms>,
    PixelImage,
    DisplayBuffer,
    DisplayBuffer,
    Coverage,
);
type OpenResult = Result<PreparedDocument, String>;
type ThresholdHistoryAction = Rc<dyn Fn(bool)>;

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

#[derive(Clone, Debug, PartialEq)]
struct CreativeSnapshot {
    recipe: Recipe,
    selected_group: Option<u64>,
    selected_sample: Option<u64>,
    expanded_site: Option<u64>,
}

#[derive(Default)]
struct CreativeHistory {
    undo: Vec<CreativeSnapshot>,
    redo: Vec<CreativeSnapshot>,
    saved_recipe: Option<Recipe>,
}

impl CreativeHistory {
    fn initialize(&mut self, recipe: &Recipe) {
        self.undo.clear();
        self.redo.clear();
        self.saved_recipe = Some(recipe.clone());
    }

    fn mark_saved(&mut self, recipe: &Recipe) {
        self.saved_recipe = Some(recipe.clone());
    }

    fn is_dirty(&self, recipe: &Recipe) -> bool {
        self.saved_recipe.as_ref() != Some(recipe)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CoalescedEdit {
    Smoothing,
    Influence(u64),
    ThresholdBands(usize),
    HueOrigin,
}

struct State {
    document: Option<Document>,
    result: Option<PixelImage>,
    preview_source: Option<PixelImage>,
    threshold_histograms: Option<std::sync::Arc<ThresholdHistograms>>,
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
    expanded_site: Option<u64>,
    sampling: Option<SamplingState>,
    sampling_previous_mode: Option<CompareMode>,
    preset_entries: Vec<PresetEntry>,
    preset_diagnostics: Vec<PresetDiagnostic>,
    creative_history: CreativeHistory,
    coalesced_edit: Option<CoalescedEdit>,
}

impl State {
    fn new() -> Self {
        let (sender, receiver) = unbounded();
        Self {
            document: None,
            result: None,
            preview_source: None,
            threshold_histograms: None,
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
            expanded_site: None,
            sampling: None,
            sampling_previous_mode: None,
            preset_entries: Vec::new(),
            preset_diagnostics: Vec::new(),
            creative_history: CreativeHistory::default(),
            coalesced_edit: None,
        }
    }
}

struct Ui {
    window: adw::ApplicationWindow,
    inspector_split: adw::OverlaySplitView,
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
    export: gtk::Button,
    document_menu: gtk::MenuButton,
    hue: gtk::Adjustment,
    low: gtk::SpinButton,
    high: gtk::SpinButton,
    out_low: gtk::SpinButton,
    out_mid: gtk::SpinButton,
    out_high: gtk::SpinButton,
    threshold_space: gtk::DropDown,
    threshold_link: adw::SwitchRow,
    threshold_link_editor: adw::ActionRow,
    threshold_editor_refresh: RefCell<Option<Rc<dyn Fn()>>>,
    threshold_editor_dialog: RefCell<Option<adw::Window>>,
    threshold_editor_return_focus: RefCell<Option<gtk::Widget>>,
    picker_dialog: RefCell<Option<adw::Window>>,
    threshold_component_rows: Vec<ThresholdComponentControls>,
    voronoi_matching: gtk::DropDown,
    voronoi_matching_row: gtk::Box,
    method_section: gtk::Expander,
    method_voronoi: gtk::ToggleButton,
    method_thresholds: gtk::ToggleButton,
    smoothing: gtk::SpinButton,
    smoothing_label: gtk::Label,
    voronoi_panel: gtk::Expander,
    thresholds_panel: gtk::Expander,
    groups: gtk::ListBox,
    source_mode: gtk::ToggleButton,
    result_mode: gtk::ToggleButton,
    split_mode: gtk::ToggleButton,
    sidebar_button: gtk::ToggleButton,
    divider: gtk::Adjustment,
    viewer_bar: gtk::Overlay,
    preset_dropdown: gtk::DropDown,
    preset_model: gtk::StringList,
    preset_info: adw::ActionRow,
    preset_save: gtk::Button,
    preset_refresh: gtk::Button,
    preset_folder: gtk::Button,
    reset_space: gtk::Button,
    reset_method: gtk::Button,
    syncing: Cell<bool>,
    screenshot_component_focused: Cell<bool>,
    cli: Cli,
    snapshot_root: adw::ToolbarView,
    audit_started: Cell<bool>,
    audit_threshold_target: RefCell<Option<gtk::DropDown>>,
    audit_threshold_space: RefCell<Option<gtk::DropDown>>,
    audit_threshold_process: RefCell<Option<gtk::Switch>>,
    audit_threshold_bands: RefCell<Option<gtk::SpinButton>>,
    audit_threshold_lock: RefCell<Option<gtk::Switch>>,
    audit_threshold_link: RefCell<Option<gtk::Switch>>,
    audit_threshold_sync: RefCell<Option<gtk::Button>>,
    audit_threshold_hue_origin: RefCell<Option<gtk::SpinButton>>,
    audit_threshold_history: RefCell<Option<ThresholdHistoryAction>>,
    audit_threshold_undo: RefCell<Option<gtk::Button>>,
    audit_threshold_redo: RefCell<Option<gtk::Button>>,
    audit_threshold_precise: RefCell<Option<gtk::SpinButton>>,
    audit_threshold_cancel: RefCell<Option<gtk::Button>>,
    audit_threshold_done: RefCell<Option<gtk::Button>>,
    audit_threshold_reset: RefCell<Option<gtk::Button>>,
    audit_threshold_plot: RefCell<Option<gtk::DrawingArea>>,
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
    audit_site_expander: RefCell<Option<adw::ExpanderRow>>,
    audit_other_site_expander: RefCell<Option<adw::ExpanderRow>>,
}

#[derive(Clone)]
struct ThresholdComponentControls {
    row: adw::ExpanderRow,
    process: adw::SwitchRow,
    bands: gtk::SpinButton,
    auto_row: adw::ActionRow,
    auto: gtk::Button,
    lock: adw::SwitchRow,
    hue_origin_row: adw::ActionRow,
    hue_origin: gtk::SpinButton,
    edit: adw::ActionRow,
}

struct InspectorControls {
    root: gtk::Box,
    hue: gtk::Adjustment,
    low: gtk::SpinButton,
    high: gtk::SpinButton,
    out_low: gtk::SpinButton,
    out_mid: gtk::SpinButton,
    out_high: gtk::SpinButton,
    threshold_space: gtk::DropDown,
    threshold_link: adw::SwitchRow,
    threshold_link_editor: adw::ActionRow,
    threshold_component_rows: Vec<ThresholdComponentControls>,
    voronoi_matching: gtk::DropDown,
    voronoi_matching_row: gtk::Box,
    method_section: gtk::Expander,
    method_voronoi: gtk::ToggleButton,
    method_thresholds: gtk::ToggleButton,
    smoothing: gtk::SpinButton,
    voronoi_panel: gtk::Expander,
    thresholds_panel: gtk::Expander,
    groups: gtk::ListBox,
    preset_dropdown: gtk::DropDown,
    preset_model: gtk::StringList,
    preset_info: adw::ActionRow,
    preset_save: gtk::Button,
    preset_refresh: gtk::Button,
    preset_folder: gtk::Button,
    reset_space: gtk::Button,
    reset_method: gtk::Button,
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
    let undo = icon_button("edit-undo-symbolic", "Undo document change (Ctrl+Z)");
    undo.set_action_name(Some("app.creative-undo"));
    undo.set_sensitive(false);
    undo.update_property(&[
        gtk::accessible::Property::Label("Undo document change"),
        gtk::accessible::Property::Description(
            "Undo the most recent committed recipe or sampling change",
        ),
    ]);
    let redo = icon_button("edit-redo-symbolic", "Redo document change (Ctrl+Shift+Z)");
    redo.set_action_name(Some("app.creative-redo"));
    redo.set_sensitive(false);
    redo.update_property(&[
        gtk::accessible::Property::Label("Redo document change"),
        gtk::accessible::Property::Description(
            "Redo the most recently undone recipe or sampling change",
        ),
    ]);
    let export = icon_label_button(
        "document-send-symbolic",
        "Export",
        "Export full-resolution image (Ctrl+Shift+E)",
    );
    export.set_sensitive(false);
    let document_menu_model = gio::Menu::new();
    document_menu_model.append(Some("Open Project…"), Some("app.open-project"));
    document_menu_model.append(Some("Save Project As…"), Some("app.save-as"));
    document_menu_model.append(Some("Export Image…"), Some("app.export"));
    let document_menu = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .tooltip_text("Document Menu")
        .menu_model(&document_menu_model)
        .build();
    document_menu.update_property(&[gtk::accessible::Property::Label("Document Menu")]);
    let sidebar_button = gtk::ToggleButton::builder()
        .icon_name("sidebar-show-symbolic")
        .tooltip_text("Show adjustments")
        .active(true)
        .build();
    let header = adw::HeaderBar::new();
    header.pack_start(&open);
    let file_spacer = gtk::Separator::new(gtk::Orientation::Vertical);
    file_spacer.add_css_class("spacer");
    header.pack_start(&file_spacer);
    header.pack_start(&save);
    let history_spacer = gtk::Separator::new(gtk::Orientation::Vertical);
    history_spacer.add_css_class("spacer");
    header.pack_start(&history_spacer);
    header.pack_start(&undo);
    header.pack_start(&redo);
    header.pack_end(&document_menu);
    header.pack_end(&sidebar_button);
    header.pack_end(&export);
    header.set_title_widget(Some(&adw::WindowTitle::new(
        "Threshiator",
        "Precision posterization",
    )));

    install_creative_focus_style();
    let canvas = gtk::DrawingArea::builder()
        .hexpand(true)
        .vexpand(true)
        .content_width(CANVAS_NATURAL_WIDTH)
        .content_height(CANVAS_NATURAL_HEIGHT)
        .focusable(true)
        .build();
    canvas.add_css_class(CREATIVE_FOCUS_CLASS);
    canvas.update_property(&[gtk::accessible::Property::Label("Image comparison")]);
    canvas.set_accessible_role(gtk::AccessibleRole::Img);
    canvas.set_tooltip_text(Some("Processed image comparison surface"));
    // The Split divider is manipulated directly on the canvas. This adjustment remains the
    // single value owner for command-driven evidence and accessibility synchronization.
    let divider = gtk::Adjustment::new(0.5, 0.0, 1.0, 0.01, 0.1, 0.0);
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
    let smoothing = gtk::SpinButton::with_range(0.0, 10.0, 1.0);
    smoothing.set_numeric(true);
    smoothing.set_digits(0);
    smoothing.set_width_chars(2);
    smoothing.set_tooltip_text(Some(
        "Gaussian preprocessing before the active method; 0 disables smoothing",
    ));
    smoothing.update_property(&[
        gtk::accessible::Property::Label("Smooth source amount from 0 to 10"),
        gtk::accessible::Property::Description(
            "Gaussian preprocessing before Threshold or Voronoi; zero disables smoothing",
        ),
    ]);
    let smooth_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    smooth_box.set_halign(gtk::Align::End);
    let smooth_label = gtk::Label::new(Some("Smoothing"));
    smooth_box.append(&smooth_label);
    smooth_box.append(&smoothing);
    let viewer_bar = gtk::Overlay::new();
    modes.set_halign(gtk::Align::Center);
    smooth_box.set_halign(gtk::Align::End);
    smooth_box.set_valign(gtk::Align::Center);
    viewer_bar.set_child(Some(&modes));
    viewer_bar.add_overlay(&smooth_box);
    let viewer = gtk::Box::new(gtk::Orientation::Vertical, 8);
    viewer.set_margin_top(12);
    viewer.set_margin_bottom(12);
    viewer.set_margin_start(12);
    viewer.set_margin_end(12);
    viewer.append(&viewer_bar);
    viewer.append(&canvas);

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
        threshold_link_editor,
        threshold_component_rows,
        voronoi_matching,
        voronoi_matching_row,
        method_section,
        method_voronoi,
        method_thresholds,
        smoothing,
        voronoi_panel,
        thresholds_panel,
        groups,
        preset_dropdown,
        preset_model,
        preset_info,
        preset_save,
        preset_refresh,
        preset_folder,
        reset_space,
        reset_method,
    } = inspector(&smoothing);
    let split = adw::OverlaySplitView::new();
    split.set_content(Some(&viewer));
    split.set_sidebar(Some(&inspector));
    split.set_min_sidebar_width(260.0);
    split.set_max_sidebar_width(340.0);
    split.set_sidebar_width_fraction(0.3);
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
    let pipeline_status = gtk::Label::builder()
        .label("32-bit float · Linear sRGB")
        .xalign(1.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["dim-label"])
        .build();
    pipeline_status.set_tooltip_text(Some(
        "Canonical pipeline: straight-alpha linear-sRGB RGBA f32; preview quantizes once for display",
    ));
    pipeline_status.update_property(&[
        gtk::accessible::Property::Label("Processing pipeline"),
        gtk::accessible::Property::Description(
            "Straight-alpha linear-sRGB RGBA floating point with one display quantization",
        ),
    ]);
    let status_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    status_box.add_css_class("toolbar");
    status_box.set_margin_start(12);
    status_box.set_margin_end(12);
    status_box.set_margin_top(6);
    status_box.set_margin_bottom(6);
    status_box.append(&status);
    status_box.append(&progress);
    status_box.append(&cancel);
    status_box.append(&pipeline_status);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_css_class("background");
    toolbar.add_top_bar(&header);
    toolbar.add_bottom_bar(&status_box);
    toolbar.set_content(Some(&stack));
    window.set_content(Some(&toolbar));
    let ui = Rc::new(Ui {
        window: window.clone(),
        inspector_split: split.clone(),
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
        export: export.clone(),
        document_menu: document_menu.clone(),
        hue,
        low,
        high,
        out_low,
        out_mid,
        out_high,
        threshold_space: threshold_space.clone(),
        threshold_link,
        threshold_link_editor,
        threshold_editor_refresh: RefCell::new(None),
        threshold_editor_dialog: RefCell::new(None),
        threshold_editor_return_focus: RefCell::new(None),
        picker_dialog: RefCell::new(None),
        threshold_component_rows,
        voronoi_matching,
        voronoi_matching_row,
        method_section,
        method_voronoi,
        method_thresholds,
        smoothing,
        smoothing_label: smooth_label.clone(),
        voronoi_panel,
        thresholds_panel,
        groups,
        source_mode: source_mode.clone(),
        result_mode: result_mode.clone(),
        split_mode: split_mode.clone(),
        sidebar_button: sidebar_button.clone(),
        divider: divider.clone(),
        viewer_bar: viewer_bar.clone(),
        preset_dropdown,
        preset_model,
        preset_info,
        preset_save,
        preset_refresh,
        preset_folder,
        reset_space,
        reset_method,
        syncing: Cell::new(false),
        screenshot_component_focused: Cell::new(false),
        cli: cli.clone(),
        snapshot_root: toolbar.clone(),
        audit_started: Cell::new(false),
        audit_threshold_target: RefCell::new(None),
        audit_threshold_space: RefCell::new(Some(threshold_space)),
        audit_threshold_process: RefCell::new(None),
        audit_threshold_bands: RefCell::new(None),
        audit_threshold_lock: RefCell::new(None),
        audit_threshold_link: RefCell::new(None),
        audit_threshold_sync: RefCell::new(None),
        audit_threshold_hue_origin: RefCell::new(None),
        audit_threshold_history: RefCell::new(None),
        audit_threshold_undo: RefCell::new(None),
        audit_threshold_redo: RefCell::new(None),
        audit_threshold_precise: RefCell::new(None),
        audit_threshold_cancel: RefCell::new(None),
        audit_threshold_done: RefCell::new(None),
        audit_threshold_reset: RefCell::new(None),
        audit_threshold_plot: RefCell::new(None),
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
    threshold_reset_controls(&ui, state.clone());
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
    mode_handler(&result_mode, CompareMode::Result, &canvas, state.clone());
    mode_handler(&split_mode, CompareMode::Split, &canvas, state.clone());
    mode_handler(&source_mode, CompareMode::Source, &canvas, state.clone());
    let split_toggle = split.clone();
    sidebar_button.connect_toggled(move |b| split_toggle.set_show_sidebar(b.is_active()));
    let medium_breakpoint = adw::Breakpoint::new(
        adw::BreakpointCondition::parse(&format!("max-width: {MEDIUM_BREAKPOINT}px"))
            .expect("valid breakpoint"),
    );
    medium_breakpoint.add_setter(&viewer_bar, "height-request", Some(&76_i32.to_value()));
    medium_breakpoint.add_setter(&modes, "halign", Some(&gtk::Align::End.to_value()));
    medium_breakpoint.add_setter(&modes, "valign", Some(&gtk::Align::Start.to_value()));
    medium_breakpoint.add_setter(&smooth_box, "valign", Some(&gtk::Align::End.to_value()));
    window.add_breakpoint(medium_breakpoint);
    let narrow_breakpoint = adw::Breakpoint::new(
        adw::BreakpointCondition::parse(&format!("max-width: {NARROW_BREAKPOINT}px"))
            .expect("valid breakpoint"),
    );
    let collapsed = true.to_value();
    narrow_breakpoint.add_setter(&split, "collapsed", Some(&collapsed));
    narrow_breakpoint.add_setter(&viewer, "margin-start", Some(&4_i32.to_value()));
    narrow_breakpoint.add_setter(&viewer, "margin-end", Some(&4_i32.to_value()));
    narrow_breakpoint.add_setter(&viewer_bar, "height-request", Some(&76_i32.to_value()));
    narrow_breakpoint.add_setter(&modes, "halign", Some(&gtk::Align::End.to_value()));
    narrow_breakpoint.add_setter(&modes, "valign", Some(&gtk::Align::Start.to_value()));
    narrow_breakpoint.add_setter(&smooth_box, "valign", Some(&gtk::Align::End.to_value()));
    window.add_breakpoint(narrow_breakpoint);
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
    actions(app, &open, &empty_project, &save, &save_as, &export);
    creative_history_actions(app, &ui, &state);
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
        0.48,
        "Analyzing full-resolution source distribution…",
    ));
    let threshold_histograms =
        ThresholdHistograms::build_cancellable(&document.source, || token.is_current())
            .map(std::sync::Arc::new)
            .ok_or_else(|| anyhow::anyhow!("open cancelled"))?;
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
        threshold_histograms,
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
    ui.document_menu.set_sensitive(false);
    ui.status.set_label(label);
    sync_document_history_ui(ui, state);
    sync_contextual_chrome(ui, state);
    Some(started)
}

fn is_separate_modal_transient(dialog: &adw::Window, parent: &adw::ApplicationWindow) -> bool {
    let parent_window: gtk::Window = parent.clone().upcast();
    let separate_surfaces = dialog
        .surface()
        .zip(parent.surface())
        .is_some_and(|(dialog_surface, parent_surface)| dialog_surface != parent_surface);
    dialog.is_modal()
        && dialog
            .transient_for()
            .is_some_and(|transient| transient == parent_window)
        && separate_surfaces
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
    let has_document = current.document.is_some();
    let dirty = current
        .document
        .as_ref()
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
    gtk::Button::builder()
        .icon_name(icon)
        .tooltip_text(tip)
        .build()
}

fn icon_label_button(icon: &str, label: &str, tip: &str) -> gtk::Button {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    content.append(&gtk::Image::from_icon_name(icon));
    content.append(&gtk::Label::new(Some(label)));
    gtk::Button::builder()
        .child(&content)
        .tooltip_text(tip)
        .build()
}

fn install_creative_focus_style() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(
        ".creative-focus:focus-visible { outline: 2px solid @accent_color; outline-offset: -3px; }",
    );
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().expect("GTK display is available"),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

fn sync_contextual_chrome(ui: &Ui, state: &Rc<RefCell<State>>) {
    let state = state.borrow();
    let chrome = contextual_chrome(state.document.is_some(), state.jobs.is_busy());
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
    section.set_expanded(expanded);
    section.set_child(Some(child));
    section.add_css_class("card");
    section
}

fn inspector(smoothing: &gtk::SpinButton) -> InspectorControls {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
    root.add_css_class("background");
    root.set_margin_top(12);
    root.set_margin_bottom(12);
    root.set_margin_start(12);
    root.set_margin_end(12);
    let method_content = gtk::Box::new(gtk::Orientation::Vertical, 8);
    method_content.set_margin_start(12);
    method_content.set_margin_end(12);
    method_content.set_margin_bottom(12);
    let method_voronoi = gtk::ToggleButton::with_label("Voronoi");
    let method_thresholds = gtk::ToggleButton::with_label("Thresholds");
    method_thresholds.set_group(Some(&method_voronoi));
    method_voronoi.set_active(true);
    let segmented = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    segmented.add_css_class("linked");
    segmented.set_homogeneous(true);
    segmented.append(&method_voronoi);
    segmented.append(&method_thresholds);
    method_content.append(&segmented);
    let voronoi_matching = gtk::DropDown::new(
        Some(gtk::StringList::new(VORONOI_MATCHING_LABELS)),
        None::<gtk::Expression>,
    );
    voronoi_matching.set_tooltip_text(Some(
        "Choose the distance model used to assign pixels to sites; HSV uses an endpoint-aware cone so Hue naturally collapses toward black",
    ));
    let voronoi_matching_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let matching_label = gtk::Label::new(Some("Matching"));
    matching_label.set_xalign(0.0);
    matching_label.set_hexpand(true);
    voronoi_matching_row.append(&matching_label);
    voronoi_matching_row.append(&voronoi_matching);
    let preset_model = gtk::StringList::new(&[]);
    let preset_dropdown = gtk::DropDown::new(Some(preset_model.clone()), None::<gtk::Expression>);
    preset_dropdown.set_hexpand(true);
    preset_dropdown.update_property(&[gtk::accessible::Property::Label("Preset selection")]);
    let preset_info = adw::ActionRow::new();
    preset_info.set_visible(false);
    let preset_save = gtk::Button::with_label("Save Current Preset…");
    let preset_refresh = gtk::Button::with_label("Refresh Presets");
    let preset_folder = gtk::Button::with_label("Open Presets Folder");
    for button in [&preset_save, &preset_refresh, &preset_folder] {
        button.add_css_class("flat");
        button.set_halign(gtk::Align::Fill);
    }
    preset_refresh.update_property(&[gtk::accessible::Property::Label("Refresh presets")]);
    preset_folder.update_property(&[gtk::accessible::Property::Label("Open presets folder")]);
    let preset_actions_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    preset_actions_box.set_margin_top(6);
    preset_actions_box.set_margin_bottom(6);
    preset_actions_box.set_margin_start(6);
    preset_actions_box.set_margin_end(6);
    preset_actions_box.append(&preset_save);
    preset_actions_box.append(&preset_refresh);
    preset_actions_box.append(&preset_folder);
    let preset_popover = gtk::Popover::builder().child(&preset_actions_box).build();
    for button in [&preset_save, &preset_refresh, &preset_folder] {
        let popover = preset_popover.clone();
        button.connect_clicked(move |_| popover.popdown());
    }
    let preset_actions = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .tooltip_text("Preset Menu")
        .popover(&preset_popover)
        .build();
    preset_actions.update_property(&[gtk::accessible::Property::Label("Preset Menu")]);
    let preset_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let preset_label = gtk::Label::builder().label("Preset").xalign(0.0).build();
    preset_row.append(&preset_label);
    preset_row.append(&preset_dropdown);
    preset_row.append(&preset_actions);
    method_content.append(&preset_row);
    method_content.append(&voronoi_matching_row);
    let method_section = compact_section("Method", &method_content, true);
    root.append(&method_section);

    let groups = gtk::ListBox::new();
    groups.add_css_class("boxed-list");
    groups.set_selection_mode(gtk::SelectionMode::Single);
    groups.update_property(&[gtk::accessible::Property::Label("Voronoi site list")]);
    let group_scroll = gtk::ScrolledWindow::builder()
        .child(&groups)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .vexpand(true)
        .build();
    let site_content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    site_content.set_vexpand(true);
    site_content.append(&group_scroll);
    let voronoi = compact_section("Color sites", &site_content, true);
    voronoi.set_vexpand(true);
    root.append(&voronoi);
    let threshold_overview = adw::PreferencesGroup::new();
    threshold_overview.set_description(Some(
        "Divide each color component into bands, then choose the output level each band produces.",
    ));
    let threshold_space = gtk::DropDown::new(
        Some(gtk::StringList::new(&["RGB", "HSV"])),
        None::<gtk::Expression>,
    );
    threshold_space.set_focusable(true);
    threshold_space.set_tooltip_text(Some(
        "Choose RGB or HSV; each Working space preserves its own mappings",
    ));
    threshold_space.update_property(&[
        gtk::accessible::Property::Label("Threshold Working space"),
        gtk::accessible::Property::Description(
            "Choose RGB or HSV before opening Edit mapping. Each space preserves its own mappings.",
        ),
    ]);
    let space_row = adw::ActionRow::builder().title("Working space").build();
    space_row.set_tooltip_text(Some(
        "Switch between RGB and HSV; each space preserves its prior mappings",
    ));
    space_row.add_suffix(&threshold_space);
    space_row.set_activatable_widget(Some(&threshold_space));
    threshold_overview.add(&space_row);
    let threshold_link = adw::SwitchRow::builder()
        .title("Link RGB mappings")
        .subtitle(
            "Future mapping edits copy to unlocked peers; existing differences remain; Process remains per-component.",
        )
        .active(true)
        .build();
    threshold_link.set_subtitle_lines(3);
    threshold_link.update_property(&[gtk::accessible::Property::Description(
        "Future mapping edits copy to unlocked peers; existing differences remain; Process remains per-component.",
    )]);
    threshold_overview.add(&threshold_link);
    let threshold_link_editor = adw::ActionRow::builder()
        .title("Edit linked RGB mapping…")
        .activatable(true)
        .build();
    threshold_link_editor.set_focusable(true);
    threshold_link_editor.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    threshold_link_editor.update_property(&[gtk::accessible::Property::Description(
        "Open the linked mapping in the separate Threshold Mapping window",
    )]);
    threshold_overview.add(&threshold_link_editor);

    let component_group = adw::PreferencesGroup::builder()
        .title("Component mappings")
        .build();
    let mut threshold_component_rows = Vec::new();
    for title in ["Red mapping", "Green mapping", "Blue mapping"] {
        let row = adw::ExpanderRow::builder()
            .title(title)
            .subtitle("3 bands · Processing · Unlocked")
            .build();
        row.set_title_lines(1);
        row.set_subtitle_lines(1);
        let process = adw::SwitchRow::builder()
            .title("Process mapping")
            .subtitle("Off bypasses this component without changing its mapping")
            .active(true)
            .build();
        process.set_subtitle_lines(2);
        let bands = gtk::SpinButton::with_range(2.0, 32.0, 1.0);
        bands.set_numeric(true);
        bands.set_digits(0);
        bands.set_width_chars(2);
        let bands_row = adw::ActionRow::builder().title("Bands").build();
        bands_row.add_suffix(&bands);
        bands_row.set_activatable_widget(Some(&bands));
        let auto = gtk::Button::with_label("Auto");
        auto.set_valign(gtk::Align::Center);
        let auto_row = adw::ActionRow::builder()
            .title("Automatic baseline")
            .subtitle("Even bands · levels from 0 to 1")
            .build();
        auto_row.set_subtitle_lines(2);
        auto_row.add_suffix(&auto);
        auto_row.set_activatable_widget(Some(&auto));
        let lock = adw::SwitchRow::builder()
            .title("Lock mapping")
            .subtitle("Protect mapping edits and incoming linked copies")
            .build();
        lock.set_subtitle_lines(2);
        let hue_origin = gtk::SpinButton::with_range(0.0, 359.9, 0.1);
        hue_origin.set_numeric(true);
        hue_origin.set_digits(1);
        hue_origin.set_width_chars(5);
        hue_origin.set_wrap(true);
        let hue_origin_row = adw::ActionRow::builder()
            .title("Hue origin (°)")
            .subtitle("Rotate the circular Hue seam")
            .build();
        hue_origin_row.add_suffix(&hue_origin);
        hue_origin_row.set_activatable_widget(Some(&hue_origin));
        hue_origin_row.set_visible(false);
        let edit = adw::ActionRow::builder()
            .title(format!(
                "Edit {} band mapping…",
                title.trim_end_matches(" mapping")
            ))
            .activatable(true)
            .build();
        edit.set_focusable(true);
        edit.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
        edit.update_property(&[gtk::accessible::Property::Description(
            "Open this exact component in the separate Threshold Mapping window",
        )]);
        row.add_row(&process);
        row.add_row(&bands_row);
        row.add_row(&auto_row);
        row.add_row(&lock);
        row.add_row(&hue_origin_row);
        row.add_row(&edit);
        component_group.add(&row);
        threshold_component_rows.push(ThresholdComponentControls {
            row,
            process,
            bands,
            auto_row,
            auto,
            lock,
            hue_origin_row,
            hue_origin,
            edit,
        });
    }
    for (index, controls) in threshold_component_rows.iter().enumerate() {
        let row = controls.row.clone();
        let peers = threshold_component_rows
            .iter()
            .enumerate()
            .filter(|(peer, _)| *peer != index)
            .map(|(_, controls)| controls.row.clone())
            .collect::<Vec<_>>();
        row.clone().connect_expanded_notify(move |expanded| {
            if expanded.is_expanded() {
                for peer in &peers {
                    peer.set_expanded(false);
                }
            }
        });
    }

    let reset_group = adw::PreferencesGroup::new();
    let resets = adw::ExpanderRow::builder().title("Reset mappings").build();
    resets.set_tooltip_text(Some(
        "Restore the active working space or the complete Threshold method",
    ));
    let reset_actions = adw::ActionRow::new();
    let reset_space = gtk::Button::with_label("Reset Working Space…");
    let reset_method = gtk::Button::with_label("Reset Method…");
    reset_method.add_css_class("destructive-action");
    reset_actions.add_prefix(&reset_space);
    reset_actions.add_suffix(&reset_method);
    resets.add_row(&reset_actions);
    reset_group.add(&resets);
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
    let threshold_content = gtk::Box::new(gtk::Orientation::Vertical, 18);
    threshold_content.append(&threshold_overview);
    threshold_content.append(&component_group);
    threshold_content.append(&reset_group);
    let threshold_scroll = gtk::ScrolledWindow::builder()
        .child(&threshold_content)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .vexpand(true)
        .build();
    let thresholds = compact_section("Threshold mappings", &threshold_scroll, true);
    thresholds.set_vexpand(true);
    thresholds.set_visible(false);
    root.append(&thresholds);
    // Common hue remains part of recipes, presets, projects, and command-driven evidence, but it
    // is intentionally not exposed in the main inspector. A later information-architecture pass
    // can place finishing controls in a workflow-appropriate surface.
    let hue = gtk::Adjustment::new(0.0, -180.0, 180.0, 0.1, 1.0, 0.0);
    InspectorControls {
        root,
        hue,
        low,
        high,
        out_low,
        out_mid,
        out_high,
        threshold_space,
        threshold_link,
        threshold_link_editor,
        threshold_component_rows,
        voronoi_matching,
        voronoi_matching_row,
        method_section,
        method_voronoi,
        method_thresholds,
        smoothing: smoothing.clone(),
        voronoi_panel: voronoi,
        thresholds_panel: thresholds,
        groups,
        preset_dropdown,
        preset_model,
        preset_info,
        preset_save,
        preset_refresh,
        preset_folder,
        reset_space,
        reset_method,
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

fn threshold_component_summary(quantizer: &ComponentQuantizer, locked: bool) -> String {
    format!(
        "{} band{} · {} · {}",
        quantizer.outputs.len(),
        if quantizer.outputs.len() == 1 {
            ""
        } else {
            "s"
        },
        if quantizer.enabled {
            "Processing"
        } else {
            "Bypassed"
        },
        if locked { "Locked" } else { "Unlocked" }
    )
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
    ui.smoothing
        .set_value(recipe.threshold.input_smoothing as f64);
    let link = if hsv {
        recipe.threshold.hsv_state.sv_link
    } else {
        recipe.threshold.rgb_state.link
    };
    let link_description = if hsv {
        "Future Saturation or Value edits copy to the unlocked peer; existing differences remain; Hue is independent; Process remains per-component."
    } else {
        "Future mapping edits copy to unlocked peers; existing differences remain; Process remains per-component."
    };
    ui.threshold_link.set_title(if hsv {
        "Link Saturation + Value"
    } else {
        "Link RGB mappings"
    });
    ui.threshold_link.set_subtitle(link_description);
    ui.threshold_link.set_active(link == LinkPolicy::Linked);
    ui.threshold_link.update_property(&[
        gtk::accessible::Property::Label(if hsv {
            "Link Saturation and Value mappings"
        } else {
            "Link RGB mappings"
        }),
        gtk::accessible::Property::Description(link_description),
    ]);
    ui.threshold_link_editor
        .set_visible(link == LinkPolicy::Linked);
    ui.threshold_link_editor.set_title(if hsv {
        "Edit linked S + V mapping…"
    } else {
        "Edit linked RGB mapping…"
    });
    ui.threshold_link_editor.update_property(&[
        gtk::accessible::Property::Label(if hsv {
            "Edit linked Saturation and Value mapping"
        } else {
            "Edit linked RGB mapping"
        }),
        gtk::accessible::Property::Description(
            "Open the semantic linked target in the separate Threshold Mapping window",
        ),
    ]);
    for (index, controls) in ui.threshold_component_rows.iter().enumerate() {
        let name = if hsv {
            ["Hue", "Saturation", "Value"][index]
        } else {
            ["Red", "Green", "Blue"][index]
        };
        let quantizer = threshold_quantizer(&recipe, index);
        let locked = if hsv {
            recipe.threshold.hsv_state.locks[index]
        } else {
            recipe.threshold.rgb_state.locks[index]
        };
        let summary = threshold_component_summary(&quantizer, locked);
        controls.row.set_title(&format!("{name} mapping"));
        controls.row.set_subtitle(&summary);
        controls.row.set_tooltip_text(Some(&format!(
            "{name} mapping — {summary}. Expand for direct controls or open the detailed editor."
        )));
        controls.row.update_property(&[
            gtk::accessible::Property::Label(&format!("{name} mapping, {summary}")),
            gtk::accessible::Property::Description(&format!(
                "{} bands, {}, {}. Expand for Process, Bands, Automatic baseline, Lock,{} and Edit band mapping.",
                quantizer.outputs.len(),
                if quantizer.enabled {
                    "Processing"
                } else {
                    "Bypassed"
                },
                if locked { "Locked" } else { "Unlocked" },
                if hsv && index == 0 {
                    " Hue origin,"
                } else {
                    ""
                }
            )),
        ]);
        controls.process.set_active(quantizer.enabled);
        controls.process.update_property(&[
            gtk::accessible::Property::Label(&format!("Process {name} mapping")),
            gtk::accessible::Property::Description(
                "Off bypasses only this component; Process remains editable while the mapping is locked",
            ),
        ]);
        controls.bands.set_value(quantizer.outputs.len() as f64);
        controls.bands.set_sensitive(!locked);
        controls.bands.update_property(&[
            gtk::accessible::Property::Label(&format!("Bands for {name} mapping")),
            gtk::accessible::Property::Description(
                "Resize from 2 to 32 bands while preserving the mapping; linked edits copy to eligible unlocked peers",
            ),
        ]);
        let automatic_outcome = if hsv && index == 0 {
            "Even circular bands · levels wrap across the Hue seam"
        } else {
            "Even bands · levels from 0 to 1"
        };
        controls.auto_row.set_subtitle(automatic_outcome);
        controls.auto.set_sensitive(!locked);
        let automatic_description = if hsv && index == 0 {
            "Replace the Hue mapping with even circular bands and seam-wrapping levels; Process and Hue origin remain unchanged"
                .to_owned()
        } else {
            format!(
                "Replace the {name} mapping with even bands and levels from 0 to 1; Process remains unchanged; when Link is on, eligible unlocked peers receive the same mapping"
            )
        };
        controls.auto.update_property(&[
            gtk::accessible::Property::Label(&format!(
                "Automatically map {name} with {automatic_outcome}"
            )),
            gtk::accessible::Property::Description(&automatic_description),
        ]);
        controls.lock.set_active(locked);
        controls.lock.update_property(&[
            gtk::accessible::Property::Label(&format!("Lock {name} mapping")),
            gtk::accessible::Property::Description(
                "Protect Bands, Automatic baseline, Hue origin, reset, and incoming copies; Process and editor inspection remain available",
            ),
        ]);
        controls.hue_origin_row.set_visible(hsv && index == 0);
        controls
            .hue_origin
            .set_sensitive(hsv && index == 0 && !locked);
        controls
            .hue_origin
            .set_value(f64::from(recipe.threshold.hsv_state.hue_origin_degrees).rem_euclid(360.0));
        controls.edit.update_property(&[
            gtk::accessible::Property::Label(&format!("Edit {name} band mapping")),
            gtk::accessible::Property::Description(&format!(
                "Open the exact {name} component in the separate Threshold Mapping window for inspection and precise editing"
            )),
        ]);
        controls
            .edit
            .set_title(&format!("Edit {name} band mapping…"));
    }
    if ui.cli.screenshot.is_some()
        && let Some(component) = ui.cli.threshold_component
    {
        ui.method_section.set_expanded(false);
        let selected = threshold_component_index(recipe.threshold.active_space, component);
        if let Some(index) = selected {
            ui.threshold_component_rows[index].row.set_expanded(true);
            if component == ThresholdComponent::Hue
                && !ui.screenshot_component_focused.replace(true)
            {
                let hue_origin = ui.threshold_component_rows[index].hue_origin.clone();
                glib::timeout_add_local_once(std::time::Duration::from_millis(500), move || {
                    hue_origin.grab_focus();
                });
            }
        }
    }
    ui.voronoi_matching
        .set_selected(visible_voronoi_matching_index(recipe.voronoi.matching));
    ui.syncing.set(false);
}

fn creative_snapshot(state: &State) -> Option<CreativeSnapshot> {
    Some(CreativeSnapshot {
        recipe: state.document.as_ref()?.recipe.clone(),
        selected_group: state.selected_group,
        selected_sample: state.selected_sample,
        expanded_site: state.expanded_site,
    })
}

fn sync_document_history_ui(ui: &Ui, state: &Rc<RefCell<State>>) {
    let state = state.borrow();
    let available = state.document.is_some()
        && !state.jobs.is_busy()
        && !state.threshold_editor_visible
        && !state.picker_visible;
    let can_undo = available && !state.creative_history.undo.is_empty();
    let can_redo = available && !state.creative_history.redo.is_empty();
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

fn record_creative_change(
    ui: &Ui,
    state: &Rc<RefCell<State>>,
    before: CreativeSnapshot,
    coalesced: Option<CoalescedEdit>,
) -> bool {
    let changed = {
        let mut current = state.borrow_mut();
        let Some(recipe) = current
            .document
            .as_ref()
            .map(|document| document.recipe.clone())
        else {
            return false;
        };
        if recipe == before.recipe {
            return false;
        }
        let State {
            creative_history,
            coalesced_edit,
            ..
        } = &mut *current;
        record_coalesced_snapshot(creative_history, coalesced_edit, before, &recipe, coalesced);
        current.creative_history.redo.clear();
        let dirty = current.creative_history.is_dirty(&recipe);
        if let Some(document) = current.document.as_mut() {
            document.dirty = dirty;
        }
        true
    };
    ui.syncing.set(true);
    ui.preset_dropdown.set_selected(0);
    ui.syncing.set(false);
    update_preset_info(ui, state);
    sync_document_history_ui(ui, state);
    changed
}

fn record_coalesced_snapshot(
    history: &mut CreativeHistory,
    active: &mut Option<CoalescedEdit>,
    before: CreativeSnapshot,
    recipe: &Recipe,
    coalesced: Option<CoalescedEdit>,
) {
    if *active == coalesced
        && coalesced.is_some()
        && history
            .undo
            .last()
            .is_some_and(|snapshot| snapshot.recipe == *recipe)
    {
        history.undo.pop();
        *active = None;
    } else if *active != coalesced || coalesced.is_none() {
        history.undo.push(before);
    }
    *active = coalesced;
}

fn finish_coalesced_edit(state: &Rc<RefCell<State>>, edit: CoalescedEdit) {
    let mut state = state.borrow_mut();
    if state.coalesced_edit == Some(edit) {
        state.coalesced_edit = None;
    }
}

fn install_document_spin_boundaries(
    control: &gtk::SpinButton,
    state: &Rc<RefCell<State>>,
    edit: CoalescedEdit,
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
    let Some(recipe) = state
        .document
        .as_ref()
        .map(|document| document.recipe.clone())
    else {
        return;
    };
    if let Some(document) = state.document.as_mut() {
        document.dirty = false;
    }
    state.creative_history.mark_saved(&recipe);
    state.coalesced_edit = None;
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
    let dirty = document.dirty;
    let recipe = document.recipe.clone();
    if let Some(source) = state.preview_source.clone() {
        state.scheduler.schedule(source, recipe);
    }
    drop(state);
    ui.save.set_sensitive(dirty);
    ui.status.set_label(message);
}

fn schedule_existing_recipe_preview(ui: &Ui, state: &Rc<RefCell<State>>, message: &str) {
    let state = state.borrow_mut();
    let Some(recipe) = state
        .document
        .as_ref()
        .map(|document| document.recipe.clone())
    else {
        return;
    };
    if let Some(source) = state.preview_source.clone() {
        state.scheduler.schedule(source, recipe);
    }
    drop(state);
    ui.status.set_label(message);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ThresholdHandle {
    Boundary(usize),
    Output(usize),
}

struct ThresholdDialogSession {
    snapshot: threshiator::document::ThresholdState,
    draft_threshold: threshiator::document::ThresholdState,
    history_snapshot: CreativeSnapshot,
    pre_dirty: bool,
    context: String,
    target: ThresholdEditTarget,
    draft: ComponentQuantizer,
    selected: ThresholdHandle,
    gesture: ThresholdEditGesture,
    undo: Vec<ThresholdLocalEntry>,
    redo: Vec<ThresholdLocalEntry>,
    completed_edits: usize,
    retain: bool,
}

#[derive(Clone)]
struct ThresholdLocalEntry {
    threshold: threshiator::document::ThresholdState,
    preview: bool,
}

fn individual_threshold_targets(space: ThresholdSpace) -> [ThresholdEditTarget; 3] {
    match space {
        ThresholdSpace::Rgb => [
            ThresholdEditTarget::Red,
            ThresholdEditTarget::Green,
            ThresholdEditTarget::Blue,
        ],
        ThresholdSpace::Hsv => [
            ThresholdEditTarget::Hue,
            ThresholdEditTarget::Saturation,
            ThresholdEditTarget::Value,
        ],
    }
}

fn automatic_linked_peer_changes(
    before: &ThresholdState,
    after: &ThresholdState,
    target: ThresholdEditTarget,
) -> usize {
    match target {
        ThresholdEditTarget::Red | ThresholdEditTarget::Green | ThresholdEditTarget::Blue => {
            let source = ThresholdState::target_index(target);
            before
                .rgb_state
                .components
                .iter()
                .zip(&after.rgb_state.components)
                .enumerate()
                .filter(|(index, (before, after))| *index != source && before != after)
                .count()
        }
        ThresholdEditTarget::Saturation => {
            usize::from(before.hsv_state.value != after.hsv_state.value)
        }
        ThresholdEditTarget::Value => {
            usize::from(before.hsv_state.saturation != after.hsv_state.saturation)
        }
        ThresholdEditTarget::Hue
        | ThresholdEditTarget::LinkedRgb
        | ThresholdEditTarget::LinkedSaturationValue => 0,
    }
}

fn threshold_link_editor_target(threshold: &ThresholdState) -> Option<ThresholdEditTarget> {
    match (threshold.active_space, threshold.active_link()) {
        (ThresholdSpace::Rgb, LinkPolicy::Linked) => Some(ThresholdEditTarget::LinkedRgb),
        (ThresholdSpace::Hsv, LinkPolicy::Linked) => {
            Some(ThresholdEditTarget::LinkedSaturationValue)
        }
        (_, LinkPolicy::Independent) => None,
    }
}

fn set_threshold_component_enabled(
    threshold: &mut ThresholdState,
    target: ThresholdEditTarget,
    enabled: bool,
) -> bool {
    let component = match target {
        ThresholdEditTarget::Red => &mut threshold.rgb_state.components[0],
        ThresholdEditTarget::Green => &mut threshold.rgb_state.components[1],
        ThresholdEditTarget::Blue => &mut threshold.rgb_state.components[2],
        ThresholdEditTarget::Hue => &mut threshold.hsv_state.hue,
        ThresholdEditTarget::Saturation => &mut threshold.hsv_state.saturation,
        ThresholdEditTarget::Value => &mut threshold.hsv_state.value,
        ThresholdEditTarget::LinkedRgb | ThresholdEditTarget::LinkedSaturationValue => {
            return false;
        }
    };
    if component.enabled == enabled {
        false
    } else {
        component.enabled = enabled;
        true
    }
}

fn resize_threshold_component(
    threshold: &mut ThresholdState,
    target: ThresholdEditTarget,
    bands: usize,
) -> bool {
    if threshold.is_locked(target) {
        return false;
    }
    let mut edited = threshold.edit_quantizer(target);
    edited.resize_preserving_mapping(bands, None, None)
        && threshold.set_edit_quantizer(target, edited)
}

fn rgb_target_index(target: ThresholdEditTarget) -> Option<usize> {
    match target {
        ThresholdEditTarget::Red | ThresholdEditTarget::LinkedRgb => Some(0),
        ThresholdEditTarget::Green => Some(1),
        ThresholdEditTarget::Blue => Some(2),
        _ => None,
    }
}

fn hsv_target_component(target: ThresholdEditTarget) -> Option<HsvHistogramComponent> {
    match target {
        ThresholdEditTarget::Hue => Some(HsvHistogramComponent::Hue),
        ThresholdEditTarget::Saturation | ThresholdEditTarget::LinkedSaturationValue => {
            Some(HsvHistogramComponent::Saturation)
        }
        ThresholdEditTarget::Value => Some(HsvHistogramComponent::Value),
        _ => None,
    }
}

fn threshold_plot_accessibility_description(
    threshold: &ThresholdState,
    target: ThresholdEditTarget,
    histograms: Option<&ThresholdHistograms>,
) -> String {
    if let Some(channel) = rgb_target_index(target) {
        let series = match target {
            ThresholdEditTarget::LinkedRgb => {
                let anchor = &threshold.rgb_state.components[0];
                let mixed = threshold.rgb_state.components[1..].iter().any(|peer| {
                    peer.boundaries != anchor.boundaries
                        || peer.outputs != anchor.outputs
                        || peer.enabled != anchor.enabled
                });
                if mixed {
                    "linked Red solid, Green dashed, and Blue dotted source distributions. Red is the editing anchor; peer mappings or Process states currently differ"
                } else {
                    "linked Red solid, Green dashed, and Blue dotted source distributions. Red is the editing anchor"
                }
            }
            _ => match channel {
                0 => "selected Red solid source distribution",
                1 => "selected Green dashed source distribution",
                _ => "selected Blue dotted source distribution",
            },
        };
        return format!(
            "Full-resolution encoded-sRGB source density before Smooth; {series}. Arrow keys select handles; Up and Down adjust the selected handle"
        );
    }
    let origin = f64::from(threshold.hsv_state.hue_origin_degrees).rem_euclid(360.0);
    let representative = histograms.map_or(0.0, |cache| cache.representative_hue_degrees);
    match hsv_target_component(target) {
        Some(HsvHistogramComponent::Hue) => format!(
            "Full-resolution alpha-weighted Hue source density before Smooth, excluding achromatic pixels. Relative Hue begins and ends at the explicit {origin:.1} degree 0/360 seam. Hue is the solid source series; circular input and mapped absolute-Hue output strips are x-aligned. Arrow keys select handles; Up and Down adjust the selected handle"
        ),
        Some(HsvHistogramComponent::Saturation) => {
            if target == ThresholdEditTarget::LinkedSaturationValue {
                let mixed = threshold.hsv_state.saturation.boundaries
                    != threshold.hsv_state.value.boundaries
                    || threshold.hsv_state.saturation.outputs
                        != threshold.hsv_state.value.outputs
                    || threshold.hsv_state.saturation.enabled
                        != threshold.hsv_state.value.enabled;
                format!(
                    "Full-resolution alpha-weighted source density before Smooth. Linked Saturation is dashed and Value is dotted; neither series is combined. Saturation is the editing anchor; {}. Separate labeled Saturation and Value input and output strips are x-aligned. Saturation uses representative source Hue {representative:.1} degrees. Arrow keys select handles; Up and Down adjust the selected Saturation handle",
                    if mixed {
                        "peer mappings or Process states currently differ"
                    } else {
                        "the peer mapping currently matches"
                    }
                )
            } else {
                format!(
                    "Full-resolution alpha-weighted Saturation source density before Smooth, shown dashed. Neutral-to-chroma input and mapped output strips use representative source Hue {representative:.1} degrees. Arrow keys select handles; Up and Down adjust the selected handle"
                )
            }
        }
        Some(HsvHistogramComponent::Value) => "Full-resolution alpha-weighted Value source density before Smooth, shown dotted. Black-to-bright neutral input and mapped output strips are x-aligned. Arrow keys select handles; Up and Down adjust the selected handle".to_string(),
        None => "Arrow keys select handles; Up and Down adjust the selected handle".to_string(),
    }
}

fn draw_threshold_rgb_histograms(
    ctx: &gtk::cairo::Context,
    bounds: (f64, f64, f64, f64),
    histograms: &ThresholdHistograms,
    _threshold: &ThresholdState,
    target: ThresholdEditTarget,
) {
    let Some(selected) = rgb_target_index(target) else {
        return;
    };
    let (left, top, right, bottom) = bounds;
    let width = (right - left).max(1.0);
    let height = (bottom - top).max(1.0);
    let linked = target == ThresholdEditTarget::LinkedRgb;
    let mask = threshold_histogram_series_mask(target);
    let series = &histograms.encoded_rgb;
    let colors = [(0.82, 0.08, 0.10), (0.04, 0.56, 0.18), (0.08, 0.25, 0.88)];

    if !linked {
        let gradient = gtk::cairo::LinearGradient::new(left, 0.0, right, 0.0);
        gradient.add_color_stop_rgba(0.0, 0.0, 0.0, 0.0, 0.08);
        let (red, green, blue) = match selected {
            0 => (1.0, 0.0, 0.0),
            1 => (0.0, 1.0, 0.0),
            _ => (0.0, 0.0, 1.0),
        };
        gradient.add_color_stop_rgba(1.0, red, green, blue, 0.14);
        ctx.rectangle(left, top, width, height);
        let _ = ctx.set_source(&gradient);
        let _ = ctx.fill();

        let input_strip = gtk::cairo::LinearGradient::new(left, 0.0, right, 0.0);
        input_strip.add_color_stop_rgb(0.0, 0.0, 0.0, 0.0);
        input_strip.add_color_stop_rgb(1.0, red, green, blue);
        ctx.rectangle(left, bottom - 12.0, width, 12.0);
        let _ = ctx.set_source(&input_strip);
        let _ = ctx.fill();
    }

    let maximum = series
        .iter()
        .enumerate()
        .filter(|(channel, _)| mask[*channel])
        .flat_map(|(_, counts)| counts.iter().copied())
        .fold(0.0_f64, f64::max);
    if maximum <= 0.0 {
        return;
    }
    let maximum_log = maximum.ln_1p();
    let dash_patterns: [&[f64]; 3] = [&[], &[8.0, 4.0], &[2.0, 3.0]];
    let labels = ["R", "G", "B"];
    let mut visible_label = 0;
    for channel in 0..3 {
        if !mask[channel] {
            continue;
        }
        let counts = &series[channel];
        let (red, green, blue) = colors[channel];
        ctx.new_path();
        ctx.move_to(left, bottom);
        for (index, count) in counts.iter().enumerate() {
            let x = left + width * index as f64 / (counts.len() - 1) as f64;
            let normalized = count.ln_1p() / maximum_log;
            ctx.line_to(x, bottom - height * normalized);
        }
        ctx.line_to(right, bottom);
        ctx.close_path();
        ctx.set_source_rgba(red, green, blue, if linked { 0.05 } else { 0.10 });
        let _ = ctx.fill();

        ctx.new_path();
        for index in 0..counts.len() {
            let start = index.saturating_sub(2);
            let end = (index + 3).min(counts.len());
            let smoothed = counts[start..end].iter().sum::<f64>() / (end - start) as f64;
            let x = left + width * index as f64 / (counts.len() - 1) as f64;
            let y = bottom - height * (smoothed.ln_1p() / maximum_log);
            if index == 0 {
                ctx.move_to(x, y);
            } else {
                ctx.line_to(x, y);
            }
        }
        ctx.set_source_rgba(red, green, blue, if linked { 0.55 } else { 0.70 });
        ctx.set_line_width(if linked { 1.0 } else { 1.4 });
        ctx.set_dash(dash_patterns[channel], 0.0);
        let _ = ctx.stroke();
        ctx.set_dash(&[], 0.0);

        ctx.set_source_rgb(red, green, blue);
        ctx.set_font_size(12.0);
        ctx.move_to(left + 8.0 + f64::from(visible_label) * 24.0, top + 16.0);
        let _ = ctx.show_text(labels[channel]);
        visible_label += 1;
    }
}

fn hsv_component_quantizer(
    threshold: &ThresholdState,
    component: HsvHistogramComponent,
) -> &ComponentQuantizer {
    match component {
        HsvHistogramComponent::Hue => &threshold.hsv_state.hue,
        HsvHistogramComponent::Saturation => &threshold.hsv_state.saturation,
        HsvHistogramComponent::Value => &threshold.hsv_state.value,
    }
}

fn draw_hsv_strip(
    ctx: &gtk::cairo::Context,
    bounds: (f64, f64, f64, f64),
    component: HsvHistogramComponent,
    quantizer: Option<&ComponentQuantizer>,
    origin_degrees: f64,
    representative_hue_degrees: f64,
    label: &str,
) {
    let (left, top, right, bottom) = bounds;
    let width = (right - left).max(1.0);
    let height = (bottom - top).max(1.0);
    let columns = width.ceil().max(2.0) as usize;
    for column in 0..columns {
        let input = column as f64 / (columns - 1) as f64;
        let color = quantizer.map_or_else(
            || hsv_input_strip_color(component, input, origin_degrees, representative_hue_degrees),
            |quantizer| {
                hsv_output_strip_color(
                    component,
                    input,
                    quantizer,
                    origin_degrees,
                    representative_hue_degrees,
                )
            },
        );
        ctx.set_source_rgb(color[0], color[1], color[2]);
        ctx.rectangle(left + column as f64, top, 1.25, height);
        let _ = ctx.fill();
    }
    let label_width = label.len() as f64 * 7.0 + 8.0;
    ctx.set_source_rgba(0.0, 0.0, 0.0, 0.62);
    ctx.rectangle(left + 2.0, top + 1.0, label_width, (height - 2.0).max(1.0));
    let _ = ctx.fill();
    ctx.set_source_rgb(1.0, 1.0, 1.0);
    ctx.set_font_size(height.min(10.0));
    ctx.move_to(left + 6.0, bottom - 2.0);
    let _ = ctx.show_text(label);
}

fn draw_threshold_hsv_histograms(
    ctx: &gtk::cairo::Context,
    bounds: (f64, f64, f64, f64),
    histograms: &ThresholdHistograms,
    threshold: &ThresholdState,
    target: ThresholdEditTarget,
) {
    let Some(component) = hsv_target_component(target) else {
        return;
    };
    let (left, top, right, bottom) = bounds;
    let width = (right - left).max(1.0);
    let height = (bottom - top).max(1.0);
    let linked = target == ThresholdEditTarget::LinkedSaturationValue;
    let mask = threshold_histogram_series_mask(target);
    let origin = f64::from(threshold.hsv_state.hue_origin_degrees).rem_euclid(360.0);
    let relative_hue = relative_hue_histogram(&histograms.hsv[0], origin);
    let maximum = histograms
        .hsv
        .iter()
        .enumerate()
        .filter(|(channel, _)| mask[*channel])
        .flat_map(|(channel, counts)| {
            if channel == 0 {
                relative_hue.iter().copied()
            } else {
                counts.iter().copied()
            }
        })
        .fold(0.0_f64, f64::max);
    if maximum > 0.0 {
        let maximum_log = maximum.ln_1p();
        let colors = [(0.48, 0.16, 0.72), (0.02, 0.52, 0.38), (0.18, 0.20, 0.25)];
        let dash_patterns: [&[f64]; 3] = [&[], &[8.0, 4.0], &[2.0, 3.0]];
        let labels = ["H", "S", "V"];
        let mut visible_label = 0;
        for channel in 0..3 {
            if !mask[channel] {
                continue;
            }
            let counts = if channel == 0 {
                relative_hue.as_slice()
            } else {
                histograms.hsv[channel].as_slice()
            };
            let (red, green, blue) = colors[channel];
            ctx.new_path();
            ctx.move_to(left, bottom);
            for (index, count) in counts.iter().enumerate() {
                let x = left + width * index as f64 / (counts.len() - 1) as f64;
                ctx.line_to(x, bottom - height * (count.ln_1p() / maximum_log));
            }
            ctx.line_to(right, bottom);
            ctx.close_path();
            ctx.set_source_rgba(red, green, blue, if linked { 0.05 } else { 0.10 });
            let _ = ctx.fill();

            ctx.new_path();
            for index in 0..counts.len() {
                let start = index.saturating_sub(2);
                let end = (index + 3).min(counts.len());
                let smoothed = counts[start..end].iter().sum::<f64>() / (end - start) as f64;
                let x = left + width * index as f64 / (counts.len() - 1) as f64;
                let y = bottom - height * (smoothed.ln_1p() / maximum_log);
                if index == 0 {
                    ctx.move_to(x, y);
                } else {
                    ctx.line_to(x, y);
                }
            }
            ctx.set_source_rgba(red, green, blue, if linked { 0.56 } else { 0.72 });
            ctx.set_line_width(if linked { 1.0 } else { 1.4 });
            ctx.set_dash(dash_patterns[channel], 0.0);
            let _ = ctx.stroke();
            ctx.set_dash(&[], 0.0);

            ctx.set_source_rgb(red, green, blue);
            ctx.set_font_size(12.0);
            ctx.move_to(
                left + 8.0 + f64::from(visible_label) * 24.0,
                top + if linked { 34.0 } else { 30.0 },
            );
            let _ = ctx.show_text(labels[channel]);
            visible_label += 1;
        }
    }

    let representative = histograms.representative_hue_degrees;
    if linked && component != HsvHistogramComponent::Hue {
        let strip_height = 12.0;
        for (row, series_component) in [
            HsvHistogramComponent::Saturation,
            HsvHistogramComponent::Value,
        ]
        .into_iter()
        .enumerate()
        {
            let series_label = if series_component == HsvHistogramComponent::Saturation {
                "S"
            } else {
                "V"
            };
            let row = row as f64;
            draw_hsv_strip(
                ctx,
                (
                    left,
                    bottom - strip_height * (2.0 - row),
                    right,
                    bottom - strip_height * (1.0 - row),
                ),
                series_component,
                None,
                origin,
                representative,
                &format!("{series_label} IN"),
            );
            draw_hsv_strip(
                ctx,
                (
                    left,
                    top + strip_height * row,
                    right,
                    top + strip_height * (row + 1.0),
                ),
                series_component,
                Some(hsv_component_quantizer(threshold, series_component)),
                origin,
                representative,
                &format!("{series_label} OUT"),
            );
        }
    } else {
        let strip_height = 12.0;
        let input_label = if component == HsvHistogramComponent::Hue {
            "H IN · 0°/360° SEAM"
        } else if component == HsvHistogramComponent::Saturation {
            "S IN"
        } else {
            "V IN"
        };
        let output_label = if component == HsvHistogramComponent::Hue {
            "H OUT"
        } else if component == HsvHistogramComponent::Saturation {
            "S OUT"
        } else {
            "V OUT"
        };
        draw_hsv_strip(
            ctx,
            (left, bottom - strip_height, right, bottom),
            component,
            None,
            origin,
            representative,
            input_label,
        );
        draw_hsv_strip(
            ctx,
            (left, top, right, top + strip_height),
            component,
            Some(hsv_component_quantizer(threshold, component)),
            origin,
            representative,
            output_label,
        );
        if component == HsvHistogramComponent::Hue {
            ctx.set_source_rgba(0.0, 0.0, 0.0, 0.86);
            ctx.set_line_width(2.0);
            for x in [left + 1.0, right - 1.0] {
                ctx.move_to(x, top);
                ctx.line_to(x, bottom);
            }
            let _ = ctx.stroke();
        }
    }
}

fn threshold_sync_copy_label(target: ThresholdEditTarget) -> &'static str {
    match target {
        ThresholdEditTarget::Red => "Copy Red to Green and Blue",
        ThresholdEditTarget::Green => "Copy Green to Red and Blue",
        ThresholdEditTarget::Blue => "Copy Blue to Red and Green",
        ThresholdEditTarget::Saturation => "Copy Saturation to Value",
        ThresholdEditTarget::Value => "Copy Value to Saturation",
        ThresholdEditTarget::Hue => "Hue has no synchronization peer",
        _ => "Copy selected mapping to peers",
    }
}

struct ThresholdSyncPresentation {
    markup: String,
    accessible_label: String,
    accessible_description: String,
}

fn joined_names(names: &[&str]) -> String {
    match names {
        [] => "none".to_string(),
        [name] => (*name).to_string(),
        [first, second] => format!("{first} and {second}"),
        _ => names.join(", "),
    }
}

fn threshold_sync_presentation(
    threshold: &threshiator::document::ThresholdState,
    target: ThresholdEditTarget,
) -> ThresholdSyncPresentation {
    let destinations: Vec<_> = match target {
        ThresholdEditTarget::Red => vec![ThresholdEditTarget::Green, ThresholdEditTarget::Blue],
        ThresholdEditTarget::Green => vec![ThresholdEditTarget::Red, ThresholdEditTarget::Blue],
        ThresholdEditTarget::Blue => vec![ThresholdEditTarget::Red, ThresholdEditTarget::Green],
        ThresholdEditTarget::Saturation => vec![ThresholdEditTarget::Value],
        ThresholdEditTarget::Value => vec![ThresholdEditTarget::Saturation],
        _ => Vec::new(),
    };
    let writable: Vec<_> = destinations
        .iter()
        .filter(|destination| !threshold.is_locked(**destination))
        .map(|destination| destination.label())
        .collect();
    let locked: Vec<_> = destinations
        .iter()
        .filter(|destination| threshold.is_locked(**destination))
        .map(|destination| destination.label())
        .collect();
    let rendered: Vec<_> = destinations
        .iter()
        .map(|destination| {
            if threshold.is_locked(*destination) {
                format!(
                    "<span strikethrough=\"true\">{}</span>",
                    destination.label()
                )
            } else {
                destination.label().to_string()
            }
        })
        .collect();
    let rendered_refs: Vec<_> = rendered.iter().map(String::as_str).collect();
    let source = target.label();
    let destination_names: Vec<_> = destinations
        .iter()
        .map(|destination| destination.label())
        .collect();
    ThresholdSyncPresentation {
        markup: if destinations.is_empty() {
            "Hue has no synchronization peer".to_string()
        } else {
            format!("Copy {source} to {}", joined_names(&rendered_refs))
        },
        accessible_label: if locked.is_empty() {
            threshold_sync_copy_label(target).to_string()
        } else {
            format!(
                "Copy {source} to {}; locked {} will be skipped",
                joined_names(&destination_names),
                joined_names(&locked)
            )
        },
        accessible_description: format!(
            "Copies Bands, Boundaries, and Outputs from {source}. Writable destinations: {}. Locked destinations skipped: {}.",
            joined_names(&writable),
            joined_names(&locked)
        ),
    }
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
        "Relative degrees from the visible Hue origin, range 0 to 360."
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
    let (target, edited) = {
        let session = session.borrow();
        (session.target, session.draft.clone())
    };
    if edited.validate(target.label()).is_err() {
        return false;
    }
    let before = session.borrow().draft_threshold.clone();
    let changed = {
        let mut session = session.borrow_mut();
        session.draft_threshold.set_edit_quantizer(target, edited)
    };
    if changed {
        commit_threshold_dialog_state(
            ui,
            state,
            session,
            before,
            true,
            "Threshold mapping changed — updating preview…",
        );
    }
    changed
}

fn commit_threshold_dialog_state(
    ui: &Ui,
    state: &Rc<RefCell<State>>,
    session: &Rc<RefCell<ThresholdDialogSession>>,
    before: threshiator::document::ThresholdState,
    preview: bool,
    message: &str,
) -> bool {
    let (threshold, context, dirty) = {
        let mut session = session.borrow_mut();
        if session.draft_threshold == before {
            return false;
        }
        session.undo.push(ThresholdLocalEntry {
            threshold: before,
            preview,
        });
        session.redo.clear();
        session.completed_edits += 1;
        (
            session.draft_threshold.clone(),
            session.context.clone(),
            threshold_dialog_dirty(
                session.pre_dirty,
                &session.snapshot,
                &session.draft_threshold,
            ),
        )
    };
    {
        let mut state = state.borrow_mut();
        let Some(document) = state.document.as_mut() else {
            return false;
        };
        if document_context(document) != context {
            return false;
        }
        document.recipe.threshold = threshold;
        document.dirty = dirty;
    }
    if preview {
        schedule_recipe_edit(ui, state, message);
    } else {
        ui.save.set_sensitive(dirty);
        ui.status.set_label(message);
        sync_threshold_ui(ui, state);
    }
    true
}

fn threshold_dialog_dirty(
    pre_dirty: bool,
    snapshot: &ThresholdState,
    draft: &ThresholdState,
) -> bool {
    pre_dirty || draft != snapshot
}

fn present_threshold_editor(ui: &Rc<Ui>, state: &Rc<RefCell<State>>) {
    if state.borrow().threshold_editor_visible {
        return;
    }
    let Some((threshold, pre_dirty, context, history_snapshot)) =
        state.borrow().document.as_ref().map(|document| {
            let current = state.borrow();
            (
                document.recipe.threshold.clone(),
                document.dirty,
                document_context(document),
                CreativeSnapshot {
                    recipe: document.recipe.clone(),
                    selected_group: current.selected_group,
                    selected_sample: current.selected_sample,
                    expanded_site: current.expanded_site,
                },
            )
        })
    else {
        return;
    };
    let requested = state
        .borrow()
        .threshold_editor_target
        .or(ui.cli.threshold_editor_target)
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
        draft_threshold: threshold.clone(),
        history_snapshot,
        pre_dirty,
        context,
        target,
        draft,
        selected,
        gesture: ThresholdEditGesture::default(),
        undo: Vec::new(),
        redo: Vec::new(),
        completed_edits: 0,
        retain: false,
    }));

    let cancel = gtk::Button::with_label("Cancel");
    let done = gtk::Button::with_label("Done");
    let local_undo = icon_button("edit-undo-symbolic", "Undo mapping edit (Ctrl+Z)");
    let local_redo = icon_button("edit-redo-symbolic", "Redo mapping edit (Ctrl+Shift+Z)");
    local_undo.set_sensitive(false);
    local_redo.set_sensitive(false);
    local_undo.update_property(&[gtk::accessible::Property::Label("Undo mapping edit")]);
    local_redo.update_property(&[gtk::accessible::Property::Label("Redo mapping edit")]);
    *ui.audit_threshold_cancel.borrow_mut() = Some(cancel.clone());
    *ui.audit_threshold_done.borrow_mut() = Some(done.clone());
    *ui.audit_threshold_undo.borrow_mut() = Some(local_undo.clone());
    *ui.audit_threshold_redo.borrow_mut() = Some(local_redo.clone());
    done.add_css_class("suggested-action");
    let header = adw::HeaderBar::new();
    header.pack_start(&cancel);
    header.pack_start(&local_undo);
    header.pack_start(&local_redo);
    header.pack_end(&done);
    let dialog_title = adw::WindowTitle::new("Edit Threshold Mapping", "Changes in this dialog");
    header.set_title_widget(Some(&dialog_title));

    let edit_targets = threshold.edit_targets();
    let target_labels: Vec<_> = edit_targets.iter().map(|target| target.label()).collect();
    let target_model = gtk::StringList::new(&target_labels);
    let target_drop = gtk::DropDown::new(Some(target_model.clone()), None::<gtk::Expression>);
    target_drop.set_focusable(true);
    *ui.audit_threshold_target.borrow_mut() = Some(target_drop.clone());
    target_drop.set_selected(
        edit_targets
            .iter()
            .position(|item| *item == target)
            .unwrap_or(0) as u32,
    );
    target_drop.set_hexpand(false);
    target_drop.update_property(&[
        gtk::accessible::Property::Label("Threshold mapping target"),
        gtk::accessible::Property::Description(
            "Select a linked editing target or inspect an individual component.",
        ),
    ]);
    let link_mappings = gtk::Switch::builder().valign(gtk::Align::Center).build();
    link_mappings.set_tooltip_text(Some(
        "Link future RGB or Saturation/Value edits; existing differences are preserved",
    ));
    link_mappings.update_property(&[
        gtk::accessible::Property::Label("Link component mappings for future edits"),
        gtk::accessible::Property::Description(
            "Future mapping edits copy to eligible unlocked peers; existing differences are preserved",
        ),
    ]);
    let reset_target = icon_button("view-refresh-symbolic", "Reset selected mapping");
    reset_target.set_tooltip_text(Some("Restore only the selected component mapping"));
    reset_target.update_property(&[
        gtk::accessible::Property::Label("Reset selected component mapping"),
        gtk::accessible::Property::Description(
            "Restores the selected component and eligible unlocked linked peers",
        ),
    ]);
    *ui.audit_threshold_reset.borrow_mut() = Some(reset_target.clone());
    let link_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let link_label = gtk::Label::new(Some("Link"));
    link_label.set_mnemonic_widget(Some(&link_mappings));
    link_box.append(&link_label);
    link_box.append(&link_mappings);
    let target_row = adw::ActionRow::builder().title("Target").build();
    target_row.set_tooltip_text(Some(
        "Choose exactly which histogram and mapping to edit. Linked targets use an anchor mapping and may contain mixed peer state.",
    ));
    target_row.add_suffix(&link_box);
    target_row.add_suffix(&reset_target);
    target_row.add_suffix(&target_drop);
    target_row.set_activatable_widget(Some(&target_drop));
    let target_group = adw::PreferencesGroup::new();
    target_group.add(&target_row);
    let management_grid = gtk::Grid::builder()
        .column_spacing(6)
        .row_spacing(6)
        .column_homogeneous(true)
        .build();

    let hue_origin = gtk::SpinButton::with_range(-3600.0, 3600.0, 0.1);
    hue_origin.set_focusable(true);
    hue_origin.set_digits(3);
    hue_origin.set_numeric(true);
    hue_origin.set_width_chars(9);
    hue_origin.update_property(&[
        gtk::accessible::Property::Label("Hue origin in degrees"),
        gtk::accessible::Property::Description(
            "Sets the zero-degree seam before relative Hue band mapping; values wrap from 0 to 360 degrees.",
        ),
    ]);
    let hue_origin_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    hue_origin_row.add_css_class("card");
    hue_origin_row.set_margin_start(6);
    hue_origin_row.set_margin_end(6);
    hue_origin_row.set_margin_top(6);
    hue_origin_row.set_margin_bottom(6);
    let hue_origin_label = gtk::Label::builder()
        .label("Hue origin")
        .hexpand(true)
        .xalign(0.0)
        .build();
    hue_origin_row.append(&hue_origin_label);
    hue_origin_row.append(&hue_origin);
    hue_origin_row.set_tooltip_text(Some("Rotate the zero-degree seam before Hue mapping"));
    management_grid.attach(&hue_origin_row, 1, 1, 1, 1);

    let process = gtk::Switch::builder().valign(gtk::Align::Center).build();
    process.update_property(&[
        gtk::accessible::Property::Label("Process selected component"),
        gtk::accessible::Property::Description(
            "When off, the selected component bypasses Threshold mapping",
        ),
    ]);
    let process_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    process_row.add_css_class("card");
    process_row.set_margin_start(6);
    process_row.set_margin_end(6);
    process_row.set_margin_top(6);
    process_row.set_margin_bottom(6);
    let process_label = gtk::Label::builder()
        .label("Process")
        .hexpand(true)
        .xalign(0.0)
        .build();
    process_row.append(&process_label);
    process_row.append(&process);
    process_row.set_tooltip_text(Some(
        "Turn Threshold processing on or bypass this component",
    ));
    management_grid.attach(&process_row, 0, 0, 1, 1);

    let bands = gtk::SpinButton::with_range(2.0, 32.0, 1.0);
    bands.set_focusable(true);
    bands.set_numeric(true);
    bands.set_width_chars(3);
    bands.update_property(&[
        gtk::accessible::Property::Label("Bands for selected component"),
        gtk::accessible::Property::Description(
            "Safely resize the selected mapping from 2 to 32 Bands",
        ),
    ]);
    let bands_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    bands_row.add_css_class("card");
    bands_row.set_margin_start(6);
    bands_row.set_margin_end(6);
    bands_row.set_margin_top(6);
    bands_row.set_margin_bottom(6);
    let bands_label_widget = gtk::Label::builder()
        .label("Bands")
        .hexpand(true)
        .xalign(0.0)
        .build();
    bands_row.append(&bands_label_widget);
    bands_row.append(&bands);
    bands_row.set_tooltip_text(Some("Plus splits a band; minus merges adjacent bands"));
    management_grid.attach(&bands_row, 1, 0, 1, 1);

    let lock_mapping = gtk::Switch::builder().valign(gtk::Align::Center).build();
    lock_mapping.set_tooltip_text(Some(
        "Protect this component's Bands, Boundaries, Outputs, and mapping reset",
    ));
    lock_mapping.update_property(&[
        gtk::accessible::Property::Label("Lock selected component mapping"),
        gtk::accessible::Property::Description(
            "Protects Bands, Boundaries, Outputs, Reset, and incoming copies; Process remains available",
        ),
    ]);
    let lock_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    lock_row.add_css_class("card");
    lock_row.set_margin_start(6);
    lock_row.set_margin_end(6);
    lock_row.set_margin_top(6);
    lock_row.set_margin_bottom(6);
    let lock_label_widget = gtk::Label::builder()
        .label("Lock")
        .hexpand(true)
        .xalign(0.0)
        .build();
    lock_row.append(&lock_label_widget);
    lock_row.append(&lock_mapping);
    management_grid.attach(&lock_row, 0, 1, 1, 1);

    let sync_presentation = threshold_sync_presentation(&threshold, target);
    let sync_label = gtk::Label::new(None);
    sync_label.set_markup(&sync_presentation.markup);
    let sync_now = gtk::Button::builder().child(&sync_label).build();
    sync_now.update_property(&[
        gtk::accessible::Property::Label(&sync_presentation.accessible_label),
        gtk::accessible::Property::Description(&sync_presentation.accessible_description),
    ]);
    let sync_row = adw::ActionRow::builder().title("Copy mapping").build();
    sync_row.set_tooltip_text(Some(
        "Copy Bands, Boundaries, and Outputs once; locked destinations are skipped",
    ));
    sync_row.add_suffix(&sync_now);
    sync_row.set_activatable_widget(Some(&sync_now));
    let sync_group = adw::PreferencesGroup::new();
    sync_group.add(&sync_row);
    *ui.audit_threshold_process.borrow_mut() = Some(process.clone());
    *ui.audit_threshold_bands.borrow_mut() = Some(bands.clone());
    *ui.audit_threshold_lock.borrow_mut() = Some(lock_mapping.clone());
    *ui.audit_threshold_link.borrow_mut() = Some(link_mappings.clone());
    *ui.audit_threshold_sync.borrow_mut() = Some(sync_now.clone());
    *ui.audit_threshold_hue_origin.borrow_mut() = Some(hue_origin.clone());

    let plot_hint = gtk::Label::builder()
        .label("Drag a Boundary or Output handle · preview updates on release")
        .xalign(0.0)
        .css_classes(["dim-label"])
        .build();
    let plot = gtk::DrawingArea::builder()
        .content_width(480)
        .content_height(280)
        .height_request(250)
        .hexpand(true)
        .vexpand(true)
        .focusable(true)
        .build();
    plot.add_css_class(CREATIVE_FOCUS_CLASS);
    *ui.audit_threshold_plot.borrow_mut() = Some(plot.clone());
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

    let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
    body.set_margin_top(12);
    body.set_margin_bottom(12);
    body.set_margin_start(12);
    body.set_margin_end(12);
    body.append(&target_group);
    body.append(&plot_hint);
    body.append(&plot);
    body.append(&precise_row);
    body.append(&management_grid);
    body.append(&sync_group);
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .child(&body)
        .build();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&scroll));
    let dialog_width = ui
        .cli
        .window_size
        .map_or(720, |(width, _)| if width <= 760 { 660 } else { 720 });
    let dialog = adw::Window::builder()
        .title("Edit Threshold Mapping")
        .transient_for(&ui.window)
        .modal(true)
        .destroy_with_parent(true)
        .default_width(dialog_width)
        .default_height(540)
        .content(&toolbar)
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
        let target_model = target_model.clone();
        let target_drop = target_drop.clone();
        let hue_origin = hue_origin.clone();
        let hue_origin_row = hue_origin_row.clone();
        let process = process.clone();
        let bands = bands.clone();
        let lock_mapping = lock_mapping.clone();
        let link_mappings = link_mappings.clone();
        let link_box = link_box.clone();
        let target_row = target_row.clone();
        let sync_now = sync_now.clone();
        let sync_label = sync_label.clone();
        let sync_row = sync_row.clone();
        let reset_target = reset_target.clone();
        let local_undo = local_undo.clone();
        let local_redo = local_redo.clone();
        let plot_hint = plot_hint.clone();
        let process_row = process_row.clone();
        let lock_row = lock_row.clone();
        move || {
            syncing.set(true);
            local_undo.set_sensitive(!session.borrow().undo.is_empty());
            local_redo.set_sensitive(!session.borrow().redo.is_empty());
            let threshold = session.borrow().draft_threshold.clone();
            let targets = threshold.edit_targets();
            let previous = session.borrow().target;
            let target = threshold.reconcile_edit_target(previous);
            let labels: Vec<_> = targets.iter().map(|target| target.label()).collect();
            target_model.splice(0, target_model.n_items(), &labels);
            target_drop.set_selected(
                targets
                    .iter()
                    .position(|candidate| *candidate == target)
                    .unwrap_or(0) as u32,
            );
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
                    if degrees {
                        "relative degrees from Hue origin"
                    } else {
                        "0 to 1"
                    }
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
            let locked = threshold.is_locked(target);
            let quantizer = threshold.edit_quantizer(target);
            process.set_active(quantizer.enabled);
            bands.set_value(quantizer.outputs.len() as f64);
            hue_origin
                .set_value(f64::from(threshold.hsv_state.hue_origin_degrees).rem_euclid(360.0));
            lock_mapping.set_active(locked);
            link_mappings.set_active(threshold.active_link() == LinkPolicy::Linked);
            let linkable = !target.is_hue();
            let linked_target = matches!(
                target,
                ThresholdEditTarget::LinkedRgb | ThresholdEditTarget::LinkedSaturationValue
            );
            hue_origin_row.set_visible(target.is_hue());
            hue_origin.set_sensitive(target.is_hue() && !locked);
            link_box.set_visible(linkable);
            process_row.set_visible(!linked_target);
            lock_row.set_visible(!linked_target);
            plot.set_sensitive(!locked);
            precise.set_sensitive(!locked);
            bands.set_sensitive(!locked);
            reset_target.set_sensitive(!locked);
            let component = target.label();
            let reset_label = format!("Reset {component} mapping");
            reset_target.update_property(&[
                gtk::accessible::Property::Label(&reset_label),
                gtk::accessible::Property::Description(
                    "Restores this component and eligible unlocked linked peers",
                ),
            ]);
            reset_target.set_tooltip_text(Some(&reset_label));
            let process_label = format!("Process {component}");
            process.update_property(&[
                gtk::accessible::Property::Label(&process_label),
                gtk::accessible::Property::Description(
                    "Off bypasses this component; Process is independent of Link and Lock",
                ),
            ]);
            let bands_label = format!("Bands for {component}");
            bands.update_property(&[
                gtk::accessible::Property::Label(&bands_label),
                gtk::accessible::Property::Description(
                    "Safely resize this component mapping from 2 to 32 Bands",
                ),
            ]);
            let lock_label = format!("Lock {component} mapping");
            lock_mapping.update_property(&[
                gtk::accessible::Property::Label(&lock_label),
                gtk::accessible::Property::Description(
                    "Protects mapping edits and incoming copies while Process remains available",
                ),
            ]);
            let sync_presentation = threshold_sync_presentation(&threshold, target);
            sync_label.set_markup(&sync_presentation.markup);
            sync_now.update_property(&[
                gtk::accessible::Property::Label(&sync_presentation.accessible_label),
                gtk::accessible::Property::Description(&sync_presentation.accessible_description),
            ]);
            link_mappings.update_property(&[
                gtk::accessible::Property::Label(
                    if threshold.active_space == ThresholdSpace::Rgb {
                        "Link RGB mappings for future edits"
                    } else {
                        "Link Saturation and Value mappings for future edits"
                    },
                ),
                gtk::accessible::Property::Description(
                    "Existing mapping differences are preserved; eligible future edits propagate",
                ),
            ]);
            let histograms = state.borrow().threshold_histograms.clone();
            let plot_description =
                threshold_plot_accessibility_description(&threshold, target, histograms.as_deref());
            if target.is_hue() {
                plot_hint.set_label("Drag relative Hue handles · values follow the visible origin");
                plot.update_property(&[
                    gtk::accessible::Property::Label("Relative Hue Input to Output transfer plot"),
                    gtk::accessible::Property::Description(&plot_description),
                ]);
            } else {
                plot_hint
                    .set_label("Drag a Boundary or Output handle · preview updates on release");
                plot.update_property(&[
                    gtk::accessible::Property::Label("Threshold Input to Output transfer plot"),
                    gtk::accessible::Property::Description(&plot_description),
                ]);
            }
            plot.set_tooltip_text(Some(&plot_description));
            let mappings_differ = match target {
                ThresholdEditTarget::Red
                | ThresholdEditTarget::Green
                | ThresholdEditTarget::Blue
                | ThresholdEditTarget::LinkedRgb => {
                    threshold.rgb_state.components.iter().any(|peer| {
                        peer.boundaries != quantizer.boundaries || peer.outputs != quantizer.outputs
                    })
                }
                ThresholdEditTarget::Saturation
                | ThresholdEditTarget::Value
                | ThresholdEditTarget::LinkedSaturationValue => {
                    threshold.hsv_state.saturation.boundaries
                        != threshold.hsv_state.value.boundaries
                        || threshold.hsv_state.saturation.outputs
                            != threshold.hsv_state.value.outputs
                }
                _ => false,
            };
            let process_states_differ = match target {
                ThresholdEditTarget::Red
                | ThresholdEditTarget::Green
                | ThresholdEditTarget::Blue
                | ThresholdEditTarget::LinkedRgb => threshold
                    .rgb_state
                    .components
                    .windows(2)
                    .any(|pair| pair[0].enabled != pair[1].enabled),
                ThresholdEditTarget::Saturation
                | ThresholdEditTarget::Value
                | ThresholdEditTarget::LinkedSaturationValue => {
                    threshold.hsv_state.saturation.enabled != threshold.hsv_state.value.enabled
                }
                ThresholdEditTarget::Hue => false,
            };
            let peers_mixed = mappings_differ || process_states_differ;
            let individual_linkable = linkable && !linked_target;
            sync_row.set_visible(individual_linkable && mappings_differ);
            sync_now.set_sensitive(individual_linkable && mappings_differ);
            target_row.set_tooltip_text(Some(if linked_target {
                if peers_mixed {
                    "Linked target: the visible transfer curve edits the Red or Saturation anchor. Peer mappings or Process states currently differ."
                } else {
                    "Linked target: the visible transfer curve edits the Red or Saturation anchor and propagates to unlocked peers."
                }
            } else {
                "Individual target: only this channel's histogram is shown, even while Link is enabled."
            }));
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
        let state = state.clone();
        move |area, ctx, width, height| {
            let session = session.borrow();
            let left = 54.0;
            let right = f64::from(width) - 20.0;
            let top = 18.0;
            let bottom = f64::from(height) - 42.0;
            let pw = (right - left).max(1.0);
            let ph = (bottom - top).max(1.0);
            let foreground = area.color();
            ctx.set_source_rgba(
                f64::from(foreground.red()),
                f64::from(foreground.green()),
                f64::from(foreground.blue()),
                0.04,
            );
            ctx.rectangle(left, top, pw, ph);
            let _ = ctx.fill();
            if let Some(histograms) = state.borrow().threshold_histograms.clone() {
                match session.draft_threshold.active_space {
                    ThresholdSpace::Rgb => draw_threshold_rgb_histograms(
                        ctx,
                        (left, top, right, bottom),
                        &histograms,
                        &session.draft_threshold,
                        session.target,
                    ),
                    ThresholdSpace::Hsv => draw_threshold_hsv_histograms(
                        ctx,
                        (left, top, right, bottom),
                        &histograms,
                        &session.draft_threshold,
                        session.target,
                    ),
                }
            }
            ctx.set_source_rgba(
                f64::from(foreground.red()),
                f64::from(foreground.green()),
                f64::from(foreground.blue()),
                0.14,
            );
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
            let relative_hue = session.target.is_hue();
            ctx.move_to(
                left + pw / 2.0 - if relative_hue { 42.0 } else { 16.0 },
                f64::from(height) - 12.0,
            );
            let _ = ctx.show_text(if relative_hue {
                "Relative input"
            } else {
                "Input"
            });
            let _ = ctx.save();
            ctx.translate(17.0, top + ph / 2.0 + 22.0);
            ctx.rotate(-std::f64::consts::FRAC_PI_2);
            ctx.move_to(0.0, 0.0);
            let _ = ctx.show_text(if relative_hue {
                "Relative output"
            } else {
                "Output"
            });
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
                if degrees {
                    "relative degrees from Hue origin"
                } else {
                    "0 to 1"
                }
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
        let session = session.clone();
        let refresh = refresh.clone();
        let syncing = syncing.clone();
        move |drop| {
            if syncing.get() {
                return;
            }
            let threshold = session.borrow().draft_threshold.clone();
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

    hue_origin.connect_value_changed({
        let ui = ui.clone();
        let state = state.clone();
        let session = session.clone();
        let refresh = refresh.clone();
        let syncing = syncing.clone();
        move |spin| {
            if syncing.get() {
                return;
            }
            let before = session.borrow().draft_threshold.clone();
            let changed = {
                let mut session = session.borrow_mut();
                session
                    .draft_threshold
                    .set_hue_origin_degrees(spin.value() as f32)
            };
            if !changed {
                if session
                    .borrow()
                    .draft_threshold
                    .is_locked(ThresholdEditTarget::Hue)
                {
                    ui.status
                        .set_label("Unlock the Hue mapping before changing Hue origin");
                }
                refresh();
                return;
            }
            commit_threshold_dialog_state(
                &ui,
                &state,
                &session,
                before,
                true,
                "Hue origin changed — updating preview…",
            );
            refresh();
        }
    });

    process.connect_active_notify({
        let ui = ui.clone();
        let state = state.clone();
        let session = session.clone();
        let refresh = refresh.clone();
        let syncing = syncing.clone();
        move |switch| {
            if syncing.get() {
                return;
            }
            let before = session.borrow().draft_threshold.clone();
            {
                let mut session = session.borrow_mut();
                let enabled = switch.is_active();
                match session.target {
                    ThresholdEditTarget::Red => {
                        session.draft_threshold.rgb_state.components[0].enabled = enabled
                    }
                    ThresholdEditTarget::Green => {
                        session.draft_threshold.rgb_state.components[1].enabled = enabled
                    }
                    ThresholdEditTarget::Blue => {
                        session.draft_threshold.rgb_state.components[2].enabled = enabled
                    }
                    ThresholdEditTarget::Hue => {
                        session.draft_threshold.hsv_state.hue.enabled = enabled
                    }
                    ThresholdEditTarget::Saturation => {
                        session.draft_threshold.hsv_state.saturation.enabled = enabled
                    }
                    ThresholdEditTarget::Value => {
                        session.draft_threshold.hsv_state.value.enabled = enabled
                    }
                    _ => unreachable!("dialog uses individual Threshold targets"),
                }
                session.draft.enabled = enabled;
            }
            commit_threshold_dialog_state(
                &ui,
                &state,
                &session,
                before,
                true,
                "Process/Bypass changed — updating preview…",
            );
            refresh();
        }
    });

    let resize_bands: Rc<dyn Fn(usize)> = Rc::new({
        let ui = ui.clone();
        let state = state.clone();
        let session = session.clone();
        let refresh = refresh.clone();
        move |requested| {
            let (before, target, selected_band, selected_boundary) = {
                let session = session.borrow();
                if session.draft_threshold.is_locked(session.target) {
                    ui.status
                        .set_label("Unlock this mapping before changing Bands");
                    return;
                }
                (
                    session.draft_threshold.clone(),
                    session.target,
                    match session.selected {
                        ThresholdHandle::Output(index) => Some(index),
                        _ => None,
                    },
                    match session.selected {
                        ThresholdHandle::Boundary(index) => Some(index),
                        _ => None,
                    },
                )
            };
            let mut edited = before.edit_quantizer(target);
            if !edited.resize_preserving_mapping(requested, selected_band, selected_boundary) {
                return;
            }
            {
                let mut session = session.borrow_mut();
                session.draft_threshold.set_edit_quantizer(target, edited);
                session.draft = session.draft_threshold.edit_quantizer(target);
            }
            commit_threshold_dialog_state(
                &ui,
                &state,
                &session,
                before,
                true,
                "Bands changed safely — updating preview…",
            );
            refresh();
        }
    });
    bands.connect_value_changed({
        let resize_bands = resize_bands.clone();
        let syncing = syncing.clone();
        move |spin| {
            if !syncing.get() {
                resize_bands(spin.value().round() as usize);
            }
        }
    });
    lock_mapping.connect_active_notify({
        let ui = ui.clone();
        let state = state.clone();
        let session = session.clone();
        let refresh = refresh.clone();
        let syncing = syncing.clone();
        move |switch| {
            if syncing.get() {
                return;
            }
            let before = session.borrow().draft_threshold.clone();
            {
                let mut session = session.borrow_mut();
                let target = session.target;
                session
                    .draft_threshold
                    .set_locked(target, switch.is_active());
            }
            commit_threshold_dialog_state(
                &ui,
                &state,
                &session,
                before,
                false,
                if switch.is_active() {
                    "Mapping locked"
                } else {
                    "Mapping unlocked"
                },
            );
            refresh();
        }
    });

    link_mappings.connect_active_notify({
        let ui = ui.clone();
        let state = state.clone();
        let session = session.clone();
        let refresh = refresh.clone();
        let syncing = syncing.clone();
        move |switch| {
            if syncing.get() {
                return;
            }
            let before = session.borrow().draft_threshold.clone();
            {
                let mut session = session.borrow_mut();
                let link = if switch.is_active() {
                    LinkPolicy::Linked
                } else {
                    LinkPolicy::Independent
                };
                match session.draft_threshold.active_space {
                    ThresholdSpace::Rgb => session.draft_threshold.rgb_state.link = link,
                    ThresholdSpace::Hsv => session.draft_threshold.hsv_state.sv_link = link,
                }
            }
            commit_threshold_dialog_state(
                &ui,
                &state,
                &session,
                before,
                false,
                "Link policy changed; existing mappings were not copied",
            );
            refresh();
        }
    });

    sync_now.connect_clicked({
        let ui = ui.clone();
        let state = state.clone();
        let session = session.clone();
        let refresh = refresh.clone();
        move |_| {
            let before = session.borrow().draft_threshold.clone();
            let (copied, skipped) = {
                let mut session = session.borrow_mut();
                let target = session.target;
                let result = session.draft_threshold.sync_from(target);
                session.draft = session.draft_threshold.edit_quantizer(target);
                result
            };
            if copied == 0 {
                ui.status.set_label(if skipped > 0 {
                    "No mappings synchronized; every destination is locked"
                } else {
                    "This component has no synchronization peer"
                });
                return;
            }
            let changed = commit_threshold_dialog_state(
                &ui,
                &state,
                &session,
                before,
                true,
                if skipped > 0 {
                    "Mappings synchronized; locked destinations were skipped — updating preview…"
                } else {
                    "Mappings synchronized — updating preview…"
                },
            );
            if !changed {
                ui.status
                    .set_label("Eligible mappings already match the selected component");
            }
            refresh();
        }
    });

    reset_target.connect_clicked({
        let ui = ui.clone();
        let state = state.clone();
        let session = session.clone();
        let plot = plot.clone();
        let update_precise = update_precise.clone();
        move |_| {
            let (target, mut threshold) = {
                let session = session.borrow();
                (session.target, session.draft_threshold.clone())
            };
            if threshold.is_locked(target) {
                ui.status
                    .set_label("Unlock this mapping before resetting it");
                return;
            }
            let before = threshold.clone();
            if !reset_component(&mut threshold, target) {
                ui.status
                    .set_label("Selected component already uses its default mapping");
                return;
            }
            {
                let mut session = session.borrow_mut();
                session.draft_threshold = threshold;
                session.draft = session.draft_threshold.edit_quantizer(target);
            }
            if commit_threshold_dialog_state(
                &ui,
                &state,
                &session,
                before,
                true,
                "Selected component mapping reset — updating preview…",
            ) {
                ui.status.set_label("Selected component mapping reset");
                plot.queue_draw();
                update_precise();
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

    let local_history_step: Rc<dyn Fn(bool)> = Rc::new({
        let ui = ui.clone();
        let state = state.clone();
        let session = session.clone();
        let refresh = refresh.clone();
        let dialog_title = dialog_title.clone();
        move |redo| {
            let (target, preview, threshold, context, dirty) = {
                let mut session = session.borrow_mut();
                let entry = if redo {
                    session.redo.pop()
                } else {
                    session.undo.pop()
                };
                let Some(entry) = entry else {
                    dialog_title.set_subtitle(if redo {
                        "Nothing to redo locally"
                    } else {
                        "Nothing to undo locally"
                    });
                    return;
                };
                let current = ThresholdLocalEntry {
                    threshold: session.draft_threshold.clone(),
                    preview: entry.preview,
                };
                if redo {
                    session.undo.push(current);
                } else {
                    session.redo.push(current);
                }
                session.draft_threshold = entry.threshold;
                session.target = session
                    .draft_threshold
                    .reconcile_edit_target(session.target);
                session.draft = session.draft_threshold.edit_quantizer(session.target);
                (
                    session.target,
                    entry.preview,
                    session.draft_threshold.clone(),
                    session.context.clone(),
                    if session.draft_threshold == session.snapshot {
                        session.pre_dirty
                    } else {
                        true
                    },
                )
            };
            {
                let mut current = state.borrow_mut();
                let Some(document) = current.document.as_mut() else {
                    return;
                };
                if document_context(document) != context {
                    return;
                }
                document.recipe.threshold = threshold;
                document.dirty = dirty;
            }
            state.borrow_mut().threshold_editor_target = Some(target);
            ui.save.set_sensitive(dirty);
            if preview {
                schedule_existing_recipe_preview(
                    &ui,
                    &state,
                    if redo {
                        "Threshold edit redone — updating preview…"
                    } else {
                        "Threshold edit undone — updating preview…"
                    },
                );
            } else {
                ui.status.set_label(if redo {
                    "Threshold edit redone"
                } else {
                    "Threshold edit undone"
                });
                sync_threshold_ui(&ui, &state);
            }
            dialog_title.set_subtitle(if redo {
                "Mapping edit redone locally"
            } else {
                "Mapping edit undone locally"
            });
            refresh();
        }
    });
    *ui.audit_threshold_history.borrow_mut() = Some(local_history_step.clone());
    local_undo.connect_clicked({
        let local_history_step = local_history_step.clone();
        move |_| local_history_step(false)
    });
    local_redo.connect_clicked({
        let local_history_step = local_history_step.clone();
        move |_| local_history_step(true)
    });

    let rollback_cancelled_edits: Rc<dyn Fn()> = Rc::new({
        let ui = ui.clone();
        let state = state.clone();
        let session = session.clone();
        move || {
            let (retain, changed, snapshot, pre_dirty, context) = {
                let session = session.borrow();
                (
                    session.retain,
                    session.draft_threshold != session.snapshot,
                    session.snapshot.clone(),
                    session.pre_dirty,
                    session.context.clone(),
                )
            };
            if !retain && changed {
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
        }
    });
    cancel.connect_clicked({
        let dialog = dialog.clone();
        move |_| dialog.close()
    });
    done.connect_clicked({
        let ui = ui.clone();
        let state = state.clone();
        let session = session.clone();
        let dialog = dialog.clone();
        move |_| {
            let (changed, snapshot, context) = {
                let mut session = session.borrow_mut();
                session.retain = true;
                (
                    session.draft_threshold != session.snapshot,
                    session.history_snapshot.clone(),
                    session.context.clone(),
                )
            };
            if changed {
                let valid = state
                    .borrow()
                    .document
                    .as_ref()
                    .is_some_and(|document| document_context(document) == context);
                if valid {
                    record_creative_change(&ui, &state, snapshot, None);
                }
            }
            dialog.close();
        }
    });
    dialog.connect_close_request({
        let ui = ui.clone();
        let state = state.clone();
        let session = session.clone();
        let rollback_cancelled_edits = rollback_cancelled_edits.clone();
        move |_| {
            if !session.borrow().retain {
                rollback_cancelled_edits();
            }
            state.borrow_mut().threshold_editor_visible = false;
            *ui.threshold_editor_refresh.borrow_mut() = None;
            *ui.threshold_editor_dialog.borrow_mut() = None;
            *ui.audit_threshold_target.borrow_mut() = None;
            *ui.audit_threshold_process.borrow_mut() = None;
            *ui.audit_threshold_bands.borrow_mut() = None;
            *ui.audit_threshold_lock.borrow_mut() = None;
            *ui.audit_threshold_link.borrow_mut() = None;
            *ui.audit_threshold_sync.borrow_mut() = None;
            *ui.audit_threshold_hue_origin.borrow_mut() = None;
            *ui.audit_threshold_history.borrow_mut() = None;
            *ui.audit_threshold_undo.borrow_mut() = None;
            *ui.audit_threshold_redo.borrow_mut() = None;
            *ui.audit_threshold_precise.borrow_mut() = None;
            *ui.audit_threshold_cancel.borrow_mut() = None;
            *ui.audit_threshold_done.borrow_mut() = None;
            *ui.audit_threshold_reset.borrow_mut() = None;
            *ui.audit_threshold_plot.borrow_mut() = None;
            sync_threshold_ui(&ui, &state);
            sync_document_history_ui(&ui, &state);
            if let Some(return_focus) = ui.threshold_editor_return_focus.borrow_mut().take() {
                return_focus.grab_focus();
            }
            glib::Propagation::Proceed
        }
    });
    let escape = gtk::EventControllerKey::new();
    escape.set_propagation_phase(gtk::PropagationPhase::Capture);
    escape.connect_key_pressed({
        let dialog = dialog.clone();
        let local_history_step = local_history_step.clone();
        move |_, key, _, modifiers| {
            if key == gtk::gdk::Key::Escape {
                dialog.close();
                glib::Propagation::Stop
            } else if key == gtk::gdk::Key::z
                && modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK)
            {
                local_history_step(modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK));
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        }
    });
    dialog.add_controller(escape);

    refresh();
    {
        let mut state = state.borrow_mut();
        state.threshold_editor_visible = true;
        state.threshold_editor_target = Some(session.borrow().target);
        state.threshold_editor_handle = Some(session.borrow().selected);
    }
    sync_document_history_ui(ui, state);
    // Give the parent a stable, always-visible return target before Mutter transfers focus.
    ui.method_thresholds.grab_focus();
    dialog.present();
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
        .set_subtitle(&format!("{summary} · {description}"));
    ui.preset_dropdown
        .set_tooltip_text(Some(&format!("{summary} · {description}")));
}

fn method_suffix(method: Method) -> &'static str {
    match method {
        Method::Thresholds => "— Threshold",
        Method::Voronoi => "— Voronoi",
    }
}

fn unified_preset_labels(entries: &[PresetEntry]) -> Vec<String> {
    let mut labels = vec!["Presets…".to_owned()];
    labels.extend(
        STARTER_LOOKS
            .iter()
            .map(|look| format!("{} {}", look.name, method_suffix(look.method))),
    );
    labels.extend(entries.iter().map(|entry| {
        format!(
            "{} {}",
            entry.preset.name,
            method_suffix(entry.preset.processing.active_method)
        )
    }));
    labels
}

fn install_preset_scan(
    ui: &Ui,
    state: &Rc<RefCell<State>>,
    scan: threshiator::preset::PresetScan,
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
        let dialog = adw::AlertDialog::builder()
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
    let before = {
        let current = state.borrow();
        let Some(document) = current.document.as_ref() else {
            return;
        };
        CreativeSnapshot {
            recipe: document.recipe.clone(),
            selected_group: current.selected_group,
            selected_sample: current.selected_sample,
            expanded_site: current.expanded_site,
        }
    };
    let result = {
        let mut current = state.borrow_mut();
        let Some(document) = current.document.as_mut() else {
            return;
        };
        if selected < STARTER_LOOKS.len() {
            Ok(apply_starter_look(document, selected))
        } else if let Some(entry) = user_preset {
            apply_to_document(document, &entry.preset)
        } else {
            return;
        }
    };
    let changed = match result {
        Ok(changed) => changed,
        Err(error) => {
            present_preset_error(ui, "Could not apply preset", &error);
            return;
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
        if current.expanded_site.is_some() {
            current.expanded_site = first;
        }
        current.selected_group = first;
        current.selected_sample = first;
    }
    record_creative_change(ui, state, before, None);
    ui.syncing.set(true);
    ui.preset_dropdown.set_selected(applied_selection as u32);
    ui.syncing.set(false);
    update_preset_info(ui, state);
    sync_method_and_threshold_controls(ui, state);
    refresh_voronoi_ui(ui, state);
    schedule_recipe_edit(ui, state, "Preset applied — updating preview…");
}

fn creative_history_step(ui: &Rc<Ui>, state: &Rc<RefCell<State>>, redo: bool) {
    if state.borrow().threshold_editor_visible || state.borrow().picker_visible {
        return;
    }
    if state.borrow().jobs.is_busy() {
        ui.status
            .set_label("Finish or cancel the file operation before using document history");
        return;
    }
    let target = {
        let mut current = state.borrow_mut();
        let Some(now) = creative_snapshot(&current) else {
            return;
        };
        current.coalesced_edit = None;
        if redo {
            let Some(target) = current.creative_history.redo.pop() else {
                ui.status.set_label("Nothing to redo");
                return;
            };
            current.creative_history.undo.push(now);
            target
        } else {
            let Some(target) = current.creative_history.undo.pop() else {
                ui.status.set_label("Nothing to undo");
                return;
            };
            current.creative_history.redo.push(now);
            target
        }
    };
    {
        let mut current = state.borrow_mut();
        let recipe = {
            let Some(document) = current.document.as_mut() else {
                return;
            };
            document.recipe = target.recipe;
            document.recipe.clone()
        };
        let dirty = current.creative_history.is_dirty(&recipe);
        if let Some(document) = current.document.as_mut() {
            document.dirty = dirty;
        }
        current.selected_group = target.selected_group;
        current.selected_sample = target.selected_sample;
        current.expanded_site = target.expanded_site;
        current.sampling = None;
        current.sampling_previous_mode = None;
        if let Some(source) = current.preview_source.clone() {
            current.scheduler.schedule(source, recipe);
        }
    }
    ui.syncing.set(true);
    ui.preset_dropdown.set_selected(0);
    ui.syncing.set(false);
    sync_method_and_threshold_controls(ui, state);
    refresh_voronoi_ui(ui, state);
    let dirty = state
        .borrow()
        .document
        .as_ref()
        .is_some_and(|document| document.dirty);
    ui.save.set_sensitive(dirty);
    sync_document_history_ui(ui, state);
    ui.status.set_label(if redo {
        "Document change redone — updating preview…"
    } else {
        "Document change undone — updating preview…"
    });
}

fn creative_history_actions(app: &adw::Application, ui: &Rc<Ui>, state: &Rc<RefCell<State>>) {
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

fn sync_method_and_threshold_controls(ui: &Ui, state: &Rc<RefCell<State>>) {
    let Some(recipe) = state
        .borrow()
        .document
        .as_ref()
        .map(|document| document.recipe.clone())
    else {
        return;
    };
    ui.syncing.set(true);
    ui.method_voronoi
        .set_active(recipe.active_method == Method::Voronoi);
    ui.method_thresholds
        .set_active(recipe.active_method == Method::Thresholds);
    ui.method_section.set_label(Some("Method"));
    ui.voronoi_panel
        .set_visible(recipe.active_method == Method::Voronoi);
    ui.voronoi_matching_row
        .set_visible(recipe.active_method == Method::Voronoi);
    ui.thresholds_panel
        .set_visible(recipe.active_method == Method::Thresholds);
    ui.syncing.set(false);
    sync_threshold_ui(ui, state);
}

fn threshold_reset_controls(ui: &Rc<Ui>, state: Rc<RefCell<State>>) {
    for (button, complete_method) in [(&ui.reset_space, false), (&ui.reset_method, true)] {
        let ui = ui.clone();
        let state = state.clone();
        button.connect_clicked(move |_| {
            let space = state
                .borrow()
                .document
                .as_ref()
                .map(|document| document.recipe.threshold.active_space);
            let Some(space) = space else { return };
            let heading = if complete_method {
                "Reset the complete Threshold method?"
            } else {
                "Reset the active Working space?"
            };
            let body = if complete_method {
                "This restores all RGB and HSV mappings, links, Process flags, alpha policy, and smoothing. You can undo this document change."
                    .to_owned()
            } else {
                format!(
                    "This restores only the {} mappings, link, and Process flags. You can undo this document change.",
                    if space == ThresholdSpace::Rgb { "RGB" } else { "HSV" }
                )
            };
            let confirm = adw::AlertDialog::builder()
                .heading(heading)
                .body(body)
                .build();
            confirm.add_response("cancel", "Cancel");
            confirm.add_response("reset", "Reset");
            confirm.set_response_appearance("reset", adw::ResponseAppearance::Destructive);
            let ui = ui.clone();
            let state = state.clone();
            glib::spawn_future_local(async move {
                if confirm.choose_future(Some(&ui.window)).await != "reset" {
                    return;
                }
                let Some(before) = creative_snapshot(&state.borrow()) else {
                    return;
                };
                let changed = {
                    let mut state = state.borrow_mut();
                    let Some(document) = state.document.as_mut() else { return };
                    if complete_method {
                        reset_method(&mut document.recipe.threshold)
                    } else {
                        reset_active_space(&mut document.recipe.threshold)
                    }
                };
                if !changed {
                    ui.status.set_label(if complete_method {
                        "Threshold method already uses its defaults"
                    } else {
                        "Active Working space already uses its defaults"
                    });
                    return;
                }
                record_creative_change(&ui, &state, before, None);
                sync_threshold_ui(&ui, &state);
                schedule_recipe_edit(
                    &ui,
                    &state,
                    if complete_method {
                        "Threshold method reset — updating preview…"
                    } else {
                        "Active Working space reset — updating preview…"
                    },
                );
            });
        });
    }
}

fn recipe_controls(ui: &Rc<Ui>, state: Rc<RefCell<State>>) {
    {
        let ui = ui.clone();
        let state = state.clone();
        ui.threshold_space
            .clone()
            .connect_selected_notify(move |control| {
                if ui.syncing.get() {
                    return;
                }
                if ui.threshold_editor_dialog.borrow().is_some() {
                    ui.status
                        .set_label("Close Edit Threshold Mapping before changing Working space");
                    sync_threshold_ui(&ui, &state);
                    return;
                }
                let requested = if control.selected() == 0 {
                    ThresholdSpace::Rgb
                } else {
                    ThresholdSpace::Hsv
                };
                let Some(before) = creative_snapshot(&state.borrow()) else {
                    return;
                };
                let changed = state
                    .borrow_mut()
                    .document
                    .as_mut()
                    .is_some_and(|document| {
                        if document.recipe.threshold.active_space == requested {
                            false
                        } else {
                            document.recipe.threshold.active_space = requested;
                            true
                        }
                    });
                if changed {
                    record_creative_change(&ui, &state, before, None);
                    sync_threshold_ui(&ui, &state);
                    schedule_recipe_edit(&ui, &state, "Working space changed — updating preview…");
                }
            });
    }
    {
        let ui = ui.clone();
        let state = state.clone();
        ui.smoothing.clone().connect_value_changed(move |control| {
            if ui.syncing.get() {
                return;
            }
            let value = control.value().round().clamp(0.0, 10.0) as f32;
            let Some(before) = creative_snapshot(&state.borrow()) else {
                return;
            };
            let changed = state
                .borrow_mut()
                .document
                .as_mut()
                .is_some_and(|document| {
                    if document.recipe.threshold.input_smoothing == value {
                        false
                    } else {
                        document.recipe.threshold.input_smoothing = value;
                        true
                    }
                });
            if changed {
                record_creative_change(&ui, &state, before, Some(CoalescedEdit::Smoothing));
                schedule_recipe_edit(&ui, &state, "Smooth source changed — updating preview…");
            }
        });
    }
    install_document_spin_boundaries(&ui.smoothing, &state, CoalescedEdit::Smoothing);
    let update_hue: Rc<dyn Fn()> = Rc::new({
        let ui = ui.clone();
        let state = state.clone();
        move || {
            if ui.syncing.get() {
                return;
            }
            let Some(before) = creative_snapshot(&state.borrow()) else {
                return;
            };
            if let Some(doc) = state.borrow_mut().document.as_mut() {
                doc.set_hue(ui.hue.value() as f32);
            }
            if record_creative_change(&ui, &state, before, None) {
                schedule_recipe_edit(&ui, &state, "Hue changed — updating preview…");
            }
        }
    });
    let u = update_hue;
    ui.hue.connect_value_changed(move |_| u());

    {
        let ui = ui.clone();
        let state = state.clone();
        ui.threshold_link
            .clone()
            .connect_active_notify(move |link| {
                if ui.syncing.get() {
                    return;
                }
                let Some(before) = creative_snapshot(&state.borrow()) else {
                    return;
                };
                let requested = if link.is_active() {
                    LinkPolicy::Linked
                } else {
                    LinkPolicy::Independent
                };
                let changed = state
                    .borrow_mut()
                    .document
                    .as_mut()
                    .is_some_and(|document| {
                        let current = match document.recipe.threshold.active_space {
                            ThresholdSpace::Rgb => &mut document.recipe.threshold.rgb_state.link,
                            ThresholdSpace::Hsv => &mut document.recipe.threshold.hsv_state.sv_link,
                        };
                        if *current == requested {
                            false
                        } else {
                            *current = requested;
                            true
                        }
                    });
                if changed {
                    record_creative_change(&ui, &state, before, None);
                    sync_threshold_ui(&ui, &state);
                    ui.status.set_label(if requested == LinkPolicy::Linked {
                        "Link enabled for future mapping edits; existing differences remain"
                    } else {
                        "Mappings are now independent; existing mappings remain"
                    });
                }
            });
    }
    {
        let ui = ui.clone();
        let state = state.clone();
        ui.threshold_link_editor
            .clone()
            .connect_activated(move |action| {
                let target =
                    state.borrow().document.as_ref().and_then(|document| {
                        threshold_link_editor_target(&document.recipe.threshold)
                    });
                let Some(target) = target else {
                    sync_threshold_ui(&ui, &state);
                    return;
                };
                *ui.threshold_editor_return_focus.borrow_mut() = Some(action.clone().upcast());
                state.borrow_mut().threshold_editor_target = Some(target);
                present_threshold_editor(&ui, &state);
            });
    }
    for (index, controls) in ui.threshold_component_rows.iter().cloned().enumerate() {
        {
            let ui = ui.clone();
            let state = state.clone();
            controls
                .process
                .clone()
                .connect_active_notify(move |process| {
                    if ui.syncing.get() {
                        return;
                    }
                    let Some(before) = creative_snapshot(&state.borrow()) else {
                        return;
                    };
                    let changed = state
                        .borrow_mut()
                        .document
                        .as_mut()
                        .is_some_and(|document| {
                            let target = individual_threshold_targets(
                                document.recipe.threshold.active_space,
                            )[index];
                            set_threshold_component_enabled(
                                &mut document.recipe.threshold,
                                target,
                                process.is_active(),
                            )
                        });
                    if changed {
                        record_creative_change(&ui, &state, before, None);
                        sync_threshold_ui(&ui, &state);
                        schedule_recipe_edit(
                            &ui,
                            &state,
                            "Component processing changed — updating preview…",
                        );
                    }
                });
        }
        {
            let ui = ui.clone();
            let state = state.clone();
            let state_handler = state.clone();
            controls.bands.clone().connect_value_changed(move |bands| {
                if ui.syncing.get() {
                    return;
                }
                let Some(before) = creative_snapshot(&state_handler.borrow()) else {
                    return;
                };
                let requested = bands.value().round() as usize;
                let changed =
                    state_handler
                        .borrow_mut()
                        .document
                        .as_mut()
                        .is_some_and(|document| {
                            let target = individual_threshold_targets(
                                document.recipe.threshold.active_space,
                            )[index];
                            resize_threshold_component(
                                &mut document.recipe.threshold,
                                target,
                                requested,
                            )
                        });
                if changed {
                    record_creative_change(
                        &ui,
                        &state_handler,
                        before,
                        Some(CoalescedEdit::ThresholdBands(index)),
                    );
                    sync_threshold_ui(&ui, &state_handler);
                    schedule_recipe_edit(&ui, &state_handler, "Bands changed — updating preview…");
                } else {
                    sync_threshold_ui(&ui, &state_handler);
                }
            });
            install_document_spin_boundaries(
                &controls.bands,
                &state,
                CoalescedEdit::ThresholdBands(index),
            );
        }
        {
            let ui = ui.clone();
            let state = state.clone();
            controls.auto.clone().connect_clicked(move |_| {
                if ui.syncing.get() {
                    return;
                }
                let Some(before) = creative_snapshot(&state.borrow()) else {
                    return;
                };
                let outcome = state.borrow_mut().document.as_mut().map(|document| {
                    let target = individual_threshold_targets(
                        document.recipe.threshold.active_space,
                    )[index];
                    let previous = document.recipe.threshold.clone();
                    let locked = document.recipe.threshold.is_locked(target);
                    let changed = document.recipe.threshold.auto_map(target);
                    let linked_peers = automatic_linked_peer_changes(
                        &previous,
                        &document.recipe.threshold,
                        target,
                    );
                    (target, changed, locked, linked_peers)
                });
                let Some((target, changed, locked, linked_peers)) = outcome else {
                    return;
                };
                let name = target.label();
                if changed {
                    record_creative_change(&ui, &state, before, None);
                    sync_threshold_ui(&ui, &state);
                    let message = match linked_peers {
                        0 => format!("{name} automatic baseline applied — updating preview…"),
                        1 => format!(
                            "{name} automatic baseline applied to 1 linked peer — updating preview…"
                        ),
                        count => format!(
                            "{name} automatic baseline applied to {count} linked peers — updating preview…"
                        ),
                    };
                    schedule_recipe_edit(&ui, &state, &message);
                } else {
                    sync_threshold_ui(&ui, &state);
                    let message = if locked {
                        format!("{name} mapping is locked")
                    } else {
                        format!("{name} already uses the automatic baseline")
                    };
                    ui.status.set_label(&message);
                }
            });
        }
        {
            let ui = ui.clone();
            let state = state.clone();
            controls.lock.clone().connect_active_notify(move |lock| {
                if ui.syncing.get() {
                    return;
                }
                let Some(before) = creative_snapshot(&state.borrow()) else {
                    return;
                };
                let changed = state
                    .borrow_mut()
                    .document
                    .as_mut()
                    .is_some_and(|document| {
                        let target =
                            individual_threshold_targets(document.recipe.threshold.active_space)
                                [index];
                        document
                            .recipe
                            .threshold
                            .set_locked(target, lock.is_active())
                    });
                if changed {
                    record_creative_change(&ui, &state, before, None);
                    sync_threshold_ui(&ui, &state);
                    ui.status.set_label(if lock.is_active() {
                        "Mapping locked"
                    } else {
                        "Mapping unlocked"
                    });
                }
            });
        }
        if index == 0 {
            let ui = ui.clone();
            let state = state.clone();
            let state_handler = state.clone();
            controls
                .hue_origin
                .clone()
                .connect_value_changed(move |origin| {
                    if ui.syncing.get() {
                        return;
                    }
                    let Some(before) = creative_snapshot(&state_handler.borrow()) else {
                        return;
                    };
                    let changed =
                        state_handler
                            .borrow_mut()
                            .document
                            .as_mut()
                            .is_some_and(|document| {
                                document
                                    .recipe
                                    .threshold
                                    .set_hue_origin_degrees(origin.value() as f32)
                            });
                    if changed {
                        record_creative_change(
                            &ui,
                            &state_handler,
                            before,
                            Some(CoalescedEdit::HueOrigin),
                        );
                        sync_threshold_ui(&ui, &state_handler);
                        schedule_recipe_edit(
                            &ui,
                            &state_handler,
                            "Hue origin changed — updating preview…",
                        );
                    } else {
                        sync_threshold_ui(&ui, &state_handler);
                    }
                });
            install_document_spin_boundaries(
                &controls.hue_origin,
                &state,
                CoalescedEdit::HueOrigin,
            );
        }
        {
            let ui = ui.clone();
            let state = state.clone();
            controls.edit.connect_activated(move |action| {
                let target = state.borrow().document.as_ref().map(|document| {
                    individual_threshold_targets(document.recipe.threshold.active_space)[index]
                });
                let Some(target) = target else { return };
                *ui.threshold_editor_return_focus.borrow_mut() = Some(action.clone().upcast());
                state.borrow_mut().threshold_editor_target = Some(target);
                present_threshold_editor(&ui, &state);
            });
        }
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
                let Some(before) = creative_snapshot(&state.borrow()) else {
                    return;
                };
                if let Some(document) = state.borrow_mut().document.as_mut() {
                    document.recipe.voronoi.matching =
                        visible_voronoi_matching_at(drop_down.selected());
                }
                if record_creative_change(&ui, &state, before, None) {
                    schedule_recipe_edit(&ui, &state, "Color matching changed — updating preview…");
                }
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
            ui.voronoi_matching_row
                .set_visible(method == Method::Voronoi);
            ui.thresholds_panel
                .set_visible(method == Method::Thresholds);
            ui.method_section.set_label(Some("Method"));
            let mut current = state.borrow_mut();
            let Some(before) = creative_snapshot(&current) else {
                return;
            };
            let Some(document) = current.document.as_mut() else {
                return;
            };
            if document.recipe.active_method == method {
                return;
            }
            document.recipe.active_method = method;
            let recipe = document.recipe.clone();
            if let Some(source) = current.preview_source.clone() {
                current.scheduler.schedule(source, recipe);
            }
            drop(current);
            record_creative_change(&ui, &state, before, None);
            ui.save.set_sensitive(
                state
                    .borrow()
                    .document
                    .as_ref()
                    .is_some_and(|document| document.dirty),
            );
            ui.status.set_label(match method {
                Method::Voronoi => "Color sites active — updating preview…",
                Method::Thresholds => "Thresholds active — updating preview…",
            });
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
        if current.selected_sample == Some(id) {
            return;
        }
        let details_were_open = current.expanded_site.is_some();
        current.selected_group = Some(id);
        current.selected_sample = Some(id);
        if details_were_open {
            current.expanded_site = Some(id);
        }
        drop(current);
        ui.canvas.queue_draw();
        let ui = ui.clone();
        let state = state.clone();
        glib::idle_add_local_once(move || refresh_voronoi_ui(&ui, &state));
    });
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

fn add_site_from_artwork(ui: &Rc<Ui>, state: &Rc<RefCell<State>>, position: [f64; 2]) -> bool {
    let Some(before) = creative_snapshot(&state.borrow()) else {
        return false;
    };
    let new_site = {
        let mut current = state.borrow_mut();
        let Some(document) = current.document.as_mut() else {
            return false;
        };
        if document.recipe.active_method != Method::Voronoi
            || threshiator::voronoi::sample_color(
                &document.source,
                position,
                SampleSize::ThreeByThree,
            )
            .is_none()
        {
            return false;
        }
        let site = threshiator::voronoi::add_site_at(
            &mut document.recipe.voronoi,
            &document.source,
            position,
        );
        let recipe = document.recipe.clone();
        current.selected_group = site;
        current.selected_sample = site;
        if current.expanded_site.is_some() {
            current.expanded_site = site;
        }
        if let Some(source) = current.preview_source.clone() {
            current.scheduler.schedule(source, recipe);
        }
        site
    };
    if new_site.is_some() {
        record_creative_change(ui, state, before, None);
        refresh_voronoi_ui(ui, state);
        ui.save.set_sensitive(
            state
                .borrow()
                .document
                .as_ref()
                .is_some_and(|document| document.dirty),
        );
        ui.status
            .set_label("Site added from artwork — updating preview…");
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
        let encoded = color.map(threshiator::processing::linear_to_srgb);
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

fn refresh_voronoi_ui(ui: &Rc<Ui>, state: &Rc<RefCell<State>>) {
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
        state.document.as_ref().map(|document| {
            (
                document.recipe.voronoi.sites.clone(),
                state.coverage.clone(),
                state.selected_group,
                state.expanded_site,
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
        let row = adw::ExpanderRow::builder()
            .title(site_label(index))
            .expanded(expanded_site == Some(site_id))
            .build();
        row.set_title_lines(1);
        row.set_subtitle_lines(1);
        row.set_subtitle(&format!(
            "{} · {:.1}%{}",
            if site.position.is_some() {
                "Attached"
            } else {
                "Detached"
            },
            covered as f64 * 100.0 / visible as f64,
            if site.locked { " · Locked" } else { "" }
        ));
        row.set_widget_name(&site.id.to_string());
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
        let arrow = gtk::Label::new(Some("→"));
        arrow.set_width_chars(1);
        arrow.set_xalign(0.5);
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
        row.add_suffix(&color_flow);
        row.add_suffix(&lock);

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
            "Choose a source position; Source and Target both reset to the sampled color",
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
        for (row_index, label_text) in ["Influence", "Source position", "Footprint"]
            .into_iter()
            .enumerate()
        {
            let label = gtk::Label::builder()
                .label(label_text)
                .xalign(0.0)
                .hexpand(true)
                .build();
            details.attach(&label, 0, row_index as i32, 1, 1);
        }
        details.attach(&influence, 1, 0, 1, 1);
        details.attach(&reattach, 1, 1, 1, 1);
        details.attach(&sample_size, 1, 2, 1, 1);
        details.attach(&delete, 1, 3, 1, 1);
        row.add_row(&details);
        ui.groups.append(&row);
        if selected_site == Some(site.id) {
            ui.groups.select_row(Some(&row));
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
                    current.selected_group = Some(site_id);
                    current.selected_sample = Some(site_id);
                    current.expanded_site = Some(site_id);
                } else if current.expanded_site == Some(site_id) {
                    current.expanded_site = None;
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

        for (button, purpose) in [
            (source, PickerPurpose::Source),
            (target, PickerPurpose::Target),
        ] {
            let ui = ui.clone();
            let state = state.clone();
            button.connect_clicked(move |_| {
                {
                    let mut current = state.borrow_mut();
                    current.selected_group = Some(site_id);
                    current.selected_sample = Some(site_id);
                    if current.expanded_site.is_some() {
                        current.expanded_site = Some(site_id);
                    }
                }
                ui.canvas.queue_draw();
                present_color_picker(&ui, &state, ui.cli.color_model, purpose);
            });
        }
        {
            let ui = ui.clone();
            let state = state.clone();
            lock.connect_toggled(move |control| {
                let Some(before) = creative_snapshot(&state.borrow()) else {
                    return;
                };
                let changed = state
                    .borrow_mut()
                    .document
                    .as_mut()
                    .is_some_and(|document| {
                        document
                            .recipe
                            .voronoi
                            .set_locked(site_id, control.is_active())
                    });
                if changed {
                    record_creative_change(&ui, &state, before, None);
                    {
                        let mut current = state.borrow_mut();
                        current.selected_group = Some(site_id);
                        current.selected_sample = Some(site_id);
                        if current.expanded_site.is_some() {
                            current.expanded_site = Some(site_id);
                        }
                    }
                    ui.save.set_sensitive(
                        state
                            .borrow()
                            .document
                            .as_ref()
                            .is_some_and(|document| document.dirty),
                    );
                    ui.status.set_label(if control.is_active() {
                        "Site locked — its parameters are protected"
                    } else {
                        "Site unlocked — editing is available"
                    });
                    refresh_voronoi_ui(&ui, &state);
                    ui.canvas.queue_draw();
                }
            });
        }
        {
            let ui = ui.clone();
            let state = state.clone();
            influence.connect_value_changed(move |control| {
                let Some(before) = creative_snapshot(&state.borrow()) else {
                    return;
                };
                let changed = state
                    .borrow_mut()
                    .document
                    .as_mut()
                    .is_some_and(|document| {
                        document
                            .recipe
                            .voronoi
                            .set_influence(site_id, control.value())
                    });
                if changed {
                    record_creative_change(
                        &ui,
                        &state,
                        before,
                        Some(CoalescedEdit::Influence(site_id)),
                    );
                    {
                        let mut current = state.borrow_mut();
                        current.selected_group = Some(site_id);
                        current.selected_sample = Some(site_id);
                    }
                    schedule_voronoi(&ui, &state, "Influence changed — updating Coverage…");
                }
            });
        }
        install_document_spin_boundaries(&influence, state, CoalescedEdit::Influence(site_id));
        {
            let ui = ui.clone();
            let state = state.clone();
            reattach.connect_clicked(move |_| {
                let locked = state
                    .borrow()
                    .document
                    .as_ref()
                    .and_then(|document| document.recipe.voronoi.site(site_id))
                    .is_some_and(|site| site.locked);
                if locked {
                    ui.status
                        .set_label("Unlock this site before changing its Source position");
                    return;
                }
                let mut current = state.borrow_mut();
                current.selected_group = Some(site_id);
                current.selected_sample = Some(site_id);
                current.sampling_previous_mode = Some(current.mode);
                current.sampling = Some(SamplingState::AddSample);
                drop(current);
                ui.source_mode.set_active(true);
                ui.status.set_label(
                    "Click a visible source color to set Source and reset Target — Escape cancels",
                );
                ui.canvas.grab_focus();
            });
        }
        {
            let ui = ui.clone();
            let state = state.clone();
            sample_size.connect_selected_notify(move |control| {
                let Some(before) = creative_snapshot(&state.borrow()) else {
                    return;
                };
                let size = match control.selected() {
                    0 => SampleSize::Point,
                    2 => SampleSize::FiveByFive,
                    _ => SampleSize::ThreeByThree,
                };
                let changed = if let Some(document) = state.borrow_mut().document.as_mut() {
                    let source = document.source.clone();
                    let position = document
                        .recipe
                        .voronoi
                        .site(site_id)
                        .and_then(|site| (!site.locked).then_some(site.position).flatten());
                    if let Some(position) = position
                        && let Some(color) =
                            threshiator::voronoi::sample_color(&source, position, size)
                    {
                        document.recipe.voronoi.set_size(site_id, size, color)
                    } else {
                        false
                    }
                } else {
                    false
                };
                if changed {
                    record_creative_change(&ui, &state, before, None);
                    schedule_voronoi(
                        &ui,
                        &state,
                        "Footprint changed; Source and Target reset — updating preview…",
                    );
                }
            });
        }
        {
            let ui = ui.clone();
            let state = state.clone();
            delete.connect_clicked(move |_| {
                let dialog = adw::AlertDialog::builder()
                    .heading("Delete this site?")
                    .body("The site can be restored with document Undo.")
                    .build();
                dialog.add_response("cancel", "Cancel");
                dialog.add_response("delete", "Delete");
                dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
                let ui = ui.clone();
                let state = state.clone();
                glib::spawn_future_local(async move {
                    if dialog.choose_future(Some(&ui.window)).await != "delete" {
                        return;
                    }
                    let mut current = state.borrow_mut();
                    let Some(before) = creative_snapshot(&current) else {
                        return;
                    };
                    let mut selection = Some(site_id);
                    let deleted = current.document.as_mut().is_some_and(|document| {
                        if document.recipe.voronoi.delete_site(site_id) {
                            selection = document.recipe.voronoi.sites.first().map(|site| site.id);
                            true
                        } else {
                            false
                        }
                    });
                    current.selected_group = selection;
                    current.selected_sample = selection;
                    current.expanded_site = selection;
                    drop(current);
                    if deleted {
                        record_creative_change(&ui, &state, before, None);
                        schedule_voronoi(&ui, &state, "Site deleted — updating preview…");
                    } else {
                        ui.status.set_label("Unlock this site before deleting it");
                    }
                    refresh_voronoi_ui(&ui, &state);
                });
            });
        }
    }
}

fn canvas_sampling(ui: &Rc<Ui>, state: Rc<RefCell<State>>) {
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
                    .document
                    .as_ref()
                    .map_or(CanvasSiteAction::Ignore, |document| {
                        canvas_site_action(
                            document.recipe.active_method,
                            site_at_canvas_point(&ui_click.canvas, &current, x, y),
                        )
                    })
            };
            if let CanvasSiteAction::Select(site_id) = action {
                let mut current = state_click.borrow_mut();
                current.selected_group = Some(site_id);
                current.selected_sample = Some(site_id);
                if current.expanded_site.is_some() {
                    current.expanded_site = Some(site_id);
                }
                drop(current);
                refresh_voronoi_ui(&ui_click, &state_click);
                ui_click.canvas.queue_draw();
                return;
            }
            if action != CanvasSiteAction::Add {
                return;
            }
            if !add_site_from_artwork(&ui_click, &state_click, position) {
                ui_click
                    .status
                    .set_label("That area is fully transparent; choose a visible source color");
            }
            return;
        }
        let Some(before) = creative_snapshot(&state_click.borrow()) else {
            return;
        };
        let mut state = state_click.borrow_mut();
        let sampling = sampling.expect("sampling was checked above");
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
        let recipe = document.recipe.clone();
        state.selected_group = new_site;
        state.selected_sample = new_site;
        state.sampling = None;
        let previous_mode = state.sampling_previous_mode.take();
        if let Some(source) = state.preview_source.clone() {
            state.scheduler.schedule(source, recipe);
        }
        drop(state);
        let changed = record_creative_change(&ui_click, &state_click, before, None);
        restore_view(&ui_click, previous_mode);
        refresh_voronoi_ui(&ui_click, &state_click);
        ui_click.save.set_sensitive(
            state_click
                .borrow()
                .document
                .as_ref()
                .is_some_and(|document| document.dirty),
        );
        if !changed {
            return;
        }
        ui_click.status.set_label(match sampling {
            SamplingState::AddColor => "Site added — updating preview…",
            SamplingState::AddSample => "Source reattached; Target reset — updating preview…",
        });
        ui_click.canvas.queue_draw();
    });
    ui.canvas.add_controller(click);

    let drag_origin = Rc::new(RefCell::new(None::<[f64; 2]>));
    let drag_history = Rc::new(RefCell::new(None::<CreativeSnapshot>));
    let split_drag_origin = Rc::new(Cell::new(None::<f64>));
    let drag = gtk::GestureDrag::new();
    {
        let origin = drag_origin.clone();
        let history = drag_history.clone();
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
                        .document
                        .as_ref()?
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
                state.selected_group = Some(site_id);
                state.selected_sample = Some(site_id);
                if locked {
                    ui.status
                        .set_label("Unlock this site before moving its Source marker");
                } else {
                    *origin.borrow_mut() = Some(position);
                    *history.borrow_mut() = creative_snapshot(&state);
                }
            }
        });
    }
    {
        let origin = drag_origin.clone();
        let split_origin = split_drag_origin.clone();
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
            if resample_selected(&state, position) {
                ui.canvas.queue_draw();
            }
        });
    }
    {
        let origin = drag_origin;
        let history = drag_history;
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
            if origin.borrow_mut().take().is_some()
                && let Some(before) = history.borrow_mut().take()
                && record_creative_change(&ui, &state, before, None)
            {
                schedule_voronoi(&ui, &state, "Site moved; Target reset — updating preview…");
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
            let Some(before) = creative_snapshot(&state_key.borrow()) else {
                return glib::Propagation::Proceed;
            };
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
                record_creative_change(&ui_key, &state_key, before, None);
                schedule_voronoi(
                    &ui_key,
                    &state_key,
                    "Site nudged; Target reset — updating preview…",
                );
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
    threshiator::voronoi::reattach_site(
        &mut document.recipe.voronoi,
        &document.source,
        sample.id,
        position,
    )
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
    let document = state.document.as_ref()?;
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

fn marker_hit_test(dx: f64, dy: f64) -> bool {
    dx.powi(2) + dy.powi(2) <= 14.0_f64.powi(2)
}

fn divider_from_canvas_x(x: f64, canvas_width: i32) -> f64 {
    if canvas_width <= 0 {
        0.5
    } else {
        (x / canvas_width as f64).clamp(0.0, 1.0)
    }
}

fn split_divider_hit(x: f64, canvas_width: i32, divider: f64) -> bool {
    canvas_width > 0 && (x - canvas_width as f64 * divider).abs() <= 12.0
}

fn show_voronoi_markers(method: Method) -> bool {
    method == Method::Voronoi
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CanvasSiteAction {
    Ignore,
    Select(u64),
    Add,
}

fn canvas_site_action(method: Method, hit: Option<u64>) -> CanvasSiteAction {
    if method != Method::Voronoi {
        CanvasSiteAction::Ignore
    } else if let Some(site_id) = hit {
        CanvasSiteAction::Select(site_id)
    } else {
        CanvasSiteAction::Add
    }
}

fn site_label(index: usize) -> String {
    format!("Site {}", index + 1)
}

fn accessible_site_label(index: usize) -> String {
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
    scales: [gtk::Scale; 3],
    wheel: gtk::DrawingArea,
    hex: gtk::Entry,
    new_swatch: gtk::DrawingArea,
    syncing: Rc<Cell<bool>>,
    model: Rc<Cell<ColorModel>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PickerGesture {
    Wheel,
    Channel(usize),
}

#[derive(Default)]
struct PickerLocalHistory {
    undo: Vec<DraftColor>,
    redo: Vec<DraftColor>,
    active: Option<PickerGesture>,
}

impl PickerLocalHistory {
    fn record(
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

    fn finish(&mut self, gesture: PickerGesture) {
        if self.active == Some(gesture) {
            self.active = None;
        }
    }

    fn step(&mut self, current: DraftColor, redo: bool) -> Option<DraftColor> {
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

fn picker_values(controls: &PickerControls) -> [f64; 3] {
    let mut values = controls
        .adjustments
        .clone()
        .map(|adjustment| adjustment.value());
    values[1] /= 100.0;
    values[2] /= 100.0;
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
        ColorModel::Okhsl => (
            ["Hue", "Saturation", "Lightness"],
            [(0.0, 360.0), (0.0, 100.0), (0.0, 100.0)],
            [2, 2, 2],
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
        controls.adjustments[index].set_step_increment(0.1);
        controls.spins[index].set_digits(digits[index]);
        controls.spins[index].update_property(&[gtk::accessible::Property::Label(names[index])]);
    }
}

fn refresh_picker_outputs(controls: &PickerControls, draft: DraftColor) {
    let model = controls.model.get();
    let (_, _, digits) = picker_model_spec(model);
    let mut values = draft.values(model);
    values[1] *= 100.0;
    values[2] *= 100.0;
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
    values[1] *= 100.0;
    values[2] *= 100.0;
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
    if distance > 1e-9 {
        current[0] = hue;
    }
    current[1] = distance;
    draft.borrow_mut().set_values(model, current);
    sync_picker(controls, *draft.borrow());
}

fn present_color_picker(
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
        PickerPurpose::Source => "Choose Source Center",
        PickerPurpose::Target => "Choose Target Color",
    };
    let dialog = adw::Window::builder()
        .title(picker_title)
        .transient_for(&ui.window)
        .modal(true)
        .destroy_with_parent(true)
        .default_width(680)
        .default_height(540)
        .build();
    *ui.picker_dialog.borrow_mut() = Some(dialog.clone());
    let model_dropdown = gtk::DropDown::from_strings(&["HSV", "HSL", "OKHSL"]);
    *ui.audit_picker_model.borrow_mut() = Some(model_dropdown.clone());
    model_dropdown.set_selected(match initial_model {
        ColorModel::Hsv => 0,
        ColorModel::Hsl => 1,
        ColorModel::Okhsl => 2,
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
        spin.set_width_chars(11);
        spin
    });
    let scales = std::array::from_fn(|index| {
        let scale = gtk::Scale::new(gtk::Orientation::Horizontal, Some(&adjustments[index]));
        scale.set_draw_value(false);
        scale.set_hexpand(true);
        scale
    });
    let controls_box = gtk::Box::new(gtk::Orientation::Vertical, 12);
    controls_box.set_width_request(280);
    controls_box.append(&model_dropdown);
    for index in 0..3 {
        let row = gtk::Box::new(gtk::Orientation::Vertical, 4);
        row.append(&labels[index]);
        let linked = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        linked.append(&scales[index]);
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
    let header = adw::HeaderBar::new();
    header.pack_start(&local_undo);
    header.pack_start(&local_redo);
    let dialog_title = adw::WindowTitle::new(picker_title, "Changes in this dialog");
    header.set_title_widget(Some(&dialog_title));
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&content));
    dialog.set_content(Some(&toolbar));
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
            if model == ColorModel::Okhsl {
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
        let dialog_title = dialog_title.clone();
        move |redo| {
            let current = *draft.borrow();
            let Some(target) = picker_history.borrow_mut().step(current, redo) else {
                dialog_title.set_subtitle(if redo {
                    "Nothing to redo locally"
                } else {
                    "Nothing to undo locally"
                });
                return;
            };
            *draft.borrow_mut() = target;
            sync_picker(&controls, target);
            dialog_title.set_subtitle(if redo {
                "Color edit redone locally"
            } else {
                "Color edit undone locally"
            });
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
            let site_id = state.borrow().selected_sample;
            let before = creative_snapshot(&state.borrow());
            let selected = draft.borrow().commit();
            let changed = state
                .borrow_mut()
                .document
                .as_mut()
                .is_some_and(|document| {
                    let Some(id) = site_id else { return false };
                    match purpose {
                        PickerPurpose::Source => {
                            let Some(site) = document.recipe.voronoi.site(id) else {
                                return false;
                            };
                            if site.source_color[..3] == selected {
                                return false;
                            }
                            let alpha = site.source_color[3];
                            document.recipe.voronoi.set_source(
                                id,
                                [selected[0], selected[1], selected[2], alpha],
                                None,
                            )
                        }
                        PickerPurpose::Target => {
                            if document
                                .recipe
                                .voronoi
                                .site(id)
                                .is_some_and(|site| site.target_color == selected)
                            {
                                false
                            } else {
                                document.recipe.voronoi.set_target(id, selected)
                            }
                        }
                    }
                });
            dialog.close();
            refresh_voronoi_ui(&ui, &state);
            if changed {
                if let Some(before) = before {
                    record_creative_change(&ui, &state, before, None);
                }
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

fn picker_plane_physical_size(width: i32, height: i32, scale: i32) -> (i32, i32) {
    let scale = scale.max(1);
    (width.max(1) * scale, height.max(1) * scale)
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

fn picker_plane_encoded_sample(
    model: ColorModel,
    fixed_axis: f64,
    x: f64,
    y: f64,
) -> Option<[f64; 3]> {
    let distance = x.hypot(y);
    if distance > 1.0 + 1.0e-9 {
        return None;
    }
    let distance = distance.min(1.0);
    let hue = y.atan2(x).to_degrees().rem_euclid(360.0);
    Some(match model {
        ColorModel::Hsv => {
            threshiator::color::hsv_to_encoded([hue, distance, fixed_axis.clamp(0.0, 1.0)])
        }
        ColorModel::Hsl => {
            threshiator::color::hsl_to_encoded([hue, distance, fixed_axis.clamp(0.0, 1.0)])
        }
        ColorModel::Okhsl => linear_to_encoded(
            okhsl_to_linear([hue / 360.0, distance, fixed_axis.clamp(0.0, 1.0)])
                .expect("the normalized OKHSL plane is completely inside bounded sRGB"),
        ),
    })
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
            let (sample_x, sample_y) = if distance > 1.0 {
                (x / distance, y / distance)
            } else {
                (x, y)
            };
            let encoded = if let Some(sampler) = &okhsl_sampler {
                sampler.encoded(sample_y.atan2(sample_x).to_degrees(), sample_distance)
            } else {
                picker_plane_encoded_sample(model, fixed_axis, sample_x, sample_y)
                    .expect("projected inside HSV/HSL picker disk")
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
    ctx.arc(
        width as f64 / 2.0,
        height as f64 / 2.0,
        radius,
        0.0,
        std::f64::consts::TAU,
    );
    ctx.clip();
    ctx.scale(
        f64::from(width) / f64::from(surface.width()),
        f64::from(height) / f64::from(surface.height()),
    );
    let _ = ctx.set_source_surface(&surface, 0.0, 0.0);
    let _ = ctx.paint();
    let _ = ctx.restore();
    let radians = fixed[0].to_radians();
    let marker = [radians.cos() * fixed[1], radians.sin() * fixed[1]];
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
                ctx.arc(x, h as f64 / 2.0, 9.0, 0.0, std::f64::consts::TAU);
                ctx.set_source_rgba(0.12, 0.12, 0.14, 0.92);
                let _ = ctx.fill_preserve();
                ctx.set_source_rgba(1.0, 1.0, 1.0, 0.95);
                ctx.set_line_width(2.0);
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
            let capture_widget: gtk::Widget = if state.borrow().picker_visible {
                ui.picker_dialog.borrow().as_ref().map_or_else(
                    || ui.window.clone().upcast(),
                    |dialog| dialog.clone().upcast(),
                )
            } else if state.borrow().threshold_editor_visible {
                ui.threshold_editor_dialog.borrow().as_ref().map_or_else(
                    || ui.window.clone().upcast(),
                    |dialog| dialog.clone().upcast(),
                )
            } else if ui.cli.show_preset_dialog {
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
                    VoronoiMatching::Okhsl => "okhsl",
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
                "model": match state.picker_model { ColorModel::Hsv => "hsv", ColorModel::Hsl => "hsl", ColorModel::Okhsl => "okhsl" },
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
                "color_model": match ui.cli.color_model { ColorModel::Hsv => "hsv", ColorModel::Hsl => "hsl", ColorModel::Okhsl => "okhsl" },
                "picker_lightness": ui.cli.picker_lightness,
                "threshold_space": ui.cli.threshold_space.map(|space| match space { ThresholdSpace::Rgb => "rgb", ThresholdSpace::Hsv => "hsv" }),
                "threshold_link": ui.cli.threshold_link.map(|link| link == LinkPolicy::Linked),
                "threshold_component": ui.cli.threshold_component.map(|component| format!("{component:?}").to_lowercase()),
                "threshold_bands": ui.cli.threshold_bands,
                "threshold_bypass": ui.cli.threshold_bypass.map(|component| format!("{component:?}").to_lowercase()),
                "threshold_lock": ui.cli.threshold_lock.map(|component| format!("{component:?}").to_lowercase()),
                "threshold_auto": ui.cli.threshold_auto.map(|component| format!("{component:?}").to_lowercase()),
                "voronoi_matching": ui.cli.voronoi_matching.map(|matching| match matching { VoronoiMatching::Perceptual => "perceptual", VoronoiMatching::Okhsl => "okhsl", VoronoiMatching::Rgb => "rgb", VoronoiMatching::Hsv => "hsv" }),
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
    canvas: &gtk::DrawingArea,
    state: Rc<RefCell<State>>,
) {
    let c = canvas.clone();
    button.connect_toggled(move |b| {
        if b.is_active() {
            state.borrow_mut().mode = mode;
            c.set_cursor_from_name(None);
            c.set_tooltip_text(Some(if mode == CompareMode::Split {
                "Drag the divider or use Left/Right to move the split"
            } else {
                "Processed image comparison surface"
            }));
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
    ui.document_menu.set_sensitive(state.document.is_some());
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
                            threshold_histograms,
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
                            let mut s = state.borrow_mut();
                            s.source_pixbuf = Some(pixbuf(source_display));
                            s.result_pixbuf = Some(pixbuf(result_display));
                            s.result = None;
                            s.preview_source = Some(preview_source);
                            s.threshold_histograms = Some(threshold_histograms);
                            s.coverage = coverage;
                            s.selected_group = d.recipe.voronoi.sites.first().map(|site| site.id);
                            s.selected_sample = s.selected_group;
                            s.expanded_site = None;
                            let method = d.recipe.active_method;
                            s.creative_history.initialize(&d.recipe);
                            s.document = Some(d);
                            s.coalesced_edit = None;
                            s.project_path = path;
                            s.document_kind = document_kind;
                            drop(s);
                            ui.method_voronoi.set_active(method == Method::Voronoi);
                            ui.method_thresholds
                                .set_active(method == Method::Thresholds);
                            ui.voronoi_panel.set_visible(method == Method::Voronoi);
                            ui.voronoi_matching_row
                                .set_visible(method == Method::Voronoi);
                            ui.thresholds_panel
                                .set_visible(method == Method::Thresholds);
                            ui.method_section.set_label(Some("Method"));
                            ui.pipeline_status.set_label(&format!(
                                "32-bit float · Linear sRGB · {pipeline_profile}"
                            ));
                            ui.pipeline_status.set_tooltip_text(Some(&format!(
                                "Straight-alpha linear-sRGB RGBA f32; {profile}; preview quantizes once for display"
                            )));
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
                                    s.expanded_site = site;
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
                            sync_threshold_ui(&ui, &state);
                            ui.stack.set_visible_child_name("document");
                            ui.save.set_sensitive(false);
                            ui.save_as.set_sensitive(true);
                            ui.export.set_sensitive(true);
                            ui.document_menu.set_sensitive(true);
                            sync_contextual_chrome(&ui, &state);
                            sync_document_history_ui(&ui, &state);
                            match ui.cli.sampling {
                                Some(SamplingState::AddColor) => ui.status.set_label("Click a visible source color to create a site — Escape cancels"),
                                Some(SamplingState::AddSample) => ui.status.set_label("Click a visible source color to reattach Source and reset Target — Escape cancels"),
                                None => ui.status.set_label("Ready"),
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
                                let component_index =
                                    ui.cli.threshold_component.and_then(|component| {
                                        state.borrow().document.as_ref().and_then(|document| {
                                            threshold_component_index(
                                                document.recipe.threshold.active_space,
                                                component,
                                            )
                                        })
                                    });
                                if let Some(index) = component_index {
                                    ui.threshold_component_rows[index]
                                        .edit
                                        .emit_by_name::<()>("activated", &[]);
                                } else {
                                    present_threshold_editor(&ui, &state);
                                }
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

fn viewer_toolbar_geometry(ui: &Ui, method: &str) -> (bool, serde_json::Value) {
    let method_active = match method {
        "thresholds" => ui.method_thresholds.is_active(),
        "voronoi" => ui.method_voronoi.is_active(),
        _ => false,
    };
    let controls_visible = [
        ui.result_mode.clone().upcast::<gtk::Widget>(),
        ui.split_mode.clone().upcast(),
        ui.source_mode.clone().upcast(),
        ui.smoothing_label.clone().upcast(),
        ui.smoothing.clone().upcast(),
    ]
    .iter()
    .all(|control| control.is_visible() && control.is_mapped());
    let controls = [
        ("result", ui.result_mode.clone().upcast::<gtk::Widget>()),
        ("split", ui.split_mode.clone().upcast()),
        ("source", ui.source_mode.clone().upcast()),
        ("smoothing-label", ui.smoothing_label.clone().upcast()),
        ("smoothing", ui.smoothing.clone().upcast()),
    ];
    let bounds = controls
        .iter()
        .filter_map(|(name, control)| {
            control
                .compute_bounds(&ui.snapshot_root)
                .map(|bounds| (*name, bounds))
        })
        .collect::<Vec<_>>();
    let sidebar_right = ui
        .inspector_split
        .sidebar()
        .and_then(|sidebar| sidebar.compute_bounds(&ui.snapshot_root))
        .map(|bounds| bounds.x() + bounds.width());
    let all_measured = bounds.len() == controls.len();
    let clear_of_sidebar =
        sidebar_right.is_some_and(|edge| bounds.iter().all(|(_, bounds)| bounds.x() + 0.5 >= edge));
    let mutually_non_overlapping = all_measured
        && bounds.iter().enumerate().all(|(index, (_, first))| {
            bounds.iter().skip(index + 1).all(|(_, second)| {
                let horizontal_overlap = (first.x() + first.width())
                    .min(second.x() + second.width())
                    - first.x().max(second.x());
                let vertical_overlap = (first.y() + first.height())
                    .min(second.y() + second.height())
                    - first.y().max(second.y());
                horizontal_overlap <= 1.5 || vertical_overlap <= 0.5
            })
        });
    let inside_window = all_measured
        && bounds.iter().all(|(_, bounds)| {
            bounds.x() >= 0.0
                && bounds.y() >= 0.0
                && bounds.x() + bounds.width() <= ui.snapshot_root.width() as f32
                && bounds.y() + bounds.height() <= ui.snapshot_root.height() as f32
        });
    let mode_bottom = bounds
        .iter()
        .take(3)
        .map(|(_, bounds)| bounds.y() + bounds.height())
        .fold(f32::NEG_INFINITY, f32::max);
    let smoothing_top = bounds
        .iter()
        .skip(3)
        .map(|(_, bounds)| bounds.y())
        .fold(f32::INFINITY, f32::min);
    let rows_separate = all_measured && mode_bottom <= smoothing_top + 0.5;
    let row_ends_aligned = all_measured
        && (bounds[2].1.x() + bounds[2].1.width() - (bounds[4].1.x() + bounds[4].1.width())).abs()
            <= 1.0;
    let geometry = bounds
        .iter()
        .map(|(name, bounds)| {
            serde_json::json!({
                "name": name,
                "x": bounds.x(),
                "y": bounds.y(),
                "width": bounds.width(),
                "height": bounds.height(),
            })
        })
        .collect::<Vec<_>>();
    let evidence = serde_json::json!({
        "method": method,
        "method_active": method_active,
        "controls_visible": controls_visible,
        "all_measured": all_measured,
        "clear_of_sidebar": clear_of_sidebar,
        "mutually_non_overlapping": mutually_non_overlapping,
        "inside_window": inside_window,
        "rows_separate": rows_separate,
        "row_ends_aligned": row_ends_aligned,
        "sidebar_right": sidebar_right,
        "controls": geometry,
        "viewer_measure": ui.viewer_bar.measure(gtk::Orientation::Horizontal, -1),
    });
    (
        method_active
            && controls_visible
            && clear_of_sidebar
            && mutually_non_overlapping
            && inside_window
            && rows_separate
            && row_ends_aligned,
        evidence,
    )
}

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
        "shell" | "narrow-core" | "adaptive-1024" => {
            let ui_check = ui.clone();
            let state_check = state.clone();
            let log_check = log.clone();
            steps.push(step(
                "document-history",
                "assert-clean-open-controls",
                serde_json::Value::Null,
                Box::new(move || {
                    let clean = state_check
                        .borrow()
                        .document
                        .as_ref()
                        .is_some_and(|document| !document.dirty);
                    if !clean || ui_check.undo.is_sensitive() || ui_check.redo.is_sensitive() {
                        log_check.event(
                            "fail",
                            "document-history",
                            "clean-open-controls",
                            serde_json::json!({"clean": clean}),
                        );
                    }
                }),
            ));
            if scenario == "adaptive-1024" {
                let ui_adaptive = ui.clone();
                let log_adaptive = log.clone();
                steps.push(step(
                    "adaptive-layout",
                    "assert-1024x600",
                    serde_json::Value::Null,
                    Box::new(move || {
                        let size = (ui_adaptive.window.width(), ui_adaptive.window.height());
                        let shell_measure = ui_adaptive
                            .snapshot_root
                            .measure(gtk::Orientation::Horizontal, -1);
                        let split_measure = ui_adaptive
                            .inspector_split
                            .measure(gtk::Orientation::Horizontal, -1);
                        let viewer_measure = ui_adaptive
                            .viewer_bar
                            .measure(gtk::Orientation::Horizontal, -1);
                        let status_measure = ui_adaptive
                            .status_bar
                            .measure(gtk::Orientation::Horizontal, -1);
                        log_adaptive.event(
                            "evidence",
                            "adaptive-layout",
                            "horizontal-measures",
                            serde_json::json!({
                                "shell": shell_measure,
                                "split": split_measure,
                                "viewer": viewer_measure,
                                "status": status_measure,
                            }),
                        );
                        let controls_visible = [
                            &ui_adaptive.result_mode,
                            &ui_adaptive.split_mode,
                            &ui_adaptive.source_mode,
                        ]
                        .iter()
                        .all(|control| control.is_visible() && control.is_mapped())
                            && ui_adaptive.smoothing.is_visible()
                            && ui_adaptive.smoothing.is_mapped();
                        let focus_ready = ui_adaptive.canvas.is_focusable()
                            && ui_adaptive.canvas.has_css_class(CREATIVE_FOCUS_CLASS);
                        if size != (1024, 600)
                            || !controls_visible
                            || !focus_ready
                            || !ui_adaptive.document_menu.is_visible()
                            || !ui_adaptive.status_bar.is_visible()
                        {
                            log_adaptive.event(
                                "fail",
                                "adaptive-layout",
                                "1024x600",
                                serde_json::json!({
                                    "size": size,
                                    "controls_visible": controls_visible,
                                    "focus_ready": focus_ready,
                                    "viewer_measure": viewer_measure,
                                }),
                            );
                        }
                    }),
                ));
            }
            for (name, section) in [
                ("method", ui.method_section.clone()),
                ("voronoi", ui.voronoi_panel.clone()),
                ("thresholds", ui.thresholds_panel.clone()),
            ] {
                for expanded in [false, true] {
                    let section = section.clone();
                    steps.push(step(
                        "sidebar-section",
                        "set-expanded",
                        serde_json::json!({"section": name, "expanded": expanded}),
                        Box::new(move || section.set_expanded(expanded)),
                    ));
                }
            }
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
            let ui_savepoint = ui.clone();
            let state_savepoint = state.clone();
            let log_savepoint = log.clone();
            steps.push(step(
                "document-history",
                "mark-savepoint-and-preserve-history",
                serde_json::Value::Null,
                Box::new(move || {
                    mark_document_saved(&mut state_savepoint.borrow_mut());
                    ui_savepoint.save.set_sensitive(false);
                    sync_document_history_ui(&ui_savepoint, &state_savepoint);
                    if !ui_savepoint.undo.is_sensitive()
                        || ui_savepoint.redo.is_sensitive()
                        || ui_savepoint.save.is_sensitive()
                    {
                        log_savepoint.event(
                            "fail",
                            "document-history",
                            "savepoint-controls",
                            serde_json::json!(false),
                        );
                    }
                }),
            ));
            let ui_undo = ui.clone();
            let state_undo = state.clone();
            steps.push(step(
                "document-history",
                "undo-away-from-savepoint",
                serde_json::Value::Null,
                Box::new(move || creative_history_step(&ui_undo, &state_undo, false)),
            ));
            let ui_check = ui.clone();
            let state_check = state.clone();
            let log_check = log.clone();
            steps.push(step(
                "document-history",
                "assert-dirty-away-from-savepoint",
                serde_json::Value::Null,
                Box::new(move || {
                    let dirty = state_check
                        .borrow()
                        .document
                        .as_ref()
                        .is_some_and(|document| document.dirty);
                    if !dirty || !ui_check.save.is_sensitive() || !ui_check.redo.is_sensitive() {
                        log_check.event(
                            "fail",
                            "document-history",
                            "undo-savepoint-state",
                            serde_json::json!({"dirty": dirty}),
                        );
                    }
                }),
            ));
            let ui_redo = ui.clone();
            let state_redo = state.clone();
            steps.push(step(
                "document-history",
                "redo-to-savepoint",
                serde_json::Value::Null,
                Box::new(move || creative_history_step(&ui_redo, &state_redo, true)),
            ));
            let ui_check = ui.clone();
            let state_check = state.clone();
            let log_check = log.clone();
            steps.push(step(
                "document-history",
                "assert-clean-at-savepoint",
                serde_json::Value::Null,
                Box::new(move || {
                    let clean = state_check
                        .borrow()
                        .document
                        .as_ref()
                        .is_some_and(|document| !document.dirty);
                    if !clean || ui_check.save.is_sensitive() || ui_check.redo.is_sensitive() {
                        log_check.event(
                            "fail",
                            "document-history",
                            "redo-savepoint-state",
                            serde_json::json!({"clean": clean}),
                        );
                    }
                }),
            ));
        }
        "responsive-thresholds-720"
        | "responsive-voronoi-720"
        | "responsive-thresholds-1024"
        | "responsive-voronoi-1024" => {
            let method = if scenario.contains("thresholds") {
                "thresholds"
            } else {
                "voronoi"
            };
            let sidebar = ui.sidebar_button.clone();
            steps.push(step(
                "adaptive-layout",
                "close-inspector",
                serde_json::Value::Null,
                Box::new(move || sidebar.set_active(false)),
            ));
            for _ in 0..5 {
                steps.push(step(
                    "adaptive-layout",
                    "settle-closed-inspector",
                    serde_json::Value::Null,
                    Box::new(|| {}),
                ));
            }
            let sidebar = ui.sidebar_button.clone();
            steps.push(step(
                "adaptive-layout",
                "open-inspector",
                serde_json::Value::Null,
                Box::new(move || sidebar.set_active(true)),
            ));
            for _ in 0..10 {
                steps.push(step(
                    "adaptive-layout",
                    "settle-open-inspector",
                    serde_json::Value::Null,
                    Box::new(|| {}),
                ));
            }
            let ui_geometry = ui.clone();
            let log_geometry = log.clone();
            steps.push(step(
                "adaptive-layout",
                "assert-toolbar-geometry",
                serde_json::json!(method),
                Box::new(move || {
                    let (passes, geometry_evidence) = viewer_toolbar_geometry(&ui_geometry, method);
                    log_geometry.event(
                        "evidence",
                        "adaptive-layout",
                        "toolbar-geometry",
                        geometry_evidence.clone(),
                    );
                    if !passes {
                        log_geometry.event(
                            "fail",
                            "adaptive-layout",
                            "toolbar-geometry",
                            geometry_evidence,
                        );
                    }
                }),
            ));
        }
        "threshold-inspector" => {
            let method = ui.method_thresholds.clone();
            steps.push(step(
                "method",
                "set-active",
                serde_json::json!("thresholds"),
                Box::new(move || method.set_active(true)),
            ));
            let ui_setup = ui.clone();
            let state_setup = state.clone();
            steps.push(step(
                "threshold-inspector",
                "establish-rgb-fixture",
                serde_json::Value::Null,
                Box::new(move || {
                    let mut state = state_setup.borrow_mut();
                    let Some(document) = state.document.as_mut() else {
                        return;
                    };
                    document.recipe.threshold = ThresholdState::default();
                    document.recipe.threshold.active_space = ThresholdSpace::Rgb;
                    document.recipe.threshold.rgb_state.link = LinkPolicy::Independent;
                    document.dirty = false;
                    let recipe = document.recipe.clone();
                    state.creative_history.initialize(&recipe);
                    state.coalesced_edit = None;
                    drop(state);
                    sync_threshold_ui(&ui_setup, &state_setup);
                    sync_document_history_ui(&ui_setup, &state_setup);
                }),
            ));
            let ui_summary = ui.clone();
            let log_summary = log.clone();
            steps.push(step(
                "threshold-inspector",
                "assert-rgb-hierarchy",
                serde_json::Value::Null,
                Box::new(move || {
                    let valid = ui_summary.threshold_space.is_sensitive()
                        && ui_summary.threshold_space.is_focusable()
                        && ui_summary.threshold_link.title() == "Link RGB mappings"
                        && ui_summary.threshold_link.subtitle().as_deref()
                            == Some("Future mapping edits copy to unlocked peers; existing differences remain; Process remains per-component.")
                        && !ui_summary.threshold_link.is_active()
                        && !ui_summary.threshold_link_editor.is_visible()
                        && ui_summary.threshold_link_editor.title()
                            == "Edit linked RGB mapping…"
                        && ui_summary.threshold_component_rows[0].row.title() == "Red mapping"
                        && ui_summary.threshold_component_rows[1].row.title() == "Green mapping"
                        && ui_summary.threshold_component_rows[2].row.title() == "Blue mapping"
                        && ui_summary.threshold_component_rows[0].edit.title()
                            == "Edit Red band mapping…"
                        && ui_summary.threshold_component_rows[1].edit.title()
                            == "Edit Green band mapping…"
                        && ui_summary.threshold_component_rows[2].edit.title()
                            == "Edit Blue band mapping…"
                        && ui_summary.threshold_component_rows.iter().all(|controls| {
                            controls.row.subtitle() == "3 bands · Processing · Unlocked"
                                && controls.process.is_sensitive()
                                && controls.auto_row.title() == "Automatic baseline"
                                && controls.auto_row.subtitle().as_deref()
                                    == Some("Even bands · levels from 0 to 1")
                                && controls.auto.label().as_deref() == Some("Auto")
                                && controls.auto.is_visible()
                                && controls.auto.is_focusable()
                                && controls.auto.is_sensitive()
                                && controls.edit.is_sensitive()
                        });
                    if !valid {
                        log_summary.event(
                            "fail",
                            "threshold-inspector",
                            "rgb-hierarchy",
                            serde_json::json!(false),
                        );
                    }
                }),
            ));
            let rows = ui.threshold_component_rows.clone();
            steps.push(step(
                "component-cards",
                "expand-red-then-green",
                serde_json::Value::Null,
                Box::new(move || {
                    rows[0].row.set_expanded(true);
                    rows[1].row.set_expanded(true);
                }),
            ));
            let ui_single = ui.clone();
            let log_single = log.clone();
            steps.push(step(
                "component-cards",
                "assert-single-open",
                serde_json::Value::Null,
                Box::new(move || {
                    if ui_single.threshold_component_rows[0].row.is_expanded()
                        || !ui_single.threshold_component_rows[1].row.is_expanded()
                        || ui_single.threshold_component_rows[2].row.is_expanded()
                    {
                        log_single.event(
                            "fail",
                            "component-cards",
                            "single-open",
                            serde_json::json!(false),
                        );
                    }
                }),
            ));
            let process_generation = Rc::new(Cell::new(0_u64));
            let process_history = Rc::new(Cell::new(0_usize));
            let state_capture = state.clone();
            let generation_capture = process_generation.clone();
            let history_capture = process_history.clone();
            steps.push(step(
                "process-mapping",
                "capture-before-red-bypass",
                serde_json::Value::Null,
                Box::new(move || {
                    let state = state_capture.borrow();
                    generation_capture.set(state.scheduler.current_generation());
                    history_capture.set(state.creative_history.undo.len());
                }),
            ));
            let process = ui.threshold_component_rows[0].process.clone();
            steps.push(step(
                "process-mapping",
                "bypass-red",
                serde_json::json!(false),
                Box::new(move || process.set_active(false)),
            ));
            let state_check = state.clone();
            let ui_check = ui.clone();
            let log_check = log.clone();
            let generation_check = process_generation.clone();
            let history_check = process_history.clone();
            steps.push(step(
                "process-mapping",
                "assert-component-only-preview-history",
                serde_json::Value::Null,
                Box::new(move || {
                    let state = state_check.borrow();
                    let valid = state.document.as_ref().is_some_and(|document| {
                        let components = &document.recipe.threshold.rgb_state.components;
                        !components[0].enabled && components[1].enabled && components[2].enabled
                    }) && state.scheduler.current_generation()
                        == generation_check.get() + 1
                        && state.creative_history.undo.len() == history_check.get() + 1
                        && ui_check.threshold_component_rows[0].row.subtitle()
                            == "3 bands · Bypassed · Unlocked";
                    if !valid {
                        log_check.event(
                            "fail",
                            "process-mapping",
                            "component-only-preview-history",
                            serde_json::json!(false),
                        );
                    }
                }),
            ));
            let lock_generation = Rc::new(Cell::new(0_u64));
            let lock_history = Rc::new(Cell::new(0_usize));
            let state_capture = state.clone();
            let generation_capture = lock_generation.clone();
            let history_capture = lock_history.clone();
            steps.push(step(
                "lock-mapping",
                "capture-before-blue-lock",
                serde_json::Value::Null,
                Box::new(move || {
                    let state = state_capture.borrow();
                    generation_capture.set(state.scheduler.current_generation());
                    history_capture.set(state.creative_history.undo.len());
                }),
            ));
            let lock = ui.threshold_component_rows[2].lock.clone();
            steps.push(step(
                "lock-mapping",
                "lock-blue",
                serde_json::json!(true),
                Box::new(move || lock.set_active(true)),
            ));
            let state_check = state.clone();
            let ui_check = ui.clone();
            let log_check = log.clone();
            let generation_check = lock_generation.clone();
            let history_check = lock_history.clone();
            steps.push(step(
                "lock-mapping",
                "assert-gating-without-preview",
                serde_json::Value::Null,
                Box::new(move || {
                    let state = state_check.borrow();
                    let blue = &ui_check.threshold_component_rows[2];
                    let valid = state
                        .document
                        .as_ref()
                        .is_some_and(|document| document.recipe.threshold.rgb_state.locks[2])
                        && state.scheduler.current_generation() == generation_check.get()
                        && state.creative_history.undo.len() == history_check.get() + 1
                        && !blue.bands.is_sensitive()
                        && !blue.auto.is_sensitive()
                        && blue.process.is_sensitive()
                        && blue.edit.is_sensitive()
                        && blue.row.subtitle() == "3 bands · Processing · Locked";
                    if !valid {
                        log_check.event(
                            "fail",
                            "lock-mapping",
                            "gating-without-preview",
                            serde_json::json!(false),
                        );
                    }
                }),
            ));
            let bands = ui.threshold_component_rows[0].bands.clone();
            steps.push(step(
                "bands",
                "establish-independent-red-mapping",
                serde_json::json!(5),
                Box::new(move || bands.set_value(5.0)),
            ));
            let link_generation = Rc::new(Cell::new(0_u64));
            let link_history = Rc::new(Cell::new(0_usize));
            let link_before = Rc::new(RefCell::new(None::<[ComponentQuantizer; 3]>));
            let state_capture = state.clone();
            let generation_capture = link_generation.clone();
            let history_capture = link_history.clone();
            let before_capture = link_before.clone();
            steps.push(step(
                "link-mappings",
                "capture-existing-differences",
                serde_json::Value::Null,
                Box::new(move || {
                    let state = state_capture.borrow();
                    generation_capture.set(state.scheduler.current_generation());
                    history_capture.set(state.creative_history.undo.len());
                    *before_capture.borrow_mut() = state
                        .document
                        .as_ref()
                        .map(|document| document.recipe.threshold.rgb_state.components.clone());
                }),
            ));
            let link = ui.threshold_link.clone();
            steps.push(step(
                "link-mappings",
                "enable-future-link",
                serde_json::json!(true),
                Box::new(move || link.set_active(true)),
            ));
            let state_check = state.clone();
            let ui_check = ui.clone();
            let log_check = log.clone();
            let generation_check = link_generation.clone();
            let history_check = link_history.clone();
            let before_check = link_before.clone();
            steps.push(step(
                "link-mappings",
                "assert-preserves-differences-without-preview",
                serde_json::Value::Null,
                Box::new(move || {
                    let state = state_check.borrow();
                    let valid = state.document.as_ref().is_some_and(|document| {
                        document.recipe.threshold.rgb_state.link == LinkPolicy::Linked
                            && before_check.borrow().as_ref()
                                == Some(&document.recipe.threshold.rgb_state.components)
                    }) && state.scheduler.current_generation()
                        == generation_check.get()
                        && state.creative_history.undo.len() == history_check.get() + 1
                        && ui_check.threshold_link_editor.is_visible();
                    if !valid {
                        log_check.event(
                            "fail",
                            "link-mappings",
                            "preserve-differences-without-preview",
                            serde_json::json!(false),
                        );
                    }
                }),
            ));
            let bands_generation = Rc::new(Cell::new(0_u64));
            let bands_history = Rc::new(Cell::new(0_usize));
            let state_capture = state.clone();
            let generation_capture = bands_generation.clone();
            let history_capture = bands_history.clone();
            steps.push(step(
                "bands",
                "capture-before-linked-resize",
                serde_json::Value::Null,
                Box::new(move || {
                    let state = state_capture.borrow();
                    generation_capture.set(state.scheduler.current_generation());
                    history_capture.set(state.creative_history.undo.len());
                }),
            ));
            for value in [6.0, 7.0] {
                let bands = ui.threshold_component_rows[0].bands.clone();
                steps.push(step(
                    "bands",
                    "resize-linked-red",
                    serde_json::json!(value),
                    Box::new(move || bands.set_value(value)),
                ));
            }
            let state_check = state.clone();
            let log_check = log.clone();
            let generation_check = bands_generation.clone();
            let history_check = bands_history.clone();
            steps.push(step(
                "bands",
                "assert-propagation-lock-process-and-coalescing",
                serde_json::Value::Null,
                Box::new(move || {
                    let state = state_check.borrow();
                    let valid = state.document.as_ref().is_some_and(|document| {
                        let rgb = &document.recipe.threshold.rgb_state;
                        rgb.components[0].outputs.len() == 7
                            && rgb.components[1].outputs.len() == 7
                            && rgb.components[2].outputs.len() == 3
                            && !rgb.components[0].enabled
                            && rgb.components[1].enabled
                            && rgb.components[2].enabled
                    }) && state.scheduler.current_generation()
                        == generation_check.get() + 2
                        && state.creative_history.undo.len() == history_check.get() + 1;
                    if !valid {
                        log_check.event(
                            "fail",
                            "bands",
                            "propagation-lock-process-coalescing",
                            serde_json::json!(false),
                        );
                    }
                }),
            ));
            let auto_generation = Rc::new(Cell::new(0_u64));
            let auto_history = Rc::new(Cell::new(0_usize));
            let auto_before = Rc::new(RefCell::new(None::<[ComponentQuantizer; 3]>));
            let state_capture = state.clone();
            let generation_capture = auto_generation.clone();
            let history_capture = auto_history.clone();
            let before_capture = auto_before.clone();
            steps.push(step(
                "automatic-baseline",
                "capture-before-linked-red-auto",
                serde_json::Value::Null,
                Box::new(move || {
                    let state = state_capture.borrow();
                    generation_capture.set(state.scheduler.current_generation());
                    history_capture.set(state.creative_history.undo.len());
                    *before_capture.borrow_mut() = state
                        .document
                        .as_ref()
                        .map(|document| document.recipe.threshold.rgb_state.components.clone());
                }),
            ));
            let auto = ui.threshold_component_rows[0].auto.clone();
            steps.push(step(
                "automatic-baseline",
                "apply-linked-red-auto",
                serde_json::Value::Null,
                Box::new(move || auto.emit_clicked()),
            ));
            let state_check = state.clone();
            let ui_check = ui.clone();
            let log_check = log.clone();
            let generation_check = auto_generation.clone();
            let history_check = auto_history.clone();
            steps.push(step(
                "automatic-baseline",
                "assert-exact-arrays-link-lock-process-preview-history",
                serde_json::Value::Null,
                Box::new(move || {
                    let expected = ComponentQuantizer::automatic_scalar(7);
                    let state = state_check.borrow();
                    let valid = state.document.as_ref().is_some_and(|document| {
                        let rgb = &document.recipe.threshold.rgb_state;
                        rgb.components[0].boundaries == expected.boundaries
                            && rgb.components[0].outputs == expected.outputs
                            && rgb.components[1].boundaries == expected.boundaries
                            && rgb.components[1].outputs == expected.outputs
                            && rgb.components[2].outputs.len() == 3
                            && !rgb.components[0].enabled
                            && rgb.components[1].enabled
                            && rgb.components[2].enabled
                    }) && state.scheduler.current_generation()
                        == generation_check.get() + 1
                        && state.creative_history.undo.len() == history_check.get() + 1
                        && ui_check.threshold_component_rows[0].bands.value() == 7.0
                        && ui_check.threshold_component_rows[1].bands.value() == 7.0
                        && !ui_check.threshold_component_rows[2].auto.is_sensitive()
                        && ui_check.status.label()
                            == "Red automatic baseline applied to 1 linked peer — updating preview…";
                    if !valid {
                        log_check.event(
                            "fail",
                            "automatic-baseline",
                            "exact-arrays-link-lock-process-preview-history",
                            serde_json::json!(false),
                        );
                    }
                }),
            ));
            let noop_generation = Rc::new(Cell::new(0_u64));
            let noop_history = Rc::new(Cell::new(0_usize));
            let state_capture = state.clone();
            let generation_capture = noop_generation.clone();
            let history_capture = noop_history.clone();
            steps.push(step(
                "automatic-baseline",
                "capture-before-repeat",
                serde_json::Value::Null,
                Box::new(move || {
                    let state = state_capture.borrow();
                    generation_capture.set(state.scheduler.current_generation());
                    history_capture.set(state.creative_history.undo.len());
                }),
            ));
            let auto = ui.threshold_component_rows[0].auto.clone();
            steps.push(step(
                "automatic-baseline",
                "repeat-identical-auto",
                serde_json::Value::Null,
                Box::new(move || auto.emit_clicked()),
            ));
            let state_check = state.clone();
            let ui_check = ui.clone();
            let log_check = log.clone();
            let generation_check = noop_generation.clone();
            let history_check = noop_history.clone();
            steps.push(step(
                "automatic-baseline",
                "assert-repeat-is-no-op",
                serde_json::Value::Null,
                Box::new(move || {
                    let state = state_check.borrow();
                    if state.scheduler.current_generation() != generation_check.get()
                        || state.creative_history.undo.len() != history_check.get()
                        || ui_check.status.label() != "Red already uses the automatic baseline"
                    {
                        log_check.event(
                            "fail",
                            "automatic-baseline",
                            "repeat-history-preview-no-op",
                            serde_json::json!(false),
                        );
                    }
                }),
            ));
            let ui_undo = ui.clone();
            let state_undo = state.clone();
            steps.push(step(
                "automatic-baseline",
                "undo-auto",
                serde_json::Value::Null,
                Box::new(move || creative_history_step(&ui_undo, &state_undo, false)),
            ));
            let state_check = state.clone();
            let ui_check = ui.clone();
            let log_check = log.clone();
            let before_check = auto_before.clone();
            steps.push(step(
                "automatic-baseline",
                "assert-undo-resyncs-cards",
                serde_json::Value::Null,
                Box::new(move || {
                    let state = state_check.borrow();
                    let valid = state.document.as_ref().is_some_and(|document| {
                        before_check.borrow().as_ref()
                            == Some(&document.recipe.threshold.rgb_state.components)
                    }) && ui_check.threshold_component_rows[0].bands.value() == 7.0
                        && ui_check.threshold_component_rows[1].bands.value() == 7.0
                        && ui_check.redo.is_sensitive();
                    if !valid {
                        log_check.event(
                            "fail",
                            "automatic-baseline",
                            "undo-card-resync",
                            serde_json::json!(false),
                        );
                    }
                }),
            ));
            let ui_redo = ui.clone();
            let state_redo = state.clone();
            steps.push(step(
                "automatic-baseline",
                "redo-auto",
                serde_json::Value::Null,
                Box::new(move || creative_history_step(&ui_redo, &state_redo, true)),
            ));
            let state_check = state.clone();
            let ui_check = ui.clone();
            let log_check = log.clone();
            steps.push(step(
                "automatic-baseline",
                "assert-redo-resyncs-cards",
                serde_json::Value::Null,
                Box::new(move || {
                    let expected = ComponentQuantizer::automatic_scalar(7);
                    let state = state_check.borrow();
                    let valid = state.document.as_ref().is_some_and(|document| {
                        let rgb = &document.recipe.threshold.rgb_state;
                        rgb.components[0].boundaries == expected.boundaries
                            && rgb.components[0].outputs == expected.outputs
                            && rgb.components[1].boundaries == expected.boundaries
                            && rgb.components[1].outputs == expected.outputs
                    }) && ui_check.threshold_component_rows[0].bands.value() == 7.0
                        && ui_check.threshold_component_rows[1].bands.value() == 7.0;
                    if !valid {
                        log_check.event(
                            "fail",
                            "automatic-baseline",
                            "redo-card-resync",
                            serde_json::json!(false),
                        );
                    }
                }),
            ));
            let edit = ui.threshold_component_rows[0].edit.clone();
            steps.push(step(
                "automatic-baseline",
                "reopen-red-editor",
                serde_json::Value::Null,
                Box::new(move || edit.emit_by_name::<()>("activated", &[])),
            ));
            let state_check = state.clone();
            let ui_check = ui.clone();
            let log_check = log.clone();
            steps.push(step(
                "automatic-baseline",
                "assert-editor-reflects-auto",
                serde_json::Value::Null,
                Box::new(move || {
                    let expected = ComponentQuantizer::automatic_scalar(7);
                    let state = state_check.borrow();
                    let valid = state.threshold_editor_target == Some(ThresholdEditTarget::Red)
                        && state.threshold_editor_visible
                        && state.document.as_ref().is_some_and(|document| {
                            let red = &document.recipe.threshold.rgb_state.components[0];
                            red.boundaries == expected.boundaries && red.outputs == expected.outputs
                        })
                        && ui_check
                            .audit_threshold_bands
                            .borrow()
                            .as_ref()
                            .is_some_and(|bands| bands.value() == 7.0);
                    if !valid {
                        log_check.event(
                            "fail",
                            "automatic-baseline",
                            "editor-reflects-auto",
                            serde_json::json!(false),
                        );
                    }
                }),
            ));
            let ui_close = ui.clone();
            steps.push(step(
                "automatic-baseline",
                "close-red-editor",
                serde_json::Value::Null,
                Box::new(move || {
                    let dialog = ui_close.threshold_editor_dialog.borrow().clone();
                    if let Some(dialog) = dialog {
                        dialog.close();
                    }
                }),
            ));
            for _ in 0..3 {
                steps.push(step(
                    "automatic-baseline",
                    "settle-close",
                    serde_json::Value::Null,
                    Box::new(|| {}),
                ));
            }
            let locked_generation = Rc::new(Cell::new(0_u64));
            let locked_history = Rc::new(Cell::new(0_usize));
            let state_capture = state.clone();
            let generation_capture = locked_generation.clone();
            let history_capture = locked_history.clone();
            steps.push(step(
                "lock-mapping",
                "capture-before-refused-blue-bands",
                serde_json::Value::Null,
                Box::new(move || {
                    let state = state_capture.borrow();
                    generation_capture.set(state.scheduler.current_generation());
                    history_capture.set(state.creative_history.undo.len());
                }),
            ));
            let bands = ui.threshold_component_rows[2].bands.clone();
            steps.push(step(
                "lock-mapping",
                "attempt-blue-bands-while-locked",
                serde_json::json!(9),
                Box::new(move || bands.set_value(9.0)),
            ));
            let state_check = state.clone();
            let log_check = log.clone();
            let generation_check = locked_generation.clone();
            let history_check = locked_history.clone();
            steps.push(step(
                "lock-mapping",
                "assert-refused-without-history-or-preview",
                serde_json::Value::Null,
                Box::new(move || {
                    let state = state_check.borrow();
                    let valid = state.document.as_ref().is_some_and(|document| {
                        document.recipe.threshold.rgb_state.components[2]
                            .outputs
                            .len()
                            == 3
                    }) && state.scheduler.current_generation()
                        == generation_check.get()
                        && state.creative_history.undo.len() == history_check.get();
                    if !valid {
                        log_check.event(
                            "fail",
                            "lock-mapping",
                            "refused-without-history-preview",
                            serde_json::json!(false),
                        );
                    }
                }),
            ));
            let edit = ui.threshold_component_rows[2].edit.clone();
            steps.push(step(
                "component-editor",
                "open-locked-blue-for-inspection",
                serde_json::Value::Null,
                Box::new(move || edit.emit_by_name::<()>("activated", &[])),
            ));
            let state_check = state.clone();
            let ui_check = ui.clone();
            let log_check = log.clone();
            steps.push(step(
                "component-editor",
                "assert-exact-blue-target",
                serde_json::Value::Null,
                Box::new(move || {
                    let state = state_check.borrow();
                    if state.threshold_editor_target != Some(ThresholdEditTarget::Blue)
                        || !state.threshold_editor_visible
                        || ui_check.threshold_editor_dialog.borrow().is_none()
                    {
                        log_check.event(
                            "fail",
                            "component-editor",
                            "exact-blue-target",
                            serde_json::json!(false),
                        );
                    }
                }),
            ));
            let ui_close = ui.clone();
            steps.push(step(
                "component-editor",
                "close-inspection-window",
                serde_json::Value::Null,
                Box::new(move || {
                    let dialog = ui_close.threshold_editor_dialog.borrow().clone();
                    if let Some(dialog) = dialog {
                        dialog.close();
                    }
                }),
            ));
            for _ in 0..3 {
                steps.push(step(
                    "component-editor",
                    "settle-close",
                    serde_json::Value::Null,
                    Box::new(|| {}),
                ));
            }
            let ui_space = ui.clone();
            steps.push(step(
                "working-space",
                "select-hsv-in-inspector",
                serde_json::json!("HSV"),
                Box::new(move || ui_space.threshold_space.set_selected(1)),
            ));
            let ui_check = ui.clone();
            let state_check = state.clone();
            let log_check = log.clone();
            steps.push(step(
                "working-space",
                "assert-hsv-labels-and-hue-origin",
                serde_json::Value::Null,
                Box::new(move || {
                    let state = state_check.borrow();
                    let valid = state.document.as_ref().is_some_and(|document| {
                        document.recipe.threshold.active_space == ThresholdSpace::Hsv
                    }) && ui_check.threshold_link.title() == "Link Saturation + Value"
                        && ui_check.threshold_link.subtitle().as_deref()
                            == Some("Future Saturation or Value edits copy to the unlocked peer; existing differences remain; Hue is independent; Process remains per-component.")
                        && ui_check.threshold_link.is_active()
                        && ui_check.threshold_link_editor.title()
                            == "Edit linked S + V mapping…"
                        && ui_check.threshold_component_rows[0].row.title() == "Hue mapping"
                        && ui_check.threshold_component_rows[1].row.title() == "Saturation mapping"
                        && ui_check.threshold_component_rows[2].row.title() == "Value mapping"
                        && ui_check.threshold_component_rows[0].edit.title()
                            == "Edit Hue band mapping…"
                        && ui_check.threshold_component_rows[0].auto_row.subtitle().as_deref()
                            == Some("Even circular bands · levels wrap across the Hue seam")
                        && ui_check.threshold_component_rows[1].auto_row.subtitle().as_deref()
                            == Some("Even bands · levels from 0 to 1")
                        && ui_check.threshold_component_rows[2].auto_row.subtitle().as_deref()
                            == Some("Even bands · levels from 0 to 1")
                        && ui_check.threshold_component_rows[0].hue_origin_row.title()
                            == "Hue origin (°)"
                        && ui_check.threshold_component_rows[0]
                            .hue_origin_row
                            .is_visible()
                        && !ui_check.threshold_component_rows[1]
                            .hue_origin_row
                            .is_visible()
                        && !ui_check.threshold_component_rows[2]
                            .hue_origin_row
                            .is_visible();
                    if !valid {
                        log_check.event(
                            "fail",
                            "working-space",
                            "hsv-labels-hue-origin",
                            serde_json::json!(false),
                        );
                    }
                }),
            ));
            let hue_generation = Rc::new(Cell::new(0_u64));
            let hue_history = Rc::new(Cell::new(0_usize));
            let sv_before = Rc::new(RefCell::new(None::<[ComponentQuantizer; 2]>));
            let state_capture = state.clone();
            let generation_capture = hue_generation.clone();
            let history_capture = hue_history.clone();
            let sv_capture = sv_before.clone();
            steps.push(step(
                "hue-origin",
                "capture-before-direct-edit",
                serde_json::Value::Null,
                Box::new(move || {
                    let state = state_capture.borrow();
                    generation_capture.set(state.scheduler.current_generation());
                    history_capture.set(state.creative_history.undo.len());
                    *sv_capture.borrow_mut() = state.document.as_ref().map(|document| {
                        [
                            document.recipe.threshold.hsv_state.saturation.clone(),
                            document.recipe.threshold.hsv_state.value.clone(),
                        ]
                    });
                }),
            ));
            let origin = ui.threshold_component_rows[0].hue_origin.clone();
            steps.push(step(
                "hue-origin",
                "set-direct-value",
                serde_json::json!(30.0),
                Box::new(move || origin.set_value(30.0)),
            ));
            let state_check = state.clone();
            let log_check = log.clone();
            let generation_check = hue_generation.clone();
            let history_check = hue_history.clone();
            let sv_check = sv_before.clone();
            steps.push(step(
                "hue-origin",
                "assert-independent-preview-history",
                serde_json::Value::Null,
                Box::new(move || {
                    let state = state_check.borrow();
                    let valid = state.document.as_ref().is_some_and(|document| {
                        document.recipe.threshold.hsv_state.hue_origin_degrees == 30.0
                            && sv_check.borrow().as_ref()
                                == Some(&[
                                    document.recipe.threshold.hsv_state.saturation.clone(),
                                    document.recipe.threshold.hsv_state.value.clone(),
                                ])
                    }) && state.scheduler.current_generation()
                        == generation_check.get() + 1
                        && state.creative_history.undo.len() == history_check.get() + 1;
                    if !valid {
                        log_check.event(
                            "fail",
                            "hue-origin",
                            "independent-preview-history",
                            serde_json::json!(false),
                        );
                    }
                }),
            ));
            let lock = ui.threshold_component_rows[0].lock.clone();
            steps.push(step(
                "hue-origin",
                "lock-hue",
                serde_json::json!(true),
                Box::new(move || lock.set_active(true)),
            ));
            let ui_check = ui.clone();
            let log_check = log.clone();
            steps.push(step(
                "hue-origin",
                "assert-lock-gates-origin-not-process-or-editor",
                serde_json::Value::Null,
                Box::new(move || {
                    let hue = &ui_check.threshold_component_rows[0];
                    if hue.hue_origin.is_sensitive()
                        || hue.auto.is_sensitive()
                        || !hue.process.is_sensitive()
                        || !hue.edit.is_sensitive()
                    {
                        log_check.event(
                            "fail",
                            "hue-origin",
                            "lock-gating",
                            serde_json::json!(false),
                        );
                    }
                }),
            ));
            let ui_undo = ui.clone();
            let state_undo = state.clone();
            steps.push(step(
                "document-history",
                "undo-direct-hue-lock",
                serde_json::Value::Null,
                Box::new(move || creative_history_step(&ui_undo, &state_undo, false)),
            ));
            let state_check = state.clone();
            let ui_check = ui.clone();
            let log_check = log.clone();
            steps.push(step(
                "document-history",
                "assert-undo-resync",
                serde_json::Value::Null,
                Box::new(move || {
                    let valid = state_check
                        .borrow()
                        .document
                        .as_ref()
                        .is_some_and(|document| !document.recipe.threshold.hsv_state.locks[0])
                        && ui_check.threshold_component_rows[0]
                            .hue_origin
                            .is_sensitive()
                        && ui_check.redo.is_sensitive();
                    if !valid {
                        log_check.event(
                            "fail",
                            "document-history",
                            "threshold-inspector-undo-resync",
                            serde_json::json!(false),
                        );
                    }
                }),
            ));
            let ui_redo = ui.clone();
            let state_redo = state.clone();
            steps.push(step(
                "document-history",
                "redo-direct-hue-lock",
                serde_json::Value::Null,
                Box::new(move || creative_history_step(&ui_redo, &state_redo, true)),
            ));
            let state_check = state.clone();
            let ui_check = ui.clone();
            let log_check = log.clone();
            steps.push(step(
                "document-history",
                "assert-redo-resync",
                serde_json::Value::Null,
                Box::new(move || {
                    let valid = state_check
                        .borrow()
                        .document
                        .as_ref()
                        .is_some_and(|document| document.recipe.threshold.hsv_state.locks[0])
                        && !ui_check.threshold_component_rows[0]
                            .hue_origin
                            .is_sensitive();
                    if !valid {
                        log_check.event(
                            "fail",
                            "document-history",
                            "threshold-inspector-redo-resync",
                            serde_json::json!(false),
                        );
                    }
                }),
            ));
        }
        "threshold-dialog" => {
            let noop_history_len = Rc::new(Cell::new(0_usize));
            let noop_generation = Rc::new(Cell::new(0_u64));
            let method = ui.method_thresholds.clone();
            steps.push(step(
                "method",
                "set-active",
                serde_json::json!("thresholds"),
                Box::new(move || method.set_active(true)),
            ));
            let state_clean = state.clone();
            let ui_clean = ui.clone();
            let history_capture = noop_history_len.clone();
            let generation_capture = noop_generation.clone();
            steps.push(step(
                "threshold-noop",
                "establish-clean-opening",
                serde_json::Value::Null,
                Box::new(move || {
                    let mut current = state_clean.borrow_mut();
                    mark_document_saved(&mut current);
                    history_capture.set(current.creative_history.undo.len());
                    generation_capture.set(current.scheduler.current_generation());
                    drop(current);
                    ui_clean.save.set_sensitive(false);
                    sync_document_history_ui(&ui_clean, &state_clean);
                }),
            ));
            let ui_open = ui.clone();
            let state_open = state.clone();
            steps.push(step(
                "threshold-dialog",
                "open",
                serde_json::Value::Null,
                Box::new(move || present_threshold_editor(&ui_open, &state_open)),
            ));
            let ui_contract = ui.clone();
            let log_contract = log.clone();
            steps.push(step(
                "threshold-dialog",
                "assert-window-contract",
                serde_json::Value::Null,
                Box::new(move || {
                    let valid = ui_contract
                        .threshold_editor_dialog
                        .borrow()
                        .as_ref()
                        .is_some_and(|dialog| {
                            is_separate_modal_transient(dialog, &ui_contract.window)
                                && dialog.height() <= 600
                        });
                    let focus_ready = ui_contract
                        .audit_threshold_plot
                        .borrow()
                        .as_ref()
                        .is_some_and(|plot| {
                            plot.is_focusable() && plot.has_css_class(CREATIVE_FOCUS_CLASS)
                        });
                    if !valid || !focus_ready {
                        log_contract.event(
                            "fail",
                            "threshold-dialog",
                            "window-contract",
                            serde_json::json!({"window": valid, "focus": focus_ready}),
                        );
                    }
                }),
            ));
            let ui_history = ui.clone();
            let log_history = log.clone();
            steps.push(step(
                "threshold-local-history",
                "assert-initial-controls",
                serde_json::Value::Null,
                Box::new(move || {
                    let local_disabled = ui_history
                        .audit_threshold_undo
                        .borrow()
                        .as_ref()
                        .is_some_and(|button| !button.is_sensitive())
                        && ui_history
                            .audit_threshold_redo
                            .borrow()
                            .as_ref()
                            .is_some_and(|button| !button.is_sensitive());
                    if !local_disabled
                        || ui_history.undo.is_sensitive()
                        || ui_history.redo.is_sensitive()
                    {
                        log_history.event(
                            "fail",
                            "threshold-local-history",
                            "initial-control-state",
                            serde_json::json!({"local_disabled": local_disabled}),
                        );
                    }
                }),
            ));
            let ui_return = ui.clone();
            steps.push(step(
                "threshold-noop",
                "edit-return-done",
                serde_json::Value::Null,
                Box::new(move || {
                    if let Some(link) = ui_return.audit_threshold_link.borrow().as_ref() {
                        link.set_active(false);
                        link.set_active(true);
                    }
                    let done = ui_return.audit_threshold_done.borrow().clone();
                    if let Some(done) = done {
                        done.emit_clicked();
                    }
                }),
            ));
            let state_check = state.clone();
            let ui_check = ui.clone();
            let history_check = noop_history_len.clone();
            let generation_check = noop_generation.clone();
            let log_check = log.clone();
            steps.push(step(
                "threshold-noop",
                "assert-done-is-clean-noop",
                serde_json::Value::Null,
                Box::new(move || {
                    let current = state_check.borrow();
                    let valid = current
                        .document
                        .as_ref()
                        .is_some_and(|document| !document.dirty)
                        && current.creative_history.undo.len() == history_check.get()
                        && current.scheduler.current_generation() == generation_check.get()
                        && !ui_check.save.is_sensitive();
                    if !valid {
                        log_check.event(
                            "fail",
                            "threshold-noop",
                            "done-dirty-history-or-preview",
                            serde_json::json!({
                                "dirty": current.document.as_ref().is_some_and(|document| document.dirty),
                                "history": current.creative_history.undo.len(),
                                "generation": current.scheduler.current_generation(),
                            }),
                        );
                    }
                }),
            ));
            let ui_reopen = ui.clone();
            let state_reopen = state.clone();
            steps.push(step(
                "threshold-noop",
                "reopen-for-cancel",
                serde_json::Value::Null,
                Box::new(move || present_threshold_editor(&ui_reopen, &state_reopen)),
            ));
            let ui_return = ui.clone();
            steps.push(step(
                "threshold-noop",
                "edit-return-cancel",
                serde_json::Value::Null,
                Box::new(move || {
                    if let Some(link) = ui_return.audit_threshold_link.borrow().as_ref() {
                        link.set_active(false);
                        link.set_active(true);
                    }
                    let cancel = ui_return.audit_threshold_cancel.borrow().clone();
                    if let Some(cancel) = cancel {
                        cancel.emit_clicked();
                    }
                }),
            ));
            let state_check = state.clone();
            let ui_check = ui.clone();
            let history_check = noop_history_len;
            let generation_check = noop_generation;
            let log_check = log.clone();
            steps.push(step(
                "threshold-noop",
                "assert-cancel-is-clean-noop",
                serde_json::Value::Null,
                Box::new(move || {
                    let current = state_check.borrow();
                    let valid = current
                        .document
                        .as_ref()
                        .is_some_and(|document| !document.dirty)
                        && current.creative_history.undo.len() == history_check.get()
                        && current.scheduler.current_generation() == generation_check.get()
                        && !ui_check.save.is_sensitive();
                    if !valid {
                        log_check.event(
                            "fail",
                            "threshold-noop",
                            "cancel-dirty-history-or-preview",
                            serde_json::json!({
                                "dirty": current.document.as_ref().is_some_and(|document| document.dirty),
                                "history": current.creative_history.undo.len(),
                                "generation": current.scheduler.current_generation(),
                            }),
                        );
                    }
                }),
            ));
            let ui_reopen = ui.clone();
            let state_reopen = state.clone();
            steps.push(step(
                "threshold-dialog",
                "reopen-after-noop-cases",
                serde_json::Value::Null,
                Box::new(move || present_threshold_editor(&ui_reopen, &state_reopen)),
            ));
            let opening_threshold = Rc::new(RefCell::new(None));
            let opening_store = opening_threshold.clone();
            let state_capture = state.clone();
            let generation = Rc::new(Cell::new(0_u64));
            let generation_capture = generation.clone();
            steps.push(step(
                "threshold-dialog",
                "capture-opening-state",
                serde_json::Value::Null,
                Box::new(move || {
                    let state = state_capture.borrow();
                    *opening_store.borrow_mut() = state
                        .document
                        .as_ref()
                        .map(|document| document.recipe.threshold.clone());
                    generation_capture.set(state.scheduler.current_generation());
                }),
            ));
            let ui_link = ui.clone();
            steps.push(step(
                "link-mappings",
                "unlink-without-copy",
                serde_json::json!(false),
                Box::new(move || {
                    if let Some(link) = ui_link.audit_threshold_link.borrow().as_ref() {
                        link.set_active(false);
                    }
                }),
            ));
            let state_check = state.clone();
            let generation_check = generation.clone();
            let log_check = log.clone();
            steps.push(step(
                "link-mappings",
                "assert-no-preview",
                serde_json::Value::Null,
                Box::new(move || {
                    let state = state_check.borrow();
                    let valid = state.scheduler.current_generation() == generation_check.get()
                        && state.document.as_ref().is_some_and(|document| {
                            document.recipe.threshold.rgb_state.link == LinkPolicy::Independent
                        });
                    if !valid {
                        log_check.event(
                            "fail",
                            "link-mappings",
                            "unexpected-preview-or-copy",
                            serde_json::json!(false),
                        );
                    }
                }),
            ));
            let bands_before = Rc::new(Cell::new(0_usize));
            let bands_store = bands_before.clone();
            let generation_capture = generation.clone();
            let state_capture = state.clone();
            steps.push(step(
                "bands",
                "capture-before-add",
                serde_json::Value::Null,
                Box::new(move || {
                    let state = state_capture.borrow();
                    bands_store.set(state.document.as_ref().map_or(0, |document| {
                        document.recipe.threshold.rgb_state.components[0]
                            .outputs
                            .len()
                    }));
                    generation_capture.set(state.scheduler.current_generation());
                }),
            ));
            let ui_add = ui.clone();
            steps.push(step(
                "bands",
                "add-split",
                serde_json::Value::Null,
                Box::new(move || {
                    if let Some(spin) = ui_add.audit_threshold_bands.borrow().as_ref() {
                        spin.set_value(spin.value() + 1.0);
                    }
                }),
            ));
            let bands_check = bands_before.clone();
            let generation_check = generation.clone();
            let state_check = state.clone();
            let log_check = log.clone();
            steps.push(step(
                "bands",
                "assert-add-one-preview",
                serde_json::Value::Null,
                Box::new(move || {
                    let state = state_check.borrow();
                    let valid = state.document.as_ref().is_some_and(|document| {
                        document.recipe.threshold.rgb_state.components[0]
                            .outputs
                            .len()
                            == bands_check.get() + 1
                    }) && state.scheduler.current_generation()
                        == generation_check.get() + 1;
                    if !valid {
                        log_check.event(
                            "fail",
                            "bands",
                            "safe-add-contract",
                            serde_json::json!(false),
                        );
                    }
                }),
            ));
            let ui_history = ui.clone();
            let log_history = log.clone();
            steps.push(step(
                "threshold-local-history",
                "assert-undo-enabled-after-edit",
                serde_json::Value::Null,
                Box::new(move || {
                    let valid = ui_history
                        .audit_threshold_undo
                        .borrow()
                        .as_ref()
                        .is_some_and(|button| button.is_sensitive())
                        && ui_history
                            .audit_threshold_redo
                            .borrow()
                            .as_ref()
                            .is_some_and(|button| !button.is_sensitive());
                    if !valid {
                        log_history.event(
                            "fail",
                            "threshold-local-history",
                            "edit-control-state",
                            serde_json::json!(false),
                        );
                    }
                }),
            ));
            let ui_undo = ui.clone();
            steps.push(step(
                "threshold-local-history",
                "undo-add",
                serde_json::Value::Null,
                Box::new(move || {
                    if let Some(undo) = ui_undo.audit_threshold_history.borrow().as_ref() {
                        undo(false);
                    }
                }),
            ));
            let ui_history = ui.clone();
            let log_history = log.clone();
            steps.push(step(
                "threshold-local-history",
                "assert-redo-enabled-after-undo",
                serde_json::Value::Null,
                Box::new(move || {
                    if !ui_history
                        .audit_threshold_redo
                        .borrow()
                        .as_ref()
                        .is_some_and(|button| button.is_sensitive())
                    {
                        log_history.event(
                            "fail",
                            "threshold-local-history",
                            "undo-control-state",
                            serde_json::json!(false),
                        );
                    }
                }),
            ));
            let ui_add = ui.clone();
            steps.push(step(
                "bands",
                "add-before-remove",
                serde_json::Value::Null,
                Box::new(move || {
                    if let Some(spin) = ui_add.audit_threshold_bands.borrow().as_ref() {
                        spin.set_value(spin.value() + 1.0);
                    }
                }),
            ));
            let ui_remove = ui.clone();
            steps.push(step(
                "bands",
                "remove-merge",
                serde_json::Value::Null,
                Box::new(move || {
                    if let Some(spin) = ui_remove.audit_threshold_bands.borrow().as_ref() {
                        spin.set_value(spin.value() - 1.0);
                    }
                }),
            ));
            let ui_undo = ui.clone();
            steps.push(step(
                "threshold-local-history",
                "undo-remove",
                serde_json::Value::Null,
                Box::new(move || {
                    if let Some(undo) = ui_undo.audit_threshold_history.borrow().as_ref() {
                        undo(false);
                    }
                }),
            ));
            let ui_lock = ui.clone();
            steps.push(step(
                "lock-mapping",
                "lock",
                serde_json::json!(true),
                Box::new(move || {
                    if let Some(lock) = ui_lock.audit_threshold_lock.borrow().as_ref() {
                        lock.set_active(true);
                    }
                }),
            ));
            let locked_mapping = Rc::new(RefCell::new(None));
            let locked_store = locked_mapping.clone();
            let generation_capture = generation.clone();
            let state_capture = state.clone();
            steps.push(step(
                "lock-mapping",
                "capture-protected-state",
                serde_json::Value::Null,
                Box::new(move || {
                    let state = state_capture.borrow();
                    *locked_store.borrow_mut() = state
                        .document
                        .as_ref()
                        .map(|document| document.recipe.threshold.rgb_state.components[0].clone());
                    generation_capture.set(state.scheduler.current_generation());
                }),
            ));
            let ui_locked = ui.clone();
            steps.push(step(
                "lock-mapping",
                "attempt-blocked-edits-and-toggle-process",
                serde_json::Value::Null,
                Box::new(move || {
                    if let Some(spin) = ui_locked.audit_threshold_bands.borrow().as_ref() {
                        spin.set_value(spin.value() + 1.0);
                    }
                    if let Some(spin) = ui_locked.audit_threshold_precise.borrow().as_ref() {
                        spin.set_value(0.91);
                    }
                    if let Some(button) = ui_locked.audit_threshold_reset.borrow().as_ref() {
                        button.emit_clicked();
                    }
                    if let Some(process) = ui_locked.audit_threshold_process.borrow().as_ref() {
                        process.set_active(!process.is_active());
                    }
                }),
            ));
            let locked_check = locked_mapping.clone();
            let generation_check = generation.clone();
            let state_check = state.clone();
            let log_check = log.clone();
            steps.push(step(
                "lock-mapping",
                "assert-protection-and-process",
                serde_json::Value::Null,
                Box::new(move || {
                    let state = state_check.borrow();
                    let valid = state.document.as_ref().is_some_and(|document| {
                        let protected = locked_check.borrow();
                        let protected = protected.as_ref().expect("captured locked mapping");
                        let current = &document.recipe.threshold.rgb_state.components[0];
                        document.recipe.threshold.rgb_state.locks[0]
                            && current.boundaries == protected.boundaries
                            && current.outputs == protected.outputs
                            && !current.enabled
                    }) && state.scheduler.current_generation()
                        == generation_check.get() + 1;
                    if !valid {
                        log_check.event(
                            "fail",
                            "lock-mapping",
                            "protection-contract",
                            serde_json::json!(false),
                        );
                    }
                }),
            ));
            let ui_unlock = ui.clone();
            steps.push(step(
                "lock-mapping",
                "unlock",
                serde_json::json!(false),
                Box::new(move || {
                    if let Some(lock) = ui_unlock.audit_threshold_lock.borrow().as_ref() {
                        lock.set_active(false);
                    }
                }),
            ));
            let ui_lock_destination = ui.clone();
            steps.push(step(
                "sync-now",
                "lock-green-destination",
                serde_json::json!("Green"),
                Box::new(move || {
                    if let Some(target) =
                        ui_lock_destination.audit_threshold_target.borrow().as_ref()
                    {
                        target.set_selected(1);
                    }
                    if let Some(lock) = ui_lock_destination.audit_threshold_lock.borrow().as_ref() {
                        lock.set_active(true);
                    }
                    if let Some(target) =
                        ui_lock_destination.audit_threshold_target.borrow().as_ref()
                    {
                        target.set_selected(0);
                    }
                }),
            ));
            let ui_sync_label = ui.clone();
            let log_sync_label = log.clone();
            steps.push(step(
                "sync-now",
                "assert-locked-destination-styling",
                serde_json::Value::Null,
                Box::new(move || {
                    let valid = ui_sync_label
                        .audit_threshold_sync
                        .borrow()
                        .as_ref()
                        .and_then(gtk::prelude::ButtonExt::child)
                        .and_then(|child| child.downcast::<gtk::Label>().ok())
                        .is_some_and(|label| {
                            let markup = label.label();
                            markup.contains("strikethrough") && markup.contains("Green")
                        });
                    if !valid {
                        log_sync_label.event(
                            "fail",
                            "sync-now",
                            "locked-destination-styling",
                            serde_json::json!(false),
                        );
                    }
                }),
            ));
            let ui_edit = ui.clone();
            steps.push(step(
                "mapping",
                "make-source-distinct",
                serde_json::json!(0.77),
                Box::new(move || {
                    if let Some(spin) = ui_edit.audit_threshold_precise.borrow().as_ref() {
                        spin.set_value(0.77);
                    }
                }),
            ));
            let generation_capture = generation.clone();
            let state_capture = state.clone();
            steps.push(step(
                "sync-now",
                "capture-generation",
                serde_json::Value::Null,
                Box::new(move || {
                    generation_capture.set(state_capture.borrow().scheduler.current_generation());
                }),
            ));
            let ui_sync = ui.clone();
            steps.push(step(
                "sync-now",
                "activate",
                serde_json::Value::Null,
                Box::new(move || {
                    if let Some(button) = ui_sync.audit_threshold_sync.borrow().as_ref() {
                        button.emit_clicked();
                    }
                }),
            ));
            let generation_check = generation.clone();
            let state_check = state.clone();
            let log_check = log.clone();
            steps.push(step(
                "sync-now",
                "assert-one-preview",
                serde_json::Value::Null,
                Box::new(move || {
                    if state_check.borrow().scheduler.current_generation()
                        != generation_check.get() + 1
                    {
                        log_check.event(
                            "fail",
                            "sync-now",
                            "preview-count",
                            serde_json::json!(false),
                        );
                    }
                }),
            ));
            let ui_done_rgb = ui.clone();
            let log_done_rgb = log.clone();
            steps.push(step(
                "threshold-dialog",
                "done-rgb",
                serde_json::Value::Null,
                Box::new(move || {
                    let button = ui_done_rgb.audit_threshold_done.borrow().clone();
                    if let Some(button) = button {
                        button.emit_clicked();
                    } else {
                        log_done_rgb.event(
                            "fail",
                            "threshold-dialog",
                            "missing-done-rgb",
                            serde_json::Value::Null,
                        );
                    }
                }),
            ));
            let ui_global_undo = ui.clone();
            let state_global_undo = state.clone();
            steps.push(step(
                "creative-history",
                "undo-rgb-dialog-transaction",
                serde_json::Value::Null,
                Box::new(move || creative_history_step(&ui_global_undo, &state_global_undo, false)),
            ));
            let opening_check = opening_threshold.clone();
            let state_check = state.clone();
            let log_check = log.clone();
            steps.push(step(
                "creative-history",
                "assert-rgb-dialog-transaction-restored",
                serde_json::Value::Null,
                Box::new(move || {
                    let restored = state_check
                        .borrow()
                        .document
                        .as_ref()
                        .map(|document| document.recipe.threshold.clone());
                    if restored != *opening_check.borrow() {
                        log_check.event(
                            "fail",
                            "creative-history",
                            "rgb-dialog-transaction-restore",
                            serde_json::json!(false),
                        );
                    }
                }),
            ));
            let ui_space = ui.clone();
            steps.push(step(
                "working-space",
                "select-hsv-in-inspector",
                serde_json::json!("HSV"),
                Box::new(move || {
                    if let Some(space) = ui_space.audit_threshold_space.borrow().as_ref() {
                        space.set_selected(1);
                    }
                }),
            ));
            let ui_reopen_hsv = ui.clone();
            let state_reopen_hsv = state.clone();
            steps.push(step(
                "threshold-dialog",
                "open-hsv-from-inspector-state",
                serde_json::Value::Null,
                Box::new(move || present_threshold_editor(&ui_reopen_hsv, &state_reopen_hsv)),
            ));
            let ui_hue = ui.clone();
            let log_hue = log.clone();
            steps.push(step(
                "hue",
                "assert-no-link-or-sync",
                serde_json::Value::Null,
                Box::new(move || {
                    let valid = ui_hue
                        .audit_threshold_sync
                        .borrow()
                        .as_ref()
                        .is_some_and(|button| !button.is_mapped());
                    if !valid {
                        log_hue.event("fail", "hue", "sync-visible", serde_json::json!(false));
                    }
                }),
            ));
            let generation_capture = generation.clone();
            let state_capture = state.clone();
            steps.push(step(
                "hue-origin",
                "capture-generation",
                serde_json::Value::Null,
                Box::new(move || {
                    generation_capture.set(state_capture.borrow().scheduler.current_generation());
                }),
            ));
            let ui_origin = ui.clone();
            steps.push(step(
                "hue-origin",
                "set-and-wrap",
                serde_json::json!(390.0),
                Box::new(move || {
                    if let Some(origin) = ui_origin.audit_threshold_hue_origin.borrow().as_ref() {
                        origin.set_value(390.0);
                    }
                }),
            ));
            let generation_check = generation.clone();
            let state_check = state.clone();
            let log_check = log.clone();
            steps.push(step(
                "hue-origin",
                "assert-canonical-one-preview",
                serde_json::Value::Null,
                Box::new(move || {
                    let state = state_check.borrow();
                    let origin = state
                        .document
                        .as_ref()
                        .map(|document| document.recipe.threshold.hsv_state.hue_origin_degrees);
                    if origin != Some(30.0)
                        || state.scheduler.current_generation() != generation_check.get() + 1
                    {
                        log_check.event(
                            "fail",
                            "hue-origin",
                            "canonical-preview-contract",
                            serde_json::json!({"origin": origin}),
                        );
                    }
                }),
            ));
            let ui_lock = ui.clone();
            steps.push(step(
                "hue-origin",
                "lock-hue",
                serde_json::json!(true),
                Box::new(move || {
                    if let Some(lock) = ui_lock.audit_threshold_lock.borrow().as_ref() {
                        lock.set_active(true);
                    }
                }),
            ));
            let generation_capture = generation.clone();
            let state_capture = state.clone();
            steps.push(step(
                "hue-origin",
                "capture-locked-generation",
                serde_json::Value::Null,
                Box::new(move || {
                    generation_capture.set(state_capture.borrow().scheduler.current_generation());
                }),
            ));
            let ui_origin = ui.clone();
            steps.push(step(
                "hue-origin",
                "attempt-while-locked",
                serde_json::json!(120.0),
                Box::new(move || {
                    if let Some(origin) = ui_origin.audit_threshold_hue_origin.borrow().as_ref() {
                        origin.set_value(120.0);
                    }
                }),
            ));
            let generation_check = generation.clone();
            let state_check = state.clone();
            let log_check = log.clone();
            steps.push(step(
                "hue-origin",
                "assert-lock-blocked",
                serde_json::Value::Null,
                Box::new(move || {
                    let state = state_check.borrow();
                    let origin = state
                        .document
                        .as_ref()
                        .map(|document| document.recipe.threshold.hsv_state.hue_origin_degrees);
                    if origin != Some(30.0)
                        || state.scheduler.current_generation() != generation_check.get()
                    {
                        log_check.event(
                            "fail",
                            "hue-origin",
                            "lock-contract",
                            serde_json::json!({"origin": origin}),
                        );
                    }
                }),
            ));
            let ui_unlock = ui.clone();
            steps.push(step(
                "hue-origin",
                "unlock-hue",
                serde_json::json!(false),
                Box::new(move || {
                    if let Some(lock) = ui_unlock.audit_threshold_lock.borrow().as_ref() {
                        lock.set_active(false);
                    }
                }),
            ));
            let ui_access = ui.clone();
            let log_access = log.clone();
            steps.push(step(
                "threshold-accessibility",
                "assert-focusable-controls",
                serde_json::Value::Null,
                Box::new(move || {
                    let target = ui_access
                        .audit_threshold_target
                        .borrow()
                        .as_ref()
                        .is_some_and(gtk::prelude::WidgetExt::is_focusable);
                    let process = ui_access
                        .audit_threshold_process
                        .borrow()
                        .as_ref()
                        .is_some_and(gtk::prelude::WidgetExt::is_focusable);
                    let bands = ui_access
                        .audit_threshold_bands
                        .borrow()
                        .as_ref()
                        .is_some_and(gtk::prelude::WidgetExt::is_focusable);
                    let lock = ui_access
                        .audit_threshold_lock
                        .borrow()
                        .as_ref()
                        .is_some_and(gtk::prelude::WidgetExt::is_focusable);
                    let hue_origin = ui_access
                        .audit_threshold_hue_origin
                        .borrow()
                        .as_ref()
                        .is_some_and(gtk::prelude::WidgetExt::is_focusable);
                    let valid = target && process && bands && lock && hue_origin;
                    if !valid {
                        log_access.event(
                            "fail",
                            "threshold-accessibility",
                            "focusability-contract",
                            serde_json::json!({
                                "component": target,
                                "working_space_in_dialog": false,
                                "process": process,
                                "bands": bands,
                                "lock": lock,
                                "hue_origin": hue_origin,
                            }),
                        );
                    }
                }),
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
            let ui_reset = ui.clone();
            let log_missing = log.clone();
            steps.push(step(
                "component-reset",
                "activate",
                serde_json::Value::Null,
                Box::new(move || {
                    if let Some(button) = ui_reset.audit_threshold_reset.borrow().as_ref() {
                        button.emit_clicked();
                    } else {
                        log_missing.event(
                            "fail",
                            "component-reset",
                            "missing-widget",
                            serde_json::Value::Null,
                        );
                    }
                }),
            ));
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
            let ui_expand = ui.clone();
            steps.push(step(
                "site-details",
                "expand-selected",
                serde_json::Value::Null,
                Box::new(move || {
                    if let Some(row) = ui_expand.audit_site_expander.borrow().as_ref() {
                        row.set_expanded(true);
                    }
                }),
            ));
            let first_expanded = Rc::new(Cell::new(None::<u64>));
            let first_capture = first_expanded.clone();
            let state_capture = state.clone();
            steps.push(step(
                "site-details",
                "capture-expanded-site",
                serde_json::Value::Null,
                Box::new(move || first_capture.set(state_capture.borrow().expanded_site)),
            ));
            let ui_expand_other = ui.clone();
            steps.push(step(
                "site-details",
                "expand-other",
                serde_json::Value::Null,
                Box::new(move || {
                    if let Some(row) = ui_expand_other.audit_other_site_expander.borrow().as_ref() {
                        row.set_expanded(true);
                    }
                }),
            ));
            let ui_check = ui.clone();
            let state_check = state.clone();
            let first_check = first_expanded;
            let log_check = log.clone();
            steps.push(step(
                "site-details",
                "assert-accordion",
                serde_json::Value::Null,
                Box::new(move || {
                    let current = state_check.borrow();
                    let identity_matches = current.expanded_site == current.selected_sample
                        && current.expanded_site.is_some()
                        && current.expanded_site != first_check.get();
                    drop(current);
                    let mut expanded_rows = 0;
                    let mut child = ui_check.groups.first_child();
                    while let Some(widget) = child {
                        if widget
                            .clone()
                            .downcast::<adw::ExpanderRow>()
                            .is_ok_and(|row| row.is_expanded())
                        {
                            expanded_rows += 1;
                        }
                        child = widget.next_sibling();
                    }
                    if !identity_matches || expanded_rows != 1 {
                        log_check.event(
                            "fail",
                            "site-details",
                            "accordion-contract",
                            serde_json::json!({
                                "identity_matches": identity_matches,
                                "expanded_rows": expanded_rows,
                            }),
                        );
                    }
                }),
            ));
            for selected in 0_u32..VORONOI_MATCHING_LABELS.len() as u32 {
                let drop = ui.voronoi_matching.clone();
                steps.push(step(
                    "matching",
                    "set-selected",
                    serde_json::json!(selected),
                    Box::new(move || drop.set_selected(selected)),
                ));
            }
            for value in [0.1, 4.0] {
                let ui = ui.clone();
                steps.push(step(
                    "influence",
                    "set-value",
                    serde_json::json!(value),
                    Box::new(move || {
                        if let Some(spin) = ui.audit_site_influence.borrow().as_ref() {
                            spin.set_value(value);
                        }
                    }),
                ));
            }
            for selected in 0_u32..3 {
                let ui = ui.clone();
                steps.push(step(
                    "sampling-footprint",
                    "set-selected",
                    serde_json::json!(selected),
                    Box::new(move || {
                        if let Some(drop) = ui.audit_site_sample_size.borrow().as_ref() {
                            drop.set_selected(selected);
                        }
                    }),
                ));
            }
            for active in [true, false] {
                let ui = ui.clone();
                steps.push(step(
                    "site-lock",
                    "set-active",
                    serde_json::json!(active),
                    Box::new(move || {
                        let lock = ui.audit_site_lock.borrow().clone();
                        if let Some(lock) = lock {
                            lock.set_active(active);
                        }
                    }),
                ));
            }
            {
                let ui = ui.clone();
                steps.push(step(
                    "site-target",
                    "open-picker",
                    serde_json::Value::Null,
                    Box::new(move || {
                        let target = ui.audit_site_target.borrow().clone();
                        if let Some(target) = target {
                            target.emit_clicked();
                        }
                    }),
                ));
            }
            {
                let state = state.clone();
                let ui = ui.clone();
                let log = log.clone();
                steps.push(step(
                    "site-target",
                    "assert-picker-title-and-initial-color",
                    serde_json::Value::Null,
                    Box::new(move || {
                        let expected = selected_output_color(&state)
                            .map(|color| display_hex(color.map(f64::from)));
                        let title = ui
                            .picker_dialog
                            .borrow()
                            .as_ref()
                            .and_then(|dialog| dialog.title())
                            .map(|title| title.to_string());
                        let hex = ui
                            .audit_picker_hex
                            .borrow()
                            .as_ref()
                            .map(|entry| entry.text().to_string());
                        if !state.borrow().picker_visible
                            || title.as_deref() != Some("Choose Target Color")
                            || hex != expected
                        {
                            log.event(
                                "fail",
                                "site-target",
                                "picker-purpose-contract",
                                serde_json::json!({"title": title, "hex": hex, "expected": expected}),
                            );
                        }
                    }),
                ));
            }
            {
                let ui = ui.clone();
                steps.push(step(
                    "site-target",
                    "cancel-picker",
                    serde_json::Value::Null,
                    Box::new(move || {
                        let cancel = ui.audit_picker_cancel.borrow().clone();
                        if let Some(cancel) = cancel {
                            cancel.emit_clicked();
                        }
                    }),
                ));
            }
            {
                let ui = ui.clone();
                steps.push(step(
                    "site-source",
                    "open-picker",
                    serde_json::Value::Null,
                    Box::new(move || {
                        if let Some(source) = ui.audit_site_source.borrow().clone() {
                            source.emit_clicked();
                        }
                    }),
                ));
            }
            {
                let state = state.clone();
                let ui = ui.clone();
                let log = log.clone();
                steps.push(step(
                    "site-source",
                    "assert-title-and-initial-color",
                    serde_json::Value::Null,
                    Box::new(move || {
                        let expected = selected_source_color(&state)
                            .map(|color| display_hex(color.map(f64::from)));
                        let title = ui
                            .picker_dialog
                            .borrow()
                            .as_ref()
                            .and_then(|dialog| dialog.title())
                            .map(|title| title.to_string());
                        let hex = ui
                            .audit_picker_hex
                            .borrow()
                            .as_ref()
                            .map(|entry| entry.text().to_string());
                        if title.as_deref() != Some("Choose Source Center") || hex != expected {
                            log.event(
                                "fail",
                                "site-source",
                                "picker-purpose-contract",
                                serde_json::json!({"title": title, "hex": hex, "expected": expected}),
                            );
                        }
                    }),
                ));
            }
            {
                let ui = ui.clone();
                steps.push(step(
                    "site-source",
                    "set-exact-and-select",
                    serde_json::json!("#123456"),
                    Box::new(move || {
                        if let Some(entry) = ui.audit_picker_hex.borrow().as_ref() {
                            entry.set_text("#123456");
                            entry.emit_by_name::<()>("activate", &[]);
                        }
                        let select = ui.audit_picker_select.borrow().clone();
                        if let Some(select) = select {
                            select.emit_clicked();
                        }
                    }),
                ));
            }
            {
                let state = state.clone();
                let log = log.clone();
                steps.push(step(
                    "site-source",
                    "assert-source-commit-destination",
                    serde_json::Value::Null,
                    Box::new(move || {
                        let expected = parse_hex("#123456").expect("audit color").map(|v| v as f32);
                        let current = state.borrow();
                        let site = current.document.as_ref().and_then(|document| {
                            current
                                .selected_sample
                                .and_then(|id| document.recipe.voronoi.site(id))
                        });
                        let valid = site.is_some_and(|site| {
                            site.source_color[..3] == expected
                                && site.target_color == expected
                                && site.position.is_none()
                        });
                        if !valid {
                            log.event(
                                "fail",
                                "site-source",
                                "commit-destination",
                                serde_json::json!(false),
                            );
                        }
                    }),
                ));
            }
            {
                let ui = ui.clone();
                steps.push(step(
                    "site-target",
                    "reopen-set-exact-and-select",
                    serde_json::json!("#abcdef"),
                    Box::new(move || {
                        if let Some(target) = ui.audit_site_target.borrow().clone() {
                            target.emit_clicked();
                        }
                        if let Some(entry) = ui.audit_picker_hex.borrow().as_ref() {
                            entry.set_text("#abcdef");
                            entry.emit_by_name::<()>("activate", &[]);
                        }
                        let select = ui.audit_picker_select.borrow().clone();
                        if let Some(select) = select {
                            select.emit_clicked();
                        }
                    }),
                ));
            }
            {
                let state = state.clone();
                let log = log.clone();
                steps.push(step(
                    "site-target",
                    "assert-target-commit-destination",
                    serde_json::Value::Null,
                    Box::new(move || {
                        let source_expected =
                            parse_hex("#123456").expect("audit color").map(|v| v as f32);
                        let target_expected =
                            parse_hex("#abcdef").expect("audit color").map(|v| v as f32);
                        let current = state.borrow();
                        let site = current.document.as_ref().and_then(|document| {
                            current
                                .selected_sample
                                .and_then(|id| document.recipe.voronoi.site(id))
                        });
                        let valid = site.is_some_and(|site| {
                            site.source_color[..3] == source_expected
                                && site.target_color == target_expected
                        });
                        if !valid {
                            log.event(
                                "fail",
                                "site-target",
                                "commit-destination",
                                serde_json::json!(false),
                            );
                        }
                    }),
                ));
            }
            {
                let ui = ui.clone();
                let state = state.clone();
                let log = log.clone();
                steps.push(step(
                    "add-site",
                    "direct-from-artwork",
                    serde_json::json!([0.05, 0.05]),
                    Box::new(move || {
                        let before = state
                            .borrow()
                            .document
                            .as_ref()
                            .map_or(0, |document| document.recipe.voronoi.sites.len());
                        let added = add_site_from_artwork(&ui, &state, [0.05, 0.05]);
                        let after = state
                            .borrow()
                            .document
                            .as_ref()
                            .map_or(0, |document| document.recipe.voronoi.sites.len());
                        if !added || after != before + 1 {
                            log.event(
                                "fail",
                                "add-site",
                                "direct-add-count",
                                serde_json::json!({"before": before, "after": after}),
                            );
                        }
                    }),
                ));
            }
            let ui_reattach = ui.clone();
            steps.push(step(
                "reattach-source",
                "arm",
                serde_json::json!("canvas click remains manual"),
                Box::new(move || {
                    if let Some(reattach) = ui_reattach.audit_site_reattach.borrow().as_ref() {
                        reattach.emit_clicked();
                    }
                }),
            ));
            {
                let log = log.clone();
                steps.push(step(
                    "add-site",
                    "assert-direct-policy",
                    serde_json::json!("empty Voronoi artwork adds without arming a tool"),
                    Box::new(move || {
                        if canvas_site_action(Method::Voronoi, None) != CanvasSiteAction::Add
                            || canvas_site_action(Method::Thresholds, None)
                                != CanvasSiteAction::Ignore
                        {
                            log.event("fail", "add-site", "direct-policy", serde_json::Value::Null);
                        }
                    }),
                ));
            }
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
            let model = ui.preset_model.clone();
            let neutral_dropdown = ui.preset_dropdown.clone();
            let log_model = log.clone();
            steps.push(step(
                "presets",
                "assert-unified-model",
                serde_json::Value::Null,
                Box::new(move || {
                    let labels = (0..model.n_items())
                        .filter_map(|index| model.string(index).map(|value| value.to_string()))
                        .collect::<Vec<_>>();
                    let builtins = [
                        "Comic Book — Threshold",
                        "Duotone Blue — Threshold",
                        "Vintage Photo — Threshold",
                        "Noir — Threshold",
                        "Pop Art — Threshold",
                    ];
                    let valid = labels.first().is_some_and(|label| label == "Presets…")
                        && labels
                            .get(1..6)
                            .is_some_and(|actual| actual.iter().map(String::as_str).eq(builtins))
                        && labels.get(6..).is_some_and(|users| {
                            users.iter().all(|label| {
                                label.ends_with("— Threshold") || label.ends_with("— Voronoi")
                            })
                        });
                    if !valid || neutral_dropdown.selected() != 0 {
                        log_model.event(
                            "fail",
                            "presets",
                            "unified-model",
                            serde_json::json!({
                                "labels": labels,
                                "selection": neutral_dropdown.selected(),
                            }),
                        );
                    }
                }),
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
            let dropdown = ui.preset_dropdown.clone();
            steps.push(step(
                "presets",
                "select-auto-apply",
                serde_json::json!("Pop Art (t)"),
                Box::new(move || dropdown.set_selected(5)),
            ));
            let state_check = state.clone();
            let generation_check = generation_before.clone();
            let dropdown_check = ui.preset_dropdown.clone();
            let log_check = log.clone();
            steps.push(step(
                "presets",
                "assert-one-generation",
                serde_json::Value::Null,
                Box::new(move || {
                    let before = generation_check.get();
                    let after = state_check.borrow().scheduler.current_generation();
                    if after != before + 1 || dropdown_check.selected() != 5 {
                        log_check.event(
                            "fail",
                            "presets",
                            "apply-generation",
                            serde_json::json!({
                                "before": before,
                                "after": after,
                                "selection": dropdown_check.selected(),
                            }),
                        );
                    }
                    generation_check.set(after);
                }),
            ));
            let ui_noop = ui.clone();
            let state_noop = state.clone();
            steps.push(step(
                "presets",
                "apply-noop-helper",
                serde_json::Value::Null,
                Box::new(move || apply_preset_selection(&ui_noop, &state_noop, 5)),
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
            let undo_generation = Rc::new(Cell::new(0_u64));
            let state_capture = state.clone();
            let generation_capture = undo_generation.clone();
            steps.push(step(
                "preset-history",
                "capture-generation",
                serde_json::Value::Null,
                Box::new(move || {
                    generation_capture.set(state_capture.borrow().scheduler.current_generation());
                }),
            ));
            let ui_undo = ui.clone();
            let state_undo = state.clone();
            steps.push(step(
                "preset-history",
                "undo",
                serde_json::Value::Null,
                Box::new(move || creative_history_step(&ui_undo, &state_undo, false)),
            ));
            let state_check = state.clone();
            let generation_check = undo_generation;
            let dropdown = ui.preset_dropdown.clone();
            let log_check = log.clone();
            steps.push(step(
                "preset-history",
                "assert-undo",
                serde_json::Value::Null,
                Box::new(move || {
                    let before = generation_check.get();
                    let after = state_check.borrow().scheduler.current_generation();
                    if after != before + 1 || dropdown.selected() != 0 {
                        log_check.event(
                            "fail",
                            "preset-history",
                            "undo-restoration",
                            serde_json::json!({"before": before, "after": after, "selection": dropdown.selected()}),
                        );
                    }
                }),
            ));
            let dropdown = ui.preset_dropdown.clone();
            steps.push(step(
                "preset-history",
                "apply-before-follow-up-edit",
                serde_json::json!("Comic Book (t)"),
                Box::new(move || dropdown.set_selected(1)),
            ));
            let ui_check = ui.clone();
            let log_check = log.clone();
            steps.push(step(
                "document-history",
                "assert-new-edit-clears-redo",
                serde_json::Value::Null,
                Box::new(move || {
                    if ui_check.redo.is_sensitive() {
                        log_check.event(
                            "fail",
                            "document-history",
                            "redo-not-cleared",
                            serde_json::json!(false),
                        );
                    }
                }),
            ));
            let smoothing_before = Rc::new(Cell::new(0.0_f32));
            let smoothing_capture = smoothing_before.clone();
            let state_capture = state.clone();
            steps.push(step(
                "document-history",
                "capture-before-smoothing",
                serde_json::Value::Null,
                Box::new(move || {
                    if let Some(value) = state_capture
                        .borrow()
                        .document
                        .as_ref()
                        .map(|document| document.recipe.threshold.input_smoothing)
                    {
                        smoothing_capture.set(value);
                    }
                }),
            ));
            let smoothing = ui.smoothing.clone();
            steps.push(step(
                "preset-history",
                "non-preset-edit",
                serde_json::json!(3),
                Box::new(move || smoothing.set_value(3.0)),
            ));
            let divergent_dropdown = ui.preset_dropdown.clone();
            let divergent_log = log.clone();
            steps.push(step(
                "preset-history",
                "assert-divergent-edit-is-neutral",
                serde_json::Value::Null,
                Box::new(move || {
                    if divergent_dropdown.selected() != 0 {
                        divergent_log.event(
                            "fail",
                            "preset-history",
                            "divergent-selection",
                            serde_json::json!(divergent_dropdown.selected()),
                        );
                    }
                }),
            ));
            let divergence_generation = Rc::new(Cell::new(0_u64));
            let state_capture = state.clone();
            let generation_capture = divergence_generation.clone();
            steps.push(step(
                "preset-history",
                "capture-follow-up-generation",
                serde_json::Value::Null,
                Box::new(move || {
                    generation_capture.set(state_capture.borrow().scheduler.current_generation())
                }),
            ));
            let ui_undo = ui.clone();
            let state_undo = state.clone();
            steps.push(step(
                "preset-history",
                "undo-follow-up-edit",
                serde_json::Value::Null,
                Box::new(move || creative_history_step(&ui_undo, &state_undo, false)),
            ));
            let state_check = state.clone();
            let generation_check = divergence_generation.clone();
            let smoothing_check = smoothing_before.clone();
            let log_check = log.clone();
            steps.push(step(
                "document-history",
                "assert-follow-up-undo",
                serde_json::Value::Null,
                Box::new(move || {
                    let state = state_check.borrow();
                    let smoothing = state
                        .document
                        .as_ref()
                        .map(|document| document.recipe.threshold.input_smoothing);
                    let safe = state.scheduler.current_generation() == generation_check.get() + 1
                        && smoothing == Some(smoothing_check.get())
                        && !state.creative_history.undo.is_empty()
                        && state.creative_history.redo.len() == 1;
                    if !safe {
                        log_check.event(
                            "fail",
                            "document-history",
                            "follow-up-undo",
                            serde_json::json!({"smoothing": smoothing}),
                        );
                    }
                }),
            ));
            let ui_redo = ui.clone();
            let state_redo = state.clone();
            steps.push(step(
                "document-history",
                "redo-follow-up-edit",
                serde_json::Value::Null,
                Box::new(move || creative_history_step(&ui_redo, &state_redo, true)),
            ));
            let state_check = state.clone();
            let generation_check = divergence_generation;
            let ui_check = ui.clone();
            let log_check = log.clone();
            steps.push(step(
                "document-history",
                "assert-follow-up-redo-and-actions",
                serde_json::Value::Null,
                Box::new(move || {
                    let current = state_check.borrow();
                    let smoothing = current
                        .document
                        .as_ref()
                        .map(|document| document.recipe.threshold.input_smoothing);
                    if current.scheduler.current_generation() != generation_check.get() + 2
                        || smoothing != Some(3.0)
                        || ui_check.redo.is_sensitive()
                        || !ui_check.undo.is_sensitive()
                    {
                        log_check.event(
                            "fail",
                            "document-history",
                            "follow-up-redo-or-action-state",
                            serde_json::json!({"smoothing": smoothing, "undo": ui_check.undo.is_sensitive(), "redo": ui_check.redo.is_sensitive()}),
                        );
                    }
                }),
            ));
            let reset_space = ui.reset_space.clone();
            let reset_method = ui.reset_method.clone();
            let log_controls = log.clone();
            steps.push(step(
                "threshold-resets",
                "assert-controls",
                serde_json::Value::Null,
                Box::new(move || {
                    if !(reset_space.is_sensitive() && reset_method.is_sensitive()) {
                        log_controls.event(
                            "fail",
                            "threshold-resets",
                            "unavailable-controls",
                            serde_json::Value::Null,
                        );
                    }
                }),
            ));
        }
        "picker" => {
            let global_history_before = Rc::new(Cell::new(0_usize));
            let global_capture = global_history_before.clone();
            let state_capture = state.clone();
            steps.push(step(
                "picker-history",
                "capture-global-history",
                serde_json::Value::Null,
                Box::new(move || {
                    global_capture.set(state_capture.borrow().creative_history.undo.len())
                }),
            ));
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
                        ColorModel::default(),
                        PickerPurpose::Target,
                    )
                }),
            ));
            let ui_contract = ui.clone();
            let log_contract = log.clone();
            steps.push(step(
                "color-picker",
                "assert-window-contract",
                serde_json::Value::Null,
                Box::new(move || {
                    let valid = ui_contract
                        .picker_dialog
                        .borrow()
                        .as_ref()
                        .is_some_and(|dialog| {
                            is_separate_modal_transient(dialog, &ui_contract.window)
                                && dialog.height() <= 600
                        });
                    let focus_ready = ui_contract
                        .audit_picker_plane
                        .borrow()
                        .as_ref()
                        .is_some_and(|plane| {
                            plane.is_focusable() && plane.has_css_class(CREATIVE_FOCUS_CLASS)
                        });
                    if !valid || !focus_ready {
                        log_contract.event(
                            "fail",
                            "color-picker",
                            "window-contract",
                            serde_json::json!({"window": valid, "focus": focus_ready}),
                        );
                    }
                }),
            ));
            let ui_history = ui.clone();
            let log_history = log.clone();
            steps.push(step(
                "picker-history",
                "assert-initial-local-and-global-controls",
                serde_json::Value::Null,
                Box::new(move || {
                    let local_disabled = ui_history
                        .audit_picker_undo
                        .borrow()
                        .as_ref()
                        .is_some_and(|button| !button.is_sensitive())
                        && ui_history
                            .audit_picker_redo
                            .borrow()
                            .as_ref()
                            .is_some_and(|button| !button.is_sensitive());
                    if !local_disabled
                        || ui_history.undo.is_sensitive()
                        || ui_history.redo.is_sensitive()
                    {
                        log_history.event(
                            "fail",
                            "picker-history",
                            "initial-control-state",
                            serde_json::json!({"local_disabled": local_disabled}),
                        );
                    }
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
                let samples: [[f64; 2]; 3] = [[20.0, 280.0], [20.0, 80.0], [25.0, 75.0]];
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
            let ui_history = ui.clone();
            let log_history = log.clone();
            steps.push(step(
                "picker-history",
                "undo-local-and-assert-redo",
                serde_json::Value::Null,
                Box::new(move || {
                    if let Some(undo) = ui_history.audit_picker_undo.borrow().clone() {
                        undo.emit_clicked();
                    }
                    let valid = ui_history
                        .audit_picker_redo
                        .borrow()
                        .as_ref()
                        .is_some_and(|button| button.is_sensitive());
                    if !valid {
                        log_history.event(
                            "fail",
                            "picker-history",
                            "local-redo-state",
                            serde_json::json!(false),
                        );
                    }
                }),
            ));
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
            let state_check = state.clone();
            let global_check = global_history_before;
            let log_check = log.clone();
            steps.push(step(
                "picker-history",
                "assert-cancel-leaks-no-global-entry",
                serde_json::Value::Null,
                Box::new(move || {
                    let after = state_check.borrow().creative_history.undo.len();
                    if after != global_check.get() {
                        log_check.event(
                            "fail",
                            "picker-history",
                            "cancel-global-history",
                            serde_json::json!({"before": global_check.get(), "after": after}),
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
                        ColorModel::Okhsl,
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
        CANVAS_NATURAL_HEIGHT, CANVAS_NATURAL_WIDTH, CREATIVE_FOCUS_CLASS, CanvasSiteAction, Cli,
        CompareMode, CreativeHistory, CreativeSnapshot, Method, PickerGesture, PickerLocalHistory,
        VORONOI_MATCHING_LABELS, accessible_site_label, application_flags, canvas_site_action,
        contextual_chrome, divider_from_canvas_x, marker_hit_test, parse_threshold_edit_target,
        parse_threshold_handle, picker_lightness_sequence, picker_plane_encoded_sample,
        picker_plane_physical_size, record_coalesced_snapshot, resize_threshold_component,
        selected_user_preset_index, set_threshold_component_enabled, show_voronoi_markers,
        site_label, split_divider_hit, threshold_component_summary, threshold_dialog_dirty,
        threshold_link_editor_target, threshold_plot_accessibility_description,
        threshold_precision_accessibility_text, threshold_sync_copy_label,
        threshold_sync_presentation, unified_preset_labels, visible_voronoi_matching_at,
        visible_voronoi_matching_index,
    };
    use threshiator::{
        color::{ColorModel, DraftColor},
        document::{LinkPolicy, ThresholdEditTarget, ThresholdSpace, VoronoiMatching},
    };

    #[test]
    fn threshold_plot_accessibility_names_rgb_histogram_series_without_color_only_cues() {
        let mut threshold = threshiator::document::ThresholdState::default();
        threshold.rgb_state.link = LinkPolicy::Independent;
        assert!(
            threshold_plot_accessibility_description(&threshold, ThresholdEditTarget::Green, None,)
                .contains("selected Green dashed source distribution")
        );
        threshold.rgb_state.link = LinkPolicy::Linked;
        let individual =
            threshold_plot_accessibility_description(&threshold, ThresholdEditTarget::Red, None);
        assert!(individual.contains("selected Red solid source distribution"));
        assert!(!individual.contains("Green dashed"));
        let linked = threshold_plot_accessibility_description(
            &threshold,
            ThresholdEditTarget::LinkedRgb,
            None,
        );
        assert!(linked.contains("Red solid, Green dashed, and Blue dotted"));
        assert!(linked.contains("Red is the editing anchor"));
        assert!(linked.contains("before Smooth"));
    }

    #[test]
    fn threshold_inspector_summary_is_compact_and_names_bypass_and_lock() {
        let mut quantizer = threshiator::document::ComponentQuantizer::evenly_spaced(3);
        assert_eq!(
            threshold_component_summary(&quantizer, false),
            "3 bands · Processing · Unlocked"
        );
        quantizer.enabled = false;
        assert_eq!(
            threshold_component_summary(&quantizer, true),
            "3 bands · Bypassed · Locked"
        );
    }

    #[test]
    fn threshold_link_editor_only_exists_for_a_semantic_linked_target() {
        let mut threshold = threshiator::document::ThresholdState::default();
        assert_eq!(
            threshold_link_editor_target(&threshold),
            Some(ThresholdEditTarget::LinkedRgb)
        );
        threshold.rgb_state.link = LinkPolicy::Independent;
        assert_eq!(threshold_link_editor_target(&threshold), None);
        threshold.active_space = ThresholdSpace::Hsv;
        threshold.hsv_state.sv_link = LinkPolicy::Linked;
        assert_eq!(
            threshold_link_editor_target(&threshold),
            Some(ThresholdEditTarget::LinkedSaturationValue)
        );
        threshold.hsv_state.sv_link = LinkPolicy::Independent;
        assert_eq!(threshold_link_editor_target(&threshold), None);
    }

    #[test]
    fn threshold_inspector_direct_edits_preserve_process_and_respect_lock_and_link() {
        let mut threshold = threshiator::document::ThresholdState::default();
        assert!(set_threshold_component_enabled(
            &mut threshold,
            ThresholdEditTarget::Green,
            false
        ));
        threshold.set_locked(ThresholdEditTarget::Blue, true);
        assert!(resize_threshold_component(
            &mut threshold,
            ThresholdEditTarget::Red,
            7
        ));
        assert_eq!(threshold.rgb_state.components[0].outputs.len(), 7);
        assert_eq!(threshold.rgb_state.components[1].outputs.len(), 7);
        assert_eq!(threshold.rgb_state.components[2].outputs.len(), 3);
        assert!(threshold.rgb_state.components[0].enabled);
        assert!(!threshold.rgb_state.components[1].enabled);
        assert!(threshold.rgb_state.components[2].enabled);
        assert!(!resize_threshold_component(
            &mut threshold,
            ThresholdEditTarget::Blue,
            9
        ));
    }

    #[test]
    fn threshold_plot_accessibility_names_hsv_seam_series_and_strips() {
        let mut threshold = threshiator::document::ThresholdState {
            active_space: threshiator::document::ThresholdSpace::Hsv,
            ..Default::default()
        };
        threshold.hsv_state.hue_origin_degrees = 30.0;
        let hue =
            threshold_plot_accessibility_description(&threshold, ThresholdEditTarget::Hue, None);
        assert!(hue.contains("30.0 degree 0/360 seam"));
        assert!(hue.contains("circular input and mapped absolute-Hue output strips"));

        threshold.hsv_state.sv_link = LinkPolicy::Linked;
        let saturation = threshold_plot_accessibility_description(
            &threshold,
            ThresholdEditTarget::Saturation,
            None,
        );
        assert!(saturation.contains("Saturation source density"));
        assert!(!saturation.contains("Value is dotted"));
        let linked = threshold_plot_accessibility_description(
            &threshold,
            ThresholdEditTarget::LinkedSaturationValue,
            None,
        );
        assert!(linked.contains("Linked Saturation is dashed and Value is dotted"));
        assert!(linked.contains("Saturation is the editing anchor"));
        assert!(linked.contains("neither series is combined"));
        assert!(linked.contains("Separate labeled Saturation and Value input and output strips"));
    }

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
    fn unified_preset_labels_keep_fixed_builtins_and_engine_suffixes() {
        let recipe = threshiator::document::Recipe {
            active_method: threshiator::document::Method::Voronoi,
            ..threshiator::document::Recipe::default()
        };
        let user = threshiator::preset::PresetEntry {
            path: std::path::PathBuf::from("user.json"),
            preset: threshiator::preset::Preset::new("My Look", None, &recipe).unwrap(),
        };
        assert_eq!(
            unified_preset_labels(&[user]),
            [
                "Presets…",
                "Comic Book — Threshold",
                "Duotone Blue — Threshold",
                "Vintage Photo — Threshold",
                "Noir — Threshold",
                "Pop Art — Threshold",
                "Hue Poster — Threshold",
                "Neon Shadows — Threshold",
                "Pastel Bands — Threshold",
                "Ink & Paper — Voronoi",
                "Desert Dusk — Voronoi",
                "Blueprint — Voronoi",
                "Arcade Four — Voronoi",
                "Night Neon — Voronoi",
                "My Look — Voronoi",
            ]
        );
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
    fn document_history_savepoint_survives_edits_and_save() {
        let saved = threshiator::document::Recipe::default();
        let mut history = CreativeHistory::default();
        history.initialize(&saved);
        assert!(!history.is_dirty(&saved));
        let mut edited = saved.clone();
        edited.threshold.input_smoothing = 3.0;
        assert!(history.is_dirty(&edited));
        history.undo.push(CreativeSnapshot {
            recipe: saved.clone(),
            selected_group: Some(1),
            selected_sample: Some(1),
            expanded_site: Some(1),
        });
        history.mark_saved(&edited);
        assert!(!history.is_dirty(&edited));
        assert!(history.is_dirty(&saved));
        assert_eq!(history.undo.len(), 1, "save preserves document history");
    }

    #[test]
    fn document_key_repeat_coalesces_until_release() {
        let a = threshiator::document::Recipe::default();
        let mut b = a.clone();
        b.threshold.input_smoothing = 1.0;
        let mut c = b.clone();
        c.threshold.input_smoothing = 2.0;
        let mut d = c.clone();
        d.threshold.input_smoothing = 3.0;
        let snapshot = |recipe| CreativeSnapshot {
            recipe,
            selected_group: None,
            selected_sample: None,
            expanded_site: None,
        };
        let mut history = CreativeHistory::default();
        history.initialize(&a);
        let mut active = None;
        record_coalesced_snapshot(
            &mut history,
            &mut active,
            snapshot(a.clone()),
            &b,
            Some(super::CoalescedEdit::Smoothing),
        );
        record_coalesced_snapshot(
            &mut history,
            &mut active,
            snapshot(b.clone()),
            &c,
            Some(super::CoalescedEdit::Smoothing),
        );
        assert_eq!(history.undo.len(), 1, "key repeat stays in one gesture");
        assert_eq!(history.undo[0].recipe, a);

        active = None; // Key release ends the first gesture.
        record_coalesced_snapshot(
            &mut history,
            &mut active,
            snapshot(c.clone()),
            &d,
            Some(super::CoalescedEdit::Smoothing),
        );
        assert_eq!(history.undo.len(), 2);
        assert_eq!(history.undo[0].recipe, a);
        assert_eq!(history.undo[1].recipe, c);
    }

    #[test]
    fn threshold_dialog_return_to_opening_restores_prior_dirty_state() {
        let opening = threshiator::document::ThresholdState::default();
        let mut edited = opening.clone();
        edited.rgb_state.link = LinkPolicy::Independent;
        assert!(!threshold_dialog_dirty(false, &opening, &opening));
        assert!(threshold_dialog_dirty(false, &opening, &edited));
        assert!(threshold_dialog_dirty(true, &opening, &opening));
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
    fn canvas_click_selects_markers_adds_empty_voronoi_space_and_ignores_thresholds() {
        assert_eq!(
            canvas_site_action(Method::Voronoi, Some(7)),
            CanvasSiteAction::Select(7)
        );
        assert_eq!(
            canvas_site_action(Method::Voronoi, None),
            CanvasSiteAction::Add
        );
        assert_eq!(
            canvas_site_action(Method::Thresholds, None),
            CanvasSiteAction::Ignore
        );
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
            "S + V linked"
        );
        assert!(parse_threshold_edit_target("dropdown-index-2".into()).is_none());
        assert_eq!(
            parse_threshold_handle("boundary:1".into()),
            Some(super::ThresholdHandle::Boundary(0))
        );
        assert!(parse_threshold_handle("boundary:0".into()).is_none());
    }

    #[test]
    fn hue_output_accessibility_is_explicitly_relative_to_origin() {
        let (_, description) = threshold_precision_accessibility_text(
            threshiator::document::ThresholdEditTarget::Hue,
            super::ThresholdHandle::Output(0),
            false,
        );
        assert!(description.contains("0 to 360"));
        assert!(description.contains("Relative degrees"));
        assert!(description.contains("Hue origin"));
        assert!(!description.contains("0 to 1"));

        let (_, normalized) = threshold_precision_accessibility_text(
            threshiator::document::ThresholdEditTarget::Saturation,
            super::ThresholdHandle::Output(0),
            false,
        );
        assert!(normalized.contains("0 to 1"));
        assert!(!normalized.contains("0 to 360"));
    }

    #[test]
    fn sync_copy_labels_name_source_and_destinations() {
        assert_eq!(
            threshold_sync_copy_label(ThresholdEditTarget::Red),
            "Copy Red to Green and Blue"
        );
        assert_eq!(
            threshold_sync_copy_label(ThresholdEditTarget::Saturation),
            "Copy Saturation to Value"
        );
        assert_eq!(
            threshold_sync_copy_label(ThresholdEditTarget::Hue),
            "Hue has no synchronization peer"
        );

        let mut threshold = threshiator::document::ThresholdState::default();
        let unlocked = threshold_sync_presentation(&threshold, ThresholdEditTarget::Red);
        assert_eq!(unlocked.markup, "Copy Red to Green and Blue");
        assert!(!unlocked.markup.contains("strikethrough"));
        assert!(
            unlocked
                .accessible_description
                .contains("Locked destinations skipped: none")
        );

        threshold.set_locked(ThresholdEditTarget::Green, true);
        let presentation = threshold_sync_presentation(&threshold, ThresholdEditTarget::Red);
        assert_eq!(
            presentation.markup,
            "Copy Red to <span strikethrough=\"true\">Green</span> and Blue"
        );
        assert!(presentation.accessible_label.contains("Green"));
        assert!(presentation.accessible_label.contains("locked"));
        assert!(
            presentation
                .accessible_description
                .contains("Writable destinations: Blue")
        );
        assert!(
            presentation
                .accessible_description
                .contains("Locked destinations skipped: Green")
        );

        threshold.set_locked(ThresholdEditTarget::Blue, true);
        let all_locked = threshold_sync_presentation(&threshold, ThresholdEditTarget::Red);
        assert!(
            all_locked
                .markup
                .contains("<span strikethrough=\"true\">Blue</span>")
        );
        assert!(
            all_locked
                .accessible_description
                .contains("Writable destinations: none")
        );

        threshold.set_locked(ThresholdEditTarget::Value, true);
        let hsv = threshold_sync_presentation(&threshold, ThresholdEditTarget::Saturation);
        assert_eq!(
            hsv.markup,
            "Copy Saturation to <span strikethrough=\"true\">Value</span>"
        );
        assert!(hsv.accessible_label.contains("Value"));
        assert!(hsv.accessible_label.contains("locked"));
    }
}
