# Chromiator

The executable is `chromiator`, projects use `.chromiator`, and application data
is stored in `$XDG_DATA_HOME/chromiator` (normally `~/.local/share/chromiator`).
This pre-alpha rename has no legacy extension or data-folder compatibility layer;
existing personal data is not automatically moved or rewritten.

Chromiator is a native Rust/GTK4 creative color-mapping application. It maps
image colors to independently editable target colors using weighted Voronoi
matching in color space. It is not a geometric tessellation drawing tool.

The application now focuses exclusively on Voronoi mapping. The separate
Thresholds processor and mode-switching UI have been removed. The GTK4 shell
adopts Toniator's canvas-first layout and grouped right inspector without a
libadwaita dependency.

## Refactor status

Implementation edits through the GTK4 shell conversion are present. The warning
cleanup now builds cleanly, passes strict Clippy and 93 tests, and has received
bounded private-Sway interaction checks. Save, drag-history, and picker follow-up
fixes are verified; **responsive-layout finding THR-065 remains open.**
The user accepted the current refactor, with THR-065 deferred as low priority.
This is not a published release checkpoint.
See `docs/BUG_HUNT_FOLLOWUP_2026-09-19.md` for current evidence and limitations.

Before the final shell conversion, the architecture checkpoint passed 88 tests
(7 library, 12 binary, 69 core). The earlier Voronoi-only checkpoint passed strict
Clippy and an exact floating-point/coverage comparison across four matching
spaces and two smoothing amounts. Those results do not validate subsequent
changes. The newer bug-hunt report supersedes those build/test counts, but
complete native workflow and performance verification remain unfinished.
See `docs/FUNCTIONAL_AUDIT.md` and `ISSUES.md`.

## Creative workflow

- Open a raster image or use the Spectrum Example.
- Edit Color mapping settings, including matching space and input smoothing.
- Adjust Transition width to blend Target colors across color-space boundaries,
  using Oklab or Linear RGB as the Blend space.
- Click visible artwork to add a Color site: Source-view samples attach to the
  original image; Result-view samples use the displayed floating-point preview
  and remain detached. Split follows the image visible on each side.
- Use the `+` button in Color sites to pick a detached Source color directly.
  Select creates one undoable site with matching Source/Target; Cancel adds none.
- Select Color sites, then edit each site's Source and Target colors.
- Adjust influence, sampling footprint, and locks; compare Source and Result.
- Reuse source-free presets, undo/redo authored edits, save projects, and export
  a full-resolution result without editor markers.

Source and Target are independent. Editing, moving, or resampling an existing
Source preserves its Target; direct Source editing detaches its image position.
The arrow button explicitly matches Target to Source and supports Undo.
Locked sites protect their
local edits and deletion. Sampling uses the original image rather than the
scaled preview. RGB and HSV matching are Voronoi metrics, not Thresholds modes.
The existing OKHSL metric remains supported internally but is not promoted in
the creation menu.

Keyboard navigation supplements Tab/Shift+Tab: F6 and Shift+F6 cycle between
the canvas, mapping controls, and color-site list. Alt+M/S/T/B focus Matching,
Smoothing, Transition width, and Blend space when available. Expanded sites
provide Alt+I/P/F for Influence, Source position, and Footprint. In the picker,
Alt+C/H focus Color model and Hex. Controls expose names and label relations
for AT-SPI; Source and Target swatches are identified by site number.

## Build and launch

Use Rust/Cargo and the GTK4 development toolchain, including GLib's resource
compiler and `pkg-config`. `build.rs` compiles the XML UI and CSS resources using
`glib-build-tools`; Blueprint and libadwaita are not required.

```sh
cargo run --release
cargo run --release -- --open /path/to/image.png
```

The old `--method` and `--threshold-space` options are removed and report an
error. The archive under `archive/webapp/` is historical reference, not the
native application's runtime.

## Color and persistence contracts

Processing uses straight-alpha linear-sRGB RGBA `f32`, with ICC interpretation
at import and format-specific encoding at export. Preserve transfer functions,
alpha, site ordering, metric definitions, and influence weighting when changing
the engine. Input smoothing precedes mapping; ordered hue operations follow it.
Preview smoothing is measured in preview pixels and export smoothing in source
pixels, preserving the existing behavior.

New projects use schema **v6** and presets use schema **v3**. Older formats are
rejected explicitly; they are not migrated, silently reinterpreted, or rewritten.
Keep older files if their contents are still needed.

A required symmetric Start/Midpoint/End transition profile now supports a global
Transition width and Oklab or Linear RGB blending. All competitive sites blend;
there is no blend-scope selector or shared-border contact solver. Width zero
preserves hard mapping and deterministic ties. Files containing the removed
`blend_scope` field are rejected; there is no compatibility layer. File formats
are experimental, not finalized. Pre-alpha format and preset cleanup is tracked
as THR-066. The user accepted the simplification. Test cleanup is complete:
103 tests, strict Clippy, build, three opt-in release checks, and a focused
native transition check pass. See `docs/TRANSITION_TEST_CLEANUP.md` for current
evidence; earlier Shared borders reports are historical.

## Development

The 2026-09-19 accessibility/picker follow-up passes 105 regular tests, strict
Clippy, app build, and three opt-in release checks. Private-Sway checks verify
site-qualified accessible names, keyboard region navigation, Source/Target
independence, explicit matching and Undo, and the corrected saturated-blue
OKHSL picker. See `docs/ACCESSIBILITY_VERIFICATION_2026-09-19.md` for evidence
and remaining accessibility limitations.

`docs/ARCHITECTURE.md` describes ownership, typed edits, processing boundaries,
and the future blending extension point. `docs/FUNCTIONAL_AUDIT.md` distinguishes
completed checkpoint evidence from pending final checks. `ISSUES.md` retains
stable issue IDs and milestone status.

Project orchestration targets `gpt-6-astra` at high reasoning effort through
`.codex/config.toml` and the `chromiator-orchestrator` skill. The adopted
`gtk-wayland-debug` skill runs native checks in a private Sway session without
controlling the normal GNOME desktop. Automated Sway evidence is not human
GNOME/Mutter acceptance.
