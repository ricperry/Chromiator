# TODO

This list tracks project work that remains useful after the initial prototype bootstrap. Keep near-term items specific enough to turn into issues.

## Before Publishing

- Scrub sensitive data from git history before making the repository public. The exposed credential has been revoked, but a clean public history is still preferable.
- Decide whether tracked `dist/` files should stay in the repository or be removed in favor of local/CI builds.
- Add a short contribution note once the public workflow is clear.

## Near Term

- Add hue rotation controls per band.
- Add percentage-based band width controls that complement the current threshold controls.
- Add undo/redo for parameter changes.
- Add keyboard shortcuts for common operations.
- Add preset categories and search.
- Expand unit coverage for HSV posterization edge cases and PNG metadata parsing.
- Add a small visual smoke test for loading the app, rendering the default image, and changing a threshold.

## Performance And Export

- Move high-resolution preview processing to a Web Worker.
- Evaluate a WebGL shader path for real-time large-image processing.
- Add progressive rendering or preview decimation for very large images.
- Add export options for full-resolution PNG and LUT files.

## Processing Ideas

- Add HSL and LAB color-space support.
- Add dithering options for gradients.
- Explore edge preservation controls.
- Explore multi-layer compositing modes.

## Future Ports

- Evaluate a G'MIC script path for GIMP/Krita/Paint.NET.
- Evaluate a Krita Python dock plugin with UI parity.
- Consider a standalone desktop build once the browser prototype stabilizes.
