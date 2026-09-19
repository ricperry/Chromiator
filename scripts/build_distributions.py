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

ROOT = Path(__file__).resolve().parents[1]
APP_ID = "io.github.chromiator.Chromiator"


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


def appimage(work, version, arch, args):
    deploy = tool(args.linuxdeploy)
    tool("linuxdeploy-plugin-gtk.sh")
    run("cargo", "build", "--release", "--locked", "--target-dir", work / "cargo")
    appdir = work / "AppDir"
    install_metadata(appdir / "usr")
    env = dict(os.environ, DEPLOY_GTK_VERSION="4", ARCH=arch,
               APPIMAGE_EXTRACT_AND_RUN="1")
    run(deploy, "--appdir", appdir, "--executable", work / "cargo/release/chromiator",
        "--desktop-file", ROOT / "packaging" / f"{APP_ID}.desktop",
        "--icon-file", ROOT / "packaging" / f"{APP_ID}.svg", "--plugin", "gtk",
        cwd=work, env=env)
    # Upstream's GTK hook forces X11. Keep explicit user overrides and GTK4 Wayland.
    hook = appdir / "apprun-hooks/linuxdeploy-plugin-gtk.sh"
    lines = hook.read_text().splitlines()
    lines = ['export GDK_BACKEND="${GDK_BACKEND:-wayland,x11}"'
             if line.startswith("export GDK_BACKEND=") else line for line in lines]
    hook.write_text("\n".join(lines) + "\n")
    artifact = work / f"Chromiator-{version}-{arch}.AppImage"
    env["OUTPUT"] = str(artifact)
    run(deploy, "--appdir", appdir, "--output", "appimage", cwd=work, env=env)
    return artifact


def flatpak(work, version, arch, args):
    source = work / "source"
    source.mkdir()
    # Explicit build inputs: never include personal projects, caches, or the archive.
    for name in ["Cargo.toml", "Cargo.lock", "build.rs", "LICENSE.md", "THIRD_PARTY_NOTICES.md"]:
        shutil.copy2(ROOT / name, source)
    for name in ["src", "resources", "assets", "licenses", "packaging"]:
        if name == "assets":
            # Includes embedded artwork, excludes user project/preset documents.
            shutil.copytree(ROOT / name, source / name,
                            ignore=shutil.ignore_patterns("*.chromiator", "*.json"))
        else:
            shutil.copytree(ROOT / name, source / name)
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
    parser.add_argument("--check", action="store_true", help="Check tool availability without building")
    args = parser.parse_args()
    arch = platform.machine()
    if arch not in ("x86_64", "aarch64"):
        parser.error(f"Unsupported native architecture: {arch}")
    tool("cargo")
    if args.format in ("flatpak", "all"):
        tool("flatpak-builder")
        tool("flatpak")
    if args.format in ("appimage", "all"):
        tool(args.linuxdeploy)
        tool("linuxdeploy-plugin-gtk.sh")
    if args.check:
        print("Required executables found. SDKs, plugins and bundle execution are not verified.")
        return
    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["package"]["version"]
    run("python3", "scripts/generate_license_notices.py", "--check")
    parent = ROOT / "target/distribution"
    parent.mkdir(parents=True, exist_ok=True)
    work = Path(tempfile.mkdtemp(prefix=f"{version}-", dir=parent))
    artifacts = []
    for name, build in [("appimage", appimage), ("flatpak", flatpak)]:
        if args.format in (name, "all"):
            destination = work / name
            destination.mkdir()
            artifacts.append(build(destination, version, arch, args))
    sums = []
    for artifact in artifacts:
        digest = hashlib.file_digest(artifact.open("rb"), "sha256").hexdigest()
        sums.append(f"{digest}  {artifact.relative_to(work)}\n")
    (work / "SHA256SUMS").write_text("".join(sums))
    print(f"Bundles and SHA256SUMS: {work}\nNot installed or published. Test before distributing.")


if __name__ == "__main__":
    main()
