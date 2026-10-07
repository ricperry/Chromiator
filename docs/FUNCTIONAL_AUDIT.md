# Voronoi refactor verification ledger

## Status

Final GTK4 conversion: **partially verified, with an open layout regression**.
The follow-up fixes passed the application build, strict Clippy, and 93 tests.
Bounded private-Sway checks verified save/history/picker fixes; THR-065 remains open;
see `BUG_HUNT_FOLLOWUP_2026-09-19.md`. Do not infer complete workflow success from those
checks or merely implemented audit routes. No performance improvement is claimed.

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

Stage 1 / THR-071 remains **in progress, acceptance pending**. Remaining gates:
resolve and verify the stock extracted-launch dependency/prerequisite contract;
repeat the supported workflow on a clean minimum-runtime system without relying
on development packages; verify ordinary launch on a FUSE-capable system; and
complete Fedora GNOME/Wayland GUI import/export/save/reopen verification. Native
CUA control was unavailable for the Fedora task, so that gate remains open.
The qualified cloud result supplies bounded workflow evidence without closing
the broader THR-073 integrated-workflow scope. This documentation update makes
no implementation change, commit, push, or acceptance claim.

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

The settled decision is to preserve current rendering and test its measured
limits. Stage 0 must make alpha and exact-target behavior explicit:
smoothing can change alpha, preview smoothing radius is measured in
preview pixels while export uses source pixels, post-mapping hue operations can
change final Target RGB, and zero sites pass through. Test Desert Dusk at width
0, smoothing 0, and hue 0 for membership of every alpha>0 output pixel in its
five encoded Target RGB values and byte-identical source alpha, including zero.
Test full winner-ID maps after Target edits and designed Source/Influence edits,
and exact reopened rendered pixels. THR-074 separately designs strict output.

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
The narrow capture used a semantic pane adjustment to position 610; default
splitter placement remains tracked by THR-065. The `.chromiator` file chooser
flow was not independently verified.

Full semantic readback is still blocked by a reproducible AT-SPI client crash
around dynamic widget removal/rebuild during a blanket object-path query. The
retained gdb trace at
`.codex-work/evidence/ui-run-20261007-121617-32737/app.stdout.log` reaches
`handle_accessible_method` → `g_variant_new` → SIGSEGV on GTK 4.22.5, GLib
2.88.3, and AT-SPI 2.60.7. A temporary relations-disabled, dispatch-only probe
also failed near an asynchronous chooser transition; the exact trigger remains
unisolated, so the application's accessibility descriptions and relations were
left unchanged. THR-077 tracks this harness limitation. Full AT-SPI coverage,
project chooser readback, and real GNOME/portal acceptance remain unverified;
these automated results alone do not establish those broader checks. The
subsequent user acceptance of THR-076 is recorded above.
