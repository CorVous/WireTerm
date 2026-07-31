# WireTerm

WireTerm is a portable Windows 11 application for preparing and sending
800 × 480 black/white/red frames to an offline ESP32 e-paper display bridge
over USB serial.

The application is one visible Playlist editor and player. It runs only while
its foreground window is open and owns one active local Playlist. It does not
install a tray process, background player, startup task, updater, or service.

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
machine-wide configuration is required. WireTerm stores immutable, atomic
Playlist revisions under the adjacent `wireterm-data/playlist-revisions`
folder so the executable and state remain portable together.

## Playlist and playback

- The Playlist default Item Interval is 15 minutes. A blank Item Interval
  inherits that default; explicit values accept 1–1,440 whole minutes.
- Built-in items select one PNG/JPEG or one folder's direct PNG/JPEG children.
  Folder items rescan at each Playback Turn and use a session-only shuffle bag.
  Preview does not consume the bag.
- Playback timing is start-to-start. Rendering, transfer, and panel refresh
  consume the interval; an overrun advances immediately after the send ends.
- Saved edits are adopted at Playback Turn boundaries. Playback cursor, pause,
  timers, errors, and interrupted turns are never persisted.
- Item failures are logged and skipped without replacing the current panel
  Frame. Disconnects retain the current item in memory and retry it as a fresh
  turn after reconnect.
- Start/Pause, Next, and local Refresh-preview controls are session-only.

## Maintained host boundaries

- `frame::PanelFrame` is the only display-ready 800 × 480 B/W/red frame type.
  It owns the validated black and red planes and an RGB preview.
- `raster` owns proportional contain scaling, edge-matched letterboxing, and
  Floyd–Steinberg palette conversion for built-in raster items.
- `transport` owns serial discovery and the WireTerm/1 handshake, CRC header,
  full-frame transfer, verification, and display completion contract.
- `host::HostBridge` is the single serial owner. It accepts prepared frames,
  never source images.
- `playlist` owns persisted revisions and built-in image/folder selection.
- `playback` is a pure session state machine; it never owns serial transport.
- `extension` owns the sandboxed Lua contract and pure-Rust SVG conversion.
- `app` is the visible egui/eframe Playlist editor and bridge orchestrator.
  Closing the window ends playback and the process.

## Extensions

An Extension folder contains one `extension.lua` and optional relative local
assets. The script returns a table with:

- `metadata`: lowercase `id`, display `name`, positive `version`, and optional
  `description`;
- `inputs`: the script-defined text, number, checkbox, choice, or named-secret
  settings shown by the editor;
- `render(context)`: a function that returns valid fixed 800 × 480 SVG.

The Lua sandbox has no direct filesystem, process, environment, or network
access. Its narrow `wireterm` API provides bounded HTTP requests, opaque
named-secret bindings, clock reads, and validated relative asset paths.
Response bodies are arbitrary bytes and capped at 5 MiB; request timeouts cap
at 60 seconds, Lua execution caps at 30 seconds and 64 MiB, and returned SVG
caps at 2 MiB. Secret values never enter Lua, the Playlist, the UI, or errors.

This slice ships and tests the complete deterministic local-fixture host API.
Live HTTP execution and app-owned secret-value storage/injection are deliberate
remaining work; the GUI does not substitute unbounded networking or expose
secret values. Extensions that do not require HTTP can render locally now.

SVG is parsed and rendered through pure Rust. Vector/text composition is mapped
directly to black/white/red without whole-frame error diffusion. Relative
PNG/JPEG assets are first resized to their declared whole-pixel placement and
then Floyd–Steinberg dithered before composition. The resulting panel-palette
composition constructs `PanelFrame` directly.

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
cargo build --release
```
