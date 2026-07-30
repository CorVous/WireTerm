//! DISPOSABLE PROTOTYPE SHELL — not production code.

mod frame_preview;
mod pipeline;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use frame_preview::PreparedFrame;
use pipeline::{HEIGHT, RenderRequest, RenderResult, WIDTH};

const IMAGE_TARGET_WIDTH: u32 = 110;
const IMAGE_TARGET_HEIGHT: u32 = 66;
const VARIANT_A_RGBA_SHA256: &str =
    "af3050dcab7a8427290c3c9e1aad7f9c115168a850f318c860564a0147418199";
const VARIANT_A_PNG_SHA256: &str =
    "2e57b645c89df498be9f995c3b04073835f1a9760127904759694b8773fd5163";
const VARIANT_B_RGBA_SHA256: &str =
    "9c2ba1ba681900204182b89cfff4908a8f57a8be9a834f9ad9fff05a85abefbd";
const VARIANT_B_PNG_SHA256: &str =
    "58967ab2507c25577d079005b7e68698bed26f5abe51d18a4e66a1f719a3f3f5";
const VARIANT_A_FRAME_SHA256: &str =
    "96912953a4dcc0b82f566a9dbb513797a654a31d372dc37c9647932845b8ea55";
const VARIANT_A_PREVIEW_SHA256: &str =
    "a95b81a185ab6e23751337195daa400b7fe65362913dad68d09bfd1523c9abd7";
const VARIANT_B_FRAME_SHA256: &str =
    "18929ecc09631a78ccb2678502fc34d447a60c15e64f5c91ab93a8b6a12bd399";
const VARIANT_B_PREVIEW_SHA256: &str =
    "099494ee5b6f55855dd70f665fefb2dc18bab1ffc486599971a16cda3a3093e8";

struct FixtureResult {
    render: RenderResult,
    frame: PreparedFrame,
}

fn main() -> Result<()> {
    let prototype_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixtures_root = prototype_root.join("fixtures");
    let output_root = prototype_root.join("output");
    fs::create_dir_all(&output_root).context("create observable output directory")?;

    let manifest: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(fixtures_root.join("extension.json"))
            .context("read prototype extension manifest")?,
    )
    .context("parse prototype extension manifest")?;
    let template_path = declared_path(&manifest, "/template")?;
    let asset_path = declared_path(&manifest, "/assets/logo")?;
    let font_path = declared_path(&manifest, "/fonts/0/path")?;

    let template = fs::read_to_string(fixtures_root.join(template_path))
        .context("read Liquid SVG template")?;
    let logo_source =
        fs::read(fixtures_root.join(asset_path)).context("read declared local PNG asset")?;
    let logo_palette =
        frame_preview::dither_raster_asset(&logo_source, IMAGE_TARGET_WIDTH, IMAGE_TARGET_HEIGHT)
            .context("preprocess raster asset")?;
    let logo_no_dither = frame_preview::resize_raster_asset_without_dither(
        &logo_source,
        IMAGE_TARGET_WIDTH,
        IMAGE_TARGET_HEIGHT,
    )
    .context("resize no-dither raster comparison")?;
    fs::write(
        output_root.join("comparison-raster-asset-dithered.png"),
        &logo_palette.png,
    )
    .context("write dithered raster-asset comparison")?;
    fs::write(
        output_root.join("comparison-raster-asset-no-dither.png"),
        &logo_no_dither,
    )
    .context("write no-dither raster-asset comparison")?;
    let logo_no_dither = Arc::new(logo_no_dither);
    let logo = Arc::new(logo_palette.png.clone());
    let font = fs::read(fixtures_root.join(font_path)).context("read declared bundled font")?;

    let variant_a = run_fixture(
        "variant-a",
        &fixtures_root.join("variant-a.json"),
        &template,
        &font,
        asset_path,
        Arc::clone(&logo),
        &output_root,
    )?;
    let variant_b = run_fixture(
        "variant-b",
        &fixtures_root.join("variant-b.json"),
        &template,
        &font,
        asset_path,
        Arc::clone(&logo),
        &output_root,
    )?;

    verify_semantics(&variant_a.render, &variant_b.render)?;
    verify_frame_semantics(&variant_a.frame, &variant_b.frame)?;
    verify_golden_hashes(&variant_a.render, &variant_b.render)?;
    verify_frame_golden_hashes(&variant_a.frame, &variant_b.frame)?;

    let no_dither_a = render_fixture(
        &fixtures_root.join("variant-a.json"),
        &template,
        &font,
        asset_path,
        Arc::clone(&logo_no_dither),
        true,
    )?;
    let no_dither_off_palette_pixels = off_palette_pixels(&no_dither_a.rgba);
    ensure!(
        no_dither_off_palette_pixels > 500,
        "no-dither comparison did not retain enough off-palette raster pixels"
    );
    let all_dither_template = template
        .replace(" shape-rendering=\"crispEdges\"", "")
        .replace(" text-rendering=\"optimizeSpeed\"", "");
    ensure!(
        all_dither_template != template,
        "all-dither baseline did not disable crisp vector/text rendering"
    );
    let all_dither_render = render_fixture(
        &fixtures_root.join("variant-a.json"),
        &all_dither_template,
        &font,
        asset_path,
        Arc::clone(&logo_no_dither),
        false,
    )?;
    let all_dither_frame = frame_preview::prepare_all_dither_baseline(&all_dither_render.png)?;
    fs::write(
        output_root.join("comparison-all-dither-baseline.png"),
        &all_dither_frame.preview_png,
    )
    .context("write all-dither baseline comparison")?;
    let hybrid_title_red = red_pixels_in_png_region(&variant_a.frame.preview_png, 30, 30, 600, 45)?;
    let all_dither_title_red =
        red_pixels_in_png_region(&all_dither_frame.preview_png, 30, 30, 600, 45)?;
    ensure!(
        hybrid_title_red == 0 && all_dither_title_red > 100,
        "all-dither baseline did not demonstrate red text-edge noise"
    );
    fs::write(
        output_root.join("comparison-layered-raster-no-dither.png"),
        &no_dither_a.png,
    )
    .context("write no-dither layered comparison")?;
    fs::write(
        output_root.join("comparison-layered-raster-dithered.png"),
        &variant_a.frame.preview_png,
    )
    .context("write dithered layered comparison")?;
    fs::write(
        output_root.join("comparison-direct-palette-vectors-text.png"),
        &variant_b.frame.preview_png,
    )
    .context("write direct-palette vector/text comparison")?;

    let repeated_a = render_fixture(
        &fixtures_root.join("variant-a.json"),
        &template,
        &font,
        asset_path,
        Arc::clone(&logo),
        true,
    )?;
    ensure!(
        variant_a.render.rgba == repeated_a.rgba,
        "variant A raw pixels changed between consecutive renders"
    );
    ensure!(
        variant_a.render.png == repeated_a.png,
        "variant A PNG bytes changed between consecutive renders"
    );
    let repeated_frame_a = frame_preview::prepare_layered_frame(&repeated_a.png)?;
    ensure!(
        variant_a.frame.payload == repeated_frame_a.payload,
        "variant A frame payload changed between consecutive preparations"
    );
    ensure!(
        variant_a.frame.preview_png == repeated_frame_a.preview_png,
        "variant A e-paper preview PNG changed between consecutive preparations"
    );

    println!();
    println!(
        "PASS: raster asset dithered at {} x {} (black={} white={} red={}).",
        logo_palette.width,
        logo_palette.height,
        logo_palette.black_pixels,
        logo_palette.white_pixels,
        logo_palette.red_pixels
    );
    println!(
        "PASS: dithered asset PNG sha256: {}",
        logo_palette.png_sha256
    );
    println!("PASS: both disposable fixtures rendered meaningful {WIDTH} x {HEIGHT} output.");
    println!("PASS: vector/text composition was already exactly black/white/red.");
    println!(
        "PASS: same-image no-dither comparison retained {no_dither_off_palette_pixels} off-palette pixels."
    );
    println!(
        "PASS: title red pixels, hybrid={hybrid_title_red}, all-dither={all_dither_title_red}."
    );
    println!("PASS: both layered PNGs produced meaningful 96,000-byte B/W/R frames.");
    println!("PASS: both e-paper preview PNGs decoded as exactly {WIDTH} x {HEIGHT}.");
    println!("PASS: variant A repeated with identical layered render, frame, and preview bytes.");
    println!("Artifacts: {}", output_root.display());
    println!("  comparison-direct-palette-vectors-text.png");
    println!("  comparison-layered-raster-dithered.png");
    println!("  comparison-layered-raster-no-dither.png");
    println!("  comparison-all-dither-baseline.png");
    println!("Not proven: cross-platform hashes, future dependency stability, hostile-input");
    println!("sandboxing, optical panel fidelity, hardware output, or shared production code.");

    Ok(())
}

fn verify_golden_hashes(variant_a: &RenderResult, variant_b: &RenderResult) -> Result<()> {
    ensure!(
        variant_a.rgba_sha256 == VARIANT_A_RGBA_SHA256
            && variant_a.png_sha256 == VARIANT_A_PNG_SHA256,
        "variant A did not match its committed golden hashes"
    );
    ensure!(
        variant_b.rgba_sha256 == VARIANT_B_RGBA_SHA256
            && variant_b.png_sha256 == VARIANT_B_PNG_SHA256,
        "variant B did not match its committed golden hashes"
    );
    Ok(())
}

fn verify_frame_golden_hashes(variant_a: &PreparedFrame, variant_b: &PreparedFrame) -> Result<()> {
    ensure!(
        variant_a.payload_sha256 == VARIANT_A_FRAME_SHA256
            && variant_a.preview_png_sha256 == VARIANT_A_PREVIEW_SHA256,
        "variant A did not match its committed frame and preview golden hashes"
    );
    ensure!(
        variant_b.payload_sha256 == VARIANT_B_FRAME_SHA256
            && variant_b.preview_png_sha256 == VARIANT_B_PREVIEW_SHA256,
        "variant B did not match its committed frame and preview golden hashes"
    );
    Ok(())
}

fn run_fixture(
    name: &str,
    context_path: &Path,
    template: &str,
    font: &[u8],
    asset_path: &str,
    asset: Arc<Vec<u8>>,
    output_root: &Path,
) -> Result<FixtureResult> {
    let result = render_fixture(context_path, template, font, asset_path, asset, true)?;
    let frame = frame_preview::prepare_layered_frame(&result.png)?;
    fs::write(output_root.join(format!("{name}.svg")), &result.svg)
        .with_context(|| format!("write {name} rendered SVG"))?;
    fs::write(output_root.join(format!("{name}.png")), &result.png)
        .with_context(|| format!("write {name} PNG"))?;
    fs::write(
        output_root.join(format!("{name}-epaper-preview.png")),
        &frame.preview_png,
    )
    .with_context(|| format!("write {name} e-paper preview PNG"))?;

    println!("fixture: {name}");
    println!("  dimensions: {WIDTH} x {HEIGHT}");
    println!("  text nodes retained: {}", result.has_text_nodes);
    println!("  non-white pixels: {}", result.non_white_pixels);
    println!("  rgba sha256: {}", result.rgba_sha256);
    println!("  png  sha256: {}", result.png_sha256);
    println!(
        "  frame palette: black={} white={} red={}",
        frame.black_pixels, frame.white_pixels, frame.red_pixels
    );
    println!("  frame bytes: {}", frame.payload.len());
    println!("  frame sha256: {}", frame.payload_sha256);
    println!("  preview png sha256: {}", frame.preview_png_sha256);
    println!(
        "  svg: {}",
        output_root.join(format!("{name}.svg")).display()
    );
    println!(
        "  png: {}",
        output_root.join(format!("{name}.png")).display()
    );
    println!(
        "  e-paper preview: {}",
        output_root
            .join(format!("{name}-epaper-preview.png"))
            .display()
    );

    Ok(FixtureResult {
        render: result,
        frame,
    })
}

fn render_fixture(
    context_path: &Path,
    template: &str,
    font: &[u8],
    asset_path: &str,
    asset: Arc<Vec<u8>>,
    require_direct_palette: bool,
) -> Result<RenderResult> {
    let context: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(context_path)
            .with_context(|| format!("read {}", context_path.display()))?,
    )
    .with_context(|| format!("parse {}", context_path.display()))?;
    ensure!(
        context.get("data").is_some() && context.get("settings").is_some(),
        "fixture must expose data and settings"
    );
    let mut assets = BTreeMap::new();
    assets.insert(asset_path.to_owned(), asset);
    pipeline::render(RenderRequest {
        template,
        context: &context,
        font_bytes: font,
        assets,
        require_direct_palette,
    })
}

fn verify_semantics(variant_a: &RenderResult, variant_b: &RenderResult) -> Result<()> {
    ensure!(
        variant_a.svg.contains("WireTerm &amp; &lt;SVG&gt;")
            && variant_b.svg.contains("WireTerm &amp; &lt;SVG&gt;"),
        "XML-sensitive title text was not escaped"
    );
    ensure!(
        variant_a.svg.contains("id=\"branch-left\"")
            && !variant_a.svg.contains("id=\"branch-right\""),
        "variant A did not select only branch A"
    );
    ensure!(
        variant_b.svg.contains("id=\"branch-right\"")
            && !variant_b.svg.contains("id=\"branch-left\""),
        "variant B did not select only branch B"
    );
    ensure!(
        variant_a.svg.contains("id=\"declared-image\"")
            && !variant_b.svg.contains("id=\"declared-image\""),
        "image-present/image-absent condition did not render"
    );
    ensure!(
        variant_a.svg.contains("id=\"row-1\"")
            && variant_a.svg.contains("id=\"row-2\"")
            && !variant_a.svg.contains("id=\"row-3\"")
            && variant_b.svg.contains("id=\"row-1\"")
            && variant_b.svg.contains("id=\"row-2\"")
            && !variant_b.svg.contains("id=\"row-3\""),
        "Liquid loop did not emit exactly two rows"
    );

    ensure!(
        non_white_in_region(&variant_a.rgba, 650, 70, 110, 88) > 1_000,
        "variant A image region was not meaningfully painted"
    );
    ensure!(
        non_white_in_region(&variant_b.rgba, 650, 70, 110, 88) == 0,
        "variant B image-absent region was unexpectedly painted"
    );
    ensure!(
        non_white_in_region(&variant_a.rgba, 280, 96, 120, 38) > 3_000,
        "variant A branch region was not painted"
    );
    ensure!(
        non_white_in_region(&variant_b.rgba, 430, 96, 120, 38) > 3_000,
        "variant B branch region was not painted"
    );
    ensure!(
        non_white_in_region(&variant_a.rgba, 30, 30, 600, 45) > 800,
        "bundled-font title was not meaningfully painted"
    );
    ensure!(
        non_white_in_region(&variant_a.rgba, 45, 215, 690, 125) > 1_000,
        "Liquid loop rows were not meaningfully painted"
    );

    Ok(())
}

fn verify_frame_semantics(variant_a: &PreparedFrame, variant_b: &PreparedFrame) -> Result<()> {
    for (name, frame) in [("variant A", variant_a), ("variant B", variant_b)] {
        ensure!(
            frame.payload.len() == 96_000,
            "{name} frame payload was not 96,000 bytes"
        );
        ensure!(
            frame.black_pixels > 10_000 && frame.white_pixels > 300_000 && frame.red_pixels > 100,
            "{name} did not produce a meaningful black/white/red frame"
        );
        ensure!(
            frame.black_pixels + frame.white_pixels + frame.red_pixels
                == WIDTH as usize * HEIGHT as usize,
            "{name} palette counts did not cover the full frame"
        );
    }
    Ok(())
}

fn non_white_in_region(rgba: &[u8], x: usize, y: usize, width: usize, height: usize) -> usize {
    let canvas_width = WIDTH as usize;
    (y..y + height)
        .flat_map(|row| (x..x + width).map(move |column| (row, column)))
        .filter(|&(row, column)| {
            let offset = (row * canvas_width + column) * 4;
            rgba[offset..offset + 3] != [255, 255, 255]
        })
        .count()
}

fn off_palette_pixels(rgba: &[u8]) -> usize {
    rgba.chunks_exact(4)
        .filter(|pixel| !matches!(&pixel[..3], [0, 0, 0] | [255, 255, 255] | [205, 35, 35]))
        .count()
}

fn red_pixels_in_png_region(
    png: &[u8],
    x: usize,
    y: usize,
    width: usize,
    height: usize,
) -> Result<usize> {
    let image = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .context("decode comparison PNG")?
        .to_rgb8();
    Ok((y..y + height)
        .flat_map(|row| (x..x + width).map(move |column| (row, column)))
        .filter(|&(row, column)| image.get_pixel(column as u32, row as u32).0 == [205, 35, 35])
        .count())
}

fn declared_path<'a>(manifest: &'a serde_json::Value, pointer: &str) -> Result<&'a str> {
    let path = manifest
        .pointer(pointer)
        .and_then(serde_json::Value::as_str)
        .with_context(|| format!("missing manifest path {pointer}"))?;
    ensure!(
        !path.contains("..")
            && !path.contains(':')
            && !path.starts_with('/')
            && !path.starts_with('\\'),
        "manifest path must be extension-relative: {path}"
    );
    Ok(path)
}
