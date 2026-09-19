# Accessibility and full test pass - 2026-09-19

Status: the approved naming corrections and picker-crash fix are verified.
This is not user acceptance or a complete accessibility certification.

## Completed follow-up

- Source/Target swatches retain their unique site-qualified names. The arrow's
  decorative child is presentation-only, exposing `Match target to source for
  Site N` with the exact tooltip `Match target to source`. Export is a named
  text button. Native AT-SPI readback confirms each correction.
- A retained loopback VNC connection verified F6: Matching -> selected site ->
  Image comparison, Shift+F6 back to the selected site, Alt+I for Influence,
  Alt+S for Smoothing, Alt+T for Transition width, and Alt+H for picker Hex.
  Alt+M opens Matching's native dropdown rather than merely focusing it.
- GTK does not expose raw label relations in this environment; computed names
  and successful mnemonic actions are the evidence, not inferred relations.
- The UI/UX reviewer inspected the corrected workspace and picker screenshots
  and found no material blocker in those states.
- The blue cusp approximation and intermediate OKHSL samples are corrected by
  bounded chroma contraction at fixed OKLab lightness and hue. Two new tests
  failed before correction and now pass, including all three native gradient
  channels for seven saturated input colors. Existing reference/roundtrip,
  processing, and export tests remain green.
- Final regular suite: **105 passed**. Strict Clippy and app build passed.
  All three opt-in release checks were rerun after the color fix and passed.
- Native Source picker for `#000080` opens and renders. A real Enter key commits
  Hex input (AT-SPI entry activation alone did not); changing Source to
  `#33AA66` preserves Target `#FF0000`. Arrow activation matches Target to
  `#33AA66`, disables itself, and document Undo restores Target `#FF0000`.
- Final app stderr was empty: no GTK warnings or panics in the exercised path.

Evidence: `.codex-work/evidence/ui-run-20260919-140416-94319/`
(`accessibility-names-fixed.png`, `keyboard-state.png`) and
`.codex-work/evidence/ui-run-20260919-141155-101688/`
(`blue-source-picker-fixed.png`, app logs). Parent inspected all three.
The private session was stopped without saving fixture edits. GNOME/Mutter and
screen-reader acceptance remain manual; deferred layout THR-065 is unchanged.
Nonblocking accessibility polish is tracked separately as THR-069.

The initial pass below is retained as historical failure evidence.

## Changes

- Existing Source edits preserve Target. Updated two obsolete reset-expectation
  tests and added footprint-preservation and explicit-match assertions.
- Added F6/Shift+F6 workspace-region navigation and inspector/picker mnemonics,
  concise expander names, icon-button names, and label/control relationships.
- UI/UX reviewer inspected source and read-only Toniator mnemonic patterns.
- No changes to user assets, presets, or the deferred application layout issue.

## Checks

- `cargo test --all-targets --all-features`: 103 passed, three opt-in tests skipped.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo build --bin chromiator`: passed.
- `cargo test --doc --all-features`: passed, no doctests.
- `cargo test --release --all-targets --all-features -- --ignored --nocapture --test-threads=1`:
  all three opt-in checks passed, including render artifacts and cancellation.
  Generated images have not received a fresh image-by-image review in this pass.

## Native findings

Private Sway run: `.codex-work/evidence/ui-run-20260919-135155-89663/`.
Parent inspected `accessibility-workspace.png` and `accessibility-picker.png`.

- Concise section names and mapping/picker labels appear through AT-SPI.
- Adding mnemonic relationships to Source/Target captions loses their unique
  site-qualified names. Correction is awaiting user approval (THR-067).
- Arrow is exposed as `->` (visually an arrow) despite its explicit accessible
  label; its Match target to source description is present. Export still has
  an empty name. Harness versus GTK behavior needs diagnosis.
- Focus and relation readback is not yet reliable: controls report no focused
  state/relations. F6/Shift+F6 is implemented, not yet verified end to end.
- Opening the Source picker for the saturated-blue Site 1 in the native
  transition fixture aborts at `src/color.rs:108` with
  `finite normalized OKHSL controls always produce bounded sRGB` (THR-068).
  Target picker for its red Target opened successfully first. The crash log is
  preserved in `app.stderr.log`. Further native transaction checks are blocked.

The private session was stopped. No host GNOME/Mutter or screen-reader
acceptance is claimed. Follow-up correction questions were sent to the user.
