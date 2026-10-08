---
name: chromiator-orchestrator
description: Coordinate substantive Chromiator engineering, Voronoi-focused GTK refactoring, and verification using the project's private Sway skill. Use for bounded project work and specialist routing; tiny incidental edits can stay with the parent.
---

# Chromiator orchestration

The selected parent owns scope, settled decisions, integration, and final
evidence. Use ROUTINE for substantive bounded delegation by default, including
implementation work that a single specialist can complete. Keep only tiny
incidental edits in the parent. Use specialist readers and reviewers when they
answer an independent question or provide meaningful verification.

## Current app and next-task boundary

Chromiator has a Rust GTK4 application; the authorized refactor removes
libadwaita. Start from its
native source, tests, README, and issue ledger. The archived browser demonstrator
is historical algorithm evidence, not the default exploration or architecture
target. Never adopt a web wrapper or embedded browser.

The authorized GTK refactor is modeled visually and interactively on
`../Toniator/`, focusing exclusively on Voronoi color-space mapping.
Its edit phase removes the separate Thresholds processing mode. Consult
`docs/FUNCTIONAL_AUDIT.md` for outstanding final verification; edits alone are
not a completed or accepted milestone.
Do not preserve dual-mode organization simply because the current app has it.
Do not begin the refactor as a side effect of skills/agent maintenance.

Voronoi here means independent Source colors mapped to independent Target
colors by weighted nearest-site matching in the selected color space. It is
not a geometric tessellation drawing tool. Preserve useful matching-space
choices; RGB and HSV Voronoi metrics are not Thresholds modes.
Treat novelty as product motivation, not an established market-exclusivity claim.

Before the refactor changes code, establish the bounded brief:
- Inventory method dispatch, UI controls, built-in/user presets, recipe and
  project schemas, CLI/evidence controls, tests, documentation, and shared code
  affected by removing Thresholds.
- Distinguish Thresholds-only code from shared color, smoothing, picker,
  sampling, image I/O, and history capabilities used by Voronoi.
- Define the destination module/state boundaries and checkable before/after
  behavior. Remove redundant mode selection; do not leave a disabled legacy
  mode masquerading as simplification.
- State the policy for existing dual-mode projects and presets before changing
  their schema. The app is pre-release: prefer explicit current-format support
  over speculative migration layers. Never silently reinterpret a Thresholds
  recipe as Voronoi or rewrite/delete incompatible user files.
- Identify intended implementation removals in the brief and honor the user's
  explicit deletion authority. Preparing that brief does not itself delete code.

## Toniator as a style reference

Read `../Toniator/docs/ui/REFERENCES.md` and inspect the current relevant
mockups and running UI/screenshots when settling presentation. Translate its
canvas emphasis, contextual controls, grouped inspector, spacing, hierarchy,
system-theme behavior, and progressive disclosure to color-site editing.
Keep Chromiator vocabulary and artistic workflow authoritative; do not import
Toniator Patterns, channels, geometry engines, schema, or stage governance.

The user explicitly selected pure GTK4 and removal of libadwaita. Preserve that
decision, native system-theme behavior, resource-backed UI composition, and the
canvas-left/grouped-right-inspector workflow.
Keep the sibling checkout read-only and use Chromiator's own fixtures.
Do not copy Toniator's protected specifications or project-specific tests.

## Preserve processing and document authority

Use one authoritative recipe/document state. GTK projects it; workers consume
snapshots and return revision-checked results. Keep domain/image processing
independent of GTK, and share processing semantics between preview and export.

Keep DocumentSession as authored-state/history/selection authority. Project v6
and preset v3 deliberately reject older files without migration or rewriting.
Persist only the supported hard transition profile `(0.5, 0.5, 0.5)` for now.
Future boundary blending belongs behind the partition/color-resolution boundary;
do not invent adjacency, interpolation, or multi-site behavior during refactoring.
See `docs/ARCHITECTURE.md` for ownership and extension guidance.

Preserve the established straight-alpha linear-sRGB `RGBA f32` working
pipeline, ICC interpretation, transfer functions, deterministic site ordering,
metric definitions and influence weighting. Avoid intermediate quantization.
Preserve source sampling from the full-resolution image, Source/Target edit
semantics, locks, footprints, marker/selection behavior, transactional edits,
undo/redo, dirty/savepoint behavior, and source-free reusable presets unless
the authorized brief explicitly changes them.

Keep decoding, processing, and exports off the GTK main thread. Preserve
cancellation, latest-result-wins preview publication, safe atomic writes,
accurate progress, and full-resolution exports without editor overlays.
Document supported formats/depths from actual library behavior; raster-only
input does not expand to SVG by analogy with Toniator. Prefer CPU-portable
algorithms; measure before proposing AMD/ROCm acceleration or a dependency.

## Scope and evidence

Before editing, inspect Git status, relevant existing changes, current task
authority, owning paths/callers, and affected tests. Reuse checkout-matching
evidence; stale summaries never override source or user decisions.
Use focused navigation; do not turn onboarding into a broad audit.

Give a writing delegate exact paths, outcome, preserved invariants, permitted
semantic changes, acceptance checks, and a stop boundary. Tell it other work
may exist and must be preserved. Keep one writer including the parent, no
nested delegation, and at most two independent read-only children.
Read-only reviewers consume parent/writer-produced runtime artifacts when
their sandbox cannot launch the stateful harness.

## Specialist routing

Use these exact profiles for delegated work:

| Profile | Assignment | When to use |
| --- | --- | --- |
| ROUTINE | `gpt-6-luna` / `max` | Default for substantive exploration, design, implementation, and review. |
| ESCALATED | `gpt-6-sol` / `xhigh` | Difficult reasoning exceeds ROUTINE after checking instructions, context, tools, and project authority, or clear complexity justifies starting there. |
| FRONTIER | `gpt-6-astra` / `high` | Architecture reconciliation, conflicting specifications, difficult cross-domain judgment, exceptional difficulty, a genuine blocker, or a demonstrated Sol capability limit. Importance or size alone is not a trigger. |

Keep each role purpose-specific: `codebase_explorer` traces ownership and
callers; `desktop_implementer` owns settled GTK/state implementation;
`product_architect` resolves consequential product or ownership choices;
`color_pipeline_specialist` checks numerical and image-processing semantics;
`test_performance_reviewer` checks regressions and measured responsiveness;
`ux_reviewer` reviews actual interaction and accessibility evidence; and
`creative_tester` assesses artistic results and creator workflow. Select the
profile for the task rather than assigning a model permanently to a role.

The parent owns consequential decisions and integration. Delegate substantive
writing to `desktop_implementer` by default; keep one writer, avoid nested
delegation, and do not run roles mechanically or repeat exploration. Correct
missing context or tooling before escalating. Role files define purpose and
permissions; project defaults provide ROUTINE, while the selected profile is
chosen for each task. Use one profile per child. These pairs are fixed; change
them only when the user explicitly instructs it. Do not tune effort separately.

Restart the session to load changed defaults and role-pin removals. In an
existing session, use an unpinned generic role and pass the explicit
model/effort pair for any profile, including ROUTINE; stale cached custom-role
pins can override the spawn choice. Full-history forks inherit the parent and
reject such overrides; use a supported fork mode that accepts the explicit
profile. Verify actual model/effort from runtime or session metadata when
available; otherwise distinguish the requested profile from verified
identity. See
[official subagent configuration](https://learn.chatgpt.com/docs/agent-configuration/subagents)
for configuration behavior; current runtime support governs what is available.

## Verification and handoff

Choose tests from affected behavior and consumers. For shared recipe/schema
or enum changes, compile the GTK target too and run relevant strict Clippy,
formatting, persistence/history, and preview/export regression checks.
Tests for removed Thresholds behavior must be reconciled with the agreed
removal; preserve tests protecting retained shared capabilities.
Do not add tests that merely match implementation wording.

Use `../gtk-wayland-debug/SKILL.md` for GTK work. Follow
wait -> scoped controls -> inspect -> semantic action -> readback.
Use coordinates for spatial canvas gestures only. Account for changed controls'
names, roles, state/value/selection, enabled state, label relations, keyboard
paths, and visible feedback. Exercise the actual relevant workflow and inspect
screenshots; source review or a screenshot alone cannot prove interaction.
Capture a usable narrow-window layout as well as the working desktop layout.
Automated Sway evidence is not human GNOME/Mutter acceptance.

For processing changes, compare representative preview/full-resolution outputs,
inspect actual images and alpha, and retain artifact paths with commands and
limits. Parent inspection of relevant rendered artifacts is required before
claiming visual verification. Report failures by severity and reproduction;
fix in-scope blockers and recheck affected behavior.

Keep reports concise: decisions, changed files, checks/artifacts, and uncertainty.
Update README on product milestone completion and ISSUES with stable IDs and
completion evidence. User acceptance is distinct from passing checks.
Stop at the assigned boundary. Never commit, push, publish, release, deploy,
delete implementations, or overwrite unrelated work without explicit authority.
