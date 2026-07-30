# WireTerm ESP32 display bridge

This is the maintained offline receiver for an ESP32 DevKit V1 and Waveshare
7.5-inch e-Paper (B) V3 on a Driver HAT Rev2.3. Waveshare documents V3 as
hardware/interface compatible with V2, so the panel sequence is based on the
official `epd7in5b_V2` full-refresh driver family.

The firmware accepts complete 800 × 480 two-plane frames over USB serial.
It buffers all 96,000 bytes, verifies CRC32, and only then performs a full
refresh. Wi-Fi and Bluetooth are explicitly disabled.

## Wiring

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

## Build and flash

```text
cd firmware/esp32_epaper_receiver
pio run
pio run -t upload --upload-port COM9
```

`pio run` only compiles. Uploading and hardware diagnostics are intentional
operator actions; the host application never invokes them.

## WireTerm/1 serial contract

USB serial is 115200 baud, newline-delimited ASCII, with a 127-byte line limit:

```text
HELLO WIRETERM/1
STATUS
PINS
ABORT
TEST BWR
BEGIN 800 480 BWR 96000 <CRC32_HEX>
```

After a valid `BEGIN`, the device replies `OK BEGIN READY bytes=96000`. The host
then sends exactly 48,000 black-plane bytes followed by 48,000 red-plane bytes.
CRC32 uses the standard IEEE polynomial over the complete 96,000-byte payload.
The device rejects an invalid contract, receive timeout, or CRC mismatch without
touching the display.

`TEST BWR` remains available as a diagnostic. Both test and frame rendering use
the official `epd7in5b_V2` initialization and full-refresh command sequence,
apply a 45-second BUSY timeout, enter panel deep sleep, and then drive PWR low.

## Safety assumptions

- PWR is treated as active-high following the Waveshare driver convention.
- CS is held high and panel PWR low whenever the panel is inactive.
- Rendering uses full refresh only; partial refresh remains unsupported.
- A complete frame must pass its length and CRC checks before panel power is
  enabled.

References: [Waveshare 7.5-inch e-Paper HAT (B)
manual](https://www.waveshare.com/wiki/7.5inch_e-Paper_HAT_%28B%29_Manual)
and [official Waveshare e-Paper
repository](https://github.com/waveshareteam/e-Paper).
