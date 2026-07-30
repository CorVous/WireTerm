//! Raster-image preparation for the initial send-image workflow.
//!
//! This module deliberately dithers imported raster images. Future SVG
//! rendering should dither raster assets before composition, map vector/text
//! paint directly to [`PanelColor`](crate::frame::PanelColor), and construct a
//! [`PanelFrame`](crate::frame::PanelFrame) without running the composition
//! through this module.

// All conversions are bounded by decoded image dimensions or the fixed
// 800 × 480 target. Arithmetic that determines the contain layout is integer
// based; these casts only address error-diffusion coordinates and averages.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

use std::path::{Path, PathBuf};

use image::{RgbImage, imageops::FilterType};
use thiserror::Error;

use crate::frame::{FRAME_HEIGHT, FRAME_WIDTH, FrameError, PIXEL_COUNT, PanelColor, PanelFrame};

const EDGE_SAMPLE_DEPTH: u32 = 8;
const PALETTE: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [255.0, 255.0, 255.0], [205.0, 35.0, 35.0]];

#[derive(Debug, Error)]
pub enum RasterError {
    #[error("could not decode image {path}: {source}")]
    Decode {
        path: PathBuf,
        #[source]
        source: image::ImageError,
    },
    #[error("image dimensions must be non-zero")]
    EmptyImage,
    #[error(transparent)]
    Frame(#[from] FrameError),
}

/// Prepare one raster image using proportional contain scaling,
/// edge-matched letterboxing, and Floyd–Steinberg B/W/red dithering.
pub fn prepare_raster_path(path: &Path) -> Result<PanelFrame, RasterError> {
    let source = image::open(path)
        .map_err(|source| RasterError::Decode {
            path: path.to_path_buf(),
            source,
        })?
        .to_rgb8();
    prepare_raster_image(&source)
}

pub fn prepare_raster_image(source: &RgbImage) -> Result<PanelFrame, RasterError> {
    let (resized_width, resized_height) = contain_dimensions(source.width(), source.height())?;
    let resized =
        image::imageops::resize(source, resized_width, resized_height, FilterType::Lanczos3);
    let resized_width_usize = resized_width as usize;
    let resized_height_usize = resized_height as usize;
    let offset_x = (FRAME_WIDTH - resized_width_usize) / 2;
    let offset_y = (FRAME_HEIGHT - resized_height_usize) / 2;

    let mut working = vec![[255.0_f32; 3]; PIXEL_COUNT];
    fill_letterbox(&mut working, &resized, offset_x, offset_y);
    for (x, y, pixel) in resized.enumerate_pixels() {
        working[(y as usize + offset_y) * FRAME_WIDTH + x as usize + offset_x] = [
            f32::from(pixel[0]),
            f32::from(pixel[1]),
            f32::from(pixel[2]),
        ];
    }

    let classes = dither_to_palette(&mut working);
    let pixels: Vec<_> = classes
        .into_iter()
        .map(|class| match class {
            0 => PanelColor::Black,
            2 => PanelColor::Red,
            _ => PanelColor::White,
        })
        .collect();
    PanelFrame::from_palette_pixels(&pixels).map_err(Into::into)
}

fn contain_dimensions(source_width: u32, source_height: u32) -> Result<(u32, u32), RasterError> {
    if source_width == 0 || source_height == 0 {
        return Err(RasterError::EmptyImage);
    }

    let target_width = FRAME_WIDTH as u64;
    let target_height = FRAME_HEIGHT as u64;
    let source_width = u64::from(source_width);
    let source_height = u64::from(source_height);

    let (width, height) = if target_width * source_height <= target_height * source_width {
        (
            target_width,
            rounded_ratio(source_height * target_width, source_width).max(1),
        )
    } else {
        (
            rounded_ratio(source_width * target_height, source_height).max(1),
            target_height,
        )
    };

    Ok((width as u32, height as u32))
}

const fn rounded_ratio(numerator: u64, denominator: u64) -> u64 {
    (numerator + denominator / 2) / denominator
}

fn fill_letterbox(working: &mut [[f32; 3]], resized: &RgbImage, offset_x: usize, offset_y: usize) {
    let width = resized.width() as usize;
    let height = resized.height() as usize;

    if offset_y > 0 {
        let sample_rows = resized.height().min(EDGE_SAMPLE_DEPTH);
        let top = average_region(resized, 0, 0, resized.width(), sample_rows);
        let bottom = average_region(
            resized,
            0,
            resized.height() - sample_rows,
            resized.width(),
            resized.height(),
        );
        fill_region(working, 0, 0, FRAME_WIDTH, offset_y, top);
        fill_region(
            working,
            0,
            offset_y + height,
            FRAME_WIDTH,
            FRAME_HEIGHT,
            bottom,
        );
    } else if offset_x > 0 {
        let sample_columns = resized.width().min(EDGE_SAMPLE_DEPTH);
        let left = average_region(resized, 0, 0, sample_columns, resized.height());
        let right = average_region(
            resized,
            resized.width() - sample_columns,
            0,
            resized.width(),
            resized.height(),
        );
        fill_region(working, 0, 0, offset_x, FRAME_HEIGHT, left);
        fill_region(
            working,
            offset_x + width,
            0,
            FRAME_WIDTH,
            FRAME_HEIGHT,
            right,
        );
    }
}

fn average_region(image: &RgbImage, left: u32, top: u32, right: u32, bottom: u32) -> [f32; 3] {
    let mut totals = [0_u64; 3];
    let mut count = 0_u64;
    for y in top..bottom {
        for x in left..right {
            let pixel = image.get_pixel(x, y);
            for channel in 0..3 {
                totals[channel] += u64::from(pixel[channel]);
            }
            count += 1;
        }
    }
    [
        totals[0] as f32 / count as f32,
        totals[1] as f32 / count as f32,
        totals[2] as f32 / count as f32,
    ]
}

fn fill_region(
    pixels: &mut [[f32; 3]],
    left: usize,
    top: usize,
    right: usize,
    bottom: usize,
    color: [f32; 3],
) {
    for y in top..bottom {
        for x in left..right {
            pixels[y * FRAME_WIDTH + x] = color;
        }
    }
}

fn dither_to_palette(working: &mut [[f32; 3]]) -> Vec<u8> {
    let mut classes = vec![1_u8; PIXEL_COUNT];
    for y in 0..FRAME_HEIGHT {
        for x in 0..FRAME_WIDTH {
            let index = y * FRAME_WIDTH + x;
            let old = working[index];
            let (class, chosen) = PALETTE
                .iter()
                .enumerate()
                .min_by(|(_, left), (_, right)| {
                    color_distance(old, **left).total_cmp(&color_distance(old, **right))
                })
                .map(|(class, color)| (class as u8, *color))
                .expect("palette is non-empty");
            classes[index] = class;
            let error = [old[0] - chosen[0], old[1] - chosen[1], old[2] - chosen[2]];
            diffuse_error(working, x, y, 1, 0, error, 7.0 / 16.0);
            diffuse_error(working, x, y, -1, 1, error, 3.0 / 16.0);
            diffuse_error(working, x, y, 0, 1, error, 5.0 / 16.0);
            diffuse_error(working, x, y, 1, 1, error, 1.0 / 16.0);
        }
    }
    classes
}

fn color_distance(left: [f32; 3], right: [f32; 3]) -> f32 {
    let red = left[0] - right[0];
    let green = left[1] - right[1];
    let blue = left[2] - right[2];
    (blue * blue).mul_add(0.11, (green * green).mul_add(0.59, red * red * 0.30))
}

fn diffuse_error(
    pixels: &mut [[f32; 3]],
    x: usize,
    y: usize,
    dx: isize,
    dy: isize,
    error: [f32; 3],
    factor: f32,
) {
    let next_x = x as isize + dx;
    let next_y = y as isize + dy;
    if next_x < 0 || next_y < 0 || next_x >= FRAME_WIDTH as isize || next_y >= FRAME_HEIGHT as isize
    {
        return;
    }
    let pixel = &mut pixels[next_y as usize * FRAME_WIDTH + next_x as usize];
    for channel in 0..3 {
        pixel[channel] = error[channel]
            .mul_add(factor, pixel[channel])
            .clamp(0.0, 255.0);
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use image::Rgb;

    use super::*;
    use crate::frame::{FRAME_BYTES, PLANE_BYTES};

    #[test]
    fn contain_layout_preserves_aspect_ratio() {
        assert_eq!(contain_dimensions(1600, 480).unwrap(), (800, 240));
        assert_eq!(contain_dimensions(400, 960).unwrap(), (200, 480));
        assert_eq!(contain_dimensions(800, 480).unwrap(), (800, 480));
    }

    #[test]
    fn portrait_letterbox_matches_image_edges() {
        let source = RgbImage::from_pixel(100, 480, Rgb([0, 0, 0]));
        let frame = prepare_raster_image(&source).expect("solid image should prepare");

        assert!(
            frame
                .preview_rgb()
                .chunks_exact(3)
                .all(|pixel| pixel == [0, 0, 0])
        );
    }

    #[test]
    fn prepares_device_ready_lighthouse_frame() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets/prototype/lighthouse-test-frame-bwr-800x480.png");
        let frame = prepare_raster_path(&path).expect("test image should convert");

        assert_eq!(frame.payload().len(), FRAME_BYTES);
        assert_eq!(frame.preview_rgb().len(), PIXEL_COUNT * 3);
        let (black, red) = frame.payload().split_at(PLANE_BYTES);
        assert!(
            black
                .iter()
                .zip(red)
                .all(|(black_byte, red_byte)| (!black_byte & red_byte) == 0)
        );
        assert_ne!(frame.crc32(), 0);
    }
}
