//! DISPOSABLE PROTOTYPE LOGIC — not production code.
//!
//! This module keeps the pipeline behind one byte-in/result-out function so
//! the terminal shell and filesystem are not part of the design question.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;
use std::sync::Arc;

use anyhow::{Context, Result, bail, ensure};
use liquid_lib::stdlib::{Escape, ForBlock, IfBlock};
use resvg::usvg;
use sha2::{Digest, Sha256};
use tiny_skia::{Color, Pixmap, Transform};

pub const WIDTH: u32 = 800;
pub const HEIGHT: u32 = 480;

pub struct RenderRequest<'a> {
    pub template: &'a str,
    pub context: &'a serde_json::Value,
    pub font_bytes: &'a [u8],
    pub assets: BTreeMap<String, Arc<Vec<u8>>>,
    pub require_direct_palette: bool,
}

pub struct RenderResult {
    pub svg: String,
    pub rgba: Vec<u8>,
    pub png: Vec<u8>,
    pub rgba_sha256: String,
    pub png_sha256: String,
    pub non_white_pixels: usize,
    pub has_text_nodes: bool,
}

pub fn render(request: RenderRequest<'_>) -> Result<RenderResult> {
    let parser = liquid::ParserBuilder::new()
        .block(ForBlock)
        .block(IfBlock)
        .filter(Escape)
        .build()
        .context("build curated Liquid parser")?;
    let template = parser
        .parse(request.template)
        .context("parse Liquid-authored SVG template")?;
    let globals = liquid::to_object(request.context).context("convert JSON to Liquid object")?;
    let svg = template
        .render(&globals)
        .context("render Liquid template to SVG text")?;

    let asset_ids = request.assets.keys().cloned().collect();
    validate_svg(&svg, &asset_ids, request.require_direct_palette)?;

    let mut fontdb = usvg::fontdb::Database::new();
    fontdb.load_font_data(request.font_bytes.to_vec());
    ensure!(
        fontdb.faces().next().is_some(),
        "declared font bytes did not load"
    );
    fontdb.set_sans_serif_family("Inter");
    fontdb.set_serif_family("Inter");
    fontdb.set_monospace_family("Inter");

    let assets = request.assets;
    let options = usvg::Options {
        font_family: "Inter".to_owned(),
        fontdb: Arc::new(fontdb),
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(move |href, _| {
                assets.get(href).cloned().map(usvg::ImageKind::PNG)
            }),
        },
        ..usvg::Options::default()
    };

    let tree = usvg::Tree::from_str(&svg, &options).context("parse and normalize SVG")?;
    ensure!(
        tree.size().width() == WIDTH as f32 && tree.size().height() == HEIGHT as f32,
        "usvg tree was not exactly {WIDTH} x {HEIGHT}"
    );
    let has_text_nodes = tree.has_text_nodes();
    ensure!(has_text_nodes, "bundled-font text was not retained");

    let mut pixmap = Pixmap::new(WIDTH, HEIGHT).context("allocate fixed pixmap")?;
    pixmap.fill(Color::WHITE);
    resvg::render(&tree, Transform::identity(), &mut pixmap.as_mut());

    let rgba = pixmap.data().to_vec();
    ensure!(
        rgba.chunks_exact(4).all(|pixel| pixel[3] == 255),
        "render produced transparent pixels"
    );
    let non_white_pixels = rgba
        .chunks_exact(4)
        .filter(|pixel| pixel[..3] != [255, 255, 255])
        .count();
    ensure!(
        non_white_pixels > 10_000,
        "render is not meaningfully non-blank"
    );

    let png = encode_png(&rgba)?;
    let decoder = png::Decoder::new(Cursor::new(&png));
    let reader = decoder.read_info().context("decode emitted PNG header")?;
    ensure!(
        reader.info().width == WIDTH && reader.info().height == HEIGHT,
        "encoded PNG was not exactly {WIDTH} x {HEIGHT}"
    );

    Ok(RenderResult {
        rgba_sha256: sha256(&rgba),
        png_sha256: sha256(&png),
        svg,
        rgba,
        png,
        non_white_pixels,
        has_text_nodes,
    })
}

fn validate_svg(
    svg: &str,
    declared_assets: &BTreeSet<String>,
    require_direct_palette: bool,
) -> Result<()> {
    let document = usvg::roxmltree::Document::parse(svg).context("parse SVG XML for allowlist")?;
    let root = document.root_element();
    ensure!(root.tag_name().name() == "svg", "root element must be svg");
    ensure!(
        root.tag_name().namespace() == Some("http://www.w3.org/2000/svg"),
        "root SVG namespace is required"
    );
    ensure!(root.attribute("width") == Some("800"), "width must be 800");
    ensure!(
        root.attribute("height") == Some("480"),
        "height must be 480"
    );
    ensure!(
        root.attribute("viewBox") == Some("0 0 800 480"),
        "viewBox must be 0 0 800 480"
    );
    if require_direct_palette {
        ensure!(
            root.attribute("shape-rendering") == Some("crispEdges"),
            "root shape-rendering must be crispEdges"
        );
        ensure!(
            root.attribute("text-rendering") == Some("optimizeSpeed"),
            "root text-rendering must disable antialiasing"
        );
    }

    let allowed_elements: BTreeSet<&str> = [
        "svg", "g", "defs", "clipPath", "rect", "circle", "ellipse", "line", "polyline", "polygon",
        "path", "use", "text", "tspan", "image",
    ]
    .into_iter()
    .collect();
    let allowed_attributes: BTreeSet<&str> = [
        "id",
        "width",
        "height",
        "viewBox",
        "x",
        "y",
        "x1",
        "y1",
        "x2",
        "y2",
        "cx",
        "cy",
        "r",
        "rx",
        "ry",
        "points",
        "d",
        "href",
        "transform",
        "clip-path",
        "fill",
        "stroke",
        "stroke-width",
        "stroke-linecap",
        "stroke-linejoin",
        "shape-rendering",
        "text-rendering",
        "image-rendering",
        "font-family",
        "font-size",
        "font-weight",
        "font-style",
        "text-anchor",
        "letter-spacing",
        "preserveAspectRatio",
    ]
    .into_iter()
    .collect();

    for node in document.descendants().filter(|node| node.is_element()) {
        let element = node.tag_name().name();
        ensure!(
            allowed_elements.contains(element),
            "SVG element is outside prototype allowlist: {element}"
        );
        for attribute in node.attributes() {
            let name = attribute.name();
            ensure!(
                allowed_attributes.contains(name),
                "SVG attribute is outside prototype allowlist: {name}"
            );
            ensure!(
                !name.to_ascii_lowercase().starts_with("on"),
                "event attributes are forbidden"
            );
            if name == "href" {
                let href = attribute.value();
                if element == "use" {
                    ensure!(href.starts_with('#'), "use href must be a local fragment");
                } else if element == "image" {
                    ensure!(
                        declared_assets.contains(href),
                        "image href is not a declared in-memory asset: {href}"
                    );
                    ensure!(
                        !href.contains("..")
                            && !href.contains(':')
                            && !href.starts_with('/')
                            && !href.starts_with('\\'),
                        "image href is not extension-relative: {href}"
                    );
                } else {
                    bail!("href is only allowed on use and image");
                }
            }
            if matches!(name, "fill" | "stroke") {
                ensure!(
                    matches!(
                        attribute.value(),
                        "none" | "#000000" | "#ffffff" | "#cd2323"
                    ),
                    "vector paint is outside the panel palette: {}",
                    attribute.value()
                );
            }
        }
    }

    Ok(())
}

fn encode_png(rgba: &[u8]) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut output, WIDTH, HEIGHT);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        encoder.set_filter(png::Filter::Paeth);
        let mut writer = encoder.write_header().context("write PNG header")?;
        writer
            .write_image_data(rgba)
            .context("write fixed PNG pixels")?;
    }
    Ok(output)
}

fn sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
