# Chromiator project guidance

Use `.agents/skills/chromiator-orchestrator/SKILL.md` for project engineering
and orchestration. The intended parent is `gpt-6-astra` with `high` reasoning.
The parent may work directly; delegate only independent work or useful review.
Keep one writer at a time, including the parent.

## Product direction

The next product task is a refactor of the existing native GTK application,
with visual and interaction style modeled after `../Toniator/`, and Voronoi
color-space mapping as the sole creative processing mode. Thresholds is to be
removed as a separate mode. This guidance records intended work, not completed
implementation or permission to start that next task during tooling maintenance.

Preserve independent Source/Target sites, matching spaces, Influence, locks,
sampling, undo/redo, preview/export fidelity, and the floating-point color
pipeline. RGB/HSV matching inside Voronoi is not the Thresholds engine.
Use Toniator as a read-only presentation reference; its pattern/channel domain,
formats, stage gates, and fixture requirements do not govern Chromiator.
Resolve GTK/libadwaita dependency changes in the refactor brief; adopting style
alone does not settle whether to remove libadwaita.

## Working rules

- Inspect relevant source, documentation, Git status, and existing edits before
  changing files. Preserve unrelated assets and worktree changes.
- Read current relevant `README.md`, `ISSUES.md`, and task evidence. Historical
  dual-mode documentation describes the current app, not the future target.
- Make the smallest coherent change. Settle state ownership, persistence,
  history, and preview/export consumers before cross-module changes.
- Run checks appropriate to the changed behavior. For GTK work use
  `.agents/skills/gtk-wayland-debug/SKILL.md`, exercise semantic actions and
  readback, and inspect screenshots. For color/output work inspect image
  artifacts as well as numeric evidence. Build success is not visual acceptance.
- Update `README.md` when a product milestone completes; maintain stable IDs,
  status, priority, context, and completion evidence in `ISSUES.md`.
  Record newly reported unrelated issues without derailing the active task.
- Target Fedora GNOME/Wayland, with CPU-portable processing and AMD/ROCm-aware
  acceleration only when justified. Prefer maintainable Linux-native libraries.
- Keep `../Toniator/`, `archive/webapp/`, and user-owned assets unchanged unless
  explicitly assigned. Never commit, push, publish, deploy, delete implementations,
  or discard user data without explicit authorization.
## Settled Voronoi refactor decisions (2026-09-13)

This section supersedes earlier next-task wording about choosing whether to
retain libadwaita. The user explicitly selected GTK4-only widgets, removal of
Thresholds and its mode switch, and Toniator-style presentation. The application
edit phase now contains that conversion; final build/runtime/performance
verification and user acceptance remain pending. Read
`docs/FUNCTIONAL_AUDIT.md` before claiming completion.

Project v6/preset v3 have no legacy migration. The persisted transition profile
is hard-only `(0.5, 0.5, 0.5)`; boundary blending is a later feature. Preserve
DocumentSession authority and the partition/color-resolution boundary described
in `docs/ARCHITECTURE.md`. Keep Toniator read-only. Do not add Rayon or claim a
performance gain without representative measurements and parity evidence.
