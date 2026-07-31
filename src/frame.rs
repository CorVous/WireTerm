//! Display-ready panel frames.

use thiserror::Error;

pub const FRAME_WIDTH: usize = 800;
pub const FRAME_HEIGHT: usize = 480;
pub const PIXEL_COUNT: usize = FRAME_WIDTH * FRAME_HEIGHT;
pub const PLANE_BYTES: usize = PIXEL_COUNT / 8;
pub const FRAME_BYTES: usize = PLANE_BYTES * 2;

/// The three logical colours supported by the panel.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PanelColor {
    Black,
    White,
    Red,
}

impl PanelColor {
    #[must_use]
    pub const fn rgb(self) -> [u8; 3] {
        match self {
            Self::Black => [0, 0, 0],
            Self::White => [255, 255, 255],
            Self::Red => [205, 35, 35],
        }
    }
}

/// A validated 800 × 480 black/white/red frame ready for WireTerm/1.
///
/// This is the shared boundary between all host renderers and transport.
/// Exact-palette SVG composition can use [`Self::from_palette_pixels`]
/// directly; it must not pass through raster-image dithering.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PanelFrame {
    payload: Box<[u8]>,
    preview_rgb: Box<[u8]>,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum FrameError {
    #[error("expected {expected} palette pixels, received {actual}")]
    PixelCount { expected: usize, actual: usize },
    #[error("expected {expected} frame bytes, received {actual}")]
    PayloadLength { expected: usize, actual: usize },
    #[error("black and red planes overlap at pixel {pixel}")]
    OverlappingPlanes { pixel: usize },
}

impl PanelFrame {
    pub fn from_palette_pixels(pixels: &[PanelColor]) -> Result<Self, FrameError> {
        if pixels.len() != PIXEL_COUNT {
            return Err(FrameError::PixelCount {
                expected: PIXEL_COUNT,
                actual: pixels.len(),
            });
        }

        let mut black = vec![0xFF; PLANE_BYTES];
        let mut red = vec![0x00; PLANE_BYTES];
        let mut preview_rgb = Vec::with_capacity(PIXEL_COUNT * 3);

        for (index, color) in pixels.iter().copied().enumerate() {
            let mask = 0x80_u8 >> (index % 8);
            match color {
                PanelColor::Black => black[index / 8] &= !mask,
                PanelColor::White => {}
                PanelColor::Red => red[index / 8] |= mask,
            }
            preview_rgb.extend_from_slice(&color.rgb());
        }

        black.extend_from_slice(&red);
        Ok(Self {
            payload: black.into_boxed_slice(),
            preview_rgb: preview_rgb.into_boxed_slice(),
        })
    }

    pub fn try_from_payload(payload: Vec<u8>) -> Result<Self, FrameError> {
        if payload.len() != FRAME_BYTES {
            return Err(FrameError::PayloadLength {
                expected: FRAME_BYTES,
                actual: payload.len(),
            });
        }

        let (black, red) = payload.split_at(PLANE_BYTES);
        let mut preview_rgb = Vec::with_capacity(PIXEL_COUNT * 3);
        for pixel in 0..PIXEL_COUNT {
            let mask = 0x80_u8 >> (pixel % 8);
            let is_black = black[pixel / 8] & mask == 0;
            let is_red = red[pixel / 8] & mask != 0;
            let color = match (is_black, is_red) {
                (true, true) => return Err(FrameError::OverlappingPlanes { pixel }),
                (true, false) => PanelColor::Black,
                (false, true) => PanelColor::Red,
                (false, false) => PanelColor::White,
            };
            preview_rgb.extend_from_slice(&color.rgb());
        }

        Ok(Self {
            payload: payload.into_boxed_slice(),
            preview_rgb: preview_rgb.into_boxed_slice(),
        })
    }

    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    #[must_use]
    pub fn preview_rgb(&self) -> &[u8] {
        &self.preview_rgb
    }

    #[must_use]
    pub fn crc32(&self) -> u32 {
        crc32fast::hash(&self.payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_palette_pixels_map_directly_to_panel_planes() {
        let mut pixels = vec![PanelColor::White; PIXEL_COUNT];
        pixels[0] = PanelColor::Black;
        pixels[1] = PanelColor::Red;

        let frame = PanelFrame::from_palette_pixels(&pixels).expect("valid frame");
        let (black, red) = frame.payload().split_at(PLANE_BYTES);

        assert_eq!(black[0], 0b0111_1111);
        assert_eq!(red[0], 0b0100_0000);
        assert_eq!(
            &frame.preview_rgb()[..9],
            &[0, 0, 0, 205, 35, 35, 255, 255, 255]
        );
    }

    #[test]
    fn rejects_overlapping_black_and_red_bits() {
        let mut payload = vec![0xFF; FRAME_BYTES];
        payload[PLANE_BYTES] = 0x80;
        payload[0] = 0x7F;

        assert_eq!(
            PanelFrame::try_from_payload(payload),
            Err(FrameError::OverlappingPlanes { pixel: 0 })
        );
    }
}
