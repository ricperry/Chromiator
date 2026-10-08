# Chromiator 0.3.0

Pre-alpha release · 2026-10-07

Chromiator edits color relationships in the image rather than pixel positions.
Choose Source colors to define color-space regions, then set independent Target
colors to simplify, segment, or recolor the image.

## What's new since 0.2.0

- Presets now live in the header menu. Select a built-in or personal preset,
  preview it before applying, load a current project or preset recipe, and save
  personal looks in the XDG data directory.
- Color mapping controls sit in a compact, collapsible group.
- New images receive up to six automatically chosen colors from distinct
  visible color groups. Candidate colors come from source pixels, and isolated
  single-pixel noise does not add sites on normal-sized images.
- Smoothing is measured in original source pixels from 0 to 10,000. Preview
  processing uses the full source before reducing the result to the preview
  bound; PNG8 export with the same sampling matches the preview's displayed
  pixels.
- The canvas divider and inspector preserve a useful workspace during resize.
  Hiding the inspector moves Save, Undo, and Redo into the Document menu and
  permits a 600-pixel compact window. Canvas corners are square.
- AppImage packaging uses a hash-pinned Debian 13 x86_64 builder and a pinned
  AppImage type-2 runtime. This records the build baseline; it does not certify
  every Linux distribution or desktop environment.

## Install

The x86_64 AppImage can be made executable and launched directly:

```sh
chmod +x Chromiator-0.3.0-x86_64.AppImage
./Chromiator-0.3.0-x86_64.AppImage
```

For the Flatpak bundle, install it for your user and launch the app:

```sh
flatpak install --user ./Chromiator-0.3.0-x86_64.flatpak
flatpak run io.github.chromiator.Chromiator
```

The GitHub release includes the matching source, license notices, and SHA-256
checksums alongside the Linux packages.

## Package verification

Both release bundles passed the six package audit routes in an isolated
private-Sway session: shell, Voronoi, presets, picker, I/O, and responsive
layout. The package logs had no warnings, GTK criticals, panics, or other
stderr output. Inspected AppImage and Flatpak captures show the square canvas
corners and populated six-site inspector.

The AppImage passed both AppDir and extracted-package ELF audits: 119 ELF files,
zero audit errors, and a GLIBC 2.41 ceiling. These package checks do not certify
GNOME/Mutter, a clean Debian desktop, every Linux distribution, or full
assistive-technology behavior.

| Package | Size | SHA-256 |
| --- | ---: | --- |
| `Chromiator-0.3.0-x86_64.AppImage` | 29,608,440 bytes | `d885280be5a8865ae0233f5b48828542adcd3244ac10bb83da0de8cc738572a7` |
| `Chromiator-0.3.0-x86_64.flatpak` | 3,332,168 bytes | `61d0ddd54174df564063f2782d1e4492e166c96cfa9264ca4c62890c655dfb1e` |

## Compatibility

This pre-alpha version reads project format v6 and preset JSON v3. Earlier
pre-alpha project and preset versions are rejected without migration or rewrite.
Keep original artwork and backups; these editable formats may change before
alpha. Flatpak personal presets are stored in the app's private data directory
and are not automatically merged with host presets.
