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

const VARIANT_A_RGBA_SHA256: &str =
    "bb67e53b58ba85d4f1376fa630f581243f392745aaa9029b882fb7708f1cb541";
const VARIANT_A_PNG_SHA256: &str =
    "e14e2b2676d40967e18ed8833900dca3d920c7777707ae9fdd829652fe1c3739";
const VARIANT_B_RGBA_SHA256: &str =
    "a87ce35bde8ae077ee6146d7d0ec3b7f4b9ef4663c3b92e1037f8cef0f12b8c6";
const VARIANT_B_PNG_SHA256: &str =
    "b1cd894647ff12f56181e9ebd46b99cb34a60429a29027d06fa278219c66a423";
const VARIANT_A_FRAME_SHA256: &str =
    "1e66db5dc8e1590cad197225e9182ae6a49225c6f851dadf8f89b0d022666787";
const VARIANT_A_PREVIEW_SHA256: &str =
    "28beed49bcbab97d9a15231d0b8a1b5158736fa0ef73b3aa64bbd65ada79775f";
const VARIANT_B_FRAME_SHA256: &str =
    "ac656ce2a6f661dfba7e200eb1e3ff8b07994e384da04189454bf185d7417f04";
const VARIANT_B_PREVIEW_SHA256: &str =
    "087e22b8b66e81f49f2f1074c44e0c175d648f7dc7349f6e4f285cee72447831";

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
    let logo = Arc::new(
        fs::read(fixtures_root.join(asset_path)).context("read declared local PNG asset")?,
    );
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
        logo,
        &output_root,
    )?;

    verify_semantics(&variant_a.render, &variant_b.render)?;
    verify_golden_hashes(&variant_a.render, &variant_b.render)?;
    verify_frame_semantics(&variant_a.frame, &variant_b.frame)?;
    verify_frame_golden_hashes(&variant_a.frame, &variant_b.frame)?;

    let repeated_a = render_fixture(
        &fixtures_root.join("variant-a.json"),
        &template,
        &font,
        asset_path,
        Arc::new(fs::read(fixtures_root.join(asset_path))?),
    )?;
    ensure!(
        variant_a.render.rgba == repeated_a.rgba,
        "variant A raw pixels changed between consecutive renders"
    );
    ensure!(
        variant_a.render.png == repeated_a.png,
        "variant A PNG bytes changed between consecutive renders"
    );
    let repeated_frame_a = frame_preview::prepare_frame(&repeated_a.png)?;
    ensure!(
        variant_a.frame.payload == repeated_frame_a.payload,
        "variant A frame payload changed between consecutive preparations"
    );
    ensure!(
        variant_a.frame.preview_png == repeated_frame_a.preview_png,
        "variant A e-paper preview PNG changed between consecutive preparations"
    );

    println!();
    println!("PASS: both disposable fixtures rendered meaningful {WIDTH} x {HEIGHT} output.");
    println!("PASS: both fixtures matched committed render, frame, and preview goldens.");
    println!("PASS: both generated PNGs produced meaningful 96,000-byte B/W/R frames.");
    println!("PASS: both e-paper preview PNGs decoded as exactly {WIDTH} x {HEIGHT}.");
    println!("PASS: variant A repeated with identical render, frame, and preview bytes.");
    println!("Artifacts: {}", output_root.display());
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
    let result = render_fixture(context_path, template, font, asset_path, asset)?;
    let frame = frame_preview::prepare_frame(&result.png)?;
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
        non_white_in_region(&variant_a.rgba, 280, 96, 120, 38) > 4_000,
        "variant A branch region was not painted"
    );
    ensure!(
        non_white_in_region(&variant_b.rgba, 430, 96, 120, 38) > 4_000,
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
            frame.black_pixels > 10_000 && frame.white_pixels > 300_000 && frame.red_pixels > 500,
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
