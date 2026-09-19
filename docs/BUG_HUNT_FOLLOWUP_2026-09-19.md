# Bug-hunt follow-up, 2026-09-19

- THR-062: fixed `save_handler` by releasing the project-path read borrow before
  starting the save job. Native Save As followed by smoothing 1 -> 2 and Save
  to the existing path completed without a chooser or panic, returning Save to
  its disabled clean-state presentation. Scratch file SHA-256 changed from
  `a24b87b63373482a2faa7a0a8121a5bcb4d1e6ff747527f353f0de9fa4d2c24d`
  to `6f53b9faab52558bfdee01ba0aafe85fdce64718c8aa992d5403ea60116127ba`.
- THR-063: fixed drag-end history presentation refresh. Native drag enabled Undo;
  Undo restored the clean savepoint, and Redo re-enabled Save and Undo.
- THR-064: fixed rounded SpinButton display reparsing that created spurious local
  picker edits. Preserve the full-precision adjustment when input is only its
  rounded display. Native local Undo restored Lightness 59.49156016103666 from
  45; Redo restored approximately 45. Cancel did not dirty document history.
- THR-065: still open. Paned child sizing flags plus the restored initial position
  do not prevent inspector clipping at 1024x600. The attempted layout change is
  not a verified fix. Further implementation requires a follow-up pass.

## Verification and limits

`cargo test --all-targets --all-features` passed 93 tests (11 library, 13 binary,
69 core); strict all-target/all-feature Clippy and the application build passed.
This is bounded regression evidence, not full refactor acceptance or a performance
claim. GNOME/Mutter human acceptance remains outstanding.

Native history/picker evidence:
`.codex-work/evidence/ui-run-20260913-203215-143361/`.
Latest Save and layout evidence:
`.codex-work/evidence/ui-run-20260919-090435-13188/`, including
`save-regression.chromiator`, `desktop-corrected.png`, and
`inspector-1024-corrected.png`. The latest app error log was empty. The native
chooser was isolated with test-only `GDK_DEBUG=no-portals`; no host preferences
were changed. The private Sway session was stopped after evidence collection.
