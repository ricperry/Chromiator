# Native Voronoi architecture

The refactor stays in one Cargo package. Module boundaries separate authored
state, processing, asynchronous work, and GTK presentation without a plugin
framework. The final GTK4 edit phase is unverified; see FUNCTIONAL_AUDIT.md.

## Ownership

| Area | Responsibility |
| --- | --- |
| `src/session.rs` | GTK-independent DocumentSession; document, history/savepoint, selected site, typed edits and invalidation |
| `src/document.rs` | Serializable recipe and document contracts, including the required transition profile |
| `src/processing.rs` | Image processing, compiled Voronoi partition, hard target resolution and coverage |
| `src/transitions.rs` | Normalized compact-support Target blending and blend gamut mapping |
| `src/transition_controls.rs` | GTK projection of session-owned transition width and blend space |
| `src/canvas.rs` | Widget-independent canvas geometry |
| `src/picker.rs` | Picker-local draft/history and color interaction helpers |
| `src/app_job.rs` | Application job integration and background work coordination |
| `src/cli.rs` | Command-line parsing and dispatch |
| `src/presentation.rs` | Canvas/picker presentation and screenshot integration |
| `src/site_editor.rs` | Site inspector projections and editing callbacks |
| `src/shell_actions.rs` | Shell actions wired to application/session operations |
| `src/components.rs` | GTK CompositeTemplate shell and presentation widgets |
| `src/native_dialog.rs` | Native GTK confirmation-dialog adaptation |
| `src/theme.rs` | Read-only system color-scheme integration |
| `src/ui_audit.rs` | Native action/assert audit routes, not document authority |
| `src/main.rs` | Application bootstrap and wiring |

`resources/window.ui` declares the GTK shell. `resources/chromiator.css`
centralizes presentation styling. `build.rs` and
`resources/chromiator.gresource.xml` compile/embed these through
`glib-build-tools`. No Blueprint or libadwaita layer is required.

## Edit and publication flow

GTK callbacks submit typed session edits instead of maintaining a second
recipe/history authority. Session outcomes distinguish authored changes,
selection changes, dirty state, and preview invalidation. For example, locking
a site changes the document but need not recompute pixels. Same-value edits must
not create history entries. Site identity is stable, not a transient row index.

Picker edits remain a local draft until confirmation. Cancel/close discards the
draft; confirmation creates one global transaction. Continuous gestures and
spin edits retain coalescing boundaries. Preserve saved-state tracking through
undo/redo and discard redo branches only for actual new authored edits.

Workers consume immutable snapshots without GTK objects. Generation/request
checks guard publication of both pixels and coverage. Cancellation does not
make a job idle until its matching acknowledgement arrives. Progress must remain
monotonic and stale results must not replace the current preview. Full document
clones belong at actual job boundaries, not routine inspector refreshes.

File decoding and export remain off the GTK main thread. Preserve atomic output
replacement and existing cancellation limits of third-party codecs. Presentation
code must not turn a cancelled job into a successful write.

## Color pipeline and future transitions

Preserve the straight-alpha linear-sRGB RGBA `f32` working representation,
ICC handling, metric definitions, transfer functions, and deterministic tie
rules. Smoothing precedes mapping; ordered hue operations follow mapping and
must not be merged if that changes clipping or rounding.

`CompiledVoronoi` compiles the recipe into stable site order and cached site
context. Influence weighting is a per-site invariant. Winner selection returns
a stable site index; coverage uses that index directly. Hard target resolution
is a separate operation that preserves the input alpha. Keep source/target
context available without allocating speculative candidate lists per pixel.

Project v6 and preset v3 require a normalized, ordered, symmetric Start/Midpoint/End
profile centered at 0.5. Width is End minus Start. Defaulted blend space is omitted
when serialized. Blend scope has been removed; strict deserialization rejects
files containing its obsolete field. Formats remain experimental and may break
before alpha; THR-066 tracks format/preset cleanup. Rejected files stay unchanged.

Positive-width blending uses the weighted squared-distance competition gap
`(score - minimum) / (score + minimum)` and compact smoothstep activations.
Zero/zero scores have gap zero. Hard rendering bypasses blending, retaining
the exact existing tie winner. Coverage remains winning-site coverage.

All competitive sites contribute through normalized activations, without an
adjacency graph or contributor cap. Shared-border filtering was removed by
product decision because its visual impact did not justify its complexity.

Target colors mix in Oklab (default) or Linear RGB independently of Source matching.
Oklab mixtures outside sRGB reduce chroma at fixed lightness/hue through 24 bisection
iterations. Single-contributor pixels copy their Target directly. Alpha passes
through unchanged. Scratch arrays are allocated once per processing job.
Compilation and rendering retain generation-cancellation checks.

## Dependencies and efficiency

Retain the existing image, moxcms, PNG, EXR, crossbeam, and tempfile facilities
for raster conversion, codecs, scheduling, and safe writes. Replacing color math
or blur with another library is not automatically an equivalent optimization:
alpha convention, transfer function, precision, and rounding are contracts.

The refactor introduces GLib resource compilation for maintainable native UI
composition. It does not require an indexing engine, GPU backend, plugin system,
or speculative parallel-processing framework. Rayon adoption is pending a
bounded experiment, not promised. Compare exact pixels and coverage alongside
warm release timings, preview latency, peak memory, and cancellation latency.
Do not parallelize floating-point reductions in a way that changes results.

## Adding features safely

1. Define authored state and validation separately from widget presentation.
2. Admit changes through session commands with explicit history and invalidation
   behavior; avoid independent widget-owned copies of document state.
3. Extend the shared processing path so preview and export share semantics.
4. Keep serialization changes explicit and reject unsupported files safely.
5. Add focused regression coverage and real native workflow checks when
   authorized; record evidence and remaining uncertainty in the audit ledger.

Use Rust module documentation (`//!`) for ownership and dependency direction,
and API documentation (`///`) for invariants, units, failure behavior, and
non-obvious numeric decisions. Document why a boundary exists rather than
restating each line of implementation. Prefer small reusable functions with
explicit inputs over speculative abstractions.
