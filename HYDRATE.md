# Project Boot Hydration (Web Canvas Prototype)

This repo uses an open-source, Linux-friendly toolchain to prototype the **Super Posterize** effect in the browser (HTML/JS/Canvas/WebGL). This document defines how your code assistant should behave in this workspace.

## Recent Changes

[2025-10-05] **PNG Metadata System Implementation**:

- Added ComfyUI-style PNG metadata embedding using tEXt chunks
- Implemented "Save Image" functionality with embedded SuperPosterize settings
- Added PNG import capability - load settings directly from saved PNG files
- Created manifest-based preset system supporting both JSON and PNG presets
- Added CRC32 validation for PNG chunk integrity
- Built comprehensive preset management without hardcoded filenames

**Key Features Added:**

- `embedMetadataInPng()` - embeds settings as PNG tEXt chunks
- `extractMetadataFromPng()` - extracts settings from PNG files
- `saveProcessedImage()` - complete save workflow with metadata embedding
- Unified preset loading from `/presets/manifest.json` for both formats
- Enhanced Import Settings to handle both JSON and PNG files

[2025-10-05] Improved histogram UX:

- Added inline level controls in histogram canvas
- Removed separate band level sliders for more compact layout
- Improved visibility of threshold and level controls

> This replaces the prior Python-oriented HYDRATE with a Node/Vite/TypeScript stack suitable for rapid web prototyping and later portability to G’MIC/Krita.

---

# Code Assistant Guidelines

You are the code assistant for this repository. **Follow this document exactly.**
For record purposes, the API key for OpenAI Codex is: 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

---

## Operator Intent

- I **APPROVE** automated command execution that is safe, local to this repo, and non-destructive.
- Perform all file edits **directly in the editor**.
- Do **not** create CLI diffs, patches, or merge prompts.
- Do **not** ask me to run a CLI merge tool.
- If an action would be destructive, write outside the repo, change global machine state, or access secrets, **pause and ask first**.
- Keep edits **atomic** and explain what changed and why.

---

## Operating Rules

### Editor Only for Edits

- Create, modify, and delete files using **editor operations only**.
- Never request or perform diff merges, patch applies, or external merge workflows.

### Commands Policy

You may run local commands inside this repo **without asking each time**. I APPROVE.

**Allowed examples (Linux-friendly, no proprietary tools):**

- `node --version`, `npm --version` (or `pnpm`/`bun` if present)
- `npm ci` / `npm install`
- `npm run dev` (local dev server)
- `npm run build` (production build)
- `npm run preview` (serve built assets locally)
- `npm run test` (Vitest unit tests)
- `npm run lint` (ESLint)
- `npm run format` (Prettier)
- `git add` / `git commit` inside this repo

**Ask first** if any command would:

- install global packages (prefer project-local devDeps),
- change OS services or system configs,
- require elevated privileges,
- contact external networks beyond typical `npm install`,
- publish or deploy artifacts.

---

## Technical Implementation Details

### PNG Metadata System

The PNG metadata system embeds SuperPosterize settings directly into PNG files using the tEXt chunk format, similar to ComfyUI workflows:

**Core Functions:**

- `embedMetadataInPng(pngData, keyword, text)` - Inserts tEXt chunk before IEND
- `extractMetadataFromPng(pngData, keyword)` - Extracts text from tEXt chunks
- `calculateCRC32(data)` - Validates PNG chunk integrity
- `findChunk(pngData, chunkType)` - Locates specific PNG chunks

**Workflow:**

1. Canvas `toBlob()` generates base PNG data
2. Settings serialized to JSON string
3. tEXt chunk created with keyword "SuperPosterize"
4. Chunk inserted before IEND with proper CRC32
5. Modified PNG available for download with embedded metadata

### Preset Management System

Manifest-driven approach eliminates hardcoded filenames:

**Structure:**

```json
{
  "jsonPresets": ["Comic_Book.json", "Noir.json"],
  "pngPresets": ["MyEffect.png", "CustomStyle.png"]
}
```

**Loading Process:**

1. Fetch `/presets/manifest.json`
2. Load JSON presets via `loadJsonPreset()`
3. Load PNG presets via `loadSinglePngPreset()`
4. Populate dropdown with unified preset list
5. Fallback to built-in presets if manifest fails

**User Workflow:**

- Add preset file to `/presets/` folder
- Update manifest.json to include filename
- Refresh page to load new preset

---

## Project Logging

- Update the **Project Diary** and **Change Log** sections **in this HYDRATE.md** after each meaningful change.
- Keep newest entries at the top.

---

## Commit Style

- Use clear commits.
- Prefer **Conventional Commits** when possible, e.g.: `feat:`, `fix:`, `refactor:`, `docs:`, `chore:`, `test:`, `build:`.

---

## Hydration Checklist (Idempotent)

> **Important:** Do **not** recreate or overwrite anything that already exists. Only create missing items. If an item exists, **verify and move on**.

### A. Verify Existing Scaffold (Web)

- ✅ `index.html` — **if missing**, create a minimal page with an `<input type="file">`, a `<canvas id="preview">`, and a `<div id="histograms">`.
- ✅ `src/` — **if missing**, create:
  - `src/main.ts` (or `main.js`) — bootstraps UI, file loader, rendering loop.
  - `src/effect.ts` — core Super Posterize algorithm (per-channel bands, widths, level, hue rotate).
  - `src/histogram.ts` — RGB histogram computation + band divider overlay.
  - `src/ui.ts` — state model, sliders, tab logic (R/G/B), presets, lock-channels.
  - `src/types.ts` — (if TS) shared types for ChannelState/Band.

- ✅ `styles/` — **if missing**, add `styles/main.css` (basic, dark theme).
- ✅ `public/` — **if missing**, include a small sample image and `favicon.svg`.
- ✅ `package.json` — **if missing**, create with scripts:
  - `dev`, `build`, `preview`, `lint`, `format`, `test`

- ✅ Vite config — **if missing**, add `vite.config.ts` tuned for static export.
- ✅ TypeScript — **if missing**, add `tsconfig.json` with strict mode. (If you prefer JS, skip TS files/config and use JSDoc types.)
- ✅ ESLint/Prettier — **if missing**, add `.eslintrc.cjs`, `.prettierrc`, and `/.editorconfig`.
- ✅ Tests — **if missing**, add `tests/effect.spec.ts` (Vitest) covering band selection, width normalization, and hue rotation correctness.
- ✅ `.gitignore` — Ensure Node ignores exist (e.g., `node_modules/`, `dist/`, `*.log`); **append** if needed.
- ✅ `.vscode/settings.json` — **do not overwrite**. Only append non-conflicting keys (TypeScript auto-imports, format on save).
- ✅ This `HYDRATE.md` — update logs only.

### B. Normalize (Only If Needed)

- Run `npm ci` if `node_modules/` does **not** exist.
- Run `npm run format` and `npm run lint` after adding files to ensure a consistent baseline.

### C. Backlog & Logs

- Add/Update **Backlog** items in this file.
- Append a **Change Log** entry titled “Hydration (verification)” that lists exactly what was created or appended (skip items already present).

---

## When Uncertain

- State assumptions.
- Propose options.
- Pick a sensible default with minimal lock-in.

---

## Repository Conventions

- **Languages and tooling**: TypeScript (or modern JS) + Vite + Canvas/WebGL.
- **Formatting**: Prettier.
- **Linting**: ESLint (typescript-eslint if TS).
- **Testing**: Vitest for unit tests; optional Playwright for basic visual regression on bands/histograms.
- **Task runner**: NPM scripts in `package.json`.

---

## Architecture Notes (Super Posterize)

- **State model**

  ```ts
  type Band = { widthPct: number; levelPct: number; hueDeg: number };
  type ChannelState = {
    levels: number;
    autoWidth: number;
    clampEdges: boolean;
    bands: Band[];
  };
  type AppState = {
    lockChannels: boolean;
    R: ChannelState;
    G: ChannelState;
    B: ChannelState;
  };
  ```

- **Algorithm**
  1. Normalize per-channel `bands[].widthPct` to sum to 100 (preserve ≥1% min; repair rounding drift).
  2. Build cumulative thresholds → map each pixel’s channel to a band index.
  3. Apply band **output level** to that channel (quantized value).
  4. Convert pixel to HSL (fast approximation ok), apply **band hue rotation**, convert back to RGB.
  5. Optional: **clampEdges** to force first/last bands to include [0, minEdge] and [maxEdge, 255].

- **Performance**
  - Start with Canvas `ImageData` loop; later add a WebGL shader path.
  - For live preview, decimate to a working resolution (e.g., longest edge 1600 px), re-render full res on “Apply/Export”.

- **Histograms**
  - Compute 256-bin R/G/B; draw with band separators from the current channel’s thresholds.

---

## Scripts (suggested `package.json`)

```json
{
  "scripts": {
    "dev": "vite",
    "build": "vite build",
    "preview": "vite preview --strictPort",
    "lint": "eslint .",
    "format": "prettier -w .",
    "test": "vitest run"
  }
}
```

---

## Linux Setup Hints (Optional)

- Install Node via your distro or **nvm** (keeps global state clean).
- No global installs required; all devDeps are local.

---

## Project Diary

Short notes that provide context and decisions as the project evolves. Keep newest at the top.

- [2025-10-05] **Major Feature:** Implemented ComfyUI-style PNG metadata embedding system. Users can now save processed images with embedded SuperPosterize settings and load settings directly from PNG files. Built manifest-based preset system supporting both JSON and PNG presets, eliminating hardcoded filenames.
- [2025-10-05] Added comprehensive preset management with `/presets/manifest.json` controlling which files are loaded as presets. Users can add custom presets without code changes.
- [2025-10-05] Built PNG tEXt chunk manipulation with CRC32 validation for reliable metadata embedding and extraction.
- [2025-10-05] Enhanced Import Settings to handle both JSON and PNG files seamlessly with proper error handling and fallbacks.
- [2025-10-05] Added per-channel band steppers and draggable histogram guides; split original vs. processed canvases with live updates and multi-channel thresholds (Apply button removed).
- [2025-10-05] Scaffolded ESLint config and baselined lint task for the TypeScript stack.
- [2025-10-05] Added per-band output controls with live posterized preview updates.
- [2025-10-05] Documented posterize helpers and added README outlining project goals and features.
- [2025-10-05] Reset button now restores panels without swapping out the loaded image.
- [2025-10-05] Channel level changes now re-linearize band outputs for predictable tonal ramps.
- [2025-10-05] Added synchronize toggle to mirror channel settings and collapse the UI when desired.
- [2025-10-05] Canvases now resize to match source resolution while CSS scales previews to fit.
- [2025-10-05] Summed histogram + RGB title when channels are synchronized.
- [2025-10-05] Added settings import/export workflow (JSON).
- [2025-10-05] Reviewed hydration guidelines and adjusted VS Code auto-import behavior to avoid invalid settings.

- [YYYY-MM-DD] Switched project hydration to web stack (Vite/TS/Canvas). Goal: fast browser prototype for Super Posterize; future port to G’MIC/Krita. (Replaces Python-scaffold focus.)

---

## Change Log

Record significant changes affecting behavior, scope, interfaces, infra, or developer workflow. Newest first.

### [2025-10-05] feat: PNG metadata embedding system

- Type: feat
- Summary: Implemented ComfyUI-style PNG metadata embedding for settings preservation and sharing.
- Details:
  - Added PNG tEXt chunk manipulation with CRC32 validation
  - Built `embedMetadataInPng()` and `extractMetadataFromPng()` functions
  - Created "Save Image" functionality with embedded SuperPosterize settings
  - Enhanced Import Settings to handle both JSON and PNG files
  - Implemented manifest-based preset system (`/presets/manifest.json`)
  - Added support for user-extensible presets without code modifications
  - Created JSON preset files for all built-in effects
- Impact: Users can share effects as PNG files, load settings from saved images, and add custom presets easily.

### [2025-10-05] feat: interactive band guides

- Type: feat
- Summary: Added per-channel band controls and draggable histogram guides with multi-channel posterize integration.
- Details:
  - Introduced numeric steppers and draggable dividers for all channel histograms
  - Split preview area into original and posterized canvases to preserve source histogram while previewing edits
  - Updated posterization pipeline to honor custom thresholds for red, green, and blue channels with live updates
- Impact: Enables visual threshold tuning prior to applying posterization to any channel.

### [2025-10-05] chore: add ESLint baseline

- Type: chore
- Summary: Added ESLint TypeScript configuration and dependencies so linting succeeds locally.
- Details:
  - Created `.eslintrc.cjs` extending recommended rules for browser TypeScript
  - Installed `@typescript-eslint/parser` and plugin; updated `package.json`
  - Ran `npm run lint` to confirm a clean baseline
- Impact: Enables consistent linting in local workflows and CI.

### [2025-10-05] feat: per-band output controls

- Type: feat
- Summary: Added UI and rendering logic to adjust per-band output levels with live feedback.
- Details:
  - Introduced draggable sliders for each band alongside histograms, clamped to their threshold ranges
  - Extended posterize algorithm to honor per-band output values and thresholds across all channels
  - Synced controls to live rendering so changes immediately update the processed canvas
- Impact: Enables fine artistic control over tonal levels within each band while retaining responsive preview.

### [2025-10-05] docs: add README and code comments

- Type: docs
- Summary: Added inline documentation for posterize helpers and created README.md to describe the prototype.
- Details:
  - Documented key functions in `src/effect.ts` and `src/main.ts`
  - Authored a README covering goals, features, scripts, and roadmap ideas
- Impact: Provides clearer orientation for collaborators exploring the codebase.

### [2025-10-05] fix: non-destructive reset

- Type: fix
- Summary: Reset button now restores panel defaults without swapping out the loaded image.
- Details:
  - Preserved the currently loaded image when resetting state
  - Ensured level steppers, band sliders, and histograms revert to defaults
- Impact: Users can experiment then revert settings without losing their chosen source image.

### [2025-10-05] fix: re-linearize band outputs on level change

- Type: fix
- Summary: Changing a channel's band count recalculates band outputs to an even 0–255 ramp.
- Details:
  - Added linear output generator used whenever levels reset or change
  - Prevented stale output values from darkening channels when bands are reduced or added
- Impact: Histogram and preview stay consistent after adjusting band counts.

### [2025-10-05] feat: synchronize channels toggle

- Type: feat
- Summary: Added a UI toggle to edit all channels in unison via a single panel.
- Details:
  - Mirrored levels, thresholds, and outputs across RGB when synchronization is enabled
  - Collapsed the histogram/controls to a single stack while locked
  - Restored per-channel panels when the toggle is disabled without mutating values
- Impact: Speeds up adjustments for unified looks while keeping fine control a toggle away.

### [2025-10-05] fix: full-resolution canvas rendering

- Type: fix
- Summary: The preview now operates on the full source resolution while shrinking in CSS.
- Details:
  - Resized processing canvases to image dimensions rather than a fixed 800×300 box
  - Updated drawing helper and render pipeline to keep canvases in sync with loaded assets
- Impact: Eliminates letterboxing/transparency and improves fidelity for export.

### [2025-10-05] fix: synchronized histogram presentation

- Type: fix
- Summary: When channels are locked, the UI now labels the panel as RGB and shows a combined histogram.
- Details:
  - Updated titles to swap between per-channel names and RGB while synchronized
  - Replaced the red histogram with a summed RGB view when lock mode is active
- Impact: Provides a clearer single-panel workflow during synchronized edits.

### [2025-10-05] feat: settings import/export

- Type: feat
- Summary: Added buttons to save or load the current posterize configuration as JSON.
- Details:
  - Serialized channel levels, thresholds, outputs, and lock state to `SuperPosterSettings.json`
  - Added file loader to restore settings without disturbing the active image
- Impact: Enables sharing looks and restoring complex setups quickly.

### [2025-10-05] fix: VS Code settings

- Type: fix
- Summary: Corrected VS Code auto-import configuration to ensure valid code actions on save.
- Details:
  - Set organize imports action to boolean to satisfy VS Code expectations
- Impact: Prevents settings sync errors when saving files.

### [YYYY-MM-DD] Hydration (verification)

- Type: chore
- Summary: Verified/created minimal web scaffold and configs; removed Python-specific assumptions.
- Details:
  - Confirmed/added `index.html`, `src/` modules, `styles/`, `public/`
  - Added `package.json` scripts, Vite/TS/ESLint/Prettier/Vitest configs (non-destructive)
  - Ensured `.gitignore` includes Node/`dist` patterns

- Impact: Idempotent setup ensures assistant won’t redo or fight existing configuration.

### Template for future entries

- Type: feat | fix | refactor | chore | docs | test | perf | build
- Summary: One-line summary
- Details:
  - Bullet 1
  - Bullet 2

- Impact: What matters for users or developers
- Related: Issue IDs or links if any

---

## Scope

- In scope:
  - Browser-based prototype of Super Posterize (HTML/JS/Canvas/WebGL)
  - Open-source dev tooling (Vite, ESLint, Prettier, Vitest)

- Out of scope:
  - Proprietary software or SDKs
  - Deployment infrastructure

---

## Backlog

1. **Prototype**: Implement CPU canvas version of Super Posterize (per-channel levels, widths, band level, per-band hue).
2. **Histograms**: Live RGB histograms with band separators; show clipping overlay toggle.
3. **Presets**: Flat / Punchy / 80s Comic / Duo-Tone (JSON).
4. **Lock-Channels**: Mirror band edits across channels when enabled.
5. **Performance**: Add WebGL shader path; compare results against CPU path.
6. **Export**: PNG/JPEG export; optional LUT export for external apps.
7. **Porting**: Evaluate **G’MIC** script path for GIMP/Krita; evaluate **Krita Python** dock plugin UI parity.
8. **Tests**: Unit tests for band mapping, width normalization, hue rotation; optional Playwright snapshots for histogram rendering.

---

## Refactor Log

- [YYYY-MM-DD] Rendering: factored color conversions into `color.ts` utilities for reuse in CPU and WebGL paths.

---

## Decision Log

- [YYYY-MM-DD] **TypeScript over JS** for safer refactors; JSDoc acceptable if TS friction arises.
- [YYYY-MM-DD] **Canvas first**, WebGL second — simpler to validate correctness before chasing perf.

---

## Operational Safeguards

- **Secrets**: never commit secrets. This is a static web project; no API keys required.
- **Data**: use sample or user-provided local images only.
- **Destructive tasks**: require explicit confirmation before deleting or overwriting user images.

---

## Editor Settings

Prefer in-editor writes. If needed, workspace settings can reinforce editor behavior.

**VS Code recommendations:**

- Enable “format on save”
- Use the ESLint and Prettier extensions
- Keep Git integration enabled for inline diffs and quick staging
