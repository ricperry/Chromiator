"""Headless packaging gates; never launch the application or pull a container."""

import importlib.util
import hashlib
import json
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parents[1]


def load_script(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


audit = load_script("audit_appimage")
sys.path.insert(0, str(ROOT / "scripts"))
build = load_script("build_distributions")


class RuntimeInputTests(unittest.TestCase):
    def test_bundle_adwaita_theme_and_copyright(self):
        with tempfile.TemporaryDirectory() as location:
            root = Path(location)
            theme = root / "source/Adwaita"
            action = theme / "symbolic/actions"
            action.mkdir(parents=True)
            icon = action / "edit-undo-symbolic.svg"
            icon.write_text("<svg id='undo'/>")
            (action / "undo-alias.svg").symlink_to(icon.name)
            copyright_file = root / "copyright"
            copyright_file.write_text("Adwaita icon theme license")
            appdir = root / "AppDir"

            build.bundle_adwaita_icons(appdir, theme, copyright_file)

            copied = appdir / "usr/share/icons/Adwaita/symbolic/actions"
            self.assertEqual((copied / icon.name).read_text(), "<svg id='undo'/>")
            self.assertTrue((copied / "undo-alias.svg").is_symlink())
            self.assertEqual(
                (appdir / "usr/share/licenses/chromiator/adwaita-icon-theme.copyright").read_text(),
                "Adwaita icon theme license",
            )

    def test_runtime_missing_hash_and_architecture_fail_closed(self):
        with tempfile.TemporaryDirectory() as location:
            runtime = Path(location) / "runtime-x86_64"
            valid_header = bytearray(20)
            valid_header[:6] = b"\x7fELF\x02\x01"
            valid_header[18:20] = (62).to_bytes(2, "little")
            lock = {"runtime_sha256": hashlib.sha256(valid_header).hexdigest(),
                    "runtime_size": 20, "baseline_glibc": "2.41"}
            with self.assertRaisesRegex(SystemExit, "Pinned tool unavailable or hash mismatch"):
                build.check_pinned_runtime(runtime, lock)
            runtime.write_bytes(b"wrong")
            with self.assertRaisesRegex(SystemExit, "Pinned tool unavailable or hash mismatch"):
                build.check_pinned_runtime(runtime, lock)
            wrong_arch = bytearray(valid_header)
            wrong_arch[18:20] = (183).to_bytes(2, "little")
            runtime.write_bytes(wrong_arch)
            lock["runtime_sha256"] = hashlib.sha256(wrong_arch).hexdigest()
            with self.assertRaisesRegex(SystemExit, "not x86_64 ELF"):
                build.check_pinned_runtime(runtime, lock)
            runtime.write_bytes(valid_header)
            lock["runtime_sha256"] = hashlib.sha256(valid_header).hexdigest()
            lock["runtime_size"] = 21
            with self.assertRaisesRegex(SystemExit, "wrong size"):
                build.check_pinned_runtime(runtime, lock)
            lock["runtime_size"] = 20
            with mock.patch.object(build.audit_appimage, "readelf",
                                   return_value="Version needs section: Name: GLIBC_2.43"):
                with self.assertRaisesRegex(SystemExit, "exceeds GLIBC 2.41"):
                    build.check_pinned_runtime(runtime, lock)
            with mock.patch.object(build.audit_appimage, "readelf", return_value=""):
                self.assertEqual(build.check_pinned_runtime(runtime, lock), runtime)

    def test_runtime_is_checked_before_snapshot_or_container_build(self):
        with tempfile.TemporaryDirectory() as location:
            work = Path(location)
            args = SimpleNamespace(linuxdeploy=work / "linuxdeploy",
                                   gtk_plugin=work / "plugin", appimage_runtime=work / "missing-runtime")
            lock = {"linuxdeploy_sha256": "a", "gtk_plugin_sha256": "b"}
            with (mock.patch.object(build, "check_pinned_tool", side_effect=lambda path, _: Path(path)),
                  mock.patch.object(build, "check_pinned_runtime", side_effect=SystemExit("runtime missing")),
                  mock.patch.object(build, "appimage_snapshot") as snapshot,
                  mock.patch.object(build, "run") as run):
                with self.assertRaisesRegex(SystemExit, "runtime missing"):
                    build.portable_appimage(work, "0.2.0", "x86_64", args, lock)
            snapshot.assert_not_called()
            run.assert_not_called()

    def test_builder_passes_verified_runtime_to_output_plugin(self):
        with tempfile.TemporaryDirectory() as location:
            work = Path(location)
            hook = work / "AppDir/apprun-hooks/linuxdeploy-plugin-gtk.sh"
            hook.parent.mkdir(parents=True)
            hook.write_text('export GDK_BACKEND="x11"\n')
            args = SimpleNamespace(linuxdeploy=work / "linuxdeploy",
                                   gtk_plugin=work / "plugin", appimage_runtime=work / "runtime")
            lock = {"linuxdeploy_sha256": "a", "gtk_plugin_sha256": "b"}
            output_calls = []

            def fake_run(*command, **kwargs):
                if command[-2:] == ("--output", "appimage"):
                    output_calls.append((command, kwargs))
                    raise RuntimeError("stop at output plugin")

            with (mock.patch.dict(build.os.environ, {"CHROMIATOR_APPIMAGE_BUILDER": "1"}),
                  mock.patch.object(build.Path, "read_text", return_value="ID=debian"),
                  mock.patch.object(build.subprocess, "check_output", return_value="glibc 2.41"),
                  mock.patch.object(build, "check_pinned_tool", side_effect=lambda path, _: Path(path)),
                  mock.patch.object(build, "check_pinned_runtime", return_value=work / "runtime"),
                  mock.patch.object(build.shutil, "copy2"),
                  mock.patch.object(build, "install_metadata"),
                  mock.patch.object(build, "bundle_adwaita_icons"),
                  mock.patch.object(build, "sanitize_loader_cache"),
                  mock.patch.object(build, "drop_generated_graphics_loaders"),
                  mock.patch.object(build, "complete_appdir_libraries"),
                  mock.patch.object(build, "run", side_effect=fake_run)):
                with self.assertRaisesRegex(RuntimeError, "stop at output plugin"):
                    build.appimage_inside_builder(work, "0.2.0", "x86_64", args, lock)
            self.assertEqual(len(output_calls), 1)
            command, kwargs = output_calls[0]
            self.assertEqual(command[:2], (work / "linuxdeploy", "--appdir"))
            self.assertEqual(command[-2:], ("--output", "appimage"))
            self.assertEqual(command[3:-2], (
                "--exclude-library=libGL.so.1", "--exclude-library=libEGL.so.1",
                "--exclude-library=libvulkan.so.1", "--exclude-library=libgbm.so.1",
                "--exclude-library=libdrm.so.2",
            ))
            self.assertEqual(kwargs["env"]["LDAI_RUNTIME_FILE"], str(work / "runtime"))
            self.assertEqual(kwargs["env"]["OUTPUT"], str(work / "Chromiator-0.2.0-x86_64.AppImage"))
            hook_contents = hook.read_text()
            self.assertIn('export GDK_PIXBUF_MODULEDIR="$APPDIR/usr/lib/gdk-pixbuf-2.0/2.10.0/loaders"',
                          hook_contents)
            self.assertIn('export LD_LIBRARY_PATH="$APPDIR/usr/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"',
                          hook_contents)


class ElfParserTests(unittest.TestCase):
    def test_version_needs_are_distinct_from_versions_defined(self):
        text = "Version definition section: Name: GLIBC_2.99\nVersion needs section: Name: GLIBC_2.35\n  Name: GLIBC_2.41\n"
        self.assertEqual(audit.required_glibc(text), {(2, 35), (2, 41)})
        self.assertEqual(audit.required_glibc("Version definition section: Name: GLIBC_2.99"), set())

    def test_dt_needed_soname_and_imported_symbols(self):
        dynamic = "0x0001 (NEEDED) Shared library: [libfontconfig.so.1]\n0x0002 (SONAME) Library soname: [libpangoft2.so.0]"
        self.assertEqual(audit.dynamic_links(dynamic), ({"libfontconfig.so.1"}, "libpangoft2.so.0"))
        symbols = "  12: 0 0 FUNC GLOBAL DEFAULT UND FcConfigSetDefaultSubstitute\n  13: 1 5 FUNC GLOBAL DEFAULT 12 hb_free"
        self.assertEqual(audit.named_symbols(symbols), ({"FcConfigSetDefaultSubstitute"}, {"hb_free"}))


class StaticClosureTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        (self.root / "usr/bin").mkdir(parents=True)
        (self.root / "usr/lib").mkdir()
        (self.root / "apprun-hooks").mkdir()
        (self.root / "usr/lib/gdk-pixbuf-2.0/2.10.0").mkdir(parents=True)
        adwaita = self.root / "usr/share/icons/Adwaita"
        for icon in audit.REQUIRED_ADWAITA_ICONS:
            path = adwaita / icon
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("<svg/>")
        (self.root / "usr/bin/chromiator").write_bytes(b"\x7fELFfake")
        for name in ("libfontconfig.so.1", "libharfbuzz.so.0"):
            (self.root / "usr/lib" / name).write_bytes(b"\x7fELF" + name.encode())
        (self.root / "apprun-hooks/linuxdeploy-plugin-gtk.sh").write_text(
            'export GDK_BACKEND="${GDK_BACKEND:-wayland,x11}"\n'
            'export GDK_PIXBUF_MODULE_FILE="$APPDIR//usr/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache"\n'
            'export GDK_PIXBUF_MODULEDIR="$APPDIR/usr/lib/gdk-pixbuf-2.0/2.10.0/loaders"\n'
            'export LD_LIBRARY_PATH="$APPDIR/usr/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"\n')
        (self.root / "usr/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache").write_text("loader.so\n")

    def tearDown(self):
        self.temp.cleanup()

    def fake_readelf(self, path, *options):
        path = Path(path)
        if "--version-info" in options:
            return "Version needs section: Name: GLIBC_2.41\n"
        if "--dynamic" in options:
            if path.name == "chromiator":
                return "(NEEDED) Shared library: [libfontconfig.so.1]\n(NEEDED) Shared library: [libharfbuzz.so.0]"
            return f"(SONAME) Library soname: [{path.name}]"
        if path.name == "chromiator":
            return "  1: 0 0 FUNC GLOBAL DEFAULT UND FcConfigSetDefaultSubstitute\n  2: 0 0 FUNC GLOBAL DEFAULT UND hb_free"
        if path.name == "libfontconfig.so.1":
            return "  1: 1 4 FUNC GLOBAL DEFAULT 12 FcConfigSetDefaultSubstitute"
        return "  1: 1 4 FUNC GLOBAL DEFAULT 12 hb_free"

    def report(self):
        with mock.patch.object(audit, "readelf", side_effect=self.fake_readelf):
            return audit.audit(self.root, (2, 41), Path("/"), set())

    def test_valid_bundled_provider_closure(self):
        self.assertTrue(self.report()["passed"])

    def test_adwaita_icon_set_is_required(self):
        icon = self.root / "usr/share/icons/Adwaita/symbolic/actions/edit-undo-symbolic.svg"
        icon.unlink()
        self.assertIn("missing bundled Adwaita icon: symbolic/actions/edit-undo-symbolic.svg",
                      "\n".join(self.report()["errors"]))

    def test_gdk_pixbuf_runtime_paths_are_required(self):
        hook = self.root / "apprun-hooks/linuxdeploy-plugin-gtk.sh"
        hook.write_text('export GDK_BACKEND="${GDK_BACKEND:-wayland,x11}"\n')
        errors = "\n".join(self.report()["errors"])
        self.assertIn("does not select the relocated GdkPixbuf loader cache", errors)
        self.assertIn("does not select the bundled GdkPixbuf loader directory", errors)
        self.assertIn("does not add bundled libraries to the runtime search path", errors)

    def test_bad_glibc_requirement_fails(self):
        base = self.fake_readelf
        def bad(path, *options):
            if Path(path).name == "chromiator" and "--version-info" in options:
                return "Version needs section: Name: GLIBC_2.43"
            return base(path, *options)
        with mock.patch.object(audit, "readelf", side_effect=bad):
            report = audit.audit(self.root, (2, 41), Path("/"), set())
        self.assertIn("GLIBC_2.43", "\n".join(report["errors"]))

    def test_missing_provider_alias_and_symbol_fail(self):
        provider = self.root / "usr/lib/libfontconfig.so.1"
        provider.rename(self.root / "usr/lib/libfontconfig-real.so")
        report = self.report()
        self.assertIn("required bundled provider missing: libfontconfig.so.1", "\n".join(report["errors"]))
        provider = self.root / "usr/lib/libharfbuzz.so.0"
        provider.write_bytes(b"\x7fELFother")
        base = self.fake_readelf
        def missing_symbol(path, *options):
            if Path(path).name == "libharfbuzz.so.0" and "--dyn-syms" in options:
                return ""
            return base(path, *options)
        with mock.patch.object(audit, "readelf", side_effect=missing_symbol):
            report = audit.audit(self.root, (2, 41), Path("/"), set())
        self.assertIn("does not export imported hb_free", "\n".join(report["errors"]))

    def test_alias_must_load_matching_soname(self):
        (self.root / "usr/lib/libfontconfig-real.so").write_bytes(b"\x7fELFreal")
        base = self.fake_readelf
        def mismatched(path, *options):
            if "--dynamic" in options and Path(path).name == "libfontconfig.so.1":
                return "(SONAME) Library soname: [libdifferent.so.1]"
            if "--dynamic" in options and Path(path).name == "libfontconfig-real.so":
                return "(SONAME) Library soname: [libfontconfig.so.1]"
            return base(path, *options)
        with mock.patch.object(audit, "readelf", side_effect=mismatched):
            report = audit.audit(self.root, (2, 41), Path("/"), set())
        self.assertIn("bundled alias libfontconfig.so.1 has mismatched SONAME", "\n".join(report["errors"]))

    def test_symlink_escape_and_prohibited_soname_fail(self):
        (self.root / "usr/lib/escape.so").symlink_to("/usr/lib64/libc.so.6")
        base = self.fake_readelf
        def bad(path, *options):
            if Path(path).name == "libharfbuzz.so.0" and "--dynamic" in options:
                return "(SONAME) Library soname: [libc.so.6]"
            return base(path, *options)
        with mock.patch.object(audit, "readelf", side_effect=bad):
            report = audit.audit(self.root, (2, 41), Path("/"), set())
        self.assertIn("escaping AppDir symlink", "\n".join(report["errors"]))
        self.assertIn("prohibited bundled system/driver ELF", "\n".join(report["errors"]))

    def test_allowed_host_provider_must_fit_baseline(self):
        with tempfile.TemporaryDirectory() as location:
            host = Path(location)
            (host / "usr/lib/x86_64-linux-gnu").mkdir(parents=True)
            (host / "usr/lib/x86_64-linux-gnu/libm.so.6").write_bytes(b"\x7fELFhost")
            base = self.fake_readelf
            def host_needed(path, *options):
                if Path(path).name == "chromiator" and "--dynamic" in options:
                    return base(path, *options) + "\n(NEEDED) Shared library: [libm.so.6]"
                return base(path, *options)
            with mock.patch.object(audit, "readelf", side_effect=host_needed):
                good = audit.audit(self.root, (2, 41), host, {"libm.so.6"})
            self.assertTrue(good["passed"])
            def too_new(path, *options):
                if Path(path).name == "libm.so.6" and "--version-info" in options:
                    return "Version needs section: Name: GLIBC_2.43"
                return host_needed(path, *options)
            with mock.patch.object(audit, "readelf", side_effect=too_new):
                bad = audit.audit(self.root, (2, 41), host, {"libm.so.6"})
            self.assertIn("allowlisted host provider libm.so.6 exceeds", "\n".join(bad["errors"]))

    def test_cache_requires_relative_bundled_modules(self):
        cache = self.root / "usr/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache"
        cache.write_text('# LoaderDir = /src/build/loaders\n"/out/libfake.so"\n')
        errors = "\n".join(self.report()["errors"])
        self.assertIn("build-host path", errors)
        self.assertIn("not bundled by relative name", errors)


class RealArtifactTests(unittest.TestCase):
    def test_retained_release_appdir_fails_known_abi(self):
        root = ROOT / "target/distribution/0.2.0-lv_dtlen/appimage/AppDir"
        if not root.is_dir():
            self.skipTest("retained release AppDir is unavailable")
        lock = json.loads((ROOT / "packaging/appimage/build-lock.json").read_text())
        report = audit.audit(root, (2, 41), Path("/"), set(lock["allowed_host_sonames"]))
        errors = "\n".join(report["errors"])
        self.assertFalse(report["passed"])
        for library in ("libgnutls", "liborc", "libglib", "liblcms2", "libgraphene", "libpixman", "libgtk-4"):
            self.assertIn(library, errors)
        self.assertIn("required bundled provider missing: libfontconfig.so.1", errors)
        self.assertIn("required bundled provider missing: libharfbuzz.so.0", errors)


if __name__ == "__main__":
    unittest.main()
