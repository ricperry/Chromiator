# Building distribution bundles

Run from the checkout with Python 3.11+, Cargo, and the normal release-build
dependencies. Version comes from Cargo.toml; only native x86_64/aarch64 builds
are supported. Scripts do not install tools, alter remotes, or publish releases.

```sh
python3 scripts/build_distributions.py appimage
python3 scripts/build_distributions.py flatpak
python3 scripts/build_distributions.py all
```

Each run uses a new directory under target/distribution. Successful runs print
the paths to versioned .AppImage/.flatpak bundles and SHA256SUMS. Failed builds
remain available for diagnosis and never overwrite previous artifacts.
Use `--check` to check executable availability without building.

## AppImage prerequisites

Supply trusted, architecture-matching [linuxdeploy](https://github.com/linuxdeploy/linuxdeploy/releases)
and [linuxdeploy-plugin-gtk.sh](https://github.com/linuxdeploy/linuxdeploy-plugin-gtk).
Place both on PATH (or supply `--linuxdeploy /absolute/path/to/linuxdeploy.AppImage`).
Use a current GTK4-capable plugin; linuxdeploy must include its AppImage output
plugin. Verify upstream downloads before making them executable. Nothing is
downloaded or executed automatically by the packaging script.

The GTK plugin bundles GTK dependencies, resources, and runtime hooks. The script
replaces its forced X11 setting with Wayland/X11 fallback while retaining an
explicit GDK_BACKEND override. The host needs the GTK development tools used by
that plugin, including GLib schema and GdkPixbuf loader tools.

Build on the oldest distribution you intend to support that supplies GTK >=4.12.
A Fedora-built AppImage does not imply compatibility with older glibc systems.
GPU drivers remain host dependencies. Test extracted execution as well as normal
FUSE execution on clean target systems; do not advertise portability from a
successful build alone.

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
