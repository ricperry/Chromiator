#!/usr/bin/env python3
"""Build local AppImage/Flatpak bundles. Never install or publish artifacts."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tempfile
import tomllib

import audit_appimage

ROOT = Path(__file__).resolve().parents[1]
APP_ID = "io.github.chromiator.Chromiator"
APPIMAGE_LOCK = ROOT / "packaging/appimage/build-lock.json"
GRAPHICS_DRIVER_SONAMES = (
    "libGL.so.1", "libEGL.so.1", "libvulkan.so.1", "libgbm.so.1", "libdrm.so.2",
)


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def check_pinned_tool(path, expected):
    path = Path(path).resolve()
    if not path.is_file() or digest(path) != expected:
        raise SystemExit(f"Pinned tool unavailable or hash mismatch: {path}")
    return path


def check_pinned_runtime(path, lock):
    """Reject missing, substituted, or wrong-architecture output runtimes."""
    runtime = check_pinned_tool(path, lock["runtime_sha256"])
    if runtime.stat().st_size != lock["runtime_size"]:
        raise SystemExit(f"Pinned AppImage runtime has wrong size: {runtime}")
    with runtime.open("rb") as source:
        header = source.read(20)
    if (len(header) != 20 or header[:4] != b"\x7fELF" or header[4:6] != b"\x02\x01"
            or int.from_bytes(header[18:20], "little") != 62):
        raise SystemExit(f"Pinned AppImage runtime is not x86_64 ELF: {runtime}")
    required = audit_appimage.required_glibc(audit_appimage.readelf(runtime, "--version-info"))
    baseline = tuple(map(int, lock["baseline_glibc"].split(".")))
    if any(version > baseline for version in required):
        raise SystemExit(f"Pinned AppImage runtime exceeds GLIBC {lock['baseline_glibc']}: {runtime}")
    return runtime


def run(*args, cwd=ROOT, env=None):
    print("+", " ".join(map(str, args)), flush=True)
    subprocess.run(list(map(str, args)), cwd=cwd, env=env, check=True)


def tool(name):
    resolved = shutil.which(name)
    if not resolved:
        raise SystemExit(f"Missing executable: {name}. See docs/DISTRIBUTION.md.")
    return str(Path(resolved).resolve())


def install_metadata(prefix):
    for suffix, directory in [
        ("desktop", "share/applications"),
        ("svg", "share/icons/hicolor/scalable/apps"),
        ("metainfo.xml", "share/metainfo"),
    ]:
        dest = prefix / directory
        dest.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / "packaging" / f"{APP_ID}.{suffix}", dest)
    notices = prefix / "share/licenses/chromiator"
    notices.mkdir(parents=True)
    for name in ["LICENSE.md", "THIRD_PARTY_NOTICES.md"]:
        shutil.copy2(ROOT / name, notices)
    shutil.copytree(ROOT / "licenses", notices / "licenses")


def copy_spectrum_example(source_snapshot, repository=ROOT):
    """Include only the built-in example, never loose user artwork."""
    example = Path(source_snapshot) / "assets/examples"
    example.mkdir(parents=True, exist_ok=True)
    shutil.copy2(Path(repository) / "assets/examples/SpectrumBreakpoint.png", example)


def bundle_adwaita_icons(appdir, source=Path("/usr/share/icons/Adwaita"),
                         copyright_file=Path("/usr/share/doc/adwaita-icon-theme/copyright")):
    """Bundle GTK's named symbolic icons and their package copyright notice."""
    source = Path(source)
    if not source.is_dir():
        raise SystemExit(f"Builder lacks Adwaita icon theme {source}")
    destination = Path(appdir) / "usr/share/icons/Adwaita"
    shutil.copytree(source, destination, symlinks=True, dirs_exist_ok=True)
    copyright_file = Path(copyright_file)
    if not copyright_file.is_file():
        raise SystemExit(f"Builder lacks Adwaita icon theme copyright file {copyright_file}")
    notices = Path(appdir) / "usr/share/licenses/chromiator"
    notices.mkdir(parents=True, exist_ok=True)
    shutil.copy2(copyright_file, notices / "adwaita-icon-theme.copyright")


def appimage_snapshot(source):
    """Stage only compilation inputs, never the checkout's personal assets or build output."""
    source.mkdir()
    for name in ["Cargo.toml", "Cargo.lock", "build.rs", "LICENSE.md", "THIRD_PARTY_NOTICES.md"]:
        shutil.copy2(ROOT / name, source / name)
    for name in ["src", "resources", "licenses"]:
        shutil.copytree(ROOT / name, source / name)
    for name in [f"{APP_ID}.desktop", f"{APP_ID}.svg", f"{APP_ID}.metainfo.xml"]:
        (source / "packaging").mkdir(exist_ok=True)
        shutil.copy2(ROOT / "packaging" / name, source / "packaging" / name)
    (source / "packaging/appimage").mkdir()
    shutil.copy2(APPIMAGE_LOCK, source / "packaging/appimage/build-lock.json")
    copy_spectrum_example(source)
    scripts = source / "scripts"
    scripts.mkdir()
    for name in ["build_distributions.py", "audit_appimage.py", "generate_license_notices.py"]:
        shutil.copy2(ROOT / "scripts" / name, scripts / name)
    vendor = source / "vendor"
    config = subprocess.check_output(["cargo", "vendor", "--locked", str(vendor)],
                                     cwd=source, text=True)
    (source / ".cargo").mkdir()
    (source / ".cargo/config.toml").write_text(config.replace(str(vendor), "vendor"))


def portable_appimage(work, version, arch, args, lock):
    if arch != "x86_64":
        raise SystemExit("Portable AppImage build path currently supports only x86_64; runtime validation is pending")
    deploy = check_pinned_tool(args.linuxdeploy, lock["linuxdeploy_sha256"])
    plugin = check_pinned_tool(args.gtk_plugin, lock["gtk_plugin_sha256"])
    runtime = check_pinned_runtime(args.appimage_runtime, lock)
    tool("podman")
    source = work / "source"
    appimage_snapshot(source)
    tools_dir = work / "tools"
    tools_dir.mkdir()
    shutil.copy2(deploy, tools_dir / "linuxdeploy")
    shutil.copy2(plugin, tools_dir / "linuxdeploy-plugin-gtk.sh")
    shutil.copy2(runtime, tools_dir / "runtime-x86_64")
    output = work / "output"
    output.mkdir()
    image = f"localhost/chromiator-appimage-builder:{lock['debian_snapshot'].lower()}"
    # Build context contains only the pinned builder definition and lock, never the checkout.
    run("podman", "build", "--pull=missing", "--file", ROOT / "packaging/appimage/Containerfile",
        "--tag", image, ROOT / "packaging/appimage")
    run("podman", "run", "--rm", "--network=none", "--userns=keep-id",
        "--security-opt=label=disable", "--env", "CHROMIATOR_APPIMAGE_BUILDER=1",
        "--env", "CARGO_HOME=/out/cargo-home",
        "--volume", f"{source.resolve()}:/src:ro",
        "--volume", f"{tools_dir.resolve()}:/tools:ro", "--volume", f"{output.resolve()}:/out:rw",
        image, "python3", "/src/scripts/build_distributions.py", "appimage",
        "--inside-builder", "--linuxdeploy", "/tools/linuxdeploy",
        "--gtk-plugin", "/tools/linuxdeploy-plugin-gtk.sh",
        "--appimage-runtime", "/tools/runtime-x86_64")
    return output / f"Chromiator-{version}-{arch}.AppImage"


def missing_appdir_libraries(appdir, allowed):
    bundled = {path.name for path in (appdir / "usr/lib").iterdir() if path.is_file()}
    missing = set()
    for path in audit_appimage.elf_files(appdir):
        names, _ = audit_appimage.dynamic_links(audit_appimage.readelf(path, "--dynamic"))
        missing.update(name for name in names if name not in bundled and name not in allowed)
    return missing


def complete_appdir_libraries(appdir, deploy, env, lock):
    """Request Debian providers for explicit font/shaping ABI and all non-host transitive links."""
    libdir = Path("/usr/lib/x86_64-linux-gnu")
    for name in ("libfontconfig.so.1", "libharfbuzz.so.0"):
        provider = libdir / name
        if not provider.is_file():
            raise SystemExit(f"Builder lacks required library {provider}")
        run(deploy, "--appdir", appdir, "--library", provider, env=env)
    allowed = set(lock["allowed_host_sonames"])
    for _ in range(16):
        missing = missing_appdir_libraries(appdir, allowed)
        if not missing:
            return
        for name in sorted(missing):
            provider = audit_appimage.host_provider(Path("/"), name)
            if provider is None:
                raise SystemExit(f"Builder cannot resolve required library {name}")
            run(deploy, "--appdir", appdir, "--library", provider, env=env)
        if missing_appdir_libraries(appdir, allowed) == missing:
            raise SystemExit(f"linuxdeploy did not close dependencies: {sorted(missing)}")
    raise SystemExit("AppDir dependency closure did not converge")


def drop_generated_graphics_loaders(appdir):
    """Leave GPU API loaders and actual drivers to the declared host desktop stack."""
    libdir = appdir / "usr/lib"
    driver_sonames = set(GRAPHICS_DRIVER_SONAMES)
    generated = set()
    for path in audit_appimage.elf_files(libdir):
        _, soname = audit_appimage.dynamic_links(audit_appimage.readelf(path, "--dynamic"))
        if soname in driver_sonames:
            generated.add(path.resolve())
    for path in libdir.iterdir():
        if path.is_symlink() and path.resolve() in generated:
            path.unlink()
    for path in generated:
        path.unlink()


def sanitize_loader_cache(appdir):
    cache = appdir / "usr/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache"
    if not cache.is_file():
        raise SystemExit("GTK plugin did not generate GdkPixbuf loaders.cache")
    lines = cache.read_text().splitlines()
    lines = ["# LoaderDir = bundled relative module names" if line.startswith("# LoaderDir =")
             else line for line in lines]
    cache.write_text("\n".join(lines) + "\n")


def appimage_inside_builder(work, version, arch, args, lock):
    if os.environ.get("CHROMIATOR_APPIMAGE_BUILDER") != "1":
        raise SystemExit("AppImage assembly must run through the pinned Debian builder")
    if arch != "x86_64" or Path("/etc/os-release").read_text().find("ID=debian") < 0:
        raise SystemExit("AppImage assembly requires Debian x86_64")
    if subprocess.check_output(["getconf", "GNU_LIBC_VERSION"], text=True).strip() != "glibc 2.41":
        raise SystemExit("AppImage assembly requires the Debian 13 glibc 2.41 floor")
    deploy = check_pinned_tool(args.linuxdeploy, lock["linuxdeploy_sha256"])
    plugin = check_pinned_tool(args.gtk_plugin, lock["gtk_plugin_sha256"])
    runtime = check_pinned_runtime(args.appimage_runtime, lock)
    shutil.copy2("/etc/chromiator-builder-packages.txt", work / "builder-packages.txt")
    env = dict(os.environ, DEPLOY_GTK_VERSION="4", ARCH=arch,
               APPIMAGE_EXTRACT_AND_RUN="1", CHROMIATOR_APPIMAGE_BUILDER="1",
               LDAI_RUNTIME_FILE=str(runtime),
               CARGO_HOME=str(work / "cargo-home"),
               PATH=f"/tools:{os.environ['PATH']}")
    run("cargo", "build", "--release", "--locked", "--offline",
        "--target-dir", work / "cargo", env=env)
    appdir = work / "AppDir"
    install_metadata(appdir / "usr")
    run(deploy, "--appdir", appdir, "--executable", work / "cargo/release/chromiator",
        "--desktop-file", ROOT / "packaging" / f"{APP_ID}.desktop",
        "--icon-file", ROOT / "packaging" / f"{APP_ID}.svg", "--plugin", "gtk",
        cwd=work, env=env)
    bundle_adwaita_icons(appdir)
    # Upstream's GTK hook forces X11. Keep explicit user overrides and GTK4 Wayland.
    hook = appdir / "apprun-hooks/linuxdeploy-plugin-gtk.sh"
    lines = hook.read_text().splitlines()
    lines = ['export GDK_BACKEND="${GDK_BACKEND:-wayland,x11}"'
             if line.startswith("export GDK_BACKEND=") else line for line in lines]
    lines.extend([
        'export GDK_PIXBUF_MODULEDIR="$APPDIR/usr/lib/gdk-pixbuf-2.0/2.10.0/loaders"',
        'export LD_LIBRARY_PATH="$APPDIR/usr/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"',
    ])
    hook.write_text("\n".join(lines) + "\n")
    sanitize_loader_cache(appdir)
    drop_generated_graphics_loaders(appdir)
    complete_appdir_libraries(appdir, deploy, env, lock)
    # A closure pass can bring back a host graphics loader through a transitive
    # dependency. Remove it again before auditing the final payload.
    drop_generated_graphics_loaders(appdir)
    run("python3", ROOT / "scripts/audit_appimage.py", appdir, "--resolve-relocations",
        "--output", work / "abi-audit.json", env=env)
    artifact = work / f"Chromiator-{version}-{arch}.AppImage"
    env["OUTPUT"] = str(artifact)
    run(deploy, "--appdir", appdir,
        *(f"--exclude-library={soname}" for soname in GRAPHICS_DRIVER_SONAMES),
        "--output", "appimage", cwd=work, env=env)
    run("python3", ROOT / "scripts/audit_appimage.py", appdir, "--resolve-relocations", "--outer-elf", artifact,
        "--output", work / "abi-audit.json", env=env)
    extraction = work / "packaged-extraction"
    extraction.mkdir()
    extraction_env = dict(env)
    extraction_env.pop("APPIMAGE_EXTRACT_AND_RUN", None)
    with (work / "extraction.log").open("w") as log:
        subprocess.run([str(artifact), "--appimage-extract"], cwd=extraction, env=extraction_env,
                       stdout=log, stderr=subprocess.STDOUT, check=True)
    packaged = extraction / "squashfs-root"
    run("python3", ROOT / "scripts/audit_appimage.py", packaged, "--resolve-relocations",
        "--outer-elf", artifact, "--output", work / "packaged-abi-audit.json", env=env)
    return artifact


def flatpak(work, version, arch, args):
    source = work / "source"
    source.mkdir()
    # Explicit build inputs: never include personal projects, loose artwork, or the archive.
    for name in ["Cargo.toml", "Cargo.lock", "build.rs", "LICENSE.md", "THIRD_PARTY_NOTICES.md"]:
        shutil.copy2(ROOT / name, source)
    for name in ["src", "resources", "licenses", "packaging"]:
        shutil.copytree(ROOT / name, source / name)
    copy_spectrum_example(source)
    vendor = source / "vendor"
    config = subprocess.check_output(
        ["cargo", "vendor", "--locked", str(vendor)], cwd=source, text=True)
    (source / ".cargo").mkdir()
    # cargo vendor prints a host path; sandbox builds need the relative source path.
    config = config.replace(str(vendor), "vendor")
    (source / ".cargo/config.toml").write_text(config)
    commands = [
        "cargo build --release --locked --offline",
        "install -Dm755 target/release/chromiator /app/bin/chromiator",
        f"install -Dm644 packaging/{APP_ID}.desktop /app/share/applications/{APP_ID}.desktop",
        f"install -Dm644 packaging/{APP_ID}.svg /app/share/icons/hicolor/scalable/apps/{APP_ID}.svg",
        f"install -Dm644 packaging/{APP_ID}.metainfo.xml /app/share/metainfo/{APP_ID}.metainfo.xml",
        "mkdir -p /app/share/licenses/chromiator",
        "cp -r LICENSE.md THIRD_PARTY_NOTICES.md licenses /app/share/licenses/chromiator/",
    ]
    manifest = {
        "app-id": APP_ID, "runtime": "org.gnome.Platform", "runtime-version": args.runtime,
        "sdk": "org.gnome.Sdk", "sdk-extensions": ["org.freedesktop.Sdk.Extension.rust-stable"],
        "command": "chromiator",
        "finish-args": ["--share=ipc", "--socket=wayland", "--socket=fallback-x11", "--device=dri"],
        "build-options": {"append-path": "/usr/lib/sdk/rust-stable/bin", "env": {"CARGO_HOME": "/run/build/chromiator/cargo-home"}},
        "modules": [{"name": "chromiator", "buildsystem": "simple", "build-commands": commands,
                     "sources": [{"type": "dir", "path": str(source)}]}],
    }
    manifest_path = work / f"{APP_ID}.json"
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n")
    repo = work / "repo"
    run("flatpak-builder", "--user", f"--arch={arch}", f"--repo={repo}",
        f"--state-dir={work / 'builder-state'}", work / "flatpak-build", manifest_path)
    artifact = work / f"Chromiator-{version}-{arch}.flatpak"
    run("flatpak", "build-bundle", f"--arch={arch}",
        "--runtime-repo=https://flathub.org/repo/flathub.flatpakrepo",
        repo, artifact, APP_ID)
    return artifact


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("format", choices=["appimage", "flatpak", "all"])
    parser.add_argument("--runtime", default="50", help="GNOME runtime/SDK branch (default: 50)")
    parser.add_argument("--linuxdeploy", default="linuxdeploy", help="Executable path or command")
    parser.add_argument("--gtk-plugin", default=str(ROOT / "target/distribution-tools/linuxdeploy-plugin-gtk.sh"))
    parser.add_argument("--appimage-runtime", default=str(ROOT / "target/distribution-tools/runtime-x86_64-20251108"),
                        help="Pinned official type-2 runtime for offline AppImage output")
    parser.add_argument("--inside-builder", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--check", action="store_true", help="Check tool availability without building")
    args = parser.parse_args()
    arch = platform.machine()
    if arch not in ("x86_64", "aarch64"):
        parser.error(f"Unsupported native architecture: {arch}")
    tool("cargo")
    lock = json.loads(APPIMAGE_LOCK.read_text())
    if args.format in ("flatpak", "all"):
        tool("flatpak-builder")
        tool("flatpak")
    if args.format in ("appimage", "all"):
        if not args.inside_builder and args.linuxdeploy == "linuxdeploy":
            args.linuxdeploy = str(ROOT / "target/distribution-tools/linuxdeploy")
        check_pinned_tool(args.linuxdeploy, lock["linuxdeploy_sha256"])
        check_pinned_tool(args.gtk_plugin, lock["gtk_plugin_sha256"])
        check_pinned_runtime(args.appimage_runtime, lock)
        if not args.inside_builder:
            tool("podman")
    if args.check:
        print("Required executables found. SDKs, plugins and bundle execution are not verified.")
        return
    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["package"]["version"]
    if not args.inside_builder:
        run("python3", "scripts/generate_license_notices.py", "--check")
    if args.inside_builder:
        work = Path("/out")
        artifact = appimage_inside_builder(work, version, arch, args, lock)
        (work / "SHA256SUMS").write_text(f"{digest(artifact)}  {artifact.name}\n")
        return
    parent = ROOT / "target/distribution"
    parent.mkdir(parents=True, exist_ok=True)
    work = Path(tempfile.mkdtemp(prefix=f"{version}-", dir=parent))
    artifacts = []
    if args.format in ("appimage", "all"):
        destination = work / "appimage"
        destination.mkdir()
        artifacts.append(portable_appimage(destination, version, arch, args, lock))
    if args.format in ("flatpak", "all"):
        destination = work / "flatpak"
        destination.mkdir()
        artifacts.append(flatpak(destination, version, arch, args))
    sums = []
    for artifact in artifacts:
        sums.append(f"{digest(artifact)}  {artifact.relative_to(work)}\n")
    (work / "SHA256SUMS").write_text("".join(sums))
    print(f"Bundles and SHA256SUMS: {work}\nNot installed or published. Test before distributing.")


if __name__ == "__main__":
    main()
