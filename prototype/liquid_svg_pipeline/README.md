# DISPOSABLE PROTOTYPE — Liquid-to-SVG pipeline

This is throwaway evidence for one question:

> Can WireTerm take bounded JSON data and settings, render a deliberately
> narrow Liquid-authored 800 × 480 SVG, resolve declared fonts and PNG assets
> only from in-memory bytes, rasterize it without WebView2 or system fonts, and
> emit observable, repeatable PNG output?

It is deliberately isolated from the production crate. Nothing under this
directory is imported by WireTerm's existing binaries or library.

## Run

From the repository root:

```powershell
& "$env:USERPROFILE\.cargo\bin\cargo.exe" run --locked --manifest-path prototype/liquid_svg_pipeline/Cargo.toml
```

The command renders both fixtures, prints their dimensions, semantic pixel
checks, raw RGBA SHA-256, and PNG SHA-256, then writes observable SVG and PNG
artifacts beneath `prototype/liquid_svg_pipeline/output/`.

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

## Fixtures

- `variant-a.json`: escaped punctuation, two loop rows, branch A, image present.
- `variant-b.json`: escaped punctuation, two loop rows, branch B, image absent.

Both use the same Liquid template, PNG asset, and bundled Inter font declared
by `extension.json`.

## What this can and cannot prove

The prototype proves on the current host that both branches produce meaningful
800 × 480 output, bundled-font text is retained and painted, the loop paints
two rows, the local image can be present or absent, output is opaque, and two
renders of the same fixture have identical raw pixels and PNG bytes.

It does **not** prove cross-platform hashes, stability across future Rust or
dependency upgrades, complete SVG/font coverage, hostile-package sandboxing,
production resource limits, or integration with the playlist/frame pipeline.
Those require CI on a second platform, golden review, negative/security tests,
and production design work.

## Observed verdict

On the Windows x64 prototype host, debug and release builds produced the same
results, and a second render of variant A repeated byte-for-byte:

| Fixture | Non-white pixels | Raw RGBA SHA-256 | PNG SHA-256 |
| --- | ---: | --- | --- |
| A | 34,119 | `bb67e53b58ba85d4f1376fa630f581243f392745aaa9029b882fb7708f1cb541` | `e14e2b2676d40967e18ed8833900dca3d920c7777707ae9fdd829652fe1c3739` |
| B | 26,908 | `a87ce35bde8ae077ee6146d7d0ec3b7f4b9ef4663c3b92e1037f8cef0f12b8c6` | `b1cd894647ff12f56181e9ebd46b99cb34a60429a29027d06fa278219c66a423` |

The command asserts these hashes as committed prototype goldens. This answers
the local feasibility question: the narrow pipeline is practical and produces
meaningful, exact-size, repeatable output without a browser or system fonts.
