# Transition implementation: verified, awaiting artistic acceptance

Current status: the solver and precise-float parsing corrections below were
approved and implemented. Their tests pass. See
`TRANSITION_SOLVER_CORRECTION.md` for superseding test, rendering, performance,
native history, save/reopen, and export evidence. Native blend-space and scope
activation and their Undo/Redo checks now pass after the test-harness correction.
The earlier failures and pending-approval notes below are historical evidence,
not the current solver or serialization status.

This is not a completion or acceptance record. THR-061 remains active.

## Implemented

- Global width 0-100%, default 0; Oklab/Linear RGB and Multisite/Shared borders.
- Typed document-session edits, continuous width coalescing, control/history sync.
- Current-format defaulted persistence; no migration of legacy user presets.
- Compact-support multi-site blending, weighted contact geometry, continuous
  neighbor restriction, bounded Oklab gamut mapping, preserved alpha and coverage.
- Existing hard renderer bypass, cancellable graph construction, reusable scratch.
- User CSS and deferred layout issue THR-065 left unchanged.

## Initial evidence (2026-09-19)

`cargo test --all-targets --all-features`: 105 passed, one failed, one ignored.
The failure is `current_hard_files_default_options_and_soft_profiles_round_trip`:
transition fields are exact, but existing f64 site-position values change by a
last-place bit through JSON decoding. Approval was requested to enable serde_json
precise float-round-trip parsing rather than weaken the whole-recipe assertion.

`cargo clippy --all-targets --all-features -- -D warnings` and
`cargo build --bin chromiator` passed after two index-loop style corrections.

The opt-in `render_transition_evidence` test passed in release mode and wrote
`target/validation/transitions/`: twelve scope/space/width comparison PNGs,
the source ramp, and a native project fixture. One-run five-site 256x256 times
were approximately 2.4-3.4 ms hard, 5.5-6.3 ms Multisite, and 8.4-10.5 ms Shared
borders. These are preliminary fixture timings, not representative performance
acceptance. Subsequent repeated larger-image/high-site-count timing and rendering
cancellation checks passed; see `TRANSITION_PERFORMANCE_2026-09-19.md`. Those results
still precede the pending geometry correction.

Inspected full-width Oklab fixture images exposed a geometry defect: the fallback
for a positive-dimensional four-site equality locus admits two occluded diagonals
in a coplanar five-site palette, making the scope results identical. Independent
mathematical review confirmed that this fallback is unnecessary: dependent rows
cannot cut their common locus, and all independent alternatives are already
enumerated among subsets of two through four sites. Remove only this fallback;
retain conservative handling for genuinely ill-conditioned numerical predicates.
Correction and a flat-palette regression were requested for approval.

Private Sway evidence:
`.codex-work/evidence/ui-run-20260919-101742-34052/`.
The width control initially read 0 with blend selectors disabled; setting 25
enabled them and visibly softened the Spectrum Example. Shared borders selection
read back successfully. Linear RGB dropdown automation failed to commit through
AT-SPI, consistent with the harness's documented retained-VNC-focus limitation;
do not count native color-space selection or subsequent Undo as verified.
The application error log was empty. Evidence was collected and Sway stopped.

## Remaining acceptance work

- Resolve the two reported blockers with explicit approval and rerun full checks.
- Confirm distinct scopes on occluded-neighbor fixtures and smooth 3D junctions.
- Finish native selector, Undo/Redo, width-zero restoration, save/reopen and export
  checks with semantic readback and inspected screenshots/output artifacts.
- Inspect both color spaces at 0%, 25% and 100% on fixtures and artwork.
- Repeat affected workload timings after correction and verify cancellation during
  contact construction, in addition to the already verified row-render cancellation.
- Record final evidence, reconcile documentation, and request visual acceptance.

No GNOME/Mutter human acceptance or complete-feature claim is implied.
