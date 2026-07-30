# Liquid-to-SVG extension rendering boundary

Status: decision-ready research for [issue #19](https://github.com/CorVous/WireTerm/issues/19)  
Evidence checked: 2026-07-30

## Recommended decision

Use this host-side pipeline for the portable, visible-window MVP:

```text
validated JSON data + validated item settings
    -> liquid-rust text rendering
    -> one fixed-canvas SVG string
    -> usvg parse/normalization
    -> resvg/tiny-skia 800 x 480 RGBA pixmap
    -> PNG encoder
    -> existing image/frame pipeline
```

Use [`liquid` 0.26.11](https://docs.rs/crate/liquid/0.26.11), but build a
versioned, curated language with `ParserBuilder::new()` and explicitly
registered standard-library blocks and filters. Do not expose the complete
standard library or add WireTerm-specific filters until a demonstrated
authoring need exists. Use the `resvg` family (`usvg`, `resvg`, and
`tiny-skia`) with system font loading and font memory mapping disabled. Load
only versioned bundled font bytes and validated extension font bytes into a
fresh font database. Replace `usvg`'s default image resolver with a WireTerm
resolver over an already validated, in-memory package snapshot.

Use the current `resvg` 0.47.x line for implementation and raise WireTerm's
declared Rust floor from 1.85 to 1.87 at that time. A toolchain floor is a
smaller and more visible dependency than carrying an older rasterizer through
the MVP. If the Rust floor cannot move, pin the proof to `resvg`/`usvg` 0.45.1
instead; do not let Cargo select 0.46.x, because that line also requires Rust
1.87.

This is the simplest practical boundary for the revised MVP:

- it removes WebView2, COM, browser installation, hidden-window capture, HTML
  layout synchronization, and browser-version drift;
- it stays inside a portable Rust dependency graph with no native renderer
  runtime to install;
- it produces pixels in an explicitly allocated 800 x 480 buffer; and
- upstream `resvg` states that, because it has no system-library dependency,
  the same SVG produces identical pixels across supported platforms.
  ([resvg project goals, portability, and reproducibility](https://github.com/linebender/resvg#readme),
  [`resvg::render`](https://docs.rs/resvg/latest/resvg/fn.render.html))

The tradeoff is intentional: SVG is not HTML. Extension authors get a small,
coordinate-based display language, not browser layout, automatic text
wrapping, JavaScript, or the TRMNL CSS framework.

The current project declares Rust 1.85 in
[`Cargo.toml`](../../Cargo.toml). `liquid` 0.26.11 declares Rust 1.83, so it
fits. The current `resvg`/`usvg` 0.47.0 release declares Rust 1.87, while
`resvg` 0.45.1 declares Rust 1.67.1 and `usvg` 0.45.1 declares Rust 1.65.
`resvg`/`usvg` 0.46.0 also require Rust 1.87. The recommendation is therefore
to move WireTerm to Rust 1.87 with `resvg` 0.47.x; 0.45.1 is the fallback if
the current MSRV is a hard constraint.
([`liquid` 0.26.11 manifest](https://docs.rs/crate/liquid/0.26.11/source/Cargo.toml.orig),
[`resvg` 0.47.0 manifest](https://github.com/linebender/resvg/blob/v0.47.0/crates/resvg/Cargo.toml),
[`usvg` 0.47.0 manifest](https://github.com/linebender/resvg/blob/v0.47.0/crates/usvg/Cargo.toml),
[`resvg` 0.46.0 manifest](https://github.com/linebender/resvg/blob/v0.46.0/crates/resvg/Cargo.toml),
[`resvg` 0.45.1 manifest](https://docs.rs/crate/resvg/0.45.1/source/Cargo.toml.orig),
[`usvg` 0.45.1 manifest](https://docs.rs/crate/usvg/0.45.1/source/Cargo.toml.orig))

## Liquid's exact role

Liquid is a generic text-template language. Objects insert text, tags provide
control flow such as conditions and loops, and filters transform values before
they are written. Nothing in Liquid parses SVG or converts data to graphics.
([Liquid introduction](https://shopify.github.io/liquid/basics/introduction/),
[control-flow tags](https://shopify.github.io/liquid/tags/control-flow/))

For WireTerm, the extension author writes literal SVG containing Liquid
expressions. Given data and settings, Liquid returns a string. That string must
already be valid SVG after substitution. `usvg`, not Liquid, parses it, and
`resvg`, not Liquid, rasterizes it.

Liquid does **not**:

- validate XML or the SVG profile;
- escape inserted values automatically;
- resolve images or load fonts;
- measure, wrap, truncate, or lay out text;
- interpret CSS as browser layout;
- enforce an 800 x 480 canvas; or
- create a PNG.

Every untrusted string inserted into text or an attribute must use the Liquid
`escape` filter. It replaces XML/HTML-sensitive characters, but it does not
make an arbitrary value safe as an SVG path, element, style declaration, URL,
or numeric coordinate. Those positions must receive typed, host-validated
values or fixed template literals.
([Liquid `escape`](https://shopify.github.io/liquid/filters/escape/),
[`liquid-lib` standard-library inventory](https://docs.rs/liquid-lib/latest/liquid_lib/all.html))

WireTerm should document and explicitly register this initial Liquid surface:

- interpolation of scalar values;
- property and array access;
- `if`/`elsif`/`else`, `unless`, and `case`;
- `for` over arrays, including the standard `forloop` values;
- whitespace control; and
- `escape`, `default`, `size`, `truncate`, and the small set of string and
  arithmetic filters needed by the reference fixture.

Do not register the standard `date` filter. `liquid-rust` interprets `"now"`
and `"today"` using the current UTC clock, so exposing it would make identical
input render differently over time. A later date feature should operate only
on an explicit timestamp in `data` or `settings`.
([`liquid-rust` date parsing source](https://github.com/cobalt-org/liquid-rust/blob/v0.26.11/crates/core/src/model/scalar/datetime.rs#L238-L240))

Do not claim Shopify Theme, Jekyll, or TRMNL compatibility. Liquid explicitly
has environment-specific variants, and `liquid-rust` supports application
defined filters, tags, and blocks. Any future WireTerm addition therefore
needs a named, versioned contract.
([Liquid variants](https://shopify.github.io/liquid/basics/variations/),
[`liquid-rust` customization](https://github.com/cobalt-org/liquid-rust#customizing-liquid))

## Rust Liquid choices

| Choice | Primary-source evidence | Decision |
| --- | --- | --- |
| `liquid` / `liquid-rust` | Upstream aims for strict Shopify-Liquid conformance. `ParserBuilder::new()` starts empty, while the builder accepts selected filters, tags, and blocks; `liquid::to_object` converts any Serde-serializable root object into the Liquid object model. ([upstream README](https://github.com/cobalt-org/liquid-rust#readme), [`ParserBuilder`](https://docs.rs/liquid/latest/liquid/struct.ParserBuilder.html), [`to_object`](https://docs.rs/liquid/latest/liquid/fn.to_object.html)) | **Use with a curated language.** It is the direct, mature native-Rust implementation and already matches the project's earlier Liquid direction. |
| `loose-liquid` | Its own README calls it a temporary fork of `liquid-rust` and says it will be deprecated when suitable upstream changes land. ([fork README](https://docs.rs/crate/loose-liquid/0.27.0/source/README.md)) | **Do not use by default.** Adopt it only if the proof identifies a required fix unavailable upstream. |
| `liquid-json` | It recursively applies Liquid to a JSON template and returns a JSON structure; its manifest depends on the temporary `loose-liquid` fork. ([crate docs](https://docs.rs/liquid-json/latest/liquid_json/), [manifest](https://docs.rs/crate/liquid-json/0.6.1/source/Cargo.toml.orig)) | **Do not use.** WireTerm needs JSON values as input and SVG text as output, the opposite boundary. |
| Tera, MiniJinja, Handlebars | Their own projects describe Jinja2-derived or Handlebars syntax, not Liquid. ([Tera](https://github.com/Keats/tera#readme), [MiniJinja](https://github.com/mitsuhiko/minijinja#readme), [Handlebars Rust](https://github.com/sunng87/handlebars-rust#readme)) | **Out of scope.** Choosing one would change the extension authoring contract rather than simplify this Liquid decision. |

## Rust SVG rasterizer choices

| Choice | Fit for this MVP |
| --- | --- |
| `usvg` + `resvg` + `tiny-skia` | **Best fit.** The project is written completely in Rust, requires no external native libraries, targets static SVG, separates parse/normalization from rendering, and renders to a caller-owned pixmap. Upstream documents a large SVG-to-PNG regression suite and identical pixels across supported platforms. ([resvg README](https://github.com/linebender/resvg#readme), [`usvg` processing model](https://docs.rs/usvg/latest/usvg/), [`tiny_skia::Pixmap`](https://docs.rs/tiny-skia/latest/tiny_skia/struct.Pixmap.html)) |
| `librsvg` Rust API | Mature, but it renders through Cairo and its build/runtime stack includes Cairo, Pango, FreeType, HarfBuzz, and fontconfig. Its CI documentation notes that text-reference results depend on those library versions. That defeats the requested pure-Rust, self-contained determinism boundary. ([librsvg Rust API](https://docs.rs/librsvg/latest/rsvg/), [librsvg dependencies](https://gnome.pages.gitlab.gnome.org/librsvg/devel-docs/compiling.html), [reference-image sensitivity](https://gnome.pages.gitlab.gnome.org/librsvg/devel-docs/ci.html)) |
| Vello / `vello_svg` | Pure-Rust direction, but Vello is a GPU-compute renderer, its repository describes non-trivial `wgpu` setup, and its status is alpha. GPU/WebGPU setup adds more portability and determinism risk than a CPU pixmap for one 800 x 480 frame. ([Vello README](https://github.com/linebender/vello#readme)) |
| `nsvg` | The Rust crate wraps the C NanoSVG implementation, so it is not pure Rust; NanoSVG also describes itself as a simple parser intended for simple icon-like SVGs. ([`nsvg` repository](https://github.com/nickbrowne/nsvg), [NanoSVG repository](https://github.com/memononen/nanosvg)) | **Reject.** It misses both the implementation-language requirement and the needed text/font confidence. |

The phrase "pure Rust" must not be read as "contains no `unsafe` anywhere."
The `resvg` README notes that some dependencies use `unsafe` and that font
memory mapping is inherently unsafe. Disable its `memmap-fonts` feature; the
project's own `unsafe_code = "forbid"` lint remains useful but does not apply
to third-party crates.
([resvg safety discussion](https://github.com/linebender/resvg#safety),
[`resvg` feature flags](https://docs.rs/crate/resvg/latest/features))

For the recommended 0.47.x integration, use `resvg` with default features
disabled and enable only `text` and `raster-images`; do not enable
`system-fonts`, `memmap-fonts`, or SVGZ. The custom resolver still restricts
the authoring contract to PNG even though the compiled raster-image feature
contains additional decoders.

## Contracted MVP surface

`usvg` supports far more than WireTerm should promise. It normalizes CSS and
presentation attributes, basic shapes, paths, `use`, nested SVG, references,
images, text and `tspan`, markers, clip paths, masks, patterns, gradients, and
filters. It also states that CSS support is minimal and that unsupported
features are ignored. Parse success is therefore not sufficient validation.
([`usvg` supported processing and limitations](https://docs.rs/usvg/latest/usvg/))

WireTerm should validate the source XML against this smaller profile before
passing it to `usvg`:

| Area | MVP support |
| --- | --- |
| Canvas | Exactly one root `<svg xmlns="http://www.w3.org/2000/svg" width="800" height="480" viewBox="0 0 800 480">`. After parsing, require `Tree::size()` to be exactly 800 by 480. ([`usvg::Tree::size`](https://docs.rs/usvg/latest/usvg/struct.Tree.html#method.size)) |
| Structure | `svg`, `g`, and `defs`; IDs and local fragment references; fixed transforms. |
| Geometry | `rect`, `circle`, `ellipse`, `line`, `polyline`, `polygon`, and `path`; presentation attributes for solid fill, stroke, stroke width, line joins/caps, and opacity. |
| Reuse and clipping | `use` and `clipPath`. These are useful for repeated rows and bounded artwork without opening the cost surface of arbitrary filters. |
| Text | `text` and `tspan` with explicit positions, family, size, weight/style, anchor, fill, and spacing. There is no automatic line layout or wrapping; the template must emit positioned lines. |
| Images | Static package-relative PNG only for the first contract. Explicit `x`, `y`, `width`, `height`, and `preserveAspectRatio`; no animation. Broader formats can be added later without changing the pipeline. |
| Styling | Presentation attributes and a small allowlist of inline `style` properties equivalent to them. Prefer attributes. No external stylesheet, web font, CSS layout, or selector-dependent authoring contract. |
| Liquid | The documented subset above, operating on `data` and `settings` namespaces. |

Defer gradients, masks, patterns, filters, markers, embedded SVG images, JPEG,
GIF, WebP, and `data:` assets until an extension needs them. `resvg` can render
many of these, but exposing them increases validation, performance, and
snapshot coverage without helping the smallest reference extension.

Reject `foreignObject`, HTML, links, scripts, event attributes, animation,
external styles, external `<use>`, remote URLs, and unknown elements or
attributes. `resvg` intentionally supports only static SVG and excludes
scripts, events, animation, and several interactive elements.
([resvg static-SVG limitations](https://github.com/linebender/resvg#svg-support))

### JSON-to-template surface

Build one explicit root object:

```json
{
  "data": {},
  "settings": {}
}
```

Both members must be JSON objects. Their nested values may be JSON strings,
finite numbers, booleans, null, arrays, or objects; `liquid::to_object`
provides the Serde conversion. Loop over arrays only. Do not make object member
iteration order part of the contract. Liquid's documented values include
strings, numbers, booleans, nil, and arrays; only `nil` and `false` are falsy,
so empty strings, empty arrays, and zero require explicit tests rather than a
bare truthiness check.
([Liquid types](https://shopify.github.io/liquid/basics/types/),
[truthiness](https://shopify.github.io/liquid/basics/truthy-and-falsy/))

Keep secrets outside both namespaces. Validate settings before exposing them,
and place stable caps on JSON depth, array length, string length, template
source size, and rendered SVG size. Liquid loops can multiply output even
though Liquid itself cannot access files, the network, or processes.

### Fonts

Create a new empty `fontdb::Database`, load the exact bundled font bytes with
`load_font_data`, set deterministic generic-family mappings, and attach that
database to `usvg::Options`. Do not call `load_system_fonts`. The database API
supports bytes, files, and directories; its system-font loading scans
platform-specific locations, which is precisely the ambient dependency this
pipeline should avoid.
([`fontdb::Database`](https://docs.rs/fontdb/latest/fontdb/struct.Database.html),
[`usvg::Options::fontdb`](https://docs.rs/usvg/latest/usvg/struct.Options.html))

The MVP should expose the five already reviewed Classic font files described
in [the bundled-font research](bundled-font-framework-contract.md), with one
documented default family and deterministic fallback order. An
extension-local font is usable only if its manifest declares the file and
license metadata and the host loads its validated bytes into that render's
font database. SVG `@font-face` and Windows-installed fonts are not part of
the contract.

Missing declared fonts, unsupported font files, and missing glyph coverage for
the proof text must be validation errors rather than silent system-font
fallback. Exact font files and hashes are render inputs and must be pinned.

### Local images

`usvg`'s default image resolver allows filesystem paths, including absolute
paths, while ignoring network URLs. It supports custom data and string
resolvers and documents relative resolution through `resources_dir`.
([`ImageHrefResolver`](https://docs.rs/usvg/latest/usvg/struct.ImageHrefResolver.html),
[`usvg::Options`](https://docs.rs/usvg/latest/usvg/struct.Options.html))

Do not use that default resolver for extensions. At package validation time,
normalize and validate every declared image path, reject absolute paths,
schemes, `..`, symlinks/reparse-point escapes, missing files, and oversized
files, then read approved assets into an immutable package snapshot. During
parse, the custom resolver maps only an exact normalized package-relative
identifier to those bytes. This removes ambient filesystem access, network
access, and read-after-validation races.

For the first contract, accept only PNG and verify its decoded dimensions and
pixel budget before SVG parsing. Dynamic data may select among manifest-
declared asset IDs, but it may not supply an arbitrary path or URL.

## Exact 800 x 480 and determinism

Allocate `tiny_skia::Pixmap::new(800, 480)` directly, fill it with opaque white,
and render the validated tree with a fixed identity root transform. A new
pixmap otherwise begins as transparent black. `resvg::render` writes into the
provided pixmap and documents sRGB output; `Pixmap` owns premultiplied RGBA
pixels and provides PNG encoding.
([`Pixmap::new`](https://docs.rs/tiny-skia/latest/tiny_skia/struct.Pixmap.html#method.new),
[`resvg::render`](https://docs.rs/resvg/latest/resvg/fn.render.html),
[`Pixmap::encode_png`](https://docs.rs/tiny-skia/latest/tiny_skia/struct.Pixmap.html#method.encode_png))

Define the deterministic contract at the pixel boundary:

> The same validated data, settings, template bytes, asset bytes, font bytes,
> WireTerm version, and locked Rust dependency graph produce the same
> 800 x 480 premultiplied RGBA pixel buffer.

This matches upstream's explicit cross-platform pixel reproducibility claim.
For the emitted artifact, encode non-interlaced RGBA8 with one explicit PNG
compression level and scanline filter, and emit no time or text metadata. The
Rust `png` encoder exposes those controls.
([`png::Encoder`](https://docs.rs/png/latest/png/struct.Encoder.html),
[`png::Compression`](https://docs.rs/png/latest/png/enum.Compression.html),
[`png::Filter`](https://docs.rs/png/latest/png/enum.Filter.html))

Pin that encoder in `Cargo.lock` and require both a raw-pixel hash and a
PNG-byte hash in the proof. Do not promise that compressed PNG bytes remain
identical across a future encoder upgrade; upstream `resvg` promises identical
pixels, not a stable PNG bitstream. Across dependency upgrades, the durable
golden is the raw pixel hash, and any encoded-byte change must be reviewed
deliberately.

## Smallest decision proof

Build one throwaway integration fixture, not production extension code:

1. Provide two small input variants over the same fixture. Variant A sets
   `show_image: true` and selects presentation branch A; variant B sets
   `show_image: false` and selects branch B. Both contain:
   - a title containing `&`, `<`, `>`, single quote, and double quote;
   - two ordered row objects with visibly different labels and values; and
   - otherwise identical deterministic values.
2. Provide one Liquid-authored SVG with the exact root canvas. It must:
   - insert the title into `<text>` through `escape`;
   - use `if` to emit one package-relative `<image>`;
   - use `for` to emit exactly two positioned row groups; and
   - use a settings conditional to change one visible shape.
3. Provide one tiny, distinctive PNG and one exact reviewed bundled font file.
   Start from an empty font database, load that font from bytes, and never call
   the system-font loader.
4. Render both variants and assert the escaped title and two rows exist in
   each SVG string. Assert that variant A contains the image and branch-A
   markup while variant B omits the image and contains branch-B markup. Parse
   both with the locked-down image resolver; assert each parsed tree is
   exactly 800 x 480 and retains the expected text and image nodes.
5. Fill an 800 x 480 pixmap white for each variant, render with a fixed
   transform, encode PNG, decode it, and assert:
   - width is 800 and height is 480;
   - each raw premultiplied RGBA SHA-256 matches its committed golden;
   - two consecutive renders of variant A have identical raw pixels and
     identical PNG bytes under the locked build; and
   - a few semantic pixels in each golden prove that the bundled-font text,
     local image, two loop rows, and selected conditional branch were actually
     painted, so a blank or partially ignored render cannot pass.
6. Run the same fixture on Windows x64 and one second CI platform, comparing
   the raw pixel hash. This is the smallest direct check of the portability
   claim; the Windows result remains the product acceptance result.
7. Add four negative cases to the same test:
   - missing bundled font;
   - missing image and `../`, absolute-path, URL, and `data:` image references;
   - unescaped XML-sensitive text; and
   - one disallowed/unknown SVG element and one unregistered Liquid filter
     such as `date`.

The decision is validated if this single fixture produces the expected exact
pixel hash on both runners without WebView2 or system fonts. Visual inspection
is useful during fixture creation, but it is not the acceptance oracle.

## Remaining risks

- **Authoring ergonomics:** fixed-coordinate SVG is substantially smaller than
  HTML/CSS, but authors must handle text wrapping, truncation, and placement.
  The reference extension should establish reusable SVG/Liquid snippets only
  after the boundary proof.
- **Silent feature loss:** `usvg` ignores unsupported features. A source-level
  allowlist and negative tests are required before treating parse success as
  validation.
- **Fonts and glyphs:** missing glyph coverage, font fallback, or changed font
  bytes change measurement and pixels. Pin files, define fallback, and fail
  missing proof glyphs.
- **Resource safety:** the default image resolver is too permissive. Snapshot
  assets into memory, cap decoded dimensions, and do not accept arbitrary
  paths from JSON.
- **Work amplification:** bound JSON collections, template and SVG size,
  referenced image size, SVG node count, and total render time. Filters and
  deep SVG effects should not be added without cost tests.
- **Ambient time:** the full Liquid standard library includes date behavior
  that recognizes `"now"` and `"today"`. Keep the parser allowlisted and make
  explicit timestamps part of input data rather than reading the render clock.
- **XML injection:** `escape` is adequate for ordinary text and quoted
  attribute values, not for markup, style fragments, paths, or URLs. Keep
  structural SVG in the template and validate typed dynamic values.
- **Dependency/MSRV choice:** keeping Rust 1.85 means pinning the older 0.45.1
  renderer family. Raising the MSRV permits current 0.47.x. Either choice must
  be locked in `Cargo.lock` and treated as a golden-render input.
- **Pure-Rust boundary:** this removes native runtime dependencies, but it is
  not a claim that every transitive crate is free of `unsafe`. Dependency
  review remains part of implementation.

## Context superseded by this decision

This recommendation applies the pivot recorded in
[Wayfinder map #7](https://github.com/CorVous/WireTerm/issues/7): local
extensions use fixed 800 x 480 Liquid-generated SVG and foreground-only
playback. It supersedes the WebView2/HTML/CSS renderer direction in
[the earlier rendering research](windows-extension-rendering-stack.md) for
the MVP only. It is also consistent with closing the tray-only capture
prototype after the product moved to a visible foreground window.
([issue #17 closure](https://github.com/CorVous/WireTerm/issues/17#issuecomment-5134911337))

It does not modify the Wayfinder map, close issue #19, or decide the extension
package schema. The package/schema ticket still owns the final manifest fields
for templates, assets, fonts, settings, and declared limits.
