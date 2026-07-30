//! DISPOSABLE PROTOTYPE MIRROR of `gui_prototype.rs::prepare_frame`.
//!
//! This intentionally has no serial or hardware-send path. It exists only to
//! pass the Liquid-to-SVG fixtures through WireTerm's current frame algorithm
//! and make the final black/white/red pixels observable as PNG files.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

use std::io::Cursor;

use anyhow::{Context, Result, ensure};
use image::{GenericImageView, ImageFormat, imageops::FilterType};
use sha2::{Digest, Sha256};

use crate::pipeline::{HEIGHT, WIDTH};

const PLANE_BYTES: usize = WIDTH as usize * HEIGHT as usize / 8;
const FRAME_BYTES: usize = PLANE_BYTES * 2;
const BLACK: [u8; 3] = [0, 0, 0];
const WHITE: [u8; 3] = [255, 255, 255];
const RED: [u8; 3] = [205, 35, 35];

pub struct PreparedFrame {
    pub payload: Vec<u8>,
    pub preview_png: Vec<u8>,
    pub payload_sha256: String,
    pub preview_png_sha256: String,
    pub black_pixels: usize,
    pub white_pixels: usize,
    pub red_pixels: usize,
}

#[allow(clippy::too_many_lines)]
pub fn prepare_frame(source_png: &[u8]) -> Result<PreparedFrame> {
    let source = image::load_from_memory_with_format(source_png, ImageFormat::Png)
        .context("decode generated PNG for frame preparation")?
        .to_rgb8();
    let scale = (WIDTH as f32 / source.width() as f32).min(HEIGHT as f32 / source.height() as f32);
    let resized_width = ((source.width() as f32 * scale).round() as u32).max(1);
    let resized_height = ((source.height() as f32 * scale).round() as u32).max(1);
    let resized =
        image::imageops::resize(&source, resized_width, resized_height, FilterType::Lanczos3);
    let offset_x = (WIDTH as usize - resized_width as usize) / 2;
    let offset_y = (HEIGHT as usize - resized_height as usize) / 2;

    let mut working = vec![[255.0_f32; 3]; WIDTH as usize * HEIGHT as usize];
    if offset_y > 0 {
        let sample_rows = resized.height().min(8);
        let top = average_region(&resized, 0, 0, resized.width(), sample_rows);
        let bottom = average_region(
            &resized,
            0,
            resized.height() - sample_rows,
            resized.width(),
            resized.height(),
        );
        fill_region(&mut working, 0, 0, WIDTH as usize, offset_y, top);
        fill_region(
            &mut working,
            0,
            offset_y + resized_height as usize,
            WIDTH as usize,
            HEIGHT as usize,
            bottom,
        );
    } else if offset_x > 0 {
        let sample_columns = resized.width().min(8);
        let left = average_region(&resized, 0, 0, sample_columns, resized.height());
        let right = average_region(
            &resized,
            resized.width() - sample_columns,
            0,
            resized.width(),
            resized.height(),
        );
        fill_region(&mut working, 0, 0, offset_x, HEIGHT as usize, left);
        fill_region(
            &mut working,
            offset_x + resized_width as usize,
            0,
            WIDTH as usize,
            HEIGHT as usize,
            right,
        );
    }
    for (x, y, pixel) in resized.enumerate_pixels() {
        working[(y as usize + offset_y) * WIDTH as usize + x as usize + offset_x] = [
            f32::from(pixel[0]),
            f32::from(pixel[1]),
            f32::from(pixel[2]),
        ];
    }

    let palette = [
        BLACK.map(f32::from),
        WHITE.map(f32::from),
        RED.map(f32::from),
    ];
    let mut classes = vec![1_u8; WIDTH as usize * HEIGHT as usize];
    for y in 0..HEIGHT as usize {
        for x in 0..WIDTH as usize {
            let index = y * WIDTH as usize + x;
            let old = working[index];
            let (class, chosen) = palette
                .iter()
                .enumerate()
                .min_by(|(_, left), (_, right)| {
                    color_distance(old, **left).total_cmp(&color_distance(old, **right))
                })
                .map(|(class, color)| (class as u8, *color))
                .expect("palette is non-empty");
            classes[index] = class;
            let error = [old[0] - chosen[0], old[1] - chosen[1], old[2] - chosen[2]];
            diffuse_error(&mut working, x, y, 1, 0, error, 7.0 / 16.0);
            diffuse_error(&mut working, x, y, -1, 1, error, 3.0 / 16.0);
            diffuse_error(&mut working, x, y, 0, 1, error, 5.0 / 16.0);
            diffuse_error(&mut working, x, y, 1, 1, error, 1.0 / 16.0);
        }
    }

    let mut black = vec![0xFF_u8; PLANE_BYTES];
    let mut red = vec![0x00_u8; PLANE_BYTES];
    let mut preview_rgb = Vec::with_capacity(WIDTH as usize * HEIGHT as usize * 3);
    let mut counts = [0_usize; 3];
    for (index, class) in classes.into_iter().enumerate() {
        let mask = 0x80_u8 >> (index % 8);
        counts[class as usize] += 1;
        match class {
            0 => {
                black[index / 8] &= !mask;
                preview_rgb.extend_from_slice(&BLACK);
            }
            2 => {
                red[index / 8] |= mask;
                preview_rgb.extend_from_slice(&RED);
            }
            _ => preview_rgb.extend_from_slice(&WHITE),
        }
    }
    let mut payload = black;
    payload.extend_from_slice(&red);
    ensure!(
        payload.len() == FRAME_BYTES,
        "prepared frame payload was not {FRAME_BYTES} bytes"
    );
    let (black_plane, red_plane) = payload.split_at(PLANE_BYTES);
    ensure!(
        black_plane
            .iter()
            .zip(red_plane)
            .all(|(black_byte, red_byte)| (!black_byte & red_byte) == 0),
        "black and red planes overlap"
    );

    let preview_png = encode_preview_png(&preview_rgb)?;
    let decoded_preview = image::load_from_memory_with_format(&preview_png, ImageFormat::Png)
        .context("decode e-paper preview PNG for dimension verification")?;
    ensure!(
        decoded_preview.dimensions() == (WIDTH, HEIGHT),
        "e-paper preview PNG was not {WIDTH} x {HEIGHT}"
    );

    Ok(PreparedFrame {
        payload_sha256: sha256(&payload),
        preview_png_sha256: sha256(&preview_png),
        payload,
        preview_png,
        black_pixels: counts[0],
        white_pixels: counts[1],
        red_pixels: counts[2],
    })
}

fn average_region(
    image: &image::RgbImage,
    left: u32,
    top: u32,
    right: u32,
    bottom: u32,
) -> [f32; 3] {
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
            pixels[y * WIDTH as usize + x] = color;
        }
    }
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
    if next_x < 0 || next_y < 0 || next_x >= WIDTH as isize || next_y >= HEIGHT as isize {
        return;
    }
    let pixel = &mut pixels[next_y as usize * WIDTH as usize + next_x as usize];
    for channel in 0..3 {
        pixel[channel] = error[channel]
            .mul_add(factor, pixel[channel])
            .clamp(0.0, 255.0);
    }
}

fn encode_preview_png(rgb: &[u8]) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    {
        let mut encoder = png::Encoder::new(Cursor::new(&mut output), WIDTH, HEIGHT);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        encoder.set_filter(png::Filter::Paeth);
        let mut writer = encoder
            .write_header()
            .context("write e-paper preview PNG header")?;
        writer
            .write_image_data(rgb)
            .context("write e-paper preview PNG pixels")?;
    }
    Ok(output)
}

fn sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
