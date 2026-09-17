# Third-party credits

OpenLaser is GPL-3.0-or-later; see [LICENSE](LICENSE). Dependency licenses retain
their own terms. The release source bundle includes the pinned Rust dependency
sources and their license files, plus the UI runtime sources. Binary packages
include the notices under `third-party-licenses/`.

## User interface

**Svelte 5.57.0**, its contributors, MIT license. The UI also uses the MIT-licensed
runtime packages **esm-env** and **clsx**. Their source and original notices are
included under `vendor-ui/` in the source bundle. License copies are embedded in
the application and included with the binary packages.

## Desktop window

**Wry 0.57.0**, **Tao 0.37.0**, and the macOS menu helper **Muda 0.20.0**, from
the Tauri project and their contributors, use Apache-2.0 OR MIT licenses.
Their complete sources and license files accompany the source bundle.
[Wry](https://github.com/tauri-apps/wry),
[Tao](https://github.com/tauri-apps/tao),
[Muda](https://github.com/tauri-apps/muda).

Windows desktop builds embed Microsoft's signed **WebView2 Evergreen
bootstrapper**. It installs Microsoft's separately licensed shared runtime only
when absent. See the [distribution notes](crates/openlaser/assets/README.md).
macOS and Linux use their platform WebKit runtimes; OpenLaser does not bundle a
private browser engine.

The Linux GTK 3 build macros still require the unmaintained `proc-macro-error`
1.0.4 crate. The narrowly scoped maintenance-advisory exception in `deny.toml`
records this dependency; it does not suppress vulnerability advisories. Windows
and macOS desktop builds do not use those GTK macros.

## Nesting

**Sparrow** by Jeroen Gardeyn. Copyright (c) 2025 Jeroen Gardeyn, KU Leuven.
MIT license. Used at commit `9ef45676695ef94d045ac8bff0530822127f1437`.
[Source](https://github.com/JeroenGar/sparrow/tree/9ef45676695ef94d045ac8bff0530822127f1437)
and [full license](ui/static/licenses/sparrow-MIT.txt).

**jagua-rs** collision detection engine by Jeroen Gardeyn, version 0.8.1.
Mozilla Public License 2.0.
[Source](https://github.com/JeroenGar/jagua-rs/tree/v0.8.1)
and [full license](ui/static/licenses/jagua-rs-MPL-2.0.txt).
The upstream source files are used without modifications.

Research credit: **Jeroen Gardeyn, Greet Vanden Berghe and Tony Wauters**,
*An open-source heuristic to reboot 2D nesting research* (2025).
[Original paper](https://arxiv.org/abs/2509.13329),
[DOI](https://doi.org/10.48550/arXiv.2509.13329), and the authors'
[preferred citation](https://github.com/JeroenGar/sparrow/blob/9ef45676695ef94d045ac8bff0530822127f1437/CITATION.cff).
OpenLaser uses Sparrow's optimizer for rectangles and its placement-search
components with jagua-rs for selected irregular stock. The fixed-stock adapter
is OpenLaser code and is not represented as the authors' published benchmark.

## Fonts

**Inter**, by Rasmus Andersson, uses the SIL Open Font License 1.1.
See the [Inter license](ui/static/licenses/Inter-OFL.txt).

**Noto Sans, Noto Serif and Noto Sans Mono**, regular and bold, by the Noto
Project Authors. SIL Open Font License 1.1. Bundled for SVG and DXF text
outlining and the text creation tool. See the [full license](ui/static/licenses/Noto-OFL.txt)
and [pinned sources and hashes](crates/openlaser-svg/fonts/SOURCES.md).

## SVG and text conversion

iOverlay, Nail Sharipov and contributors, MIT or Apache-2.0. Resolves overlapping
text fills into cutting boundaries while retaining holes.
[Source](https://github.com/iShape-Rust/iOverlay).

Lobster Two by Pablo Impallari, SIL Open Font License 1.1, is used as font-import
test data. See [the fixture license](fixtures/fonts/OFL.txt) and
[pinned fixture sources](fixtures/fonts/SOURCES.md). It is not bundled into the
application's default font choices.

**usvg** from [resvg](https://github.com/linebender/resvg), version 0.48.1,
MIT or Apache-2.0. Resolves SVG geometry and shapes text with its font stack;
OpenLaser converts the resolved paths to cutting contours.

## XML and content hashing

**quick-xml 0.41.0**, by the quick-xml contributors, MIT license. Its streaming
reader validates parameter-file syntax; OpenLaser retains the vendor-specific
rules and original bytes. [Source](https://github.com/tafia/quick-xml/tree/v0.41.0).

**sha2 0.10.9**, by the RustCrypto contributors, MIT OR Apache-2.0 license.
Computes the library's SHA-256 content identities.
[Source](https://github.com/RustCrypto/hashes/tree/sha2-v0.10.9).
The pinned sources and license files accompany the source bundle.
