//! WireTerm/1 serial discovery and full-frame transfer.

#![allow(clippy::cast_precision_loss)]

use std::{
    io::{self, Read, Write},
    thread,
    time::Duration,
};

use thiserror::Error;

use crate::frame::{FRAME_BYTES, FRAME_HEIGHT, FRAME_WIDTH, PanelFrame};

const BAUD_RATE: u32 = 115_200;
const CHUNK_BYTES: usize = 1024;
const CP210X_VID: u16 = 0x10C4;
const CP210X_PID: u16 = 0xEA60;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceInfo {
    pub port_name: String,
    pub description: String,
    pub is_wireterm_candidate: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransferStage {
    Connecting,
    Handshaking,
    Sending { sent: usize, total: usize },
    Verified,
    Refreshing,
    Complete,
}

impl TransferStage {
    #[must_use]
    pub fn progress(&self) -> f32 {
        match self {
            Self::Connecting | Self::Handshaking => 0.0,
            Self::Sending { sent, total } => *sent as f32 / *total as f32 * 0.65,
            Self::Verified => 0.72,
            Self::Refreshing => 0.78,
            Self::Complete => 1.0,
        }
    }
}

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("could not enumerate serial devices: {0}")]
    Discovery(#[source] serialport::Error),
    #[error("could not open {port}: {source}")]
    Open {
        port: String,
        #[source]
        source: serialport::Error,
    },
    #[error("{operation}: {source}")]
    Serial {
        operation: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("{operation}: {source}")]
    PortConfiguration {
        operation: &'static str,
        #[source]
        source: serialport::Error,
    },
    #[error("unexpected device response during {phase}: {response}")]
    UnexpectedResponse {
        phase: &'static str,
        response: String,
    },
    #[error("device response exceeded 256 bytes")]
    ResponseTooLong,
}

pub fn discover_devices() -> Result<Vec<DeviceInfo>, TransportError> {
    let mut devices: Vec<_> = serialport::available_ports()
        .map_err(TransportError::Discovery)?
        .into_iter()
        .map(|port| {
            let (description, is_wireterm_candidate) = match port.port_type {
                serialport::SerialPortType::UsbPort(info) => {
                    let candidate = info.vid == CP210X_VID && info.pid == CP210X_PID;
                    let label = info
                        .product
                        .or(info.manufacturer)
                        .unwrap_or_else(|| format!("USB {:04X}:{:04X}", info.vid, info.pid));
                    (label, candidate)
                }
                serialport::SerialPortType::BluetoothPort => {
                    ("Bluetooth serial port".to_owned(), false)
                }
                serialport::SerialPortType::PciPort => ("PCI serial port".to_owned(), false),
                serialport::SerialPortType::Unknown => ("Serial port".to_owned(), false),
            };
            DeviceInfo {
                port_name: port.port_name,
                description,
                is_wireterm_candidate,
            }
        })
        .collect();
    devices.sort_by(|left, right| {
        right
            .is_wireterm_candidate
            .cmp(&left.is_wireterm_candidate)
            .then_with(|| left.port_name.cmp(&right.port_name))
    });
    Ok(devices)
}

pub fn send_frame(
    port_name: &str,
    frame: &PanelFrame,
    mut progress: impl FnMut(TransferStage),
) -> Result<(), TransportError> {
    progress(TransferStage::Connecting);
    let mut port = serialport::new(port_name, BAUD_RATE)
        .timeout(Duration::from_secs(3))
        .open()
        .map_err(|source| TransportError::Open {
            port: port_name.to_owned(),
            source,
        })?;

    // CP210x boards commonly reset when the serial port opens.
    thread::sleep(Duration::from_secs(2));
    port.clear(serialport::ClearBuffer::Input)
        .map_err(|source| TransportError::PortConfiguration {
            operation: "could not clear device input",
            source,
        })?;

    progress(TransferStage::Handshaking);
    handshake(&mut port)?;
    begin_and_write(&mut port, frame, &mut progress)?;

    port.set_timeout(Duration::from_secs(8)).map_err(|source| {
        TransportError::PortConfiguration {
            operation: "could not set verification timeout",
            source,
        }
    })?;
    expect_response(&mut port, "frame verification", "OK FRAME VERIFIED")?;
    progress(TransferStage::Verified);

    port.set_timeout(Duration::from_mins(1)).map_err(|source| {
        TransportError::PortConfiguration {
            operation: "could not set display timeout",
            source,
        }
    })?;
    progress(TransferStage::Refreshing);
    expect_response(&mut port, "panel refresh", "OK FRAME DISPLAYED")?;
    progress(TransferStage::Complete);
    Ok(())
}

fn handshake(stream: &mut (impl Read + Write)) -> Result<(), TransportError> {
    let mut last_response = String::new();
    for attempt in 0..3 {
        write_all(stream, b"HELLO WIRETERM/1\n", "could not write handshake")?;
        flush(stream, "could not flush handshake")?;
        last_response = read_response(stream)?;
        if last_response.starts_with("OK WIRETERM/1") {
            return Ok(());
        }
        if attempt < 2 {
            thread::sleep(Duration::from_millis(150));
        }
    }
    Err(TransportError::UnexpectedResponse {
        phase: "handshake",
        response: last_response,
    })
}

fn begin_and_write(
    stream: &mut (impl Read + Write),
    frame: &PanelFrame,
    progress: &mut impl FnMut(TransferStage),
) -> Result<(), TransportError> {
    let begin = format!(
        "BEGIN {FRAME_WIDTH} {FRAME_HEIGHT} BWR {FRAME_BYTES} {:08X}\n",
        frame.crc32()
    );
    write_all(stream, begin.as_bytes(), "could not write frame header")?;
    flush(stream, "could not flush frame header")?;
    expect_response(stream, "frame begin", "OK BEGIN READY")?;

    for (chunk_index, chunk) in frame.payload().chunks(CHUNK_BYTES).enumerate() {
        write_all(stream, chunk, "could not write frame payload")?;
        let sent = ((chunk_index + 1) * CHUNK_BYTES).min(frame.payload().len());
        progress(TransferStage::Sending {
            sent,
            total: frame.payload().len(),
        });
    }
    flush(stream, "could not flush frame payload")
}

fn expect_response(
    stream: &mut (impl Read + Write),
    phase: &'static str,
    prefix: &str,
) -> Result<(), TransportError> {
    let response = read_response(stream)?;
    if response.starts_with(prefix) {
        Ok(())
    } else {
        Err(TransportError::UnexpectedResponse { phase, response })
    }
}

fn write_all(
    stream: &mut impl Write,
    bytes: &[u8],
    operation: &'static str,
) -> Result<(), TransportError> {
    stream
        .write_all(bytes)
        .map_err(|source| TransportError::Serial { operation, source })
}

fn flush(stream: &mut impl Write, operation: &'static str) -> Result<(), TransportError> {
    stream
        .flush()
        .map_err(|source| TransportError::Serial { operation, source })
}

fn read_response(stream: &mut impl Read) -> Result<String, TransportError> {
    let mut line = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        stream
            .read_exact(&mut byte)
            .map_err(|source| TransportError::Serial {
                operation: "could not read device response",
                source,
            })?;
        if byte[0] == b'\n' {
            break;
        }
        if byte[0] != b'\r' {
            line.push(byte[0]);
        }
        if line.len() >= 256 {
            return Err(TransportError::ResponseTooLong);
        }
    }
    Ok(String::from_utf8_lossy(&line)
        .trim_matches(|character: char| character.is_control() || character == '\u{FFFD}')
        .to_owned())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::frame::{PIXEL_COUNT, PanelColor};

    struct ScriptedStream {
        responses: Cursor<Vec<u8>>,
        writes: Vec<u8>,
    }

    impl Read for ScriptedStream {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            self.responses.read(buffer)
        }
    }

    impl Write for ScriptedStream {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.writes.extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn protocol_writes_versioned_crc_frame() {
        let frame = PanelFrame::from_palette_pixels(&vec![PanelColor::White; PIXEL_COUNT])
            .expect("valid frame");
        let responses =
            b"OK WIRETERM/1 state=READY\nOK BEGIN READY\nOK FRAME VERIFIED\nOK FRAME DISPLAYED\n";
        let mut stream = ScriptedStream {
            responses: Cursor::new(responses.to_vec()),
            writes: Vec::new(),
        };
        let mut stages = Vec::new();

        handshake(&mut stream).expect("handshake");
        begin_and_write(&mut stream, &frame, &mut |stage| stages.push(stage)).expect("frame write");
        expect_response(&mut stream, "frame verification", "OK FRAME VERIFIED")
            .expect("verification");
        expect_response(&mut stream, "panel refresh", "OK FRAME DISPLAYED").expect("display");

        let expected_header = format!(
            "HELLO WIRETERM/1\nBEGIN 800 480 BWR 96000 {:08X}\n",
            frame.crc32()
        );
        assert!(stream.writes.starts_with(expected_header.as_bytes()));
        assert_eq!(
            &stream.writes[expected_header.len()..],
            frame.payload(),
            "the complete payload follows the header"
        );
        assert_eq!(
            stages.last(),
            Some(&TransferStage::Sending {
                sent: FRAME_BYTES,
                total: FRAME_BYTES
            })
        );
    }

    #[test]
    fn protocol_rejects_wrong_verification_response() {
        let mut stream = ScriptedStream {
            responses: Cursor::new(b"ERR CRC\n".to_vec()),
            writes: Vec::new(),
        };

        let error = expect_response(&mut stream, "frame verification", "OK FRAME VERIFIED")
            .expect_err("CRC failure must be rejected");
        assert!(error.to_string().contains("ERR CRC"));
    }
}
