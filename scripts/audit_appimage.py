#!/usr/bin/env python3
"""Static, fail-closed AppDir ELF and GTK resource audit for the Debian 13 floor."""

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys


GLIBC_RE = re.compile(r"Name: GLIBC_(\d+)\.(\d+)")
NEEDED_RE = re.compile(r"\(NEEDED\).*\[([^]]+)\]")
SONAME_RE = re.compile(r"\(SONAME\).*\[([^]]+)\]")
SYMBOL_RE = re.compile(r"\b(FcConfigSetDefaultSubstitute|hb_free)(?:@\S*)?\s*$")
PROVIDERS = {"FcConfigSetDefaultSubstitute": "libfontconfig.so.1", "hb_free": "libharfbuzz.so.0"}
FORBIDDEN_BUNDLES = {
    "libc.so.6", "libm.so.6", "libdl.so.2", "libpthread.so.0", "librt.so.1",
    "libresolv.so.2", "libutil.so.1", "ld-linux-x86-64.so.2", "libGL.so.1",
    "libEGL.so.1", "libvulkan.so.1", "libgbm.so.1", "libdrm.so.2",
}
FORBIDDEN_NAME_RE = re.compile(r"^(?:lib(?:nvidia|amdgpu|GLX_mesa|EGL_mesa|vulkan_(?:radeon|intel))|.*_dri\.so)")
REQUIRED_ADWAITA_ICONS = (
    "symbolic/actions/document-open-symbolic.svg",
    "symbolic/actions/document-save-as-symbolic.svg",
    "symbolic/actions/document-save-symbolic.svg",
    "symbolic/actions/edit-redo-symbolic.svg",
    "symbolic/actions/edit-undo-symbolic.svg",
    "symbolic/actions/open-menu-symbolic.svg",
    "symbolic/actions/sidebar-show-symbolic.svg",
    "symbolic/actions/view-refresh-symbolic.svg",
    "symbolic/places/user-trash-symbolic.svg",
    "symbolic/status/changes-allow-symbolic.svg",
    "symbolic/status/changes-prevent-symbolic.svg",
    "symbolic/status/folder-open-symbolic.svg",
)


def readelf(path: Path, *options: str) -> str:
    result = subprocess.run(["readelf", "--wide", *options, str(path)],
                            text=True, capture_output=True, check=False)
    if result.returncode:
        raise RuntimeError(f"readelf failed for {path}: {result.stderr.strip()}")
    return result.stdout


def required_glibc(version_info: str) -> set[tuple[int, int]]:
    """Read requirements only; definitions are not a minimum-host requirement."""
    needs = version_info.split("Version needs section", 1)
    if len(needs) < 2:
        return set()
    return {(int(major), int(minor)) for major, minor in GLIBC_RE.findall(needs[1])}


def dynamic_links(dynamic: str) -> tuple[set[str], str | None]:
    names = set(NEEDED_RE.findall(dynamic))
    soname = SONAME_RE.search(dynamic)
    return names, soname.group(1) if soname else None


def named_symbols(table: str) -> tuple[set[str], set[str]]:
    imported: set[str] = set()
    exported: set[str] = set()
    for line in table.splitlines():
        match = SYMBOL_RE.search(line)
        if match:
            (imported if re.search(r"\bUND\b", line) else exported).add(match.group(1))
    return imported, exported


def is_elf(path: Path) -> bool:
    try:
        with path.open("rb") as source:
            return source.read(4) == b"\x7fELF"
    except OSError:
        return False


def elf_files(root: Path) -> list[Path]:
    found = []
    seen = set()
    for path in root.rglob("*"):
        if not path.is_file():
            continue
        resolved = path.resolve()
        if resolved in seen or not resolved.is_relative_to(root.resolve()):
            continue
        seen.add(resolved)
        if is_elf(path):
            found.append(path)
    return sorted(found)


def host_provider(root: Path, soname: str) -> Path | None:
    for prefix in ("usr/lib/x86_64-linux-gnu", "lib/x86_64-linux-gnu", "usr/lib64", "lib64"):
        candidate = root / prefix / soname
        if candidate.is_file():
            return candidate
    return None


def audit(root: Path, baseline: tuple[int, int], host_root: Path,
          allowed_host: set[str], resolve_relocations: bool = False) -> dict:
    root = root.resolve()
    errors: list[str] = []
    records: list[dict] = []
    executable = root / "usr/bin/chromiator"
    if not executable.is_file() or not executable.resolve().is_relative_to(root) or not is_elf(executable):
        errors.append("missing or invalid in-root ELF usr/bin/chromiator")
    for path in root.rglob("*"):
        if path.is_symlink() and (
            not path.resolve().exists() or not path.resolve().is_relative_to(root)
        ):
            errors.append(f"broken or escaping AppDir symlink: {path.relative_to(root)}")
    paths = elf_files(root)
    if not paths:
        errors.append("no ELF files in AppDir")
    bundled: dict[str, Path] = {}
    symbols: dict[Path, tuple[set[str], set[str]]] = {}
    for path in paths:
        rel = path.relative_to(root).as_posix()
        required = required_glibc(readelf(path, "--version-info"))
        needed, soname = dynamic_links(readelf(path, "--dynamic"))
        symbols[path] = named_symbols(readelf(path, "--dyn-syms"))
        if soname:
            prior = bundled.setdefault(soname, path)
            if prior.resolve() != path.resolve() and (
                hashlib.sha256(prior.read_bytes()).digest() != hashlib.sha256(path.read_bytes()).digest()
            ):
                errors.append(f"duplicate bundled SONAME {soname}: {prior} and {path}")
        if (path.name in FORBIDDEN_BUNDLES or soname in FORBIDDEN_BUNDLES
                or FORBIDDEN_NAME_RE.match(path.name)):
            errors.append(f"prohibited bundled system/driver ELF: {rel}")
        high = sorted(v for v in required if v > baseline)
        if high:
            errors.append(f"{rel} requires GLIBC_{high[-1][0]}.{high[-1][1]} > {baseline[0]}.{baseline[1]}")
        records.append({"path": rel, "soname": soname, "needed": sorted(needed),
                        "max_glibc": ".".join(map(str, max(required))) if required else None})

    loadable = {}
    for name in bundled:
        alias = root / "usr/lib" / name
        if alias.is_file() and alias.resolve().is_relative_to(root) and is_elf(alias):
            _, alias_soname = dynamic_links(readelf(alias, "--dynamic"))
            if alias_soname == name:
                loadable[name] = alias
            else:
                errors.append(f"bundled alias {name} has mismatched SONAME {alias_soname}")

    for record in records:
        for name in record["needed"]:
            if name in loadable:
                continue
            if name not in allowed_host:
                errors.append(f"{record['path']} has unresolved DT_NEEDED {name}")
                continue
            provider = host_provider(host_root, name)
            if provider is None:
                errors.append(f"allowlisted host provider missing: {name}")
            else:
                required = required_glibc(readelf(provider, "--version-info"))
                if any(v > baseline for v in required):
                    errors.append(f"allowlisted host provider {name} exceeds GLIBC_{baseline[0]}.{baseline[1]}")

    for symbol, soname in PROVIDERS.items():
        provider = loadable.get(soname)
        if provider is None:
            errors.append(f"required bundled provider missing: {soname} for {symbol}")
        elif any(symbol in imports for imports, _ in symbols.values()):
            if symbol not in named_symbols(readelf(provider, "--dyn-syms"))[1]:
                errors.append(f"{soname} does not export imported {symbol}")

    if resolve_relocations:
        # In the pinned builder only: use the Debian loader to catch symbol/version
        # mismatches that DT_NEEDED and the explicit provider checks cannot express.
        for path in paths:
            result = subprocess.run(["ldd", "-r", str(path)], capture_output=True, text=True,
                                    env={"PATH": "/usr/bin:/bin", "LANG": "C"}, check=False)
            output = result.stdout + result.stderr
            if result.returncode or "not found" in output or "undefined symbol:" in output:
                errors.append(f"Debian relocation resolution failed for {path.relative_to(root)}: {output.strip()[-500:]}")

    hook = root / "apprun-hooks/linuxdeploy-plugin-gtk.sh"
    cache = root / "usr/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache"
    if not hook.is_file():
        errors.append("missing GTK AppRun hook")
    else:
        content = hook.read_text(errors="replace")
        if "GDK_BACKEND=\"${GDK_BACKEND:-wayland,x11}\"" not in content:
            errors.append("GTK hook does not preserve Wayland/X11 fallback")
        if 'GDK_PIXBUF_MODULE_FILE="$APPDIR//usr/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache"' not in content:
            errors.append("GTK hook does not select the relocated GdkPixbuf loader cache")
        if 'GDK_PIXBUF_MODULEDIR="$APPDIR/usr/lib/gdk-pixbuf-2.0/2.10.0/loaders"' not in content:
            errors.append("GTK hook does not select the bundled GdkPixbuf loader directory")
        if 'LD_LIBRARY_PATH="$APPDIR/usr/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"' not in content:
            errors.append("GTK hook does not add bundled libraries to the runtime search path")
        if re.search(r"/home/|/out/|/src/|/usr/lib64/|/usr/lib/x86_64-linux-gnu/|/tmp/\.mount", content):
            errors.append("GTK hook contains a build-host path")
    adwaita = root / "usr/share/icons/Adwaita"
    for icon in REQUIRED_ADWAITA_ICONS:
        if not (adwaita / icon).is_file():
            errors.append(f"missing bundled Adwaita icon: {icon}")
    if not cache.is_file():
        errors.append("missing GdkPixbuf loaders.cache")
    else:
        content = cache.read_text(errors="replace")
        if re.search(r"/home/|/out/|/src/|/usr/lib64/|/usr/lib/x86_64-linux-gnu/|/tmp/\.mount", content):
            errors.append("GdkPixbuf loaders.cache contains a build-host path")
        loader_dir = root / "usr/lib/gdk-pixbuf-2.0/2.10.0/loaders"
        modules = re.findall(r'^"([^"\n]+\.so)"$', content, re.MULTILINE)
        for module in modules:
            if Path(module).name != module or not (loader_dir / module).is_file():
                errors.append(f"GdkPixbuf loader is not bundled by relative name: {module}")
    return {"appdir": str(root), "baseline_glibc": ".".join(map(str, baseline)),
            "elf_count": len(paths), "records": records, "errors": sorted(set(errors)),
            "passed": not errors}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("appdir", type=Path)
    parser.add_argument("--baseline", default="2.41")
    parser.add_argument("--host-root", type=Path, default=Path("/"))
    parser.add_argument("--lock", type=Path, default=Path(__file__).resolve().parents[1] /
                        "packaging/appimage/build-lock.json")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--outer-elf", type=Path, help="Audit the produced AppImage runtime too")
    parser.add_argument("--resolve-relocations", action="store_true",
                        help="Use ldd -r in the pinned Debian builder; never launch the app")
    args = parser.parse_args()
    lock = json.loads(args.lock.read_text())
    baseline = tuple(map(int, args.baseline.split(".")))
    report = audit(args.appdir, baseline, args.host_root, set(lock["allowed_host_sonames"]),
                   args.resolve_relocations)
    if args.outer_elf:
        if not is_elf(args.outer_elf):
            report["errors"].append("produced AppImage is not ELF")
        else:
            required = required_glibc(readelf(args.outer_elf, "--version-info"))
            high = sorted(v for v in required if v > baseline)
            if high:
                report["errors"].append(
                    f"AppImage runtime requires GLIBC_{high[-1][0]}.{high[-1][1]} > {args.baseline}"
                )
        report["passed"] = not report["errors"]
    encoded = json.dumps(report, indent=2) + "\n"
    if args.output:
        args.output.write_text(encoded)
    print(f"AppDir audit: {report['elf_count']} ELF files, {len(report['errors'])} errors")
    for error in report["errors"]:
        print(f"  {error}")
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
