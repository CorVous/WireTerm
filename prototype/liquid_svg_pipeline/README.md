# DISPOSABLE PROTOTYPE — Liquid-to-SVG pipeline

This is throwaway evidence for one question:

> Can WireTerm take bounded JSON data and settings, render a deliberately
> narrow Liquid-authored 800 × 480 SVG, resolve declared fonts and PNG assets
> only from in-memory bytes, rasterize it without WebView2 or system fonts, and
> emit observable, repeatable PNG output? Does that output remain meaningful
> after WireTerm's existing black/white/red frame preparation and dithering?

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
- The generated PNG is decoded and passed through a disposable mirror of the
  current `src/bin/gui_prototype.rs::prepare_frame` routine: aspect-fit
  Lanczos3 resize, edge-color letterbox fill, weighted RGB nearest-palette
  classification, Floyd–Steinberg error diffusion, and packed black/red
  planes.
- The prototype contains no serial dependency or hardware-send path. It
  encodes the final palette pixels as an RGB preview PNG instead.

## Fixtures

- `variant-a.json`: escaped punctuation, two loop rows, branch A, image present.
- `variant-b.json`: escaped punctuation, two loop rows, branch B, image absent.

Both use the same Liquid template, PNG asset, and bundled Inter font declared
by `extension.json`.

## What this can and cannot prove

The prototype proves on the current host that both branches produce meaningful
800 × 480 output, bundled-font text is retained and painted, the loop paints
two rows, the local image can be present or absent, output is opaque, and both
generated PNGs become non-empty, non-overlapping 96,000-byte black/red frame
payloads. Both preview PNGs decode as exactly 800 × 480. Two runs of variant A
produce identical render, frame, and preview bytes.

It does **not** prove cross-platform hashes, stability across future Rust or
dependency upgrades, complete SVG/font coverage, hostile-package sandboxing,
production resource limits, or a shared production boundary: the private frame
routine is mirrored, so it can drift until production design work exposes a
reusable function. The preview is a digital palette visualization, not a proof
of physical e-paper appearance; it cannot represent panel pigment, ghosting,
refresh behavior, temperature effects, calibration, or transfer correctness.
No bytes are sent to hardware.

## Observed verdict

On the Windows x64 prototype host, debug and release builds produced the same
render, frame, and preview results. The frame follow-up produced these
exact-size previews:

| Fixture | Non-white pixels | Raw RGBA SHA-256 | PNG SHA-256 |
| --- | ---: | --- | --- |
| A | 34,119 | `bb67e53b58ba85d4f1376fa630f581243f392745aaa9029b882fb7708f1cb541` | `e14e2b2676d40967e18ed8833900dca3d920c7777707ae9fdd829652fe1c3739` |
| B | 26,908 | `a87ce35bde8ae077ee6146d7d0ec3b7f4b9ef4663c3b92e1037f8cef0f12b8c6` | `b1cd894647ff12f56181e9ebd46b99cb34a60429a29027d06fa278219c66a423` |

| Fixture | Black / white / red pixels | Frame SHA-256 | Preview PNG SHA-256 |
| --- | ---: | --- | --- |
| A | 17,774 / 359,278 / 6,948 | `1e66db5dc8e1590cad197225e9182ae6a49225c6f851dadf8f89b0d022666787` | `28beed49bcbab97d9a15231d0b8a1b5158736fa0ef73b3aa64bbd65ada79775f` |
| B | 19,388 / 362,080 / 2,532 | `ac656ce2a6f661dfba7e200eb1e3ff8b07994e384da04189454bf185d7417f04` | `087e22b8b66e81f49f2f1074c44e0c175d648f7dc7349f6e4f285cee72447831` |

Visual inspection found both previews legible, kept variant A's recognizable
image and variant B's empty image region, and preserved their intended branch
and loop content. Floyd–Steinberg diffusion visibly introduces red/black
speckling around antialiased black text, borders, and pale rules. That behavior
matches the current RGB nearest-palette algorithm, but it is a display-quality
risk worth evaluating on hardware later.

The command asserts these hashes as disposable prototype goldens and repeats
variant A byte-for-byte. This answers the local feasibility question: the
narrow pipeline remains meaningful after current frame preparation and emits
exact-size, repeatable previews without a browser, system fonts, or hardware
I/O.
