# History

This file keeps durable project context without preserving the old assistant bootstrap instructions.

## 2026-06-19

- Added README examples using `SpectrumBreakpoint.png`, `Demo1.png`, `Demo2.png`, and `Demo3.png`.
- Added strict TypeScript checking and initial Vitest coverage for posterize/color conversion kernels.
- Fixed built-in preset fallback typing to match manifest preset entries.
- Removed a plaintext credential from the root bootstrap notes. The exposed credential was revoked before publication planning continued.
- Replaced the root `HYDRATE.md` and `Roadmap.md` files with public-facing docs under `docs/`.

## 2025-10-05

- Implemented PNG metadata embedding using tEXt chunks so exported PNGs can carry Threshiator settings.
- Added PNG settings import and JSON settings import/export.
- Added manifest-based preset loading for JSON and PNG presets.
- Added per-channel band steppers, draggable histogram guides, and live dual-canvas preview.
- Added per-band output controls with live posterized preview updates.
- Added channel synchronization and combined histogram presentation.
- Added reset behavior that preserves the currently loaded image.
- Added ESLint and Prettier baseline configuration.
- Added README coverage for project goals, features, scripts, and roadmap ideas.
