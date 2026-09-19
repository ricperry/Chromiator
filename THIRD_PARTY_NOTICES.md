# Third-party notices

Chromiator is distributed under GPL-3.0-or-later; see [LICENSE.md](LICENSE.md).
Dependencies and adapted code retain their upstream copyrights and licenses.

## Rust libraries

The locked dependency inventory and license/notice files shipped with those
crates are in [licenses/RUST_DEPENDENCIES.md](licenses/RUST_DEPENDENCIES.md).
It includes transitive, build, test, and platform-specific dependencies, not just
the crates named directly in Cargo.toml. Regenerate it after dependency changes
with `python3 scripts/generate_license_notices.py`.

| Libraries | Purpose |
| --- | --- |
| gtk4, glib, gio, gdk-pixbuf | Native GTK interface, event loop, files, and display integration |
| glib-build-tools | Compile embedded GTK resources at build time |
| image, png, exr | Raster decoding and PNG/OpenEXR output |
| moxcms | Embedded RGB ICC profile interpretation and conversion |
| serde, serde_json, zip | Project and preset serialization and containers |
| crossbeam-channel | Communication with background jobs |
| anyhow, thiserror | Error handling |
| tempfile | Safe temporary files and test fixtures |

The inventory includes upstream license alternatives and additional terms such
as Unicode data and LLVM exceptions. It does not replace them with the
application's GPL or claim every resolved crate is linked into every build.

## Native system libraries

The Rust GTK bindings have MIT licenses, but the native libraries they bind have
their own licenses. The principal native dependencies are:

| Project | License summary | Source |
| --- | --- | --- |
| GTK4 | LGPL-2.1-or-later for the toolkit; bundled components have additional terms | https://gitlab.gnome.org/GNOME/gtk |
| GLib / GObject / GIO | LGPL-2.1-or-later | https://gitlab.gnome.org/GNOME/glib |
| GdkPixbuf | LGPL-2.1-or-later | https://gitlab.gnome.org/GNOME/gdk-pixbuf |
| Cairo | LGPL-2.1-only OR MPL-1.1 | https://cairographics.org/ |
| Pango | LGPL-2.0-or-later | https://gitlab.gnome.org/GNOME/pango |
| Graphene | MIT | https://github.com/ebassi/graphene |

Copies of the installed packages' principal license texts are retained under
`licenses/system/`. Distribution packages can contain more components than the
libraries used by Chromiator; package-specific notices remain authoritative.
System libraries and their transitive dependencies are normally supplied by the
Linux distribution. A self-contained binary package must inventory the libraries
it actually redistributes and supply required notices and corresponding source
or other applicable license materials. The Rust inventory alone is not a
complete self-contained binary distribution compliance audit.

## Development tools and presentation reference

The optional private-Wayland test harness uses Sway, WayVNC, grim, AT-SPI,
Python, and VNCDoTool installed separately by the developer. These executables
are not bundled into Chromiator. Harness conventions and native presentation
were adapted from the sibling Toniator project; its code and artwork are not
redistributed wholesale as part of the application.

## Oklab and OKHSL color conversion

The Oklab and OKHSL conversion code in `src/color.rs`, and related Oklab matrices
in `src/voronoi.rs` and `src/processing.rs`, use Björn Ottosson's published work:
<https://bottosson.github.io/posts/oklab/> and
<https://bottosson.github.io/posts/colorpicker/>. The OKHSL implementation is a
Rust adaptation, with application-specific gamut-boundary handling. The original
MIT notice follows.

Copyright (c) 2021 Björn Ottosson

Permission is hereby granted, free of charge, to any person obtaining a copy of
this software and associated documentation files (the "Software"), to deal in
the Software without restriction, including without limitation the rights to
use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of
the Software, and to permit persons to whom the Software is furnished to do so,
subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
