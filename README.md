# Threshiator

Threshiator is a native GTK 4/libadwaita posterization studio for Fedora GNOME. Independent Voronoi color sites are the default creative method; variable-band RGB and HSV Thresholds are a first-class alternate. Both method states persist in a straight-alpha linear-sRGB `RGBA f32` pipeline.

The welcome screen offers three direct starts: **Open Image**, **Open Threshiator Project**, or **Try Spectrum Example**. The included Spectrum artwork is compiled into the native application and opens as a clean, pathless document through the same raster decoding and default Voronoi initialization used by imported images.

## Build and run

Install Rust plus the GTK 4 and libadwaita development packages, then:

```sh
cargo run --release
```

Checks:

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build
```

The app-owned interactive stability audit runs every scenario in a disposable GTK process with an external heartbeat watchdog:

```sh
python3 scripts/run_ui_audit.py
```

Its JSONL logs and `summary.json` are stored under `tests/artifacts/audit/`; the full coverage matrix and known manual-only paths are documented in [`docs/FUNCTIONAL_AUDIT.md`](docs/FUNCTIONAL_AUDIT.md). It exercises actual widget methods and signals for the shell, Threshold inspector/dialog, Voronoi controls, picker, I/O lock, and a narrow window. It does not claim real pointer gestures or portal chooser completion.

Deterministic command-driven UI evidence can be produced without pointer automation:

```sh
cargo run --release -- --open image.png --method voronoi --view split \
  --window-size 1180x760 --select-site 1 \
  --screenshot evidence.png --quit-after-screenshot
```

Supported evidence controls are `--open`/`--project`, `--example`, `--method`, `--view`, `--window-size`, `--select-site` (with `--select-group` and `--select-sample` retained as compatibility aliases), `--sampling-state`, `--hue`, `--voronoi-matching perceptual|rgb|hsv`, `--threshold-space rgb|hsv`, `--threshold-link linked|independent`, `--threshold-component red|green|blue|hue|saturation|value`, `--threshold-bands 2..32`, `--threshold-bypass COMPONENT`, `--show-threshold-editor`, `--threshold-editor-target red|green|blue|linked-rgb|hue|saturation|value|linked-sv`, `--threshold-editor-handle boundary:N|output:N`, `--show-color-picker`, `--color-model hsv|hsl|oklab`, `--picker-lightness 0..1`, `--screenshot`, and `--quit-after-screenshot`. Screenshot sidecars expose every independent Voronoi site's ID/order, Source, Target, Influence, lock, optional position, and sampling footprint. With no input, screenshot mode captures the welcome screen.

The historical browser demonstrator is runnable from [`archive/webapp`](archive/webapp/README.md).

## Current image contract

Inputs are still PNG, JPEG, TIFF, WebP, BMP, and single-frame GIF. SVG and animated GIF are rejected explicitly. Grayscale and grayscale-alpha inputs expand to RGBA. Decoded color samples are interpreted as encoded sRGB and converted once to linear-sRGB `f32`; alpha remains straight and is preserved.

Embedded ICC data is detected through the selected decoder but is not transformed in this milestone. The application labels that limitation; untagged images default to sRGB. It does not claim profile preservation. Display conversion is the only preview quantization step: linear straight-alpha pixels become straight encoded-sRGB RGBA8 for GdkPixbuf, preserving RGB accuracy even at low alpha.

Exports currently supported:

| Destination | Samples | Color representation |
| --- | --- | --- |
| PNG | 8-bit integer per channel RGBA | encoded sRGB, tagged sRGB |
| PNG | 16-bit integer per channel RGBA | encoded sRGB, tagged sRGB |
| OpenEXR | 32-bit float per channel RGBA | linear sRGB values; no ICC claim |

JPEG and other export formats are intentionally unavailable rather than silently down-converted. All project and image writes use a temporary file in the destination directory followed by atomic replacement.

## Processing model

New images deterministically initialize up to four materially distinct independent Voronoi sites from a bounded OKLab clustering proxy, then sample authoritative full-resolution pixels at representative normalized locations. Each site owns a canonical linear-sRGB `f32` Source, its own Target, Influence, lock, stable ID/order, optional normalized marker position, and sampling footprint. Source sampling accepts any positive footprint alpha and computes alpha-weighted RGB; new and resampled sites begin with Target equal to Source. Point, 3×3, and 5×5 footprints are available.

Processing derives OKLab, encoded-sRGB RGB, and HSV coordinates from canonical Source values for every compiled recipe; no per-space coordinate cache is serialized. HSV uses the true cylinder `(S·cos(H), S·sin(H), V)`. Influence retains the weighted rule `distance² × 2^-clamp(Influence,-4,4)`, and ties resolve by stable order then ID. Every visible pixel is assigned directly to the winning site's Target; coverage is reported by site ID. Transparent pixels remain canonical.

Source and Target are edited separately with the transactional color picker. Replacing Source by drag, nudge, resample, footprint change, or direct edit always resets Target to the new Source; a direct Source edit also detaches the old image position. Lock protects Source, Target, Influence, footprint, marker movement, and deletion without promising a fixed boundary when neighboring sites or the global matching mode change. Lock/unlock marks the project dirty but schedules no image work. Presets and undo remain separate tracked work.

Thresholds retain independent RGB and HSV states when the Working space changes. Every logical component has a visible Process/Bypass state and 2–32 Bands. **Edit mapping…** opens a dedicated adaptive dialog with a focusable Input → Output transfer plot: vertical blue handles edit Boundaries and horizontal red handles edit each Band's Output. The selected handle also has a precise numeric control; Hue is displayed in degrees while remaining normalized internally. Pointer motion changes only the dialog draft, then one preview is scheduled on release. A numeric or keyboard adjustment similarly schedules one completed update. Done retains the live edits; Cancel, Escape, or window close restores the pre-dialog Threshold state and dirty flag, scheduling a restoring preview only when an edit had been committed.

A value exactly on a Boundary belongs to the lower Band. Link RGB and Link Saturation & Value copy future edits and Band changes without copying Process/Bypass flags; hue is never linked. The dialog uses semantic targets rather than dropdown positions: linked RGB appears as **Red, Green & Blue**, and linked HSV appears as separate **Hue** and **Saturation & Value** targets. Enabling or disabling Link immediately reconciles that target menu without processing an image merely because the selection changed.

New RGB work quantizes floating-point encoded sRGB and converts directly back to canonical linear-sRGB `f32`, with no intermediate integer quantization. HSV operates over encoded sRGB: S and V are linear from 0 to 1, hue is circular and shown in degrees, and hue quantization cannot tint achromatic input. Bypassed components survive the required working-space round trip within floating-point tolerance. Alpha remains straight and preserved; transparent RGB is canonicalized. Alpha quantization and input smoothing are represented for schema evolution but intentionally deferred. Common hue rotation still uses floating-point degrees in OKLCH; out-of-gamut results are clipped to linear sRGB. Creative recipe changes mark the document dirty; comparison and divider changes do not.

Interactive previews are bounded to 1600 pixels on their longest edge and use cheaply shared pixel storage, while attached site Source colors always come from the authoritative full-resolution image. Saving and full-resolution export keep the authoritative source and embedded bytes intact; export reprocesses that full-resolution source through the active method off the GTK main thread. Markers and selection affordances never export.

Active open, save, and full-resolution export jobs can be cancelled. Third-party codec calls are not themselves preemptible; cancellation invalidates the job immediately, discards any codec result or temporary output, and prevents destination replacement.

While an open, save, or export job owns the document, the document workspace is insensitive and footer Cancel remains available. A file operation is rejected while a modal creative dialog is open, so the dialog can never hide an unreachable job-cancellation control. This prevents edits from racing a stale save snapshot or an incoming document replacement. A second job request is also rejected gracefully rather than panicking. The Stability & Observability milestone removed callback-driven model recreation from the Threshold target and Voronoi Source sample dropdowns; both now use stable, idempotently synchronized models.

## Projects and resuming work

`.threshiator` v4 is the canonical editable project format. It stores the original source bytes, source interpretation, complete RGB and HSV Threshold states, independent Voronoi sites, the active method, downstream common hue, and export defaults. Version-3 Color groups migrate by flattening every Source sample into an independent site that inherits the former group Target; empty groups are dropped, locks default off, stable sample IDs/order and sampling metadata are retained, and the next site ID is collision-safe. Version-2 projects additionally migrate to the exact `LinearSrgbLegacy` three-band Threshold path so their existing pixels do not change. Current projects validate unique IDs/order, finite/ranged Source, Target, Influence and positions, plus a safe next ID. Opening a project restores the recipe without rerunning initialization. The old browser JSON format is not imported yet.

Open Image, Open Project, and Try Spectrum Example share one replacement guard. A dirty document offers Save, Discard, or Cancel. Save performs the requested replacement only after a successful project write; a cancelled or failed save keeps the current document.

## Output color picker

Click an Output color swatch to open Threshiator's draft color picker. HSV is the default model; HSL and OKLab are available without changing the underlying canonical draft, which remains floating-point linear sRGB. HSV/HSL operate over encoded sRGB. The OKLab plane uses fixed-lightness polar a/b geometry, visibly hatches out-of-sRGB areas, and projects drags to the valid gamut boundary. Hex accepts `#RGB` and `#RRGGBB`; displaying hex never replaces the higher-precision draft.

The three slider/precision rows stay synchronized with the color plane and hex entry. Achromatic edits retain latent hue. The wheel is focusable: arrows adjust it, Shift provides fine adjustment, and Home returns to neutral. Original and New swatches make the pending change explicit. Select commits once and schedules one preview; Cancel, Escape, or closing the dialog makes no document, dirty-state, or scheduler change. Source alpha is not editable and passes through unchanged.

## Forward color direction (deferred)

Threshiator is an intermediate creative tool, so its long-term color policy favors preserving palette intent for finishing elsewhere. Familiar RGB and HSV editing remain primary; OKLCH is planned as an optional **Perceptual** picker representation, while Cartesian OKLab remains the perceptual Voronoi metric. An extended-gamut selected color should remain distinct from the nearest proxy the current display path can render. The project keeps the selected floating-point intent; a constrained export applies an explicit format-specific gamut policy.

The existing shared `f32` pipeline is retained as the precision foundation, but wide gamut must not be described as HDR. A credible HDR handoff additionally requires declared primaries, white point, luminance semantics, transfer behavior, and format metadata. The current linear-sRGB OpenEXR export is useful as a high-precision intermediate, but its present “no ICC claim” contract is not a finished HDR workflow. PQ/HLG, Rec.2020, display profiling, and tone mapping remain deferred until a dedicated workflow justifies them. HWB and other picker models are not planned merely for completeness.
