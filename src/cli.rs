//! Command-line admission for automation and developer-facing launch options.
//!
//! Parsing is independent of GTK so removed options, stable site aliases, and
//! recipe overrides can be tested without constructing an application.

use std::path::PathBuf;

use crate::color::ColorModel;
use crate::document::{Recipe, VoronoiMatching};

/// Comparison surface requested at startup.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CompareMode {
    Result,
    Split,
    Source,
}

/// One-shot sampling interaction requested by UI automation.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SamplingState {
    AddColor,
    AddSample,
}

/// Validated startup and UI-audit options.
#[derive(Clone, Default)]
pub struct Cli {
    pub open: Option<PathBuf>,
    pub example: bool,
    pub invalid_argument: Option<String>,
    pub view: Option<CompareMode>,
    pub window_size: Option<(i32, i32)>,
    pub select_site: Option<usize>,
    pub lock_site: Option<usize>,
    pub sampling: Option<SamplingState>,
    pub hue: Option<f32>,
    pub show_color_picker: bool,
    pub color_model: ColorModel,
    pub picker_lightness: Option<f64>,
    pub voronoi_matching: Option<VoronoiMatching>,
    pub show_preset_dialog: bool,
    pub screenshot: Option<PathBuf>,
    pub quit_after_screenshot: bool,
    pub ui_audit_scenario: Option<String>,
    pub ui_audit_log: Option<PathBuf>,
}

impl Cli {
    /// Parses process arguments, rejecting every removed Thresholds option.
    pub fn parse() -> Self {
        Self::parse_from(std::env::args().skip(1))
    }

    /// Parses an explicit argument stream for deterministic tests.
    pub fn parse_from(arguments: impl IntoIterator<Item = String>) -> Self {
        let mut cli = Self::default();
        let mut args = arguments.into_iter();
        while let Some(argument) = args.next() {
            if argument == "--method"
                || argument.starts_with("--threshold")
                || argument == "--show-threshold-editor"
            {
                cli.invalid_argument = Some(format!(
                    "{argument} was removed with Thresholds mode; Chromiator now processes Voronoi color sites only"
                ));
                break;
            }
            match argument.as_str() {
                "--open" | "--project" => cli.open = args.next().map(PathBuf::from),
                "--example" => cli.example = true,
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
                    cli.select_site = value;
                }
                "--select-sample" => {
                    let value = args.next().and_then(|value| value.parse().ok());
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
                "--voronoi-matching" => {
                    cli.voronoi_matching = args.next().and_then(|value| match value.as_str() {
                        "perceptual" => Some(VoronoiMatching::Perceptual),
                        "okhsl" => Some(VoronoiMatching::Okhsl),
                        "rgb" => Some(VoronoiMatching::Rgb),
                        "hsv" => Some(VoronoiMatching::Hsv),
                        _ => None,
                    })
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
                            "rgb" => Some(ColorModel::Rgb),
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
                _ if argument.starts_with('-') => {
                    cli.invalid_argument = Some(format!("unknown option: {argument}"));
                    break;
                }
                _ => {}
            }
        }
        cli
    }
}

/// Applies CLI recipe overrides before a document enters a session.
pub fn apply_cli_recipe(cli: &Cli, recipe: &mut Recipe) {
    if let Some(matching) = cli.voronoi_matching {
        recipe.voronoi.matching = matching;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_selection_aliases_feed_the_single_site_authority() {
        for option in ["--select-group", "--select-sample", "--select-site"] {
            let cli = Cli::parse_from([option.to_owned(), "3".to_owned()]);
            assert_eq!(cli.select_site, Some(3));
        }
    }

    #[test]
    fn removed_threshold_options_are_actionable_errors() {
        let cli = Cli::parse_from(["--method".to_owned(), "threshold".to_owned()]);
        assert!(
            cli.invalid_argument
                .as_deref()
                .is_some_and(|message| message.contains("removed with Thresholds mode"))
        );
    }
}
