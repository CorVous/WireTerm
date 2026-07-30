# TRMNL text and image dithering behavior

Status: decision-ready research for [issue #20](https://github.com/CorVous/WireTerm/issues/20)  
Evidence checked: 2026-07-30

## Decision

Keep full-frame Floyd–Steinberg as WireTerm's baseline for arbitrary-color BWR
art: it closely matches the color converter in TRMNL's official open-source
BYOS server. Do not switch the complete frame to TRMNL's non-dithered **Text**
path as a supposed BWR fix. In Terminus, that path is monochrome and would
discard red.

TRMNL's strongest applicable lesson is to protect text before or around palette
reduction. Its Framework uses native-size pixel fonts, pixel-grid alignment,
palette-aware styling, and designed patterns to avoid sub-pixel/anti-aliasing
artifacts. Dithering is explicitly selected for images/art, while exact text
treatment happens upstream of final image conversion.

The smallest safe next step is a prototype-only A/B panel test, not a production
change:

1. Build one exact 800 × 480 test card containing native-palette fills, one-pixel
   rules, black and red small text, antialiased Inter text, and one photograph.
2. Produce the current full-frame BWR Floyd–Steinberg result.
3. Produce one hybrid result: Floyd–Steinberg-dither only the photographic
   region, then composite text and line art rendered at final size directly in
   exact black/white/red palette pixels.
4. Compare both at 1:1 and on the physical panel, recording small-text stroke
   continuity, false red/black edge pixels, flat-color edges, and photo quality.

If the hybrid is materially clearer without damaging the image region, record a
later host-renderer requirement to preserve exact-palette text/line-art layers
and reserve error diffusion for continuous-tone imagery. The experiment should
not modify production code or yet define how a general flattened raster would
be segmented.

## Direct evidence

### TRMNL publishes separate conversion and dithering recipes

TRMNL's official ImageMagick guide presents two different 1-bit PNG commands:
plain conversion uses `-monochrome -colors 2`, while dithering uses
`-dither FloydSteinberg -remap pattern:gray50`. It does not describe dithering
as mandatory. The same guide requires BMP3 output to be 800 × 480, 1-bit, and
two-color, with the example's 48,062-byte result matching exactly. It also
documents an experimental 2-bit PNG route that remaps into an explicit
four-entry grayscale palette.
([ImageMagick guide](https://docs.trmnl.com/go/diy/imagemagick-guide))

For images passed directly through the Alias plugin, TRMNL requires an exact
800 × 480 image in 1-bit BMP3, 1-bit PNG, or (on suitable OG firmware) 2-bit
PNG. This is a delivery contract for already prepared images, not a promise
that the device will repair arbitrary full-color input.
([Alias plugin requirements](https://help.trmnl.com/en/articles/10701448-alias-plugin))

The public Palettes API currently defines `color-3bwr` as exactly
`#000000`, `#FF0000`, and `#FFFFFF`, with grayscale bit depth 1.
([Palettes API](https://trmnl.com/api/palettes))

### The official BYOS reference separates text and art

TRMNL describes Terminus as its flagship open-source BYOS application. In the
reviewed Terminus commit, the extension UI calls the default mode **Text** and
the alternative **Art**, with Art intended for images or mixtures of images,
gradients, and text.
([mode UI at `06e1819`](https://github.com/usetrmnl/terminus/blob/06e1819b572bf223b65fce901218bd62956bdd6f/app/templates/extensions/shared/popovers/_mode.html.erb#L1-L7))

The corresponding monochrome converter routes a 1-bit dither-mode frame through
Floyd–Steinberg plus a two-color remap. Without dither mode it uses
ImageMagick's `monochrome` operation instead. For 2–8-bit non-dithered output it
explicitly sets dithering to `None` before posterization.
([monochrome converter](https://github.com/usetrmnl/terminus/blob/06e1819b572bf223b65fce901218bd62956bdd6f/app/aspects/screens/converters/monochrome.rb#L20-L64))

Terminus's color converter does use Floyd–Steinberg. It resizes to the target
dimensions, normalizes, increases brightness/saturation with
`modulate "110,150"`, builds a palette from the model's color codes, and remaps
through that palette.
([color converter](https://github.com/usetrmnl/terminus/blob/06e1819b572bf223b65fce901218bd62956bdd6f/app/aspects/screens/converters/color.rb#L20-L46))

That color path is available only when the mold is both in dither mode and has
model color codes. Otherwise the top-level converter chooses monochrome. Thus
Terminus's non-dithered Text behavior is not evidence for a non-dithered BWR
algorithm; it is a monochrome alternative.
([color predicate](https://github.com/usetrmnl/terminus/blob/06e1819b572bf223b65fce901218bd62956bdd6f/app/aspects/screens/mold.rb#L34-L40),
[converter selection](https://github.com/usetrmnl/terminus/blob/06e1819b572bf223b65fce901218bd62956bdd6f/app/aspects/screens/converter.rb#L6-L10))

Terminus's API documentation says dither mode is useful for photos or images
with very little text. It also permits a `preprocessed` image, in which case the
server assumes the image already matches the device's grayscale, bit depth,
color depth, dimensions, and other requirements and does not convert it.
([Terminus API documentation](https://github.com/usetrmnl/terminus/blob/06e1819b572bf223b65fce901218bd62956bdd6f/doc/api.adoc#L1330-L1337),
[preprocessed image contract](https://github.com/usetrmnl/terminus/blob/06e1819b572bf223b65fce901218bd62956bdd6f/doc/api.adoc#L1573-L1581))

These facts establish the behavior of TRMNL's official open-source BYOS
reference. Terminus itself says it aims for Core compatibility while also
behaving differently, so these facts do not establish that the proprietary
hosted TRMNL service uses identical commands.
([Terminus compatibility scope](https://github.com/usetrmnl/terminus/blob/06e1819b572bf223b65fce901218bd62956bdd6f/README.adoc#L43-L45))

### TRMNL treats text as a pixel-grid problem, not only a palette problem

TRMNL's Pixel Perfect documentation says browser anti-aliasing creates
partially opaque edge pixels and that forcing those pixels into a 1-bit color
space can make text randomly bold, distorted, and difficult to read. Its
mitigation combines pixel fonts designed for particular pixel sizes with line
width adjustment that aligns text to the pixel grid.
([Pixel Perfect](https://trmnl.com/framework/docs/3.1/pixel_perfect))

The current Text Size documentation assigns low-density displays pixel-font
families for the three smallest sizes: NicoPups/TRMNL12, NicoClean/TRMNL16, and
BlockKie/TRMNL21. It uses Inter Variable for larger sizes and says the pixel
fonts render correctly only at their native sizes.
([Text Size](https://trmnl.com/framework/docs/3.1/text_size))

TRMNL also documents dithering as a scoped presentation choice. Framework
images opt into it with `image-dither`, while grayscale text shades use
carefully designed bitmap patterns rather than relying solely on a final
full-frame error-diffusion pass.
([Image](https://trmnl.com/framework/docs/3.1/image),
[Text Color](https://trmnl.com/framework/docs/3.1/text_color))

Framework v3 maps requested colors to a device's supported palette and
documents closest-hue mapping plus generated color-pattern images for
limited-palette devices. This is palette-aware authoring behavior; the docs do
not specify the final screenshot-to-panel quantizer or its distance metric.
([Framework v3 overview](https://trmnl.com/framework/docs/3.1/v3_overview))

TRMNL's palette guidance directly acknowledges that more gray levels allow
jagged font edges to be anti-aliased into smoother curves. A three-color BWR
palette exposes only one-bit grayscale according to the Palettes API, so it
does not have those intermediate gray levels.
([Understanding Color Palettes](https://help.trmnl.com/en/articles/12985974-understanding-color-palettes),
[Palettes API](https://trmnl.com/api/palettes))

### Upload behavior depends on the entry point

TRMNL's Image Display plugin accepts any source size or format, cover-fits and
center-crops mismatched aspect ratios, and recommends sources at or above the
target slot's native dimensions for crisp dithering.
([Image Display](https://help.trmnl.com/en/articles/11479051-image-display))

Webhook Image accepts PNG, JPEG, and BMP up to 5 MB, but performs passthrough
storage without processing. Its documentation recommends matching the OG's
800 × 480 dimensions and warns that an image may not display unless it already
meets the device's needs.
([Webhook Image](https://help.trmnl.com/en/articles/13213669-webhook-image))

Together with the stricter Alias contract, these routes show that “TRMNL accepts
an image” is not one conversion guarantee: some entry points preprocess, while
others require or strongly favor device-ready input.

## Comparison with WireTerm's prototype

WireTerm's current prototype:

- decodes the input to RGB;
- resizes with Lanczos3 while preserving aspect ratio;
- selects among black, white, and `#CD2323` using a weighted RGB distance; and
- applies Floyd–Steinberg error diffusion to every pixel in the frame.

The implementation is visible in
[`prepare_frame`](https://github.com/CorVous/WireTerm/blob/fa593c6bb2e9c0a574701243a0a8bd518545d2b6/src/bin/gui_prototype.rs#L264-L350).

| Concern | WireTerm prototype | Primary-source TRMNL evidence |
| --- | --- | --- |
| Dither scope | Always full-frame | Terminus's BWR-capable color path is full-frame Floyd–Steinberg; its non-dithered Text path is monochrome; Framework scopes authoring-time image dithering explicitly |
| Text treatment | Receives already-rasterized RGB pixels; no text-specific handling | Pixel fonts, native pixel sizes, pixel-grid alignment, and a non-error-diffused BYOS text conversion |
| Palette reduction | Weighted nearest color plus Floyd–Steinberg to black/white/`#CD2323` | Public BWR palette is black/white/`#FF0000`; Terminus color conversion normalizes/modulates then Floyd–Steinberg-remaps |
| Input sizing | Aspect-fit Lanczos3 resize, then synthesized side/top/bottom fill | Direct-delivery docs require an already prepared exact 800 × 480 image; Terminus unprocessed conversion force-resizes to model dimensions |
| Already prepared input | Always reconverts | Terminus can bypass conversion for preprocessed, device-compatible images |

## Inference

The most likely explanation for WireTerm's jagged text is that anti-aliased or
resampled edge pixels enter a full-frame error-diffusion pass. Their error is
then distributed into neighboring pixels, producing edge noise that is useful
for tone reproduction in photographs but harmful to glyph stroke continuity.
TRMNL's Pixel Perfect explanation supports the anti-aliasing part of this
diagnosis. Terminus's BWR-capable converter nevertheless applies global
Floyd–Steinberg, so the primary sources point more strongly to upstream
pixel-font/grid/palette treatment than to a hidden text-aware color quantizer.
No reviewed primary source directly tests WireTerm's rasterizer, `#CD2323`
palette, weighted distance, or Waveshare BWR panel, so this remains an inference
until the proposed A/B panel test.

TRMNL's use of `#FF0000` in its logical palette does not prove that WireTerm
should replace `#CD2323`. A logical authoring color and a panel's measured
pigment appearance are different concerns, and the official sources reviewed
do not publish a calibrated BWR distance function for WireTerm's panel.

## What primary sources do not establish

- The exact conversion pipeline used by TRMNL's proprietary hosted service.
- A documented hosted-service threshold, gamma transform, color-distance
  metric, or error-diffusion boundary rule.
- Whether hosted TRMNL performs content-aware segmentation between text and
  photographs after a page has been flattened.
- A red-specific font rasterization or anti-aliasing policy for three-color BWR
  output.
- That TRMNL's OG 1-bit typography choices transfer unchanged to WireTerm's
  Waveshare black/white/red panel.
- Whether exact-palette upstream rendering, post-dither compositing, or another
  approach will be best for WireTerm's exact SVG rasterizer and font bundle.

Those gaps are why the safe conclusion is a small controlled experiment that
keeps the documented color baseline and changes only text/line-art treatment,
not an attempt to clone an undocumented hosted TRMNL pipeline.
