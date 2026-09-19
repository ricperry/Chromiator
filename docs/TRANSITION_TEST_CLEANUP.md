# Voronoi transition test cleanup - 2026-09-19

The user accepted the single-blending-behavior simplification and reported that
the transitions work superbly in their own testing. This record supersedes the
pending-test-cleanup notes and the former Shared borders verification records.

## Scope

- No Thresholds-processing tests remained in `src/` or `tests/`. Retained the
  CLI regression that rejects removed Thresholds options: it tests current
  Voronoi-only behavior, not the removed engine.
- Removed the Shared-borders-only flat-palette regression. Its contact-solver
  unit tests were already removed with the solver.
- Retained competitive-site continuity, junctions, influence, alpha, gamut,
  hard mapping, history/savepoints, cancellation, persistence and export checks.
- Removed scope variants and commands from transition tests and workloads.
- Asserted rejection of the obsolete `blend_scope` field, without migration.
- Generated fresh artifacts under `target/validation/transitions-multisite/`,
  separate from historical two-scope evidence.

## Verification

- `cargo test --all-targets --all-features`: 103 passed (11 library, 13 binary,
  69 core, 10 transition); three opt-in tests skipped in this command.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo build --bin chromiator`: passed.
- `cargo test --release --test transitions render_transition_evidence -- --ignored --nocapture`:
  passed; all six output images inspected (Oklab/Linear RGB at 0, 25, 100 percent).
- `cargo test --release --test transition_performance -- --ignored --nocapture --test-threads=1`:
  both workload and cancellation tests passed.

Representative blended-render medians at width 50 percent, including compile
and processing but excluding GTK and file I/O:

| Sites / dimensions | Oklab ms | Linear RGB ms |
| --- | ---: | ---: |
| 5 / 1024x1024 | 155.329 | 156.191 |
| 16 / 512x512 | 52.757 | 51.636 |
| 64 / 256x256 | 29.701 | 29.300 |

Cancellation returned after progress in 1.632 ms (Oklab) and 1.631 ms (Linear RGB).
These are local measurements, not performance guarantees.

## Native check

Private Sway preflight passed. The rebuilt app opened Spectrum Example. Semantic
readback confirmed width zero, disabled Oklab selection, and no Blend scope row.
Width changed to 50 percent, Linear RGB selection succeeded, Undo restored Oklab,
and Redo restored Linear RGB. Waited for `preview current` before capturing and
inspecting `multisite-cleanup.png`.

Evidence: `.codex-work/evidence/ui-run-20260919-112556-59337/`.
The private session was stopped. This bounded check does not claim a fresh
GNOME/Mutter, narrow-layout, or full application workflow audit. THR-065 remains
deferred and THR-066 remains open. No user presets, assets or CSS were changed.
