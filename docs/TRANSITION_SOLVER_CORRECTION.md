# Transition solver correction: 2026-09-19

This record supersedes the pending-approval solver and serialization blockers in
`TRANSITION_VERIFICATION.md`. Both corrections were explicitly approved and are
implemented. Implementation verification is complete; artistic user acceptance
and GNOME/Mutter acceptance remain separate from these private Sway checks.

## Changes

- Removed the overly broad rank-deficient contact fallback. Dependent equality
  constraints cannot cut an existing locus; independent subsets supply witnesses.
- Tightened geometry error handling, retained tiny positive intersection circles,
  and added flat-palette and clustered-site regression coverage.
- Enabled `serde_json/float_roundtrip`; exact project/preset recipe round trips pass.

## Completed verification

- `cargo test --all-targets --all-features`: 111 passed, three opt-in tests ignored.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo build --bin chromiator`: passed.
- Separately ran the release artifact generator and both release performance tests:
  all three passed. Construction-time cancellation has separate unit coverage.
- Inspected all twelve generated images: both blend spaces and scopes at widths
  0, 25, and 100 percent. Shared borders visibly differs from Multisite at wide
  transitions. Flat-palette non-neighbor exclusion is asserted numerically.

Rendered artifacts are in `target/validation/transitions/`. Native screenshot:
`.codex-work/evidence/ui-run-20260919-103500-43526/transitions-native.png`.
The native app loaded the saved 25-percent, Linear RGB, Shared borders recipe;
width readback and its rendered preview were inspected. Width zero disabled the
blend-space selector; Undo restored 25 percent and Redo restored zero.

In `.codex-work/evidence/ui-run-20260919-104150-46337`, width was changed to
50 percent and the project saved through the native Save button. The native
export precision dialog and file chooser produced
`target/validation/transitions/native-export-16.png`. The exported image was
inspected and identified as 1254x1254, 16-bit RGBA PNG.

The app was closed and the saved scratch project reopened in
`.codex-work/evidence/ui-run-20260919-104412-48143`. Width read back as 50 percent
and Save was disabled, confirming the restored clean savepoint.

The initial persistent-VNC harness attempt failed because generic combo-box
activation became ambiguous with an already-open popup. The approved correction
targets the actual toggle button, reads the expanded state, and retains one VNC
connection across popup reopening and Home/Down/Enter. Child-process diagnostics
are now surfaced rather than hidden behind an exit-code-only exception.

Final native evidence is in
`.codex-work/evidence/ui-run-20260919-105526-50437`:

- Linear RGB to Oklab and back: selected-item readback passed.
- Shared borders to Multisite and back: selected-item readback passed.
- Undo/Redo of both dropdown edits: selected-item readback passed.
- Both closed-popup and already-open-popup routes were exercised successfully.
- Waited for `preview current`, then captured and inspected
  `transitions-final.png`; earlier history screenshots retain their transient
  updating-preview status and are not used to prove preview completion.

Together with the earlier width, history, save/reopen, and native PNG 16-bit export
checks above, this closes the remaining transition native-verification blocker.
No application-code change was necessary for the dropdown automation failure.

## Post-correction performance

These results supersede the pre-correction measurements in
`TRANSITION_PERFORMANCE_2026-09-19.md`. Times include contact construction and
rendering, not GTK, decoding, or encoding; medians follow warmup and three runs.

| Sites / pixels | Blend space | Hard ms | Multisite ms | Shared borders ms |
| --- | --- | ---: | ---: | ---: |
| 5 / 1024x1024 | Oklab | 74.079 | 153.589 | 192.303 |
| 5 / 1024x1024 | Linear RGB | 73.468 | 149.569 | 191.073 |
| 16 / 512x512 | Oklab | 25.537 | 51.848 | 88.031 |
| 16 / 512x512 | Linear RGB | 25.519 | 51.246 | 87.658 |
| 64 / 256x256 | Oklab | 15.733 | 29.671 | 518.485 |
| 64 / 256x256 | Linear RGB | 15.785 | 29.524 | 517.769 |

Cancellation after row progress returned in 1.650 ms for Multisite and 4.072 ms
for Shared borders. Shared borders has a substantial high-site-count cost; these
measurements are not a performance improvement claim. No Rayon or GPU dependency
was introduced.
