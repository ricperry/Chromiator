# Building distribution bundles

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
an explicit GDK_BACKEND override. Fontconfig and HarfBuzz, including their
non-system dependencies, are explicitly requested from the Debian builder and
must appear as loadable bundled providers. System font configuration and fonts
remain host resources. GLIBC and GPU drivers remain host dependencies. The
audited host SONAME set is `ld-linux-x86-64.so.2`, `libc.so.6`,
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
A passing static audit is not clean-system acceptance.

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
pre-existing local icon edit copied by the build snapshot. See
`docs/FUNCTIONAL_AUDIT.md` for provenance and failed-attempt history. The
candidate is for internal testing; clean-Debian and Fedora GUI workflows have
not run. Do not advertise Debian 13 compatibility until those checks pass.

When package launches are authorized on a clean target system, run
`scripts/smoke_appimage.sh /absolute/package.AppImage /new/output/directory
/absolute/fixture.png`. It records package/fixture hashes and separate normal
FUSE and extracted-AppRun launch attempts. Exit status alone does not establish
GUI, import, save, export, or accessibility success; inspect the window, logs,
screenshots, and output files separately. This runner has not been executed in
the current Stage 1 work.

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
