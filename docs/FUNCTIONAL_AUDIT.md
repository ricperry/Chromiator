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
| Shell | Result, Split, Source; normalized divider state; both Methods; compact Method/Presets disclosure; every top-level sidebar section collapses and re-expands; clean-open history controls; savepoint-preserving Undo/Redo action state and dirty transitions | Pass |
| Threshold inspector | native Threshold mappings hierarchy with persistent Working space, direct future-edits Link switch, conditional linked editor, and a single-open three-card component group; exact RGB/HSV labels, summaries, and focusable per-component Auto actions; Process component isolation; mapping-preserving Bands propagation; exact scalar Automatic baseline arrays with ordinary linked propagation to unlocked peers while preserving Process and locked peers; repeat Auto no-op; Lock gating Bands/Auto/Hue Origin but not Process or editor inspection; Hue Origin independence; exact component editor target and Auto mapping on reopen; coalesced history plus Auto undo/redo resynchronization; one-preview changed edits and no-preview Link/Lock/no-op transitions | Pass |
| Threshold dialog | Separate modal transient contract at no more than 600 px height; graph-first RGB source-density presentation with individual/linked target churn; visible standard focus treatment and local Undo/Redo sensitivity with parent-history isolation; clean edit→exact-opening→Done and edit→exact-opening→Cancel paths preserve dirty/history/preview state; Working space absent and inherited from the inspector; Link without copy/preview; exact Bands-stepper increase/decrease with one preview and local undo; Lock blocking Bands/precise/reset while Process remains usable; direction-named explicit copy with live strikethrough for locked destinations and one preview; Hue exclusion; nonzero Hue origin canonicalization with one preview and lock blocking; precise edits; Done as one global transaction; whole-state Cancel rollback; close/reopen | Pass |
| Voronoi | single-open per-site details accordion with selection/detail identity; creator-facing Perceptual (OKLab), RGB, and HSV matching; stable-ID Influence low/high and lock/unlock; rendered Source and Target swatches each prove exact picker title, initial color, and commit destination; all sampling footprints; direct empty-artwork site creation policy | Pass |
| Perceptual color pipeline | embedded RGB ICC conversion to canonical linear sRGB; native unscaled Cartesian OKLab distance; seam-safe canonical OKHSL cylinder with neutral Hue collapse; full-source automatic-site distribution; documented whole-metric Influence scale | Pass (core fixtures plus inspected representative GTK artifacts) |
| Unified presets | GTK model strings are machine-asserted as fixed-order built-ins first, then users, all with readable Threshold/Voronoi suffixes; neutral refresh cannot alias user preset index 0; selection auto-applies exactly once and remains selected until a divergent edit returns to Presets…; identical apply schedules none; preset and later Smoothing edits undo/redo independently; a new post-undo edit clears redo; guarded Save Current and reset controls remain | Pass |
| Color picker | separate modal transient surface at no more than 600 px height; standard focus treatment on the color plane; isolated visible local Undo/Redo with parent-history isolation and Cancel leakage check; pointer press/release and key boundaries split separate channel interactions while retaining focus-loss fallback; OKHSL default plus HSV and HSL alternatives with two samples for every channel in each model; continuous bounded-sRGB OKHSL disk; valid-invalid-valid Hex activation; Select, Cancel, normal close request, reopen | Pass; missing RGB logged as a THR-013 skip |
| I/O workflow | creative-dialog conflict rejection; workspace lock; visible/sensitive Cancel click; cancellation and finish restoration; second-job rejection | Pass |
| Responsive viewer toolbar | isolated Threshold and Voronoi runs at 720 by 700 and 1024 by 600; after the real inspector transition settles, root-relative allocation checks prove Result, Split, Source, the Smoothing label, and its spin are mapped in two end-aligned rows, mutually non-overlapping, inside the window, and entirely beyond the method-specific sidebar edge | Pass |
| Adaptive 1024 | exact 1024×600 application allocation; warning-free GTK/GDK/GLib/Adwaita diagnostics; contextual document chrome and feedback bar; Result/Split/Source/Smoothing visibility; focusable themed canvas | Pass |

The current summary records 13 passing scenarios and 0 failing scenarios. Skip events are explicit coverage metadata, not passes substituted for interactions. Any JSONL `phase=fail`, malformed nonblank JSONL line, missing required widget, panic, GTK/GDK/GLib/GObject critical, or toolkit runtime warning makes its process fail even if the scenario reaches its completion marker. The summary exposes malformed-line counts and matched fatal diagnostics per scenario.

## Diagnosed P0 failures

- The Threshold target dropdown rebuilt and reselected its own `GtkStringList` from inside `selected-notify`. It now owns one stable model; individual target changes update the whole-state dialog session without recreating the model. The exact formerly freezing path completes 40 alternating notifications with a live 100 ms heartbeat.
- The expanded Threshold audit exposed a nested `RefCell` borrow in the Add Band click path. The click handler now ends its immutable session borrow before invoking the safe-resize callback; the formerly aborting native signal path completes under the watchdog.
- Voronoi now selects one independent numbered site row directly. Its stable-ID details accordion owns Influence, Source position, Footprint, and Delete; no global details panel, Marker/ID block, nested Source-sample dropdown, or callback-driven model recreation remains in the visible workflow.
- Save/open/export could own a stale document snapshot while creative controls remained editable. A running I/O job now disables the entire document workspace while keeping footer Cancel available. Starting a job while a creative modal is open is rejected, avoiding an insensitive modal above an unreachable Cancel button. Busy second-job starts are rejected instead of panicking.

## Limited or manual coverage

- Native portal chooser completion, permission denial, and overwrite confirmation require desktop integration testing with the real portal.
- Actual pointer dragging, canvas sampling, focus/hover behavior, and compositor-specific gestures are not synthesized by this runner. Split geometry and state are unit/audit tested and artifacts prove the secondary slider is absent, but physically dragging its canvas grip remains a manual check. The runner verifies that each creative dialog has a distinct native surface with the main window as its modal transient parent, but moving that surface and clicking the blocked parent also remain manual GNOME/Wayland checks.
- Real pointer placement on empty artwork, destructive confirmation responses, real marker dragging, file destinations, and export file inspection retain focused backend/unit coverage and need a manual release checklist.
- RGB is not currently offered by the custom picker. The audit records this as a deliberate skip tied to THR-013 rather than silently treating OKHSL/HSV/HSL coverage as RGB coverage.
- The current audit checks main-loop heartbeat continuity and GTK/panic diagnostics. It does not yet measure every preview generation against a per-control oracle.

## Redesign sequence

1. Stability and recoverability: finish signal-loop, job ownership, undo, and dialog lifetime work.
2. Document shell and feedback: unify Open, document identity, dirty/progress/errors, and job state.
3. Voronoi palette workflow: continue responsive polish around the independent site list, direct Source/Target editors, lock, and explicit Reattach/Resample armed state.
4. Threshold workflow: add the direct histogram/Boundary feedback tracked by THR-015 now that Bands changes are safe, reversible, and transactionally scoped.
5. Finishing: make Export creator-first and complete the picker and responsive pass; compact pipeline/status ownership and in-canvas comparison are now established.

This ordering deliberately treats the current UI as a functional framework, not a finished interaction design.
