---
status: accepted
---

# Render extensions through fixed SVG

Future extensions evaluate Liquid into a fixed 800 × 480 SVG and rasterize it with a pure-Rust renderer. Text and vector art map directly to the B/W/red panel palette and only raster image assets are dithered before composition, replacing the earlier WebView2/HTML capture direction and preventing full-frame dithering from degrading authored glyphs and line art.
