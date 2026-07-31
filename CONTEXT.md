# WireTerm glossary

## Wired display bridge

The Windows-hosted system that renders and delivers display frames to the ESP32 over USB serial. It does not require a network connection.

## Frame

One display-ready image prepared for the connected e-paper panel and transferred to the display bridge.

## Panel frame contract

An 800 × 480 frame consists of a 48,000-byte black plane followed by a 48,000-byte red plane. In the black plane, zero bits produce black; in the red plane, one bits produce red; pixels absent from both planes remain white.

## Host renderer

The Windows application component that turns source content into a frame. Raster images are palette-dithered, while exact-palette vector and text content is composed directly.

## Host bridge

The single host-side owner of display-bridge discovery and serial transfer. Frame producers submit complete frames to it and do not access serial transport directly.

## Send-image workflow

The first host interaction: choose one image file, prepare it as a frame, and send it to the display bridge. Clipboard, automation, and template rendering are deferred.

## Host GUI

The visible foreground Windows application for selecting an image, previewing the prepared frame, choosing a serial device, and observing transfer progress. Closing the window ends the host process.

## Focused sender layout

The preferred WireTerm GUI direction is a deliberately simple sender: one selected image, one large panel preview, one visible device status, and one clear send action. Advanced settings remain out of the initial screen.

## Extension frame rendering

The host path that runs an extension script to produce fixed 800 × 480 SVG and rasterizes it in pure Rust. SVG text and vector art use exact panel colors; only raster image assets are dithered before composition.

## Display bridge firmware

The intentionally thin ESP32 firmware that receives validated frames over USB serial and drives the e-paper panel; it does not perform host-side rendering work.

## Receiver product name

The stable human-readable identity a display bridge supplies during WireTerm/1 discovery. The Host GUI presents it with the current serial port so the receiver remains recognizable when Windows assigns a different port.

## Reference firmware

The currently installed TRMNL firmware, retained only as a reference while WireTerm replaces it with custom offline serial firmware.

## Hardware-specific power control

This WireTerm setup uses ESP32 GPIO32 as the panel power-control connection. This project-specific wiring overrides generic Waveshare reference mappings.

## Hardware assembly

WireTerm uses a separate ESP32 DevKit V1 wired to a Waveshare e-Paper Driver HAT Rev2.3. The HAT header exposes PWR, BUSY, RST, DC, CS, CLK, DIN, GND, and VCC. It is not the integrated Waveshare ESP32 Driver Board.

## WireTerm HAT pin map

The e-Paper Driver HAT Rev2.3 maps to the ESP32 DevKit V1 as follows: `DIN` D14, `CLK` D13, `CS` D15, `DC` D27, `RST` D26, `BUSY` D25, `VCC` 3V3, and `GND` GND. Unlike the reference wiring diagram, this build connects `PWR` to D32 rather than D33.

## Full-frame refresh

The initial update model: one complete panel-ready frame is transferred and rendered for each update. Partial updates are outside the first version.

## Offline mode

The operating mode in which the ESP32's Wi-Fi and Bluetooth radios remain disabled; USB serial is the only host communication path.

## Extension

A reusable host-side content definition made of one self-describing Lua script plus relative local assets. The script exposes metadata, an input schema, and a render entry point, and an extension may be instantiated more than once with different settings.

## Extension library

The portable collection of Extension folders discoverable from the Playlist editor and available to instantiate as Playlist Items.

## Playlist

An ordered collection of playlist items that the host cycles through on the connected display.

## Playlist revision

One complete, valid saved state of the playlist. Playback adopts the latest playlist revision between playback turns; an active turn continues against the revision with which it began.

## Playlist item

One independently configured entry in a playlist. An item selects a content source, such as one image, a random image from a collection, or an extension instance, and produces the frame for its turn in the cycle.

## Playback turn

One playlist item's passage from the start of refresh until its send completes and its item interval has elapsed. A playback turn remains current while the host process is running, including across a display-bridge disconnect, until it completes or fails and is skipped. A new host process begins a new cycle rather than recovering an interrupted turn.

## Item interval

The minimum start-to-start time assigned to a playlist item. Its timer begins when the item's playback turn starts; refresh, rendering, transfer, and panel refresh all consume the interval. The next turn starts after both the interval has elapsed and the current send has completed.

## Extension capability

A bounded host operation available to an extension script, such as an HTTP request, named-secret binding, clock read, or relative local-asset lookup. Capabilities are mediated by WireTerm and granted independently to each playlist item.

## Named secret

A credential value stored and injected into extension-requested HTTP operations by WireTerm under a script-defined logical name. Extension scripts, response data, logs, errors, and the UI may use references but never receive the value.

## Extension host API

The narrow WireTerm-owned interface through which an extension script requests bounded HTTP, names credential bindings, reads the clock, and resolves relative local assets. The Lua runtime has no direct filesystem, process, environment, or network access.
