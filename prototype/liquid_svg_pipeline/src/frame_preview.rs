//! DISPOSABLE layered panel-conversion prototype.
//!
//! Raster assets are palette-reduced before SVG composition. The composed
//! 800 x 480 render must already contain only panel colors, so frame packing
//! performs no full-frame quantization or dithering.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

use std::io::Cursor;

use anyhow::{Context, Result, bail, ensure};
use image::{GenericImageView, ImageFormat, imageops::FilterType};
use sha2::{Digest, Sha256};

use crate::pipeline::{HEIGHT, WIDTH};

const PLANE_BYTES: usize = WIDTH as usize * HEIGHT as usize / 8;
const FRAME_BYTES: usize = PLANE_BYTES * 2;
const BLACK: [u8; 3] = [0, 0, 0];
const WHITE: [u8; 3] = [255, 255, 255];
const RED: [u8; 3] = [205, 35, 35];

pub struct PaletteRaster {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub black_pixels: usize,
    pub white_pixels: usize,
    pub red_pixels: usize,
    pub png_sha256: String,
}

pub struct PreparedFrame {
    pub payload: Vec<u8>,
    pub preview_png: Vec<u8>,
    pub payload_sha256: String,
    pub preview_png_sha256: String,
    pub black_pixels: usize,
    pub white_pixels: usize,
    pub red_pixels: usize,
}

pub fn dither_raster_asset(
    source_png: &[u8],
    target_width: u32,
    target_height: u32,
) -> Result<PaletteRaster> {
    let source = image::load_from_memory_with_format(source_png, ImageFormat::Png)
        .context("decode declared raster asset")?
        .to_rgb8();
    let source =
        image::imageops::resize(&source, target_width, target_height, FilterType::Lanczos3);
    let classes = dither_classes(&source)?;
    let (rgb, counts) = palette_rgb(&classes);
    let png = encode_rgb_png(&rgb, source.width(), source.height())?;
    Ok(PaletteRaster {
        png_sha256: sha256(&png),
        png,
        width: target_width,
        height: target_height,
        black_pixels: counts[0],
        white_pixels: counts[1],
        red_pixels: counts[2],
    })
}

pub fn resize_raster_asset_without_dither(
    source_png: &[u8],
    target_width: u32,
    target_height: u32,
) -> Result<Vec<u8>> {
    let source = image::load_from_memory_with_format(source_png, ImageFormat::Png)
        .context("decode no-dither raster comparison")?
        .to_rgb8();
    let resized =
        image::imageops::resize(&source, target_width, target_height, FilterType::Lanczos3);
    encode_rgb_png(resized.as_raw(), target_width, target_height)
}

pub fn prepare_layered_frame(composed_png: &[u8]) -> Result<PreparedFrame> {
    let image = image::load_from_memory_with_format(composed_png, ImageFormat::Png)
        .context("decode layered SVG render")?
        .to_rgb8();
    ensure!(
        image.dimensions() == (WIDTH, HEIGHT),
        "layered SVG render was not {WIDTH} x {HEIGHT}"
    );

    let mut classes = Vec::with_capacity(WIDTH as usize * HEIGHT as usize);
    for (index, pixel) in image.pixels().enumerate() {
        let class = match pixel.0 {
            BLACK => 0,
            WHITE => 1,
            RED => 2,
            color => {
                let x = index % WIDTH as usize;
                let y = index / WIDTH as usize;
                bail!(
                    "composed SVG pixel at ({x}, {y}) escaped the panel palette: rgb({}, {}, {})",
                    color[0],
                    color[1],
                    color[2]
                );
            }
        };
        classes.push(class);
    }

    prepare_classes(classes)
}

pub fn prepare_all_dither_baseline(composed_png: &[u8]) -> Result<PreparedFrame> {
    let image = image::load_from_memory_with_format(composed_png, ImageFormat::Png)
        .context("decode all-dither baseline")?
        .to_rgb8();
    ensure!(
        image.dimensions() == (WIDTH, HEIGHT),
        "all-dither baseline was not {WIDTH} x {HEIGHT}"
    );
    prepare_classes(dither_classes(&image)?)
}

fn prepare_classes(classes: Vec<u8>) -> Result<PreparedFrame> {
    ensure!(
        classes.len() == WIDTH as usize * HEIGHT as usize,
        "prepared classes were not exactly {WIDTH} x {HEIGHT}"
    );
    let (preview_rgb, counts) = palette_rgb(&classes);
    let preview_png = encode_rgb_png(&preview_rgb, WIDTH, HEIGHT)?;
    let mut black = vec![0xFF_u8; PLANE_BYTES];
    let mut red = vec![0x00_u8; PLANE_BYTES];
    for (index, class) in classes.into_iter().enumerate() {
        let mask = 0x80_u8 >> (index % 8);
        match class {
            0 => black[index / 8] &= !mask,
            2 => red[index / 8] |= mask,
            _ => {}
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
    let decoded_preview = image::load_from_memory_with_format(&preview_png, ImageFormat::Png)
        .context("decode layered preview PNG for dimension verification")?;
    ensure!(
        decoded_preview.dimensions() == (WIDTH, HEIGHT),
        "layered preview PNG was not {WIDTH} x {HEIGHT}"
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

fn dither_classes(image: &image::RgbImage) -> Result<Vec<u8>> {
    let width = image.width() as usize;
    let height = image.height() as usize;
    ensure!(width > 0 && height > 0, "raster image was empty");
    let mut working: Vec<[f32; 3]> = image
        .pixels()
        .map(|pixel| {
            [
                f32::from(pixel[0]),
                f32::from(pixel[1]),
                f32::from(pixel[2]),
            ]
        })
        .collect();
    let palette = [
        BLACK.map(f32::from),
        WHITE.map(f32::from),
        RED.map(f32::from),
    ];
    let mut classes = vec![1_u8; width * height];
    for y in 0..height {
        for x in 0..width {
            let index = y * width + x;
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
            diffuse_error(&mut working, width, height, x, y, 1, 0, error, 7.0 / 16.0);
            diffuse_error(&mut working, width, height, x, y, -1, 1, error, 3.0 / 16.0);
            diffuse_error(&mut working, width, height, x, y, 0, 1, error, 5.0 / 16.0);
            diffuse_error(&mut working, width, height, x, y, 1, 1, error, 1.0 / 16.0);
        }
    }
    Ok(classes)
}

fn palette_rgb(classes: &[u8]) -> (Vec<u8>, [usize; 3]) {
    let mut rgb = Vec::with_capacity(classes.len() * 3);
    let mut counts = [0_usize; 3];
    for class in classes {
        counts[*class as usize] += 1;
        rgb.extend_from_slice(match class {
            0 => &BLACK,
            2 => &RED,
            _ => &WHITE,
        });
    }
    (rgb, counts)
}

fn color_distance(left: [f32; 3], right: [f32; 3]) -> f32 {
    let red = left[0] - right[0];
    let green = left[1] - right[1];
    let blue = left[2] - right[2];
    (blue * blue).mul_add(0.11, (green * green).mul_add(0.59, red * red * 0.30))
}

#[allow(clippy::too_many_arguments)]
fn diffuse_error(
    pixels: &mut [[f32; 3]],
    width: usize,
    height: usize,
    x: usize,
    y: usize,
    dx: isize,
    dy: isize,
    error: [f32; 3],
    factor: f32,
) {
    let next_x = x as isize + dx;
    let next_y = y as isize + dy;
    if next_x < 0 || next_y < 0 || next_x >= width as isize || next_y >= height as isize {
        return;
    }
    let pixel = &mut pixels[next_y as usize * width + next_x as usize];
    for channel in 0..3 {
        pixel[channel] = error[channel]
            .mul_add(factor, pixel[channel])
            .clamp(0.0, 255.0);
    }
}

fn encode_rgb_png(rgb: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    {
        let mut encoder = png::Encoder::new(Cursor::new(&mut output), width, height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        encoder.set_filter(png::Filter::Paeth);
        let mut writer = encoder.write_header().context("write palette PNG header")?;
        writer
            .write_image_data(rgb)
            .context("write palette PNG pixels")?;
    }
    Ok(output)
}

fn sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
