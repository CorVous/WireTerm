# DISPOSABLE PROTOTYPE — Liquid-to-SVG pipeline

This is throwaway evidence for one question:

> Can WireTerm take bounded JSON data and settings, render a deliberately
> narrow Liquid-authored 800 × 480 SVG, resolve declared fonts and PNG assets
> only from in-memory bytes, rasterize it without WebView2 or system fonts, and
> emit observable, repeatable PNG output? Can text and intentional SVG artwork
> map directly to the panel palette while dithering only raster-image assets?

It is deliberately isolated from the production crate. Nothing under this
directory is imported by WireTerm's existing binaries or library.

## Run

From the repository root:

```powershell
& "$env:USERPROFILE\.cargo\bin\cargo.exe" run --locked --manifest-path prototype/liquid_svg_pipeline/Cargo.toml
```

The command renders both fixtures, prints their dimensions, semantic pixel
checks, hashes, frame palette counts, and packed frame size, then writes
observable SVG, source PNG, and e-paper-style preview PNG artifacts beneath
`prototype/liquid_svg_pipeline/output/`.

## Deliberate boundary

- Liquid is a generic text-template step. It receives only `{data, settings}`
  and has only `for`, `if`, and `escape` registered.
- The rendered SVG is validated against a small allowlist before parsing.
- The SVG root must be exactly `width="800" height="480"
  viewBox="0 0 800 480"`.
- Only static positioned shapes, `text`/`tspan`, simple `clipPath`/`use`, and
  the declared `assets/logo.png` image are accepted.
- The font database starts empty and receives only the declared bundled font
  bytes. System-font and font-memory-map features are disabled.
- The image resolver sees only preloaded bytes keyed by the declared
  extension-relative path. It rejects data URLs, remote URLs, absolute paths,
  and undeclared paths.
- PNG encoding is explicit: RGBA8, non-interlaced, fixed compression and
  filter, with no optional metadata.
- SVG `fill` and `stroke` values are restricted to exact black, white, panel
  red, or `none`. Root `shape-rendering="crispEdges"` and
  `text-rendering="optimizeSpeed"` make vector edges and bundled-font glyphs
  one-bit rather than antialiased.
- The declared raster asset is resized to its fixed 110 × 66 painted size,
  palette-reduced with the current weighted RGB choice and Floyd–Steinberg
  diffusion, explicitly PNG-encoded, then supplied to `usvg`'s in-memory image
  resolver. `image-rendering="optimizeSpeed"` avoids creating new colors during
  composition.
- The complete `resvg` render is accepted only if every one of its 384,000
  pixels is already exactly black, white, or panel red. Frame preparation then
  packs those colors directly into black/red planes with no full-frame
  quantization or dithering.
- The prototype contains no serial dependency or hardware-send path. It
  encodes the final palette pixels as an RGB preview PNG instead.

## Fixtures

- `variant-a.json`: escaped punctuation, two loop rows, branch A, image present.
- `variant-b.json`: escaped punctuation, two loop rows, branch B, image absent.

Both use the same Liquid template, PNG asset, and bundled Inter font declared
by `extension.json`.

## What this can and cannot prove

The prototype proves on the current host that palette-native crisp SVG text
and vectors can be composed with a separately dithered PNG asset into an exact
800 × 480 three-color image. Both fixtures become non-empty, non-overlapping
96,000-byte black/red frame payloads. Both preview PNGs decode as exactly
800 × 480, and two runs of variant A produce identical render, frame, and
preview bytes.

It does **not** prove cross-platform hashes, stability across future Rust or
dependency upgrades, complete SVG/font coverage, hostile-package sandboxing,
production resource limits, or a general layer compositor. `usvg` exposes
normalized text/path/image node kinds, but `resvg` does not expose a high-level
ordered-layer callback that can independently convert a painted image region
and then resume composition. Reassembling arbitrary nodes would require
reproducing ancestor transforms, paint order, clips, masks, opacity, filters,
and blending. Preprocessing declared image bytes is the smallest clean
boundary demonstrated here.

That boundary has costs: a raster used at multiple painted sizes needs a
preprocessed variant or cache entry for each size; cropped, transformed, or
partly transparent images need more policy; and crisp vector/text rendering
trades colored dither noise for ordinary one-bit stair-stepping. Opacity,
gradients, filters, and off-palette vector colors are excluded because they
would create intermediate colors. The preview is a digital palette
visualization, not proof of panel pigment, ghosting, refresh behavior,
temperature effects, calibration, or transfer correctness. No bytes are sent
to hardware.

## Observed verdict

On the Windows x64 prototype host, debug and release builds produced the same
layered render, frame, and preview results:

| Fixture | Black / white / red pixels | Frame SHA-256 | Preview PNG SHA-256 |
| --- | ---: | --- | --- |
| A, dithered image present | 20,384 / 358,659 / 4,957 | `96912953a4dcc0b82f566a9dbb513797a654a31d372dc37c9647932845b8ea55` | `a95b81a185ab6e23751337195daa400b7fe65362913dad68d09bfd1523c9abd7` |
| B, vector/text only | 22,447 / 361,329 / 224 | `18929ecc09631a78ccb2678502fc34d447a60c15e64f5c91ab93a8b6a12bd399` | `099494ee5b6f55855dd70f665fefb2dc18bab1ffc486599971a16cda3a3093e8` |

The 110 × 66 image asset contains 1,947 black, 4,571 white, and 742 red pixels
after preprocessing. Native-resolution inspection found solid, readable
black text and exact red/black/white shapes in both fixtures. The dithered
lighthouse remains recognizable. The same-sized no-dither comparison retains
the source image's smoother grays and muted colors—6,875 off-palette pixels in
the composed frame—which look better on a normal monitor but are not directly
packable for the panel.

For the all-dither baseline, the prototype disables crisp vector/text
rendering, composes the same resized source image, and runs Floyd–Steinberg
over the full 800 × 480 frame. Visual inspection shows the earlier red/black
text-edge noise: the title region contains 398 red pixels, versus zero in the
hybrid result. This isolates the improvement to the conversion boundary rather
than a different font or layout.

The command writes these explicit comparisons:

- `comparison-direct-palette-vectors-text.png`
- `comparison-layered-raster-dithered.png`
- `comparison-layered-raster-no-dither.png`
- `comparison-all-dither-baseline.png`
- `comparison-raster-asset-dithered.png`
- `comparison-raster-asset-no-dither.png`

The command asserts the layered hashes as disposable prototype goldens and
repeats variant A byte-for-byte. The verdict is yes for the narrow surface:
preprocessing extension-relative image assets at their declared painted size
is a practical layered boundary. General per-node layer extraction is not.
