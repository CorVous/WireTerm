# WireTerm

WireTerm is a portable Windows 11 application for preparing and sending
800 × 480 black/white/red frames to an offline ESP32 e-paper display bridge
over USB serial.

The application is one Playlist editor and player process with a Windows
notification-area icon. Minimize keeps normal Windows taskbar behavior;
closing the editor hides it while Playlist playback continues. Use **Open
WireTerm** from the tray menu to restore the editor and **Quit WireTerm** to
stop playback and exit completely. WireTerm does not install a separate tray
process, second daemon, startup task, updater, or service.

## Run

Install the stable Rust toolchain, then:

```text
cargo run
```

For the unsigned Windows 11 portable ZIP:

```text
powershell -ExecutionPolicy Bypass -File scripts/package-portable.ps1
```

Extract the ZIP and run `wireterm.exe`. Windows may show a SmartScreen warning
because the MVP archive is unsigned. There is no installer or machine-wide
configuration. WireTerm stores immutable, atomic Playlist revisions under the
adjacent `wireterm-data` folder, so deleting the extracted
folder removes the executable and all WireTerm-owned state.

On a fresh extraction with no Playlist revision, WireTerm creates revision 1
with one enabled **Red image collection** folder Item. It uses the normal
15-minute default interval and session-reset, non-recursive shuffle bag over
the bundled `wireterm-data/images/default-playlist` JPEGs. Existing Playlist
revisions always win, so updating package files does not replace user edits.
Image credits and the supplied-metadata caveat are in
[`docs/default-playlist-attribution.md`](docs/default-playlist-attribution.md).

To update manually, choose **Quit WireTerm** from its tray menu and replace
`wireterm.exe` plus the shipped
documentation/font/example files while preserving `wireterm-data`. To remove
WireTerm, quit it and delete the extracted folder. There is no updater,
registry entry, startup task, service, separately installed tray process, or
OS-owned credential record to clean up.

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
- `app` is the egui/eframe Playlist editor and bridge orchestrator. Hiding the
  editor preserves the single process, Playlist session, and Host bridge;
  explicit tray Quit ends them.

## Extensions

An Extension folder contains one `extension.lua` and optional relative local
assets. The script returns a table with:

- `metadata`: lowercase `id`, display `name`, positive `version`, and optional
  `description`;
- `inputs`: the script-defined text, number, checkbox, choice, or masked secret
  settings shown by the editor;
- `render(context)`: a function that returns valid fixed 800 × 480 SVG.

The Lua sandbox allowlists only coroutine/table/string/UTF-8/math helpers and
has no direct filesystem, process, environment, package, or network access.
Its narrow `wireterm` API provides bounded live HTTP requests, clock reads,
and validated relative asset paths.
Response bodies are arbitrary bytes and capped at 5 MiB; request timeouts cap
at 60 seconds, Lua execution caps at 30 seconds and 64 MiB, and returned SVG
caps at 2 MiB. Secret inputs enter Lua like other Extension settings and are
masked in the editor; WireTerm does not include their values in its errors.

The host app performs HTTP on its bounded render worker, defaults to
denying redirects, and retains normal TLS certificate and hostname validation.
MVP secret inputs are stored locally without encryption in their Extension
Playlist Item revisions. Protect access to the portable folder accordingly.
The editor discovers and scaffolds Extensions under
`wireterm-data/extensions`; see
[`docs/extension-author-guide.md`](docs/extension-author-guide.md) and the
shipped [`examples/http-extension`](examples/http-extension).

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

The current receiver identifies itself with a required token-safe product field:

```text
OK WIRETERM/1 state=READY render=FULL_FRAME product=WireTerm%20USB%20Device
```

The host strictly percent-decodes and validates this bounded ASCII field before
discovering the receiver. It presents the decoded product name together with the
current port, for example `WireTerm USB Device · COM9`. Firmware without a valid
product field is not a current WireTerm/1 receiver.

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
powershell -ExecutionPolicy Bypass -File scripts/package-portable.ps1 -SkipBuild
```
