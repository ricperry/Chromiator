# Chromiator

Chromiator is an artist-directed perceptual color-space partitioning and remapping
application for Linux. It uses editable Voronoi sites in color space to simplify,
segment, recolor, and blend an image's color structure while preserving its
spatial structure.

Use it for limited-palette artwork, grayscale studies, graphic posters, print-like
color reduction, or softly blended recoloring. You control which Source colors
define the regions and which Target colors replace them. Regions are determined
by color similarity, not by a pixel's location on the canvas.

## Installation

Chromiator uses native GTK4 widgets and runs on Linux, with Fedora and
GNOME/Wayland as its primary environment. It requires GTK 4.12 or newer and a
recent stable Rust toolchain to build from source. No GPU compute setup is needed.

On Fedora, install the build prerequisites:

```sh
sudo dnf install gcc git cargo rust gtk4-devel pkgconf-pkg-config
```

Download and build the application:

```sh
git clone https://github.com/ricperry/Chromiator.git
cd Chromiator
cargo build --release --locked
./target/release/chromiator
```

Optionally install the executable for your user:

```sh
install -Dm755 target/release/chromiator ~/.local/bin/chromiator
```

Run `chromiator` when `~/.local/bin` is on your `PATH`, or open a file directly:

```sh
chromiator --open /path/to/artwork.png
chromiator --open /path/to/project.chromiator
```

## Getting started

1. Open an image, browse an existing project, or try the Spectrum Example from
   the welcome screen.
2. Choose a preset for a starting point, or edit the automatically created sites.
3. Select a site and change its Target swatch to recolor the colors it represents.
4. Adjust Matching, Influence, and Transition width to control the result.
5. Compare Source and Result, save a project to keep editing, or export an image.

## Canvas and color sites

**Source** displays the original image. **Result** displays the processed image.
**Split** displays Result on the left and Source on the right; drag the divider
to reveal either side. Hide the adjustments panel when you want more canvas space.

Click visible artwork to create a site from that color. A Source sample attaches
to the original image; a Result sample uses the processed color and is detached.
Clicking an existing site marker selects it. To sample the original image, expose
it with Source or Split first.

The **+** button in Color sites opens the picker for a detached Source color.
A new site's Target initially matches its Source. Canceling the picker adds nothing.

Expand a site to edit its controls:

| Setting | What it does |
| --- | --- |
| Source | The color used to determine which image colors belong to this site. Editing it directly detaches the site from the image. |
| Target | The replacement color. Editing or moving Source does not change Target. |
| Arrow between swatches | Explicitly assigns the Source color to Target. |
| Influence | Expands or contracts the site's reach in color space. Higher values attract more colors; this is not a spatial brush radius. |
| Source position | Moves an attached sample within the original image. |
| Footprint | Samples a point or averages a 3 x 3 or 5 x 5 neighborhood in the original image. |
| Lock | Protects the site's local settings and prevents its deletion. |

Source and Target pickers support perceptual color controls as well as RGB values
and hexadecimal input. Changes are committed when you select a color; Cancel
leaves the original value alone.

## Color mapping settings

| Setting | What it does |
| --- | --- |
| Matching | Chooses the color-distance model for assigning pixels to sites. Perceptual (OKLab) is a useful starting point; RGB and HSV produce different partitions. |
| Smoothing | Smooths the input before color mapping. Zero preserves fine detail; higher values favor broader structures. |
| Transition width | At 0%, each pixel takes its winning site's Target color. Increasing the width blends competitive sites across color-space boundaries. This is not an image blur. |
| Blend space | Mixes Target colors in Oklab or Linear RGB, independently of Matching. Available when Transition width is greater than zero. |

Blending can produce more output colors than the number of Target swatches.
Hard transitions are useful for strict palette reduction; wider transitions are
useful for smooth recoloring. Smoothing is measured in preview pixels for the
preview and source pixels for export, so fine smoothed details can differ at
different resolutions.

## Presets

The built-in library offers nineteen starting points:

| Character | Presets |
| --- | --- |
| Graphic and colorful | Ink & Paper, Desert Dusk, Blueprint, Arcade Four, Night Neon, Teal and Tangerine, Primary Print |
| Muted and organic | Moss and Clay, Soft Pastel, Watercolor |
| Tonal and monochrome | Graphite, Sepia Press, Smudged Graphite |
| Reduced-ink looks | Mimeograph, Photocopy, Carbon Copy, Old Newsprint, Two-Color Press, Risograph |

Presets are editable recipes, not separate processing modes. Some deliberately
discard subtle color information; others use smoothing and soft transitions.
Print-inspired names describe the palette and reduction style, not simulated
paper texture, halftones, or physical ink behavior.

Choosing a preset applies it immediately. Use Undo to return to your previous
settings. **Save Preset...** stores your current processing settings for reuse;
it does not include source artwork or image-marker positions. Built-ins are not
overwritten when you save a personal preset.

Personal presets live in `$XDG_DATA_HOME/chromiator/presets`, normally
`~/.local/share/chromiator/presets`. Use **Open personal preset folder** to find
them and **Refresh personal presets** after making changes outside the app.
The app reports invalid preset files without preventing valid presets from loading.

## Projects and export

**Save Project** writes a `.chromiator` file containing the original artwork and
editable processing settings. **Save Project As...** creates another project file.
Recent projects appear on the welcome screen.

Open PNG, JPEG, TIFF, WebP, BMP, or still GIF images. SVG and animated GIF input
are not supported. Embedded RGB color profiles are interpreted on import; the
working image uses floating-point linear sRGB with transparency preserved.

Export the full-resolution result without editor markers:

| Format | Use |
| --- | --- |
| PNG 8-bit | Everyday sharing and viewing. |
| PNG 16-bit | Higher-precision integer output for further editing. |
| OpenEXR 32-bit float | Linear-sRGB floating-point output for compatible creative tools. |

Exporting an image does not replace saving an editable project. Project and
preset formats are still pre-alpha and may change; keep original artwork and
backups. Older unsupported formats and the former project extension are not
automatically migrated. Application data is stored under
`$XDG_DATA_HOME/chromiator`, normally `~/.local/share/chromiator`.

## Keyboard navigation

| Shortcut | Action |
| --- | --- |
| Ctrl+O | Open artwork or a project. |
| Ctrl+S | Save project. |
| Ctrl+Shift+S | Save project as. |
| Ctrl+Shift+E | Export image. |
| Tab / Shift+Tab | Move between controls. |
| F6 / Shift+F6 | Move between the canvas, mapping controls, and site list. |
| Alt+M / S / T / B | Focus Matching, Smoothing, Transition width, or Blend space. |
| Alt+I / P / F | Focus Influence, Source position, or Footprint in an expanded site. |
| Alt+C / H | Focus Color model or Hex in a color picker. |

Undo and Redo are available in the toolbar. Controls expose accessible names
and state through GTK's accessibility interfaces.

## License and credits

Chromiator is free software under the GNU General Public License, version 3 or
any later version. See [LICENSE.md](LICENSE.md) for the unmodified license text
and [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) for library and color-science
credits. Third-party components retain their respective licenses.
