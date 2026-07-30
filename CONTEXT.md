# WireTerm glossary

## Wired display bridge

The Windows-hosted system that renders and delivers display frames to the ESP32 over USB serial. It does not require a network connection.

## Frame

One display-ready image prepared for the connected e-paper panel and transferred to the display bridge.

## Panel frame contract

An 800 × 480 frame consists of a 48,000-byte black plane followed by a 48,000-byte red plane. In the black plane, zero bits produce black; in the red plane, one bits produce red; pixels absent from both planes remain white.

## Host renderer

The Windows application that prepares a frame for the target panel, including color conversion and dithering, before serial transfer.

## Send-image workflow

The first host interaction: choose one image file, prepare it as a frame, and send it to the display bridge. Clipboard, automation, and template rendering are deferred.

## Host GUI

The intended first host interface is a Windows GUI for selecting an image, previewing the prepared frame, choosing a serial device, and observing transfer progress. The GUI toolkit is a short prototype decision, beginning with egui and eframe.

## Focused sender layout

The preferred WireTerm GUI direction is a deliberately simple sender: one selected image, one large panel preview, one visible device status, and one clear send action. Advanced settings remain out of the initial screen.

## Display bridge firmware

The intentionally thin ESP32 firmware that receives validated frames over USB serial and drives the e-paper panel; it does not perform host-side rendering work.

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

A reusable host-side content definition that produces display content from configured data and presentation logic. An extension may be instantiated more than once with different settings.

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

A network host, named secret, or transform-execution privilege declared by an extension and granted independently to each playlist item. Undeclared capabilities are unavailable, and declaration changes require renewed approval.

## Named secret

A credential value stored and injected into declared HTTP requests by WireTerm under an extension-defined name. Extension templates, transforms, response data, logs, and errors may reference the name but never receive the value.

## Transform

An optional extension-local Lua function that reshapes non-secret inputs and named HTTP responses before template rendering. WireTerm supplies the Lua runtime and a data-only API; transforms cannot directly access files, processes, the environment, or the network.
