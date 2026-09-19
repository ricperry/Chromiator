//! Transitional UI-audit driver isolated from application state and worker orchestration.

use std::rc::Weak;
use std::time::Instant;

use super::*;

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
                    "dirty": state.session.document().is_some_and(|document| document.dirty),
                    "dialog_state": {
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

pub(super) fn maybe_start_ui_audit(ui: &Rc<Ui>, state: &Rc<RefCell<State>>) {
    let Some(scenario) = ui.cli.ui_audit_scenario.clone() else {
        return;
    };
    if ui.audit_started.get()
        || state.borrow().session.document().is_none()
        || state.borrow().jobs.is_busy()
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
    if !["shell", "voronoi", "picker", "presets", "io", "responsive"]
        .contains(&scenario.as_str())
    {
        log.event(
            "fail",
            "scenario",
            "unknown-scenario",
            serde_json::json!({
                "scenario": scenario,
                "reason": "scenario is not part of the retained native GTK audit contract"
            }),
        );
        let window = ui.window.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(80), move || {
            if let Some(app) = window.application() {
                app.quit();
            }
        });
        return;
    }

    let initial_generation = state.borrow().scheduler.current_generation();
    let initial_history = state.borrow().session.undo_len();
    let action_ui = ui.clone();
    let action_state = state.clone();
    let action_log = log.clone();
    let action_scenario = scenario.clone();
    glib::timeout_add_local_once(std::time::Duration::from_millis(100), move || {
        let action_ok = if action_scenario == "shell" {
            action_ui.source_mode.set_active(true);
            let source = action_state.borrow().mode == CompareMode::Source;
            action_ui.split_mode.set_active(true);
            let split = action_state.borrow().mode == CompareMode::Split;
            action_ui.result_mode.set_active(true);
            source && split && action_state.borrow().mode == CompareMode::Result
        } else if action_scenario == "voronoi" {
            action_ui.voronoi_matching.set_selected(1);
            action_ui.voronoi_matching.set_selected(2);
            let no_op_generation = action_state.borrow().scheduler.current_generation();
            let no_op_history = action_state.borrow().session.undo_len();
            action_ui.voronoi_matching.set_selected(2);
            let no_op = action_state.borrow().scheduler.current_generation() == no_op_generation
                && action_state.borrow().session.undo_len() == no_op_history;
            let adjusted = action_ui
                .audit_site_influence
                .borrow()
                .clone()
                .is_some_and(|influence| {
                    let previous = influence.value();
                    influence.set_value(if previous < 3.9 {
                        previous + 0.1
                    } else {
                        previous - 0.1
                    });
                    influence.value() != previous
                });
            let expanded = action_ui
                .audit_other_site_expander
                .borrow()
                .clone()
                .is_some_and(|row| {
                    row.set_expanded(true);
                    row.is_expanded()
                });
            no_op && adjusted && expanded
        } else if action_scenario == "picker" {
            let selected_before = action_state.borrow().session.selection().selected;
            let colors_before = action_state.borrow().session.document().and_then(|document| {
                let site = document.recipe.voronoi.site(selected_before?)?;
                Some((site.source_color, site.target_color))
            });
            let cancelled_draft = action_ui
                .audit_site_source
                .borrow()
                .clone()
                .is_some_and(|button| {
                    button.emit_clicked();
                    if let Some(entry) = action_ui.audit_picker_hex.borrow().clone() {
                        entry.set_text("#E85D04");
                        entry.emit_activate();
                    }
                    if let Some(cancel) = action_ui.audit_picker_cancel.borrow().clone() {
                        cancel.emit_clicked();
                    }
                    action_state.borrow().session.undo_len() == initial_history
                });
            let opened = action_ui
                .audit_site_target
                .borrow()
                .clone()
                .is_some_and(|button| {
                    button.emit_clicked();
                    action_state.borrow().picker_visible
                });
            let edited = action_ui.audit_picker_hex.borrow().clone().is_some_and(|entry| {
                entry.set_text("#2F80ED");
                entry.emit_activate();
                true
            });
            let committed = action_ui
                .audit_picker_select
                .borrow()
                .clone()
                .is_some_and(|button| {
                    button.emit_clicked();
                    true
                });
            let target_only = action_state.borrow().session.document().is_some_and(|document| {
                let Some(site) = selected_before.and_then(|id| document.recipe.voronoi.site(id))
                else {
                    return false;
                };
                colors_before.is_some_and(|(source, target)| {
                    site.source_color == source && site.target_color != target
                })
            });
            cancelled_draft
                && opened
                && edited
                && committed
                && target_only
                && action_state.borrow().session.selection().selected == selected_before
        } else if action_scenario == "presets" {
            let applied = action_ui.preset_model.n_items() > 1;
            if applied {
                action_ui.preset_dropdown.set_selected(1);
                action_ui.preset_save.emit_clicked();
                if let Some(cancel) = action_ui.audit_preset_cancel.borrow().clone() {
                    cancel.emit_clicked();
                }
            }
            applied
        } else if action_scenario == "responsive" {
            action_ui.window.set_default_size(700, 700);
            action_ui.sidebar_button.set_active(false);
            !action_ui.inspector_scroll.is_visible()
        } else {
            let Some((_, token)) = begin_job(
                &action_ui,
                &action_state,
                "Audit cancellable file operation…",
            ) else {
                return;
            };
            action_ui.cancel.emit_clicked();
            let cancelled = !token.is_current();
            let acknowledged = action_state
                .borrow_mut()
                .jobs
                .acknowledge(token.generation());
            finish_job(&action_ui, &action_state);
            cancelled && acknowledged
        };
        action_log.event(
            if action_ok { "action" } else { "fail" },
            "retained-controls",
            "exercise",
            serde_json::json!({"success": action_ok}),
        );
    });

    let assert_ui = ui.clone();
    let assert_state = state.clone();
    let assert_log = log.clone();
    let assert_scenario = scenario.clone();
    let window = ui.window.clone();
    glib::timeout_add_local_once(std::time::Duration::from_millis(450), move || {
        let current = assert_state.borrow();
        let common = current.session.document().is_some()
            && !current.jobs.is_busy()
            && !current.picker_visible
            && assert_ui.result_mode.is_sensitive()
            && assert_ui.source_mode.is_sensitive()
            && assert_ui.split_mode.is_sensitive();
        let scenario_ok = if assert_scenario == "shell" {
            current.mode == CompareMode::Result
                && assert_ui.result_mode.is_active()
                && assert_ui.window.width() > 0
                && assert_ui.window.height() > 0
        } else if assert_scenario == "voronoi" {
            current.session.document().is_some_and(|document| {
                document.recipe.voronoi.matching == VoronoiMatching::Hsv
                    && document.recipe.voronoi.transition
                        == chromiator::document::TransitionProfile::HARD
                    && current.session.selection().selected == current.session.selection().expanded
                    && current.session.selection().expanded.is_some()
                    && document
                        .recipe
                        .voronoi
                        .sites
                        .iter()
                        .any(|site| site.source_color[..3] != site.target_color)
            }) && current.scheduler.current_generation() > initial_generation
                && current.session.undo_len() > initial_history
                && assert_ui.audit_site_source.borrow().is_some()
                && assert_ui.audit_site_target.borrow().is_some()
        } else if assert_scenario == "picker" {
            !current.picker_visible
                && current.session.undo_len() == initial_history + 1
                && current.scheduler.current_generation() > initial_generation
                && current.session.selection().selected.is_some()
        } else if assert_scenario == "presets" {
            !current.picker_visible
                && assert_ui.audit_preset_name.borrow().is_none()
                && current.session.undo_len() == initial_history + 1
                && current.scheduler.current_generation() > initial_generation
        } else if assert_scenario == "responsive" {
            !assert_ui.inspector_scroll.is_visible()
                && assert_ui.canvas.width() > 0
                && assert_ui.canvas.height() > 0
                && assert_ui.sidebar_button.is_sensitive()
        } else {
            !current.jobs.is_busy()
                && current.session.undo_len() == initial_history
                && current.scheduler.current_generation() == initial_generation
                && assert_ui.open.is_sensitive()
                && assert_ui.save_as.is_sensitive()
                && assert_ui.export.is_sensitive()
        };
        let valid = common && scenario_ok;
        assert_log.event(
            if valid { "assert" } else { "fail" },
            "retained-controls",
            "readback",
            serde_json::json!({
                "valid": valid,
                "matching": current.session.document().map(|document| format!("{:?}", document.recipe.voronoi.matching)),
                "generation_delta": current.scheduler.current_generation().saturating_sub(initial_generation),
                "history_delta": current.session.undo_len().saturating_sub(initial_history),
                "expanded_site": current.session.selection().expanded,
                "window": [assert_ui.window.width(), assert_ui.window.height()],
            }),
        );
        drop(current);
        log.event("settled", "scenario", "complete", serde_json::Value::Null);
        if let Some(app) = window.application() {
            app.quit();
        }
    });
}
