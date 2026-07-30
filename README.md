# WireTerm

WireTerm is a portable Windows 11 application for preparing and sending
800 × 480 black/white/red frames to an offline ESP32 e-paper display bridge
over USB serial.

The current application is intentionally focused: choose one PNG or JPEG,
inspect the prepared frame, choose a serial device, and send it. WireTerm runs
only while its foreground window is open. It does not install a tray process,
background player, startup task, updater, or system service.

## Run

Install the stable Rust toolchain, then:

```text
cargo run
```

For a portable executable:

```text
cargo build --release
```

Copy `target/release/wireterm.exe` wherever it should run. No installer or
machine-wide configuration is required.

## Maintained host boundaries

- `frame::PanelFrame` is the only display-ready 800 × 480 B/W/red frame type.
  It owns the validated black and red planes and an RGB preview.
- `raster` owns the initial send-image workflow: proportional contain scaling,
  edge-matched letterboxing, and Floyd–Steinberg palette conversion.
- `transport` owns serial discovery and the WireTerm/1 handshake, CRC header,
  full-frame transfer, verification, and display completion contract.
- `host::HostBridge` is the single serial owner exposed to the GUI and future
  playback logic. It accepts prepared frames, never source images.
- `app` is the visible egui/eframe editor and sender. Closing the window ends
  the application.

Future extensions will render Liquid-authored, fixed 800 × 480 SVG through a
pure-Rust rasterizer. SVG text and vector paint must map directly to the panel
palette. Only embedded raster image assets are dithered before composition.
The resulting exact-palette composition constructs `PanelFrame` directly, so
it is not passed through the send-image raster dither a second time. Playlist
and extension behavior are not implemented yet.

## WireTerm/1

The host sends:

```text
HELLO WIRETERM/1
BEGIN 800 480 BWR 96000 <CRC32_HEX>
<48,000 black-plane bytes><48,000 red-plane bytes>
```

The display bridge accepts a frame only after the contract and complete
payload CRC verify. It then performs one full refresh, enters panel deep
sleep, powers the panel off, and reports completion. Wi-Fi and Bluetooth stay
disabled.

## Firmware

The maintained PlatformIO project is in
[`firmware/esp32_epaper_receiver`](firmware/esp32_epaper_receiver). A normal
`pio run` is build-only. Flashing and panel diagnostics must be initiated
explicitly by an operator.

## Development checks

```text
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```
