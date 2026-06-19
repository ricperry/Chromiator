# Roadmap for Threshiator Prototype

## ✅ Completed Features (Phase 1)

### Core Posterization Engine

- ✅ **Per-channel posterization** with independent threshold controls
- ✅ **RGB/HSV posterize modes** (posterize in RGB or convert→posterize in HSV→convert back)
- ✅ **Optional alpha posterization** with its own histogram + controls
- ✅ **Live dual-canvas preview** with original and processed images
- ✅ **Interactive RGB/HSV histograms** with draggable threshold dividers
- ✅ **Per-band output sliders** for aggressive tonal remapping
- ✅ **Channel synchronization** toggle for unified editing
- ✅ **Real-time processing** with instant visual feedback

### PNG Metadata System

- ✅ **PNG metadata embedding** using tEXt chunks (ComfyUI-style)
- ✅ **Save Image functionality** with embedded Threshiator settings
- ✅ **Import from PNG** - load settings directly from saved images
- ✅ **CRC32 validation** for data integrity
- ✅ **Dual format support** for both JSON and PNG imports

### Preset Management System

- ✅ **Manifest-based preset loading** from `/presets/manifest.json`
- ✅ **Built-in presets** (Comic Book, Duotone Blue, Vintage Photo, Noir, Pop Art)
- ✅ **User-extensible presets** without code modifications
- ✅ **PNG preset discovery** with folder icon indicators
- ✅ **Graceful fallback** handling for missing files

---

## 🔄 Current Phase: Algorithm Refinement

### Core Algorithm Definition

- **Input**: Image (RGBA).
- **Parameters**:
  - Posterize space: RGB (R/G/B) or HSV (H/S/V).
  - Optional alpha posterization toggle.
  - Per channel (and optional alpha): number of bands (2-32), thresholds, per-band output level (0-255).
- **Processing** (RGB mode):
  1. Quantize each pixel channel value into its band.
  2. Replace with the band's output level.
- **Processing** (HSV mode):
  1. Convert RGB→HSV.
  2. Quantize H/S/V into bands and replace each with its output level.
  3. Convert HSV→RGB.
- **Alpha** (optional):
  - Quantize alpha using its own thresholds/bands/outputs.
- **Output**: Posterized image with embedded metadata.

---

## 🎯 Next Phase: Advanced Features

### UI/UX Enhancements

- ⏳ **Hue rotation controls** per band (–180° to +180°)
- ⏳ **Band width controls** (percentage-based thresholds)
- ⏳ **Keyboard shortcuts** for common operations
- ⏳ **Undo/Redo system** for parameter changes
- ⏳ **Preset categories** and search functionality

### Performance & Quality

- ⏳ **WebGL shader implementation** for real-time high-resolution processing
- ⏳ **Web Workers** for background processing of large images
- ⏳ **Progressive rendering** for better responsiveness
- ⏳ **Export options** (full-resolution PNG, LUT files)

### Advanced Processing

- ⏳ **HSL/LAB color space support** for alternative posterization methods
- ⏳ **Dithering options** for smoother gradients
- ⏳ **Edge detection** and preservation algorithms
- ⏳ **Multi-layer compositing** modes

---

## 🚀 Future Phase: Plugin Development

### A. **G'MIC (for GIMP, Krita, Paint.NET)**

- Language: G'MIC script (C-like, image pipeline oriented).
- Benefits: Runs inside GIMP/Krita, comes with UI definition syntax.
- Roadmap:
  1. Translate algorithm into G'MIC commands.
  2. Use `-split`, `-threshold`, `-blend`, `-hsv` ops to implement logic.
  3. Define per-channel UI with sliders and per-band controls.
  4. Port PNG metadata embedding to G'MIC format.
- Resources: [https://gmic.eu](https://gmic.eu)

### B. **Krita Python Plugin**

- Language: Python + PyQt.
- Benefits: Full UI control, direct access to Krita's pixel data.
- Roadmap:
  1. Build dockable widget with per-channel sliders.
  2. Use Krita's Python API for pixel manipulation.
  3. Implement preview rendering in a separate buffer for speed.
  4. Integrate PNG metadata system for preset sharing.
- Resources: [Krita Python API docs](https://docs.krita.org/en/user_manual/python_scripting/introduction_to_python_scripting.html)

### C. **Standalone Desktop Application**

- Language: Electron + TypeScript (reuse web codebase).
- Benefits: Cross-platform, full filesystem access, native performance.
- Features: Batch processing, drag-and-drop, native file dialogs.

---

## 4. Testing & Iteration

- **Test images**: use high-contrast photos, gradients, and color wheels to stress thresholds.
- Add **preset system** (JSON configs) for quick switching.
- Collect feedback from actual art workflows (comic effect, retro posters, duotone designs).

---

## 5. Future Enhancements

- GPU acceleration (GLSL shader or OpenCL).
- Add LAB/CMYK mode (banding in perceptual space).
- Live side-by-side view (original vs processed).
- Export LUT (lookup table) so effect can be applied in video editors (DaVinci Resolve, Blender, etc.).

---

👉 If you want, I can start you off with a **minimal HTML/JS prototype** that just:

- takes an image,
- applies simple per-channel posterization,
- and shows bands in a histogram.
