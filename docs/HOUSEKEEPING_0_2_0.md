# Chromiator 0.2.0 housekeeping

## Scope

- Set the application package version to 0.2.0; Cargo updates its lockfile entry.
- Replace the development-oriented README with an application user guide.
- Rename LICENSE to LICENSE.md without changing its text. It matches GNU's
  official GPL version 3 text; the package remains GPL-3.0-or-later.
- Expand THIRD_PARTY_NOTICES.md and collect the locked Rust dependency notices
  plus principal native GTK-stack license texts.
- Remove the duplicate development declaration of the existing tempfile dependency.
- Remove the unused examples/generate_artifacts.rs exporter. Its historical
  input artwork remains untouched.
- Correct the project-open filter to accept case-insensitive .chromiator suffixes,
  with a regression against the actual GTK FileFilter.
- Replace obsolete audit scenario names with the current six-scenario contract.

## Retained intentionally

The CLI and native UI audit driver have live consumers. Removed-Thresholds option
rejection is an actionable-error regression, not a surviving Thresholds engine.
CLI selection aliases still route into the single current site authority.

The archived web implementation and design artwork are retained as historical
algorithm provenance, not linked application code. A raster fixture in that
archive remains a current test input. Personal presets, user artwork, and the
user's CSS adjustments are outside this cleanup's edit scope.

## License maintenance

After changing dependencies, fetch the locked sources and regenerate notices:

```sh
cargo fetch --locked
python3 scripts/generate_license_notices.py
python3 scripts/generate_license_notices.py --check
```

The generator includes all 131 third-party packages in the resolved lockfile,
including platform-specific and build dependencies. It fails rather than silently
omitting a package without a license file. Recorded upstream exceptions live in
licenses/upstream, with version-specific provenance. The inventory is not proof
that every listed package is linked into a given binary, nor a complete audit of
a future binary distribution's system-library/source obligations.

## Verification on 2026-09-19

- cargo test --all-targets: 106 passed; five display/artifact/performance tests
  ignored by default.
- The new ignored chooser regression was run explicitly in private Sway and
  passed all six filename cases.
- Dependency notice regeneration/check: passed, 131 packages.
- LICENSE.md comparison against https://www.gnu.org/licenses/gpl-3.0.txt: exact.
- Strict Clippy passed after moving the chooser test module to the end of
  shell_actions.rs.
- The approved harness repair releases widget registry borrows before callbacks
  and verifies that matching, influence, and expansion preserve the initial
  Source/Target colors instead of assuming the fixture has unequal colors.
- The six native audit routes (shell, Voronoi, presets, picker, I/O, responsive)
  all passed in private Sway after the repair: zero failures, skipped actions,
  malformed log entries, or reported GTK/panic diagnostics.
- All 13 binary tests and the separate display-dependent chooser regression
  passed again after the harness repair. Strict Clippy also passed again.

Native audit logs are in tests/artifacts/audit. These bounded automated results
are not a substitute for full human GNOME/Wayland visual acceptance.
