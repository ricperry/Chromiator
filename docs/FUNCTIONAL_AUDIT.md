# Voronoi refactor verification ledger

## Status

Final GTK4 conversion: **partially verified, with an open layout regression**.
The follow-up fixes passed the application build, strict Clippy, and 93 tests.
Bounded private-Sway checks verified save/history/picker fixes; THR-065 remains open;
see `BUG_HUNT_FOLLOWUP_2026-09-19.md`. Do not infer complete workflow success from those
checks or merely implemented audit routes. No performance improvement is claimed.

## Completed earlier checkpoints

- Voronoi-only checkpoint: 83 tests passed; strict all-target/all-feature Clippy
  passed. Removed CLI mode flags returned actionable errors.
- Exact parity: eight cases covering Perceptual/OKHSL/RGB/HSV and smoothing
  amounts 0/1.25 matched baseline pixel `f32` bits and integer site coverage.
  Inputs included transparency, weighted sites, and ordered hue operations.
- Intermediate native smoke: Spectrum Example, smoothing edit/readback, and
  selected-site preservation across preview refresh; clean stderr. Screenshot:
  `.codex-work/evidence/ui-run-20260913-133351-65895/milestone2-voronoi-clean.png`.
  This screenshot predates the pure-GTK4 shell.
- Architecture checkpoint before final shell edits: 88 tests passed (7 library,
  12 binary, 69 core), including same-value session edits producing no history.

Local oracle artifacts are under `target/validation/refactor/`; the retained
fixture is `tests/fixtures/refactor-voronoi-baseline.txt`. Tiny oracle timings
are correctness evidence only, not performance measurements.

## Final checks still required

1. Build all application targets; format/lint and run retained tests. Confirm
   GResource XML, GTK API use, and native-dialog lifetimes work in the final tree.
2. Repeat exact pixel/coverage parity after the compiled-partition changes.
3. Run all six native audit routes: shell, voronoi, picker, presets, io, and
   responsive. Implemented action/assert bodies are not evidence of a pass.
   The io route exercises job begin/cancel/ack locking, not a file chooser;
   test actual file-dialog and filesystem workflows separately.
4. Exercise matching controls, smoothing, independent Source/Target picker
   destinations, influence, locks, footprints, and stable site selection.
5. Exercise history coalescing, no-op edits, dirty/savepoints, undo branching,
   picker-local undo/redo, one-transaction commit, cancel, and window close.
6. Exercise preset auto-apply/no-op behavior, stable user-preset identity,
   incompatible-file errors, and unchanged rejected files.
7. Exercise modal exclusion, cancellation acknowledgement, stale preview
   rejection, monotonic progress, full-resolution export, and atomic writes.
8. Inspect desktop, 1024x600, and narrow layouts in light and dark themes.
   Check allocated geometry, scrolling, hidden inspector, focus, keyboard paths,
   canvas add/drag gestures, and Source/Result comparison. Arbitrary live resize
   does not yet automatically collapse the inspector; manual toggling and
   narrow automation are present but unverified.
9. Measure release preview and 3840x2160 processing on representative images,
   with repeated warm medians, peak RSS, cancellation latency, and exact output.
   Adopt bounded Rayon only if measurements support it; no speculative
   parallel-processing dependency is required for the refactor.

Use `.agents/skills/gtk-wayland-debug/SKILL.md`: wait, scoped controls, inspect,
semantic action, readback. Use coordinates only for spatial canvas interactions.
Retain raw logs, screenshots, and output artifacts; stop the private session
when finished. Native automation does not replace human GNOME acceptance.
