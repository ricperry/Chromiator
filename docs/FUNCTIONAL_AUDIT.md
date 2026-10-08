# Voronoi refactor verification ledger

## Status

The user accepted the current implemented GTK4/Voronoi work on 2026-10-07.
Automated verification is documented below, with its environment and coverage
limits. This acceptance does not certify untested desktop or assistive-technology
configurations. The exact 0.3.0 AppImage and Flatpak packages passed six
package-specific private-Sway routes each; both AppImage ELF audits also passed.
Hashes and the complete bounded package evidence are in `DISTRIBUTION.md`.
These results do not certify a clean-Debian desktop, GNOME/Mutter, or full
assistive-technology behavior. The follow-up fixes passed application builds,
strict Clippy, and focused tests. Private-Sway
checks verified save/history/picker and the integrated workflow. The isolated
GNOME/Mutter run verified THR-065 maximize/restore, portal open/cancel, and PNG8
export/source/project readback; it did not repeat the full preset/edit/reopen
path. See `BUG_HUNT_FOLLOWUP_2026-09-19.md` and the evidence recorded below.
Large-image timings are synthetic and machine-specific.

## Planning update (2026-10-03) — Stages 0–1 implementation in progress

The user approved headless implementation of Stages 0 and 1, including release
characterization, a fallible welcome-decoration decoder, and packaging build
and audit code. Fresh GUI launches and clean-Debian runtime verification remain
blocked by the current approval/environment gate. Stage 2 requires review of Stage 0/1
evidence; Stage 3 follows only after its own scope is approved. Keep the
published v0.2.0 behavior, project v6, preset v3, and all 19 built-in presets
as read-only comparison anchors. No current-format migration, preset rewrite,
or silent output-policy change is proposed.

The user's 2026-10-03 decision preserves existing rendering and saved-project
compatibility, requires visible feedback for settings conflicting with exact
tones/alpha, and defers an opt-in strict-palette mode to THR-074. This overrides
THR-066's earlier compatibility-break allowance for this scope. The hard-only
transition wording in AGENTS.md is historical; retain released blending.

### Implementation update (2026-10-03–04)

Headless Stage 0 characterization and the Stage 1 welcome-decoration fallback
are implemented and focused-tested. Release fixtures and SHA-256 provenance are
in `tests/fixtures/release-0.2.0/`; inspect source, hard, and blended PNGs plus
results under `target/validation/stage-0/release-anchor-candidate/`. The
release processing, project, preset, raster, color, and example sources were
verified unchanged from `7f7f7353ff315c73636e5e3afc7a82ac9e7a37be` before
fixture capture. `cargo test --locked --test release_regressions` passes 5
tests; its first-capture generator is ignored. Both focused welcome tests pass
individually. Parent verification passed 113 tests with 6 ignored under
`cargo test --locked --all-targets`, strict all-target/all-feature Clippy, and
`cargo build --locked --release` on Fedora 44. This establishes headless
evidence only. No GUI, Sway, package
launch, or clean-Debian run was attempted; Stage 0 workflow acceptance and
Stage 1 package/runtime acceptance remain blocked.

Stage 1 packaging code stages explicit source and vendored dependencies in a
pinned Debian 13 x86_64 builder. The 2026-10-03 builder run used image
`3e74a6b29fcfaa36ce01a16c40f356ba68509920002a37360129defbf09ec537`; glibc
2.41 was confirmed, the Rust 1.97.1 archive hash verified, and offline release
compilation passed. Its staged AppDir audit passed for 119 ELF files, maximum
required GLIBC 2.39, zero errors against 2.41, Debian `ldd -r` resolution, and
loadable bundled Fontconfig/HarfBuzz providers. The retained v0.2.0 AppDir
still fails the proposed floor for seven GLIBC_2.43 libraries and missing
providers.

The output runtime is now pinned to the versioned official AppImage/type2-runtime
20251108 asset, SHA-256
`2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d`. Host and
builder checks cover hash, size, ELF64 little-endian x86_64 identity, and the
GLIBC ceiling; the file is mounted read-only and supplied through
`LDAI_RUNTIME_FILE=/tools/runtime-x86_64` to the output plugin. The container
has network disabled, with no implicit runtime download fallback. Retrieval and
provenance details are in `docs/DISTRIBUTION.md`.

The exact earlier-task 2026-10-04 build attempt
`python3 -B scripts/build_distributions.py appimage > target/distribution/stage1-build-5.log 2>&1`
was rejected before execution with `approval required by policy, but
AskForApproval is set to Never`; it was not retried. No `stage1-build-5.log`
result, candidate, candidate hash, extracted-payload audit, package launch, or
clean-Debian GUI workflow result exists. The prior 2026-10-03 AppDir audit
remains build evidence only, not candidate evidence. Stage 1 package/runtime
acceptance and Stage 2 remain pending.

Writer-reported verification commands:

```sh
python3 -B -m unittest discover -s tests -p test_packaging.py -q  # PASS: 13 tests
python3 -B scripts/build_distributions.py appimage --check          # PASS
python3 -B -m py_compile scripts/build_distributions.py scripts/audit_appimage.py  # PASS
sh -n scripts/smoke_appimage.sh                                     # PASS
git diff --check                                                     # PASS
python3 -B scripts/build_distributions.py appimage > target/distribution/stage1-build-5.log 2>&1  # REJECTED BEFORE EXECUTION
```

No build-5 log output was produced; the passing checks did not run the rejected
build or launch a package.

### Candidate continuation (2026-10-04)

The new task's supported approval path permitted the documented pinned build.
Build 5 created an AppImage but the strict post-output audit rejected it because
linuxdeploy reintroduced `libvulkan.so.1`. Build 6 stopped on exclusion-option
syntax, before creating another candidate. The final invocation now repeats
`--exclude-library=<SONAME>` for the five existing host graphics loaders.
The exact pinned linuxdeploy binary accepted those options on `--list-plugins`;
13 packaging tests, Python syntax, and diff checks passed. No host allowlist,
audit rule, runtime pin, rendering, preset, or saved-format behavior changed.

Build 7 completed the full offline-container pipeline and extracted-payload
audit. Candidate: `target/distribution/0.2.0-vsd173k4/appimage/output/Chromiator-0.2.0-x86_64.AppImage`,
28,228,088 bytes, SHA-256 `a123e544a6069701235a338c2a0174a972db1fe7e5a0a2f782318732c3bdf621`.
Both post-output AppDir and packaged-payload audits report 119 ELF files,
zero errors, maximum GLIBC 2.39 against the 2.41 ceiling, with Debian
relocation resolution and the pinned outer runtime checked. Reports:
`target/distribution/0.2.0-vsd173k4/appimage/output/abi-audit.json` and
`target/distribution/0.2.0-vsd173k4/appimage/output/packaged-abi-audit.json`.
Log: `target/distribution/stage1-build-7.log`.

The application source is based on checkpoint `425760cd51ad868b2a6c06c48885e3958d22c476`
plus the uncommitted packaging fix. The existing pipeline also copies the
pre-existing modified packaging icon; this is declared in source-input
provenance and remains uncommitted. Other unrelated edits and assets are
preserved and excluded from the candidate inputs.

This is an internal test candidate, not package/runtime acceptance. Native CUA
control is unavailable in this task (browser surfaces only), so no Fedora GUI,
FUSE launch, import/export, save/reopen, or new screenshot result is claimed.
Clean-Debian runtime testing remains for the separate environment. The test
handoff includes the same candidate, fixtures, exact source snapshot and
patches, checksums, audit reports, prerequisites, and GUI/parity instructions.
No additional commit, push, PR, release, main update, or Stage 2 work occurred.

### Bounded Debian cloud result (2026-10-04; qualified pass)

The cloud test handoff reports results for the unchanged experimental AppImage,
SHA-256 `a123e544a6069701235a338c2a0174a972db1fe7e5a0a2f782318732c3bdf621`.
Evidence is retained in ChatGPT Library as `libfile_edc44f21a56c81918da4a0391cefe8ec`.
This local update records the reported cloud evidence; it does not rerun or
independently reverify those GUI actions.

Environment: Debian 13.6, glibc 2.41, XFCE/X11, with development tools installed.
Extracted AppRun worked with a Cairo renderer environment override and without
replacing host libraries. This is a qualified Cairo/X11 result, not a pristine
runtime, stock launch, Fedora, or Wayland acceptance result.

- Five GUI-generated exports were 256 × 128 and matched the golden RGBA pixels
  exactly. At hard width, smoothing 0 and hue 0, output used five RGB colors and
  preserved source alpha exactly. These conditions do not promise exact Target
  tones or source alpha under smoothing or post-mapping hue operations.
- Save, close, relaunch, and reopen of the golden v6 project preserved its
  embedded source bytes and recipe unchanged.
- The cloud static audit passed: 119 ELF files, zero errors, maximum GLIBC
  requirement 2.39 against the 2.41 ceiling.
- Ordinary FUSE launch failed because `/dev/fuse` was absent. This records the
  cloud environment's mount limitation; a FUSE-capable launch remains untested.
- Stock extracted launch aborted because `libGLESv2.so.2` was missing. The
  successful Cairo override does not resolve that stock-runtime dependency.

The workflow also exposed THR-075: the Ctrl+O tooltip advertises image or
project opening, but its image-only chooser rejects `.chromiator` files. The
dedicated Open Project action worked. Record this as an open UX defect rather
than inferring a project-format failure.

### THR-075 implementation update (2026-10-07)

The header Open action and Ctrl+O now use one chooser filter for the decoder's
supported PNG, JPEG, TIFF, WebP, BMP, and GIF rasters plus `.chromiator`
projects. The selected path is classified before dispatch, preserving the
existing project reader and raster decoder. Welcome Start New Project remains
image-only, and the dedicated Open Project route remains project-only. The
existing dirty-document guard and success-only document replacement path are
unchanged.

Pure routing and replacement-guard tests passed, as did all four display-backed
GTK file-filter tests. `cargo build --locked --bin chromiator` and
`cargo clippy --locked --bin chromiator -- -D warnings` passed. The filter
tests validate accepted and rejected names and content types; the private-Sway
interaction evidence below exercises keyboard activation and chooser readback.
THR-075 remains in progress until user and real GNOME/portal acceptance.

With the THR-077 harness repair, keyboard Ctrl+O from the editor opened the
combined chooser with its raster/project filter. Selecting
`tests/fixtures/release-0.2.0/stage0-scene.png` displayed the four-site image;
repeating Ctrl+O with `desert-dusk-v6.chromiator` displayed the five-site
project. The dedicated Document Menu > Open Project route still showed a
project-only filter; Cancel closed it and retained Site 5. After a temporary
site edit, Ctrl+O showed Save/Discard/Cancel and Cancel retained the unsaved
site. After a preset change, the same guard's Discard continued to the
combined chooser; selecting a disposable invalid `.chromiator` produced a
readable error and left the current image, sites, and processing state visible.
Screenshots: `.codex-work/evidence/ui-run-20261007-145946-166754/` and
`.codex-work/evidence/ui-run-20261007-150424-172142/`. These are private Sway
with GTK's Cairo renderer, not a GNOME/portal acceptance pass.

Commands used:

```sh
cargo test --locked --test core open_intent
cargo test --locked --test core replacement_gate_only_replaces_after_explicit_resolution
cargo test --locked --bin chromiator shell_actions::tests::combined_open_filter_accepts_supported_rasters_and_projects -- --ignored --exact
cargo test --locked --bin chromiator shell_actions::tests::image_only_filter_accepts_supported_rasters_without_projects -- --ignored --exact
cargo test --locked --bin chromiator shell_actions::tests::project_filter_accepts_chromiator_files -- --ignored --exact
cargo test --locked --bin chromiator shell_actions::tests::preset_load_filter_accepts_current_project_and_json_files -- --ignored --exact
cargo build --locked --bin chromiator
cargo clippy --locked --bin chromiator -- -D warnings
```

### THR-078 automatic initial palette update (2026-10-07)

Automatic initialization now accumulates visible full-resolution pixels directly
into a bounded equal-volume 40×40×40 OKLab grid with 0.025 spacing over
`L=[0,1]`, `a,b=[-0.5,0.5]`. Alpha contributes proportionally. A second full-source
pass chooses a real source pixel nearest each occupied voxel's alpha-weighted
OKLab center, with row-major order breaking exact ties. This deliberately
replaces the previous local 3×3 exact-color-support tie-break; the selected
representative is now tied to the voxel center rather than neighborhood color
frequency.

The density pass splats occupied voxels into a normalized Gaussian core with a
0.05 OKLab radius and an equal-cell shell from 0.05 through 0.10. Both averages
use fixed equal-volume kernel weights, including empty cells. A local peak must
reach a 1.5 core-to-shell density ratio and have more than two alpha support for
images whose total alpha exceeds 64; smaller images use a positive-support floor.
Connected equal-density maxima are collapsed before prominence checks, and
plateaus wider than the 0.05 distinctness radius do not become accent sites.
The strongest complete-link coalesced color family remains the main site.
Additional sites come from separated compact density modes, are checked using
their actual source-pixel swatches, stop at 0.05 OKLab distance, and cap at six
without padding. If no compact modes exist, coalesced family centers provide the
smooth-gradient fallback. No renderer, project schema, or saved-recipe behavior
changed.

`tests/core.rs` covers nearby dominant shades with white/cyan accents and
isolated noise, 27-voxel accent support at two or fewer pixels per voxel,
equal-mass diffuse fields with more than two pixels per voxel at two grid phases,
flat plateaus, gradient fallback, a complete-link gray-tone chain, and the six
site cap. `cargo test --test core auto_` passes seven focused tests; the full core
suite passes 80 tests with the artifact generator ignored. The existing 1600²
initializer regression takes 0.69 seconds in debug mode. The 10,000-pixel
dominant-shade regression takes 0.02 seconds for two initializations in debug
mode. These are bounded local timings, not a speedup claim. Strict binary
Clippy and the debug build pass.

The synthetic 100×100 source is
`target/validation/auto-palette-diversity/source.png` (SHA-256
`a9f1f0329537da4a0263b88b86a23abfc13c9cbc1674741a1d0b19500437c0aa`). The
ignored `generate_auto_palette_diversity_processed_artifact` test writes
`after-identity.png` and its site sidecar using `export::export_recipe`; this
shows the initial identity-target result with the brown source, white highlight,
and cyan accent intact. `diagnostic-recolored.png` is a separately labeled
artificial-target rendering used only to distinguish the three regions.

Native review passed in private Sway using the rebuilt debug binary. The
synthetic image moved from one initial site to three: one representative for
the nearby brown shades, one white highlight, and one cyan accent. Spectrum
moved from four sites to six. Native readback of the synthetic site manifest
exactly matches `after-identity.png.json`, including identity Targets equal to
the sampled Source colors. The 100×100 raw identity export preserves source
alpha byte-for-byte and contains 40 white, 40 cyan, 9,919 brown, and one
transparent black pixel. Native app screenshots and sidecars are in
`target/validation/auto-palette-diversity/`; correlated logs are in
`.codex-work/evidence/ui-run-20261007-182741-323846/` and
`.codex-work/evidence/ui-run-20261007-182807-324545/`. Both app stderr logs are
empty. The prior AppImage was not rebuilt for this initializer, so no packaged
application result is claimed. Real GNOME/portal testing and user acceptance
remain pending; the ignored artifact test uses the native export API and does
not establish GUI export-chooser behavior.

### Previous candidate update (2026-10-07)

THR-071 remains **in progress; human acceptance is pending**. This section
records the superseded q93 candidate:
`target/distribution/0.2.0-q93_3i2j/appimage/output/Chromiator-0.2.0-x86_64.AppImage`,
29,583,864 bytes, SHA-256
`6919478aa842246f16f871ae254da54d78f35274e6f6c97daeb088ef136b45a5`. It is
based on HEAD `aff7001ed23e3d3ac63cdec39cd5e57fba17e3c0` plus uncommitted Ctrl+O
and packaging edits. Captured Cargo manifests, application source, build script,
and original packaging icon match the retained build snapshot; the original icon
was preserved byte-for-byte. The audit script's two Adwaita symbolic/status
paths were corrected after snapshot capture, then the completed AppDir and final
AppImage payload were audited. Both final reports cover 119 ELF objects with
zero errors and maximum GLIBC 2.39 against the 2.41 ceiling. Candidate provenance,
all relevant hashes, logs, and audits are retained under
`target/distribution/0.2.0-q93_3i2j/appimage/`.

The stock extracted AppRun passed the required checks in a pinned clean Debian
13.7/glibc 2.41 container without compilers, `pkg-config`, GTK development
packages, app-launch network access, or GPU-device passthrough. Direct runtime
packages were `fontconfig`, `fonts-dejavu-core`, `libegl1`, `libgbm1`,
`libgl1-mesa-dri`, `libgles2`, `libvulkan1`, `mesa-vulkan-drivers`,
`shared-mime-info`, and `xkb-data`; the full 128-package manifest is retained.
Adwaita icons are bundled. An A/B run showed that host `shared-mime-info` is
needed for the desktop MIME database: the minimal image without it had missing
SVG icons, and installing it restored the app, titlebar, and toolbar glyphs.
The AppRun hook loaded the relocated bundled SVG loader and librsvg. The clean
runtime process had `GSK_RENDERER` unset and mapped Debian EGL, GLES, and
Mesa/Gallium userspace providers; no `/dev/dri` device was passed, so this
confirms the default userspace path but not hardware acceleration.

In clean Debian, the welcome and Spectrum example rendered; the v6 project
fixture opened, saved, and reopened with matching embedded source bytes and
complete manifest; raw PNG import reached Ready with four attached sites; and
GUI export was 256 × 128 RGBA with exact pixel equality to the hard golden.
Ordinary FUSE launch on the Fedora host mounted the candidate and rendered both
welcome and Spectrum. The helper first timed out because it searched for
application name `Chromiator`, while the packaged root appeared as
`AppRun.wrapped`. Explicitly selecting `AppRun.wrapped` still exposed only a
top-level frame; the cause is unconfirmed and is recorded under THR-077.
Spectrum was selected using the screenshot. The packaged GUI launch succeeded,
but package AT-SPI readback remains unverified.

All screenshots and workflow artifacts for the q93 candidate are private-Sway
evidence under
`target/distribution/0.2.0-q93_3i2j/appimage/runtime-clean-debian-20261007/`.
That package snapshot was not tested on GNOME/Mutter or through desktop portals.
The later isolated GNOME/portal and THR-073 workflow checks used the debug
application and do not transfer to this older candidate; see the release-
readiness record below and the prior-candidate record in `docs/DISTRIBUTION.md`.
Earlier statements that FUSE was unavailable and stock launch lacked
`libGLESv2.so.2` refer to the superseded build-7 candidate and cloud environment;
they are historical, not current results. Human GUI/accessibility acceptance
and package-specific runtime verification remain separate gates. The general
smoke script was not used. No Stage 2 or acceptance decision is implied.

### Evidence at planning time

- `main` and live `origin/main` are `f2c74ab`; the live v0.2.0 release tag is
  `7f7f735`, published 2026-09-19. The published x86_64 AppImage is 36,891,128
  bytes with SHA-256
  `e5005adb8ed75796bbf222f919d3821695eb18bdcbec719a00aaa828e6168c70`; it
  matches the downloaded and locally built artifact and local `SHA256SUMS`.
  Since that release, only README and DISTRIBUTION differ; there are no app
  changes from the evaluation. Preserve pre-existing tooling/AGENTS, packaging
  artwork, and untracked user assets.
- Static inspection of the AppDir found the main executable's maximum required
  glibc symbol version is 2.35, but that is not the bundle maximum. Seven
  bundled libraries require GLIBC_2.43: libgnutls.so.30, liborc-0.4.so.0,
  libglib-2.0.so.0, liblcms2.so.2, libgraphene-1.0.so.0, libpixman-1.so.0,
  and libgtk-4.so.1. `libpangoft2-1.0.so.0` imports
  `FcConfigSetDefaultSubstitute`; `libharfbuzz-subset.so.0` imports `hb_free`;
  Fontconfig and HarfBuzz providers are not bundled and are resolved from the
  host. This proves the current bundle is not compatible with the proposed
  Debian 13/glibc 2.41 floor by ELF requirements; no clean Debian 13 runtime
  reproduction has been run. A reported glycin runtime failure and the welcome
  decoder's startup panic path remain unreproduced at runtime.
- The host is Fedora 44/glibc 2.43. UI preflight passed, but the new private
  Sway launch was rejected by automatic approval review. There is no fresh GUI
  workflow or clean-system result. No isolated Debian runtime is available.
  The prior release `flatpak-example.png` was inspected and shows the 19-preset
  example with Transition width 0; older screenshots predate the current UI.
- Current focused host results are 69 core tests, 10 transition tests, and one
  preset-library test passing, with two opt-in tests ignored. These are not a
  release, clean-runtime, full-suite, or visual acceptance claim.
  Commands: `cargo test --test core`, `cargo test --test transitions`, and
  `cargo test --test preset_library`.
- Earlier functional results are user-reported and not reverified in this
  planning pass: Desert Dusk at width 0 reportedly produced five RGB values
  with nonzero alpha; alpha was reported exact; width 100 blended; target-only
  edits reportedly left the partition fixed; source-byte embedding and recipe
  round-trip worked; invalid PNG and picker cancellation worked; and six audit
  routes reportedly passed with a workaround. Stage 0 must reproduce these.

### Stage 0 — freeze and characterize the release

Create deterministic, small golden inputs and machine-readable evidence before
changing the renderer or UI: RGB/HSV/OKLab ramps crossing site boundaries,
saturated and neutral colors, near-black and highlights, 256 alpha levels,
transparent pixels carrying nonblack RGB, thin boundary patches, and a flat
illustration. Capture rendered pixels plus the winner site ID for every fixture
pixel; coverage counts alone cannot prove the partition is unchanged. Compare
preview with full-resolution render/export, PNG8 alpha and RGB, dimensions,
source bytes embedded in projects, and reopened recipe values. Exercise hard
width 0 and blended width 100, smoothing, ordered hue operations, and the
zero-site pass-through separately. Keep all existing v6/v3-accepted data and
the 19 preset definitions unchanged; old v5/v2 inputs must still reject
unchanged. Include the Desert Dusk and target-only edit cases above.

The settled decision is to preserve current Voronoi mapping semantics and test
their measured limits. Stage 0 must make alpha and exact-target behavior
explicit: smoothing can change alpha, smoothing σ uses original source-image
pixels for both preview and export, initial and scheduled previews process the
full source before nearest-neighbor reduction, post-mapping hue operations can
change final Target RGB, and zero sites pass through. Test Desert Dusk at width
0, smoothing 0, and hue 0 for membership of every alpha>0 output pixel in its
five encoded Target RGB values and byte-identical source alpha, including zero.
Test full winner-ID maps after Target edits and designed Source/Influence edits,
and exact reopened rendered pixels. THR-074 separately designs strict output.

THR-017 smoothing follow-up, 2026-10-07: the initial and scheduled GTK preview
paths now share the full-source processing helper with export, then publish a
bounded processed float result. PNG8 export pixels reduced with the same
nearest-neighbor sampling are byte-identical to the displayed preview for a
2048×32 patterned, alpha-bearing source at σ 0, 0.1, 10, 11, 25, and 10,000;
the recipes also exercise nonidentity sites and a hue operation. The synthetic
2048×512 native review source and σ=25 identity-target project are generated by
the ignored `write_source_sigma_visual_fixture` test under
`target/validation/source-sigma-parity/`. Private-Sway run
`ui-run-20261007-192021-355905` opened the σ=25 project, read back σ=10,000,
verified Undo returned to 25 and Redo to 10,000, and captured an unclipped
control screenshot with empty stderr. The user accepted this smoothing and
preview/export parity work on 2026-10-07.

Release-readiness checks on 2026-10-07 used optimized synthetic 2048×1024
(2MP) and 4096×2048 (8MP) sources with fine edges and transparent/partial-alpha
pixels. `release_large_image_preview_export_and_cancellation_benchmark`
compared full processing, full-source preview reduction, and PNG8 export at
σ 0, 10, 11, 25, and 10,000; all bounded float results and sampled PNG8 pixels
matched exactly. At 8MP, σ=11 took 3,439 ms for preview, 3,430 ms for full
processing, and 303 ms for PNG8 export; isolated peak RSS was 679,992 KiB.
At σ=25 the corresponding times were 1,571/1,555/297 ms and peak RSS was
549,656 KiB. The 8MP σ=10,000 render exited 9.15 ms after cancellation at
2.00% progress. `release_8mp_latest_request_supersedes_running_smoothing_job`
cancelled an 8MP σ=10,000 preview at 2.23% and produced the replacement
512×256 result 15.28 ms later. These are synthetic, machine-specific results,
not general performance guarantees. Raw JSONL is retained under
`target/validation/release-readiness-20261007/large-image-benchmark/`.

The same release-readiness stage verified the native workflow from PNG import
through built-in preset selection, σ edit, Split comparison, PNG8 export,
project save, and native reopen. The reopened project embedded the exact
imported source bytes; the saved recipe's full render matched the native export
and sampled preview pixels exactly. Cancellation and malformed raster/project
checks preserved the preceding UI state. The ignored verifier is
`verify_native_workflow_source_project_export_artifacts` in `tests/core.rs`;
the native project, export, source, and state snapshots are retained under
`target/validation/release-readiness-20261007/`.

THR-065 was verified on private Sway and automated isolated GNOME 50.5/Mutter.
The workspace keeps the minimum canvas and right-pinned inspector visible; when
the inspector is hidden, Save/Undo/Redo move into the Document menu and the
window can shrink to 600 px. From a 1024 px workspace, the divider was set to
500; hiding the inspector and shrinking to 600 px, showing it again, and growing
to 1180 px restored divider position 656 with the selected inspector width
preserved. A further Sway run clamped extreme drags to 344 and 808 at 1180 px,
then retained visible, vertically scrollable inspector controls at 716×450.
Captures are in `.codex-work/evidence/ui-run-20261007-211111-471880/`,
`ui-run-20261007-211456-488880/`, and
`ui-run-20261007-212156-527488/`. In isolated GNOME/Mutter, maximize at
1280×800 and restore at 1144×736 retained mapping and five-site controls. The
isolated portal open/cancel and PNG8 export/source/project pixel verifier passed;
artifacts are under `target/validation/release-readiness-20261007/gnome/`, with
an empty final app log. The focused geometry test, strict Clippy, and build
passed. These are automated checks, not human GNOME/Mutter or assistive-
technology acceptance. The current-source AppImage build status is tracked
separately in `docs/DISTRIBUTION.md`.

A 2026-10-07 held-mouse follow-up exposed a divider oscillation that the
release-only extreme-position checks above missed. Explicit start/end pane
properties retain GTK's non-shrink child settings, and the viewer now requests
its 344 px minimum; position notifications no longer queue an idle clamp during
dragging. While the window cannot fit the saved inspector width, only actual
pointer or focused-pane keyboard adjustment can replace it. At 1180 px, the
previous binary's 100 rightward held samples included
808, 1117, and 1125 before release returned to 808. The current binary stayed
at 815 for 100 moving and 30 stationary held samples and on release; leftward
dragging held and released at 346. The 1024/500 to 1180/656 resize restoration
still passed. Raw traces are in `target/validation/divider-drag-20261007/`.
The canvas corners are square in the inspected private-Sway capture
`.codex-work/evidence/ui-run-20261007-220459-591391/fixed-ready.png`. Human
GNOME/Mutter acceptance remains pending.
In the final real-pointer workflow, 1024/500 survived hide, 600 px compact
resize, show, and growth to 1180/656. A subsequent narrow 800 px window let
the user deliberately drag to 420, then growth to 1180 px restored 800. The
focused split test, strict binary Clippy, and debug build passed after the
input-origin change. On that final rebuild, 100 moving and 30 stationary held
samples plus release all stayed at 815 on the right and 346 on the left;
`final-right.json` and `final-left.json` are in the same validation directory.
The final-source isolated GNOME/Mutter semantic check reported a 1280 px pane
at position 820 when maximized and a 1144 px pane at position 684 after restore;
the expanded Color sites control and Site 1 remained available in both
`gnome/toggled-controls.json` and `gnome/restored-controls.json`. The GNOME
portal screenshot request returned Response 2 or timed out, so this final pass
has semantic readback but no GNOME visual capture. The inspected private-Sway
`ui-run-20261007-221156-624990/final-restored.png` shows the square canvas
corners and visible inspector. Human GNOME/Mutter visual acceptance remains
pending.

The final-source AppImage `0.2.0-icn3p06p` passed AppDir and extracted-package
audits (119 ELF files, zero errors) and launched through normal FUSE in private
Sway. It loaded the saved project and populated inspector. At a 1438 px pane,
all 100 moving, 30 stationary-held, and release samples stayed at 1073 on the
right and 346 on the left (`packaged-right.json` and `packaged-left.json` in
`target/validation/divider-drag-20261007/`). The inspected private-Sway
`ui-run-20261007-222015-660143/packaged-corners.png` shows square corners
and the populated inspector; packaged stderr was empty. This was a scoped
package smoke. The earlier candidate's clean-Debian project and GNOME portal
export workflows were not repeated for this package; see `docs/DISTRIBUTION.md`.

### Stage 1 — P0 package/runtime repair

Treat Debian 13 x86_64/glibc 2.41 as a proposed minimum, not a certified
baseline, and Fedora 44/GNOME/Wayland as the current native target. Establish a
pinned build environment whose complete bundled ELF closure fits the proposed
floor. Audit all bundled ELF files and helpers, `DT_NEEDED`, symbol versions,
and external Fontconfig/HarfBuzz/glycin providers; test fault handling for
missing optional decoration assets without masking real artwork decode or
Spectrum Example errors.

The smoke harness must accept an explicit package path and unique output
directory, use prebuilt fixtures, never invoke Cargo, and never hard-code
`target/debug`. On a clean minimum-baseline machine, exercise both FUSE launch
and extracted `AppRun` separately. Start normally without `--example` to cover
welcome decoding, then use the GUI to import PNG, export PNG8, save a project,
and reopen it. Record package/hash, environment, dependency closure, logs,
screenshots, and output hashes. Confirm no development dependencies or
unbundled custom libraries are needed beyond declared desktop/runtime
dependencies. A static ELF pass or host-only smoke is not clean-machine
acceptance.

### Stage 2 — observe and propose the editing workflow

Only after Stage 0/1 review, observe small-palette remapping, Target edits,
independent Source/Target changes, Influence, geometry, and hard-versus-blended
results using useful before/after artwork. Then submit a bounded visual/UI
proposal using the existing Transition width and one undo transaction. Explain
Source anchors versus output colors, matching versus blend space, and what
markers do not enter export. Do not add a new serialized mode, change current
defaults/presets, promise that a Target swatch always equals final output, or
redesign the engine in this stage. THR-034/035/061 remain cross-references;
implementation requires a later accepted scope.
Make conflicting settings visible with state-derived feedback; do not silently
reset smoothing or saved hue operations. Keep deliberate gradients available.

### Stage 3 — integrated workflow and review

After separate approval, verify welcome → import → edit → compare → export →
save → close/reopen, including preset choice and the requested regressions.
Compare reopened recipe values, embedded source bytes, output dimensions and
full pixel results; inspect marker-free exports beside screenshots showing
editor markers. Test dialog cancellation by snapshotting document, selection,
history, redo, dirty state, and output. Actual long-running render/export
cancellation is a separate unverified follow-up, not proved by dialog Cancel.
Invalid raster/project input must
preserve the active document. Run focused regression tests immediately, then
the appropriate locked full suite and strict Clippy/build checks. Retain image
artifacts and logs. Automated Sway evidence is not human GNOME acceptance.

### Proposed sequencing and limits

Keep one writer at a time. After approval, Sol owns the bounded packaging
script/build changes; Luna owns welcome fallback and its focused regression,
then the bounded fixture tests; Sol reviews color/output evidence; Astra
coordinates and reviews integration. Confirm the exact file allowlist before
implementation and do not change agent/model configuration. Defer broad
refactoring, GPU/Rayon, THR-065 layout, THR-066 breaking cleanup,
PNG16/EXR/ICC expansion, large-image performance, long-running cancellation,
Flatpak validation, and full assistive-technology acceptance unless separately
scoped and tested. Passing host tests, implementation, package construction,
and user acceptance are distinct gates.

Proposed ownership: Luna owns fixtures in `tests/fixtures/` and focused
`tests/core.rs`, `tests/transitions.rs`, `tests/preset_library.rs` changes;
Sol owns `scripts/build_distributions.py`, bounded new packaging build/smoke
files, and `docs/DISTRIBUTION.md`; Luna owns the welcome fallback in
`src/main.rs`. Later UI scope is `src/transition_controls.rs`, `src/site_editor.rs`,
and bounded wiring in `src/main.rs`, using existing session commands. No engine
or schema edits are authorized by this plan. Update this ledger and ISSUES at
each stage, README at milestone completion. Sol reviews numerical regressions;
Astra reviews integration. Use the requested fixed profile pairs sequentially.
Photography breadth, PNG16/EXR, ICC/profile combinations, large-image performance,
Flatpak, and real assistive-technology testing remain unverified in this scope.

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
   canvas add/drag gestures, and Source/Result comparison. THR-065's right-pinned
   sizing, scrolling, extreme split positions, hide/show, and 600 px compact
   window behavior passed automated Sway and isolated GNOME/Mutter checks. The
   inspector remains user-toggled rather than auto-collapsing during resize;
   broader theme, focus, keyboard, and canvas-gesture review remains open.
9. Measure release preview and 3840x2160 processing on representative images,
   with repeated warm medians, peak RSS, cancellation latency, and exact output.
   Adopt bounded Rayon only if measurements support it; no speculative
   parallel-processing dependency is required for the refactor.

Use `.agents/skills/gtk-wayland-debug/SKILL.md`: wait, scoped controls, inspect,
semantic action, readback. Use coordinates only for spatial canvas interactions.
Retain raw logs, screenshots, and output artifacts; stop the private session
when finished. Native automation does not replace human GNOME acceptance.

### Preset menu and Color mapping follow-up (2026-10-07)

User acceptance: the user confirmed that the UI works as intended and accepted
the accumulated work on 2026-10-07. THR-076 is closed. The automation limitation
THR-077 and remaining THR-071 packaging checks remain separate follow-ups; this
acceptance does not manufacture missing runtime evidence.

The preset commands now live in the header menu. The selector requires Apply,
supports Cancel/reopen, combines built-in looks with the XDG personal store,
and distinguishes personal entries. Load accepts current v3 preset JSON and
current v6 project archives. Project loading uses the validated `project::open`
reader on the file worker, then builds a detached `Preset` from its recipe;
source bytes, interpretation, export defaults, and document identity are not
replaced. Save writes processing settings only. The Color mapping controls now
sit inside a collapsed-by-default disclosure, with their explanations in
tooltips and accessible descriptions.

Headless coverage exercises the real loader against
`tests/fixtures/release-0.2.0/desert-dusk-v6.chromiator` and
`resources/presets/arcade-four.json`. A temporary v6 copy includes a source
position to verify that project-derived preset sites detach. Applying the
imported JSON through `EditCommand::ReplaceRecipe` in `DocumentSession` creates
one undo step;
undo/redo preserve the active source, interpretation, and export defaults. A
rejected v5 project leaves the active document and existing undo/redo history
unchanged. `cargo test --locked --test core preset` passed 14 tests; the focused
loader, session-apply, and rejected-project tests also passed individually.
Binary tests passed 16 tests with 2 display-only cases ignored; the locked GTK
build and strict binary Clippy passed.

Private-Sway screenshots show the header and collapsed/expanded mapping section
at standard size (`.codex-work/evidence/ui-run-20261007-120516-18881/01-collapsed.png`,
`02-expanded.png`) and the adjusted 1024×600 layout
(`.codex-work/evidence/ui-run-20261007-123435-41527/15-narrow-adjusted-expanded.png`,
`16-narrow-adjusted-collapsed.png`). `17-narrow-presets-menu.png` shows all
three menu entries and `18-matching-tooltip.png` shows the moved explanation.
Focused interactions applied a built-in and an XDG personal preset, verified
Undo availability, cancelled and reopened the selector, loaded Arcade Four
JSON, and saved matching processing JSON at
`/tmp/chromiator-preset-menu-check/data/chromiator/presets/preset-4824c10d7b76b422.json`.
The narrow capture used a semantic pane adjustment to position 610; THR-065
layout enforcement was later verified separately. The `.chromiator` file
chooser flow was not independently verified in this preset-menu check.

The earlier blanket semantic readback crashed around dynamic widget removal.
The retained gdb trace at
`.codex-work/evidence/ui-run-20261007-121617-32737/app.stdout.log` reaches
`handle_accessible_method` → `g_variant_new` → SIGSEGV on GTK 4.22.5, GLib
2.88.3, and AT-SPI 2.60.7. A temporary relations-disabled, dispatch-only probe
also failed near an asynchronous chooser transition. A later fatal-critical gdb
trace at `.codex-work/evidence/ui-run-20261007-145840-165580/app.stdout.log`
pinpointed GTK's application-root `GetChildAtIndex` branch: after iterating
top-level windows, `gtk_at_spi_context_get_context_path` returned NULL and
GTK passed that path into `g_variant_new`. This is an upstream GTK 4.22.5
serialization fault triggered by a disappearing window, not evidence that
Chromiator's application relations or descriptions are invalid.

The private AT-SPI helper now obtains public `Accessible.GetChildren` snapshots
and indexes only the returned local child list. It preserves normalized roles,
canonical actions, value/editable-text/selection/relations, and live focus
readback. The method trace at
`.codex-work/evidence/ui-run-20261007-150424-172142/adapter-bus-methods.log`
has five application-root `GetChildren` calls and zero application
`GetChildAtIndex` calls (the registry's own root indexing remains). Eighty
concurrent full-tree polls and 12 chooser open/Cancel/after cycles passed;
fresh chooser absence and Site 4 readback followed, with no GTK critical in
the app log. Select Preset Cancel/reopen, Ink & Paper selection and Apply,
numeric Value and editable-text commits, and Ctrl+O dirty/error paths also
passed. The only later stderr entry is the expected invalid-Zip error from
the deliberately corrupt project. The application's accessibility metadata
was left unchanged. THR-077 tracks this bounded harness repair; real
GNOME/portal and full assistive-technology acceptance remain unverified. The
subsequent user acceptance of THR-076 is recorded above.
