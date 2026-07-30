# Waveshare panel frame contract

## Confirmed hardware

WireTerm uses:

- ESP32 DevKit V1 through a CP210x USB serial bridge.
- Waveshare e-Paper Driver HAT Rev2.3.
- Waveshare 7.5-inch e-Paper (B) V3, identified by flex marking `FPC-8612`.
- 800 × 480 black/white/red panel using the V2-compatible `epd7in5b_V2`
  command family documented by Waveshare.

The confirmed HAT wiring is:

| HAT | ESP32 |
| --- | ---: |
| DIN | GPIO14 |
| CLK | GPIO13 |
| CS | GPIO15 |
| DC | GPIO27 |
| RST | GPIO26 |
| BUSY | GPIO25 |
| PWR | GPIO32 |
| VCC | 3V3 |
| GND | GND |

GPIO32 is specific to this physical build and replaces the GPIO33 power
connection shown in the user's reference wiring diagram.

## Validated plane contract

The physical black/red/white polarity test completed successfully on
2026-07-29. The display showed three vertical bands in the expected order:
**black | red | white**.

WireTerm therefore uses two row-major 1-bit planes:

| Property | Contract |
| --- | --- |
| Resolution | 800 × 480 |
| Bytes per row | 100 |
| Black plane length | 48,000 bytes |
| Red plane length | 48,000 bytes |
| Full payload | 96,000 bytes |
| Black plane | `0` bit = black; `1` bit = not black |
| Red plane | `1` bit = red; `0` bit = not red |
| White pixel | not black and not red |
| Plane order | black, then red |
| Refresh | full frame only |

The firmware sends the black plane after controller command `0x10`, the red
plane after `0x13`, and triggers refresh with `0x12`. BUSY on GPIO25 is
active-low and must return high before continuing.

## Ownership and safety

- The Windows host owns resize, colour conversion, dithering, and production of
  the two display-ready planes.
- Firmware receives and validates a complete frame before refreshing the panel.
- Wi-Fi and Bluetooth remain disabled.
- Panel power is enabled only during initialization and refresh. Firmware then
  enters panel deep sleep and drives GPIO32 low.
- Partial refresh remains outside the first version.

## Sources

- [Official Waveshare V3 specification](https://files.waveshare.com/upload/8/8c/7.5inch-e-paper-b-v3-specification.pdf)
- [Official Waveshare 7.5-inch (B) manual](https://www.waveshare.com/wiki/7.5inch_e-Paper_HAT_%28B%29_Manual)
- [Official Waveshare e-Paper repository](https://github.com/waveshareteam/e-Paper)
