# Functional audit

The GTK signal-path audit is the primary stability evidence for this milestone. Earlier screenshot-only checks proved layout and static state, but they did not exercise callbacks and therefore missed a self-replacing dropdown model that could freeze the application.

Run the complete isolated audit with:

```sh
python3 scripts/run_ui_audit.py
```

Each scenario runs in a fresh native process. The external supervisor watches JSONL heartbeat progress, kills a stale or over-budget process, records the last begun action, and continues with the remaining scenarios. Logs and the machine-readable summary are written under `tests/artifacts/audit/`.

## Exercised controls

| Scenario | Native signal paths exercised | Result |
| --- | --- | --- |
| Shell | Result, Split, Source; common Hue samples; both Methods | Pass |
| Threshold inspector | In both RGB and HSV: Link on/off; Process/Bypass for all components; Bands 2, 8, 32 | Pass |
| Threshold dialog | Open; actual Hue to linked Saturation & Value `selected-notify` in both directions 20 times; precise values; Done; edit then Cancel with rollback assertion; normal close request; reopen | Pass |
| Voronoi | all matching metrics; Influence low/high; all sampling footprints; site lock/unlock | Pass |
| Color picker | HSV, HSL, OKLab with two samples for every channel in each model; valid-invalid-valid Hex activation; Select, Cancel, normal close request, reopen | Pass; missing RGB logged as a THR-013 skip |
| I/O workflow | creative-dialog conflict rejection; workspace lock; visible/sensitive Cancel click; cancellation and finish restoration; second-job rejection | Pass |
| Narrow core | shell and method changes at 720 by 700 | Pass |

The current summary records 7 passing scenarios and 0 failing scenarios. Skip events are explicit coverage metadata, not passes substituted for interactions. Any JSONL `phase=fail`, malformed nonblank JSONL line, missing required widget, panic, GTK/GDK/GLib/GObject critical, or toolkit runtime warning makes its process fail even if the scenario reaches its completion marker. The summary exposes malformed-line counts and matched fatal diagnostics per scenario.

## Diagnosed P0 failures

- The Threshold target dropdown rebuilt and reselected its own `GtkStringList` from inside `selected-notify`. It now owns one stable model; selection changes update the session and render state without recreating the model. The exact formerly freezing path completes 40 alternating notifications with a live 100 ms heartbeat.
- Voronoi now selects one independent site row directly; no nested Source-sample dropdown or callback-driven model recreation remains in the visible workflow.
- Save/open/export could own a stale document snapshot while creative controls remained editable. A running I/O job now disables the entire document workspace while keeping footer Cancel available. Starting a job while a creative modal is open is rejected, avoiding an insensitive modal above an unreachable Cancel button. Busy second-job starts are rejected instead of panicking.

## Limited or manual coverage

- Native portal chooser completion, permission denial, and overwrite confirmation require desktop integration testing with the real portal.
- Actual pointer dragging, canvas sampling, focus/hover behavior, and compositor-specific gestures are not synthesized by this runner. The runner invokes the same GTK widget methods and signals but does not use remote desktop or mouse automation.
- Add Site placement, destructive confirmation responses, real marker dragging, file destinations, and export file inspection retain focused backend/unit coverage and need a manual release checklist.
- RGB is not currently offered by the custom picker. The audit records this as a deliberate skip tied to THR-013 rather than silently treating HSV/HSL/OKLab coverage as RGB coverage.
- The current audit checks main-loop heartbeat continuity and GTK/panic diagnostics. It does not yet measure every preview generation against a per-control oracle.

## Redesign sequence

1. Stability and recoverability: finish signal-loop, job ownership, undo, and dialog lifetime work.
2. Document shell and feedback: unify Open, document identity, dirty/progress/errors, and job state.
3. Voronoi palette workflow: continue responsive polish around the independent site list, Source/Target editors, lock, and explicit sampling-tool state.
4. Threshold workflow: make Bands changes safe and reversible, then add direct histogram/Boundary feedback.
5. Finishing: reorganize common pipeline/status controls, make Export creator-first, add in-canvas comparison, and complete the picker and responsive pass.

This ordering deliberately treats the current UI as a functional framework, not a finished interaction design.
