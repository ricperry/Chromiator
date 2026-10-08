# Building distribution bundles

## Current 0.3.0 checkpoint (2026-10-07)

The 0.3.0 pre-alpha GitHub release targets x86_64 AppImage and Flatpak bundles.
The user accepted the current implemented application work on 2026-10-07;
distribution acceptance remains separate from that product decision. Final
0.3.0 build, package, and runtime results are to be recorded here before
publication. The 0.2.0 artifact paths, hashes, and workflow evidence below are
historical and do not transfer to rebuilt 0.3.0 packages.

Both build snapshots now include only
`assets/examples/SpectrumBreakpoint.png` from the artwork directory. Loose
artwork, editable `.kra` files, personal projects, and other local assets are
excluded from AppImage and Flatpak source snapshots.

The [0.2.0 Linux preview](https://github.com/ricperry/Chromiator/releases/tag/v0.2.0)
was published from checkpoint `7f7f7353ff315c73636e5e3afc7a82ac9e7a37be`.
Both x86_64 bundles passed six private-Sway audit scenarios each; uploaded
downloads passed SHA-256 verification. Release notes and build-info.json record
the remaining compatibility, portal, and accessibility verification limits.

Run from the checkout with Python 3.11+ and Cargo. AppImage runtime validation
also requires host `readelf` from binutils. Version comes from
Cargo.toml. The AppImage path currently supports x86_64 only; aarch64 has no
pinned baseline or acceptance evidence. Flatpak remains an independent build
path. Scripts do not install packages on the host, alter remotes, or publish.

```sh
python3 scripts/build_distributions.py appimage
python3 scripts/build_distributions.py flatpak
python3 scripts/build_distributions.py all
```

Each run uses a new directory under target/distribution. Successful runs print
the paths to versioned .AppImage/.flatpak bundles and SHA256SUMS. Failed builds
remain available for diagnosis and never overwrite previous artifacts.
Use `--check` to check executable availability and pinned tool hashes without
building. It does not certify the build image or package runtime.

## AppImage prerequisites

The AppImage entrypoint builds inside a pinned Debian 13 x86_64 container with
a proposed GLIBC 2.41 ceiling. `packaging/appimage/build-lock.json` records the
amd64 base-image digest, 2026-09-18 Debian and security archive snapshot, Rust
1.97.1 toolchain archive and SHA-256, and the exact linuxdeploy and GTK-plugin
hashes. Debian archive signatures remain checked. The container build installs
packages from that snapshot, checks GLIBC again after installation, records
installed package versions, and verifies the Rust tarball before installation.
The pinned Debian 13 x86_64 builder ran on 2026-10-03 with glibc 2.41; the Rust
1.97.1 archive hash verified and offline release compilation passed. The staged
AppDir audit passed for 119 ELF files, with maximum GLIBC 2.39 requirements,
zero errors against 2.41, successful Debian `ldd -r` resolution, and loadable
bundled Fontconfig/HarfBuzz providers. This is build and AppDir evidence;
package runtime validation remains pending.

### Pinned AppImage output runtime

The output plugin receives the official, versioned AppImage/type2-runtime
20251108 x86_64 asset. Recorded provenance includes source commit
`dd6cebedcbddde9c82f89b011e8e1d40b6e43868`, release ID 260789861, asset ID
326011592, size 944632, and SHA-256
`2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d`. The
release is not marked immutable; the content hash is the build pin. The local
asset's digest was checked against the GitHub API metadata.

Inspect release metadata and retrieve the pinned asset with:

```sh
curl --fail --silent --show-error https://api.github.com/repos/AppImage/type2-runtime/releases/tags/20251108
mkdir -p target/distribution-tools
curl --fail --location --silent --show-error https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-x86_64 --output target/distribution-tools/runtime-x86_64-20251108
printf '%s  %s\n' '2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d' 'target/distribution-tools/runtime-x86_64-20251108' | sha256sum -c -
```

The build uses `--appimage-runtime` (default:
`target/distribution-tools/runtime-x86_64-20251108`). Host and builder both
verify the hash, size, ELF64 little-endian x86_64 identity, and GLIBC ceiling.
The file is staged read-only at `/tools/runtime-x86_64` and passed as
`LDAI_RUNTIME_FILE`, the runtime-file input consumed by
[linuxdeploy-plugin-appimage](https://github.com/linuxdeploy/linuxdeploy-plugin-appimage/blob/master/src/main.cpp).
The builder runs with `--network=none`; missing or mismatched input fails
closed and cannot fall back to the mutable `continuous` runtime URL.

Supply the exact retained x86_64 linuxdeploy and GTK plugin tools in
`target/distribution-tools/` or pass `--linuxdeploy` and `--gtk-plugin` paths.
Hash mismatches stop the build. The former `continuous` release URL is mutable;
do not fetch it as a substitute for the pinned binary. Linuxdeploy contains
the AppImage output plugin; the pinned GTK plugin uses local GLib, GTK, and
GdkPixbuf tools. Podman builds from the narrow `packaging/appimage` context.
After dependencies are staged, the source snapshot, Cargo vendor tree, and
tools are mounted into the container; compilation and deployment run with
network access disabled. Only the embedded Spectrum example is copied from
`assets/`; user assets and unrelated checkout files are excluded.

The GTK plugin bundles GTK dependencies, resources, and runtime hooks. The
script replaces its forced X11 setting with Wayland/X11 fallback while retaining
an explicit GDK_BACKEND override. Its runtime hook points GdkPixbuf at the
relocated bundled loader cache, module directory, and library directory.
Fontconfig and HarfBuzz, including their non-system dependencies, are explicitly
requested from the Debian builder and must appear as loadable bundled providers.
The Adwaita icons used by the UI and their copyright notice are bundled.
System MIME data (`shared-mime-info`), font configuration and fonts, EGL/GLES
userspace, GLIBC, and GPU drivers remain host resources. The audited host SONAME
set is `ld-linux-x86-64.so.2`, `libc.so.6`,
`libgcc_s.so.1`, `libm.so.6`, `libresolv.so.2`, and `libvulkan.so.1`.
The builder does not bundle GLIBC or GPU drivers.

Before AppImage creation, `scripts/audit_appimage.py` checks every AppDir ELF,
including GTK modules and image loaders, against GLIBC 2.41; resolves DT_NEEDED
from the bundle or the lock's narrow host list; checks the reported Fontconfig
and HarfBuzz imports against bundled providers; checks the loader cache and
runtime hook for stale build paths; and runs relocation resolution with the
Debian loader. The script also extracts and audits the completed AppImage and its outer
runtime. The 2026-10-04 build-7 candidate passed both final audits with 119 ELF
files, zero errors, and maximum GLIBC 2.39. Reports are
`target/distribution/0.2.0-vsd173k4/appimage/output/abi-audit.json` and
`target/distribution/0.2.0-vsd173k4/appimage/output/packaged-abi-audit.json`.
That candidate is superseded by the 2026-10-07 internal candidate below. A
passing static audit alone is not clean-system acceptance.

The published v0.2.0 Fedora-built AppImage fails this proposed ceiling: seven
bundled libraries require GLIBC 2.43, and Fontconfig/HarfBuzz providers are
host-resolved. The 2026-10-03 output attempt predates the runtime pin and failed
on appimagetool's mutable runtime fetch. The updated command was rejected before execution in the earlier task.
The continuation task obtained approval through its supported execution path.
Build 5 then exposed final-pass Vulkan-loader reinsertion; build 6 exposed
exclusion-option syntax. Final packaging now repeats one
`--exclude-library=<SONAME>` per existing prohibited graphics loader, preserving
the audited closure. Build 7 completed with the same strict audits and runtime
pin. Candidate: `target/distribution/0.2.0-vsd173k4/appimage/output/Chromiator-0.2.0-x86_64.AppImage`,
28,228,088 bytes, SHA-256 `a123e544a6069701235a338c2a0174a972db1fe7e5a0a2f782318732c3bdf621`.
It is based on checkpoint `425760c` plus the local packaging fix and the
pre-existing local icon edit copied by the build snapshot. The build-7 candidate
is superseded; its provenance and failed-attempt history remain historical
records in `docs/FUNCTIONAL_AUDIT.md`.

### Previous internal candidate (2026-10-07)

The pinned build produced
`target/distribution/0.2.0-q93_3i2j/appimage/output/Chromiator-0.2.0-x86_64.AppImage`,
29,583,864 bytes, SHA-256
`6919478aa842246f16f871ae254da54d78f35274e6f6c97daeb088ef136b45a5`. It is
based on HEAD `aff7001ed23e3d3ac63cdec39cd5e57fba17e3c0` plus the retained dirty
Ctrl+O and packaging edits. The captured Cargo manifests, application source,
build script, and original packaging icon match the build snapshot byte-for-byte;
the original icon SHA-256 is
`423e47d2f1a488b2d5439da12b0e2b07cbff18612edbc33c3013c1d71b4770ee`. The audit
script's two Adwaita symbolic/status paths were corrected after source snapshot
capture; the corrected audit was run on the completed AppDir and final packaged
payload. Both final audits report 119 ELF objects, zero errors, and maximum
GLIBC 2.39 against the 2.41 ceiling. Evidence, source hashes, and logs are under
`target/distribution/0.2.0-q93_3i2j/appimage/`; no audit was bypassed.

This candidate predates the accepted source-pixel smoothing and preview/export
parity changes, as well as the later THR-065 layout follow-up. It remains
package-validation evidence for its captured source snapshot. The replacement
current-source candidate is documented below.

The clean runtime reproduction used Debian 13.7/glibc 2.41 from the pinned
Debian 13 base digest, with no compiler, `pkg-config`, GTK development package,
network access during app launch, or passed-through GPU device. The direct
runtime packages were `fontconfig`, `fonts-dejavu-core`, `libegl1`, `libgbm1`,
`libgl1-mesa-dri`, `libgles2`, `libvulkan1`, `mesa-vulkan-drivers`,
`shared-mime-info`, and `xkb-data`; the retained manifest records all 128
installed packages including dependencies. The AppDir was mounted read-only.
`shared-mime-info` is required by the tested desktop path: without its MIME
database SVG icons were absent; installing it restored app, titlebar, and
toolbar icons. The Adwaita icon files themselves are bundled. The AppRun hook
resolves the bundled SVG loader and librsvg from the relocated bundle. The host
supplies `libEGL.so.1` through `libegl1` and `libGLESv2.so.2` through `libgles2`.

The stock AppRun completed the welcome and Spectrum example, opened the v6
project fixture, saved and reopened it with the same embedded source bytes and
full manifest, imported a PNG, and exported a 256 × 128 RGBA image with exact
pixel equality to the hard golden. The container had no `/dev/dri` device and
`GSK_RENDERER` was unset; process maps show EGL, GLES, and Mesa/Gallium userspace
libraries loaded. This verifies the default renderer path with these userspace
providers, not hardware acceleration. An ordinary FUSE launch also mounted and
rendered the welcome and Spectrum screens on the Fedora host. The helper's
AT-SPI name lookup first timed out because the packaged root is
`AppRun.wrapped` while the helper requested `Chromiator`. Explicitly selecting
`AppRun.wrapped` still exposed only a top-level frame; the cause is unconfirmed
and is tracked under THR-077. Spectrum was selected from the screenshot. The
FUSE GUI launch succeeded, but package AT-SPI readback remains unverified.

Screenshots, logs, package/provider manifests, saved project, output, and hashes
are retained in
`target/distribution/0.2.0-q93_3i2j/appimage/runtime-clean-debian-20261007/`.
These were private-Sway checks. Fedora GNOME/Mutter, desktop-portal behavior,
and human workflow/accessibility acceptance remain open; THR-071 stays
`in-progress`. The earlier build-7 and 2026-10-04 cloud notes below are historical
evidence for the superseded candidate and do not describe this candidate's FUSE
or clean-runtime results. The generic `scripts/smoke_appimage.sh` was not used
for this run.

When package launches are authorized on a clean target system, run
`scripts/smoke_appimage.sh /absolute/package.AppImage /new/output/directory
/absolute/fixture.png`. It records package/fixture hashes and separate normal
FUSE and extracted-AppRun launch attempts. Exit status alone does not establish
GUI, import, save, export, or accessibility success; inspect the window, logs,
screenshots, and output files separately. It was not used for the 2026-10-07
candidate checks; manual private-Sway and clean-container procedures were used.

### Superseded source-pixel candidate (2026-10-07)

The pinned build produced
`target/distribution/0.2.0-zdff2hnm/appimage/output/Chromiator-0.2.0-x86_64.AppImage`,
29,608,440 bytes, SHA-256
`2de350f485847f2df97a8991393c6b1e6d009a6d281d5e14d43d2697b41f97e5`. It is
based on checkpoint `98c28be` plus the retained worktree changes at that build.
Its captured `src/main.rs` SHA-256 was
`baff160f12ef8689f96a8560d4ac8238cef02aa2b593b15b5c8ae113d16e8668`.
That source still used a deferred GtkPaned position clamp and predates the
held-drag and canvas-corner fixes below. Both the AppDir and packaged-payload audits
pass for 119 ELF files, zero errors, and maximum GLIBC 2.39 against the 2.41
baseline. The run and its audits are under
`target/distribution/0.2.0-zdff2hnm/appimage/`; both the run-level and output
`SHA256SUMS` checks pass.

Package runtime verification passed in two automated environments. On Fedora's
isolated GNOME 50.5/Mutter Wayland session, the ordinary FUSE AppImage launched
without extraction or renderer overrides. AT-SPI readback through the packaged
`AppRun.wrapped` root found the mapping and five color sites, which remained
visible after maximize/restore. Portal Open/Cancel and PNG8 Save passed; the
portal export at
`target/validation/release-readiness-20261007/gnome/packaged-portal-export.png`
is byte-identical to the native export. Packaged app stderr was empty.
`packaged-restored.png` is the maximized capture; `packaged-final-restored.png`
shows the window restored, both in the same `gnome/` evidence directory.

In the clean Debian 13.7/glibc 2.41 runtime, stock AppRun completed with the
default renderer and no `GSK_RENDERER` override, compiler, `pkg-config`, host
Adwaita theme, or launch-time network. The packaged project loaded, exported
PNG8, saved with Save As, and reopened through the native chooser. The artifact
verifier confirmed exact embedded source bytes and pixel equality among the
reopened recipe render, export, and bounded preview samples. Evidence includes
`target/validation/release-readiness-20261007/clean-debian/packaged-saved.chromiator`,
`packaged-export.png`, and the private-session capture
`.codex-work/evidence/ui-run-20261007-212156-527488/packaged-debian-reopened.png`;
the clean-runtime log records Debian 13.7/glibc 2.41. This Debian project
workflow and the GNOME portal checks do not represent a full preset/edit/reopen
workflow on GNOME; that path passed in private Sway and is documented in the
functional audit. Package AT-SPI was read successfully, but human desktop review
and user acceptance remain pending.

### Current-source divider-fix candidate (2026-10-07)

The pinned build produced
`target/distribution/0.2.0-icn3p06p/appimage/output/Chromiator-0.2.0-x86_64.AppImage`,
29,608,440 bytes, SHA-256
`aac5f8904f65321b573414656c5ece703054585d0ba0b802ab8c6ee40f4eff69`.
The captured source matches the final local `src/main.rs`,
`resources/window.ui`, and `resources/chromiator.css` hashes
`1debcc5149b9438adee791c23764bbd00062257dac45d80bc207e3e04f10a3d7`,
`2484316bc9dde9e2d8dc93ece73d89adea3b69e4f43ab8a71bc129e6fa061884`,
and `1ca80f62e53cb0112880f4d1d76a49be7db55bb798431c53130ccda275e8f6e9`.
All 22 unrelated worktree file hashes remained unchanged from the preflight
inventory. The AppDir and extracted package audits each passed with 119 ELF
files and zero errors
against the proposed GLIBC 2.41 baseline. Build log and scoped evidence are
under `target/validation/divider-drag-20261007/`; the two audit JSON files
are under `target/distribution/0.2.0-icn3p06p/appimage/output/`.

The ordinary FUSE AppImage launched in private Sway, loaded a saved project,
and exposed the populated Color sites inspector. In the package itself, all
100 moving, 30 stationary-held, and release divider samples stayed at 1073
on the right and 346 on the left of the 1438 px pane; raw readback is in
`packaged-right.json` and `packaged-left.json`. The inspected
`.codex-work/evidence/ui-run-20261007-222015-660143/packaged-corners.png`
shows square canvas corners and the populated inspector; packaged stderr was
empty. The generic app-start helper's default `Chromiator` selector timed out
because this packaged root is named `AppRun.wrapped`; an explicit selector
found Color sites and the checks completed. This scoped smoke did not rerun
the earlier candidate's GNOME portal/export or clean-Debian project workflows.
Those prior results remain historical evidence for that build. Human desktop
review and user acceptance remain pending.

## Flatpak prerequisites

Install flatpak-builder and configure Flathub yourself. Install matching
org.gnome.Platform and org.gnome.Sdk (default branch 50), plus the compatible
org.freedesktop.Sdk.Extension.rust-stable extension. Its branch is determined by
the GNOME SDK's extension metadata, not necessarily the GNOME branch number.
An alternate installed SDK can be selected with `--runtime BRANCH`.

Cargo dependencies are vendored from Cargo.lock before entering the sandbox;
the actual build is locked and offline. Fetch dependencies first if building
without network. The generated manifest and source snapshot remain alongside
the bundle. No host Rust binary is copied into the Flatpak.

The sandbox grants Wayland, fallback X11, IPC, and graphics access, but no network
or broad host filesystem access. File access uses GTK's desktop portals.
Personal presets live inside the app's private Flatpak data directory; host
presets are not automatically migrated. Verify open/save/export and recent-file
access through portals before distribution.

```sh
flatpak install --user /path/to/Chromiator-0.2.0-x86_64.flatpak
flatpak run io.github.chromiator.Chromiator
```

## Release checks and licensing

Both bundles carry LICENSE.md, THIRD_PARTY_NOTICES.md, and licenses/. Regenerate
Rust notices after dependency changes. AppImage bundling adds host libraries,
themes, and icons: inventory those exact files and their licenses/source
obligations before distributing. The checked-in notice inventory is not a full
binary-distribution compliance audit. Provide corresponding source for the
exact application build, including local modifications and build instructions.

Before publishing, test both packages on clean systems: launch, keyboard and
accessibility, source/project open, source/target picker, preset save, project
save/reopen, and PNG/EXR export. Verify the checksums from the printed run
directory with `sha256sum -c SHA256SUMS`. Build success is not release acceptance.

The packaging icon is original project artwork. AppStream metadata is CC0-1.0;
the application and original icon use the project's GPL-3.0-or-later license.
