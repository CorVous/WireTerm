//! Self-describing Lua Extension and pure-Rust SVG-to-panel boundary.
//!
//! An Extension is one `extension.lua` script plus relative local assets. The
//! script returns metadata, its input schema, and a `render(context)` function.
//! There is no Liquid stage, separate transform, or host-declared URL.

use std::{
    collections::BTreeMap,
    io::Cursor,
    path::{Path, PathBuf},
    str,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use image::{DynamicImage, ImageFormat};
use mlua::{
    Function, HookTriggers, Lua, LuaOptions, LuaSerdeExt, StdLib, Table, Value as LuaValue, VmState,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::{
    frame::{FrameError, PIXEL_COUNT, PanelColor, PanelFrame},
    playlist::is_safe_relative_asset_path,
    raster::dither_raster_asset,
};

pub const EXTENSION_SCRIPT_NAME: &str = "extension.lua";
pub const DEFAULT_HTTP_TIMEOUT: Duration = Duration::from_secs(15);
pub const MAX_HTTP_TIMEOUT: Duration = Duration::from_mins(1);
pub const MAX_RESPONSE_BYTES: usize = 5 * 1024 * 1024;
pub const MAX_SVG_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_LUA_TIME: Duration = Duration::from_secs(30);
pub const MAX_LUA_MEMORY_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExtensionMetadata {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub version: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExtensionInput {
    pub key: String,
    pub label: String,
    pub kind: InputKind,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub default: Value,
    #[serde(default)]
    pub choices: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InputKind {
    Text,
    Number,
    Checkbox,
    Choice,
    NamedSecret,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostHttpRequest {
    pub method: String,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    /// Header name to app-owned named-secret reference. Secret values are
    /// injected only by the eventual live HTTP host and never enter Lua.
    pub secret_headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
    pub timeout: Duration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostHttpResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostClock {
    pub unix_seconds: i64,
    pub utc_offset_minutes: i16,
}

#[derive(Debug, Error)]
pub enum HostApiError {
    #[error("HTTP request failed")]
    Http,
    #[error("HTTP response exceeded the 5 MiB limit")]
    ResponseTooLarge,
    #[error("named secret reference is not bound")]
    SecretNotBound,
    #[error("asset path must be local, relative, and contained by the Extension")]
    UnsafeAssetPath,
    #[error("local asset is unavailable")]
    MissingAsset,
}

/// Narrow capability surface supplied to a Lua Extension.
///
/// A live implementation may perform bounded HTTP and app-owned secret
/// injection. The first production slice intentionally ships a complete local
/// fixture implementation, not an unsafe partial live-network shortcut.
pub trait ExtensionHostApi: Send + Sync {
    fn http(&self, request: HostHttpRequest) -> Result<HostHttpResponse, HostApiError>;
    fn clock(&self) -> HostClock;
    fn asset(&self, relative_path: &Path) -> Result<PathBuf, HostApiError>;
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct FixtureRequestKey {
    method: String,
    url: String,
}

/// Deterministic host used by local previews, fixtures, and conformance tests.
pub struct LocalFixtureHost {
    extension_root: PathBuf,
    named_secret_refs: BTreeMap<String, String>,
    responses: BTreeMap<FixtureRequestKey, HostHttpResponse>,
    clock: HostClock,
    requests: Mutex<Vec<HostHttpRequest>>,
}

impl LocalFixtureHost {
    #[must_use]
    pub const fn new(
        extension_root: PathBuf,
        named_secret_refs: BTreeMap<String, String>,
        clock: HostClock,
    ) -> Self {
        Self {
            extension_root,
            named_secret_refs,
            responses: BTreeMap::new(),
            clock,
            requests: Mutex::new(Vec::new()),
        }
    }

    #[must_use]
    pub fn with_response(
        mut self,
        method: impl Into<String>,
        url: impl Into<String>,
        response: HostHttpResponse,
    ) -> Self {
        self.responses.insert(
            FixtureRequestKey {
                method: method.into().to_ascii_uppercase(),
                url: url.into(),
            },
            response,
        );
        self
    }

    #[must_use]
    pub fn requests(&self) -> Vec<HostHttpRequest> {
        self.requests.lock().map_or_else(
            |poisoned| poisoned.into_inner().clone(),
            |guard| guard.clone(),
        )
    }
}

impl ExtensionHostApi for LocalFixtureHost {
    fn http(&self, mut request: HostHttpRequest) -> Result<HostHttpResponse, HostApiError> {
        for logical_name in request.secret_headers.values_mut() {
            let Some(named_reference) = self.named_secret_refs.get(logical_name) else {
                return Err(HostApiError::SecretNotBound);
            };
            named_reference.clone_into(logical_name);
        }
        self.requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(request.clone());
        let key = FixtureRequestKey {
            method: request.method,
            url: request.url,
        };
        let response = self.responses.get(&key).ok_or(HostApiError::Http)?.clone();
        if response.body.len() > MAX_RESPONSE_BYTES {
            return Err(HostApiError::ResponseTooLarge);
        }
        Ok(response)
    }

    fn clock(&self) -> HostClock {
        self.clock.clone()
    }

    fn asset(&self, relative_path: &Path) -> Result<PathBuf, HostApiError> {
        if !is_safe_relative_asset_path(relative_path) {
            return Err(HostApiError::UnsafeAssetPath);
        }
        let path = self.extension_root.join(relative_path);
        if path.is_file() {
            Ok(path)
        } else {
            Err(HostApiError::MissingAsset)
        }
    }
}

#[derive(Debug, Error)]
pub enum ExtensionError {
    #[error("could not read Extension script")]
    ReadScript,
    #[error("Extension Lua failed: {0}")]
    Lua(String),
    #[error("Extension metadata is invalid: {0}")]
    Metadata(String),
    #[error("Extension renderer did not return UTF-8 SVG")]
    SvgNotUtf8,
    #[error("Extension renderer returned more than 2 MiB of SVG")]
    SvgTooLarge,
    #[error("Extension SVG is invalid: {0}")]
    Svg(String),
    #[error("Extension SVG must be a fixed 800 × 480 canvas")]
    WrongCanvas,
    #[error("Extension SVG assets must be local relative PNG or JPEG files")]
    UnsafeAsset,
    #[error("could not decode a local raster asset")]
    AssetDecode,
    #[error("could not encode a prepared raster asset")]
    AssetEncode,
    #[error(transparent)]
    Frame(#[from] FrameError),
}

pub struct LoadedExtension {
    pub metadata: ExtensionMetadata,
    pub inputs: Vec<ExtensionInput>,
    lua: Lua,
    render: mlua::RegistryKey,
    started: Instant,
}

impl LoadedExtension {
    pub fn load(
        script_path: &Path,
        host: Arc<dyn ExtensionHostApi>,
    ) -> Result<Self, ExtensionError> {
        let script = std::fs::read(script_path).map_err(|_| ExtensionError::ReadScript)?;
        let lua = Lua::new_with(StdLib::ALL_SAFE, LuaOptions::default())
            .map_err(|error| ExtensionError::Lua(safe_lua_error(&error)))?;
        lua.set_memory_limit(MAX_LUA_MEMORY_BYTES)
            .map_err(|error| ExtensionError::Lua(safe_lua_error(&error)))?;
        let started = Instant::now();
        install_render_limit(&lua, started)?;
        install_host_api(&lua, host)?;
        let module = lua
            .load(&script)
            .set_name(EXTENSION_SCRIPT_NAME)
            .eval::<Table>()
            .map_err(|error| ExtensionError::Lua(safe_lua_error(&error)))?;
        let metadata_value: LuaValue = module
            .get("metadata")
            .map_err(|error| ExtensionError::Metadata(safe_lua_error(&error)))?;
        let metadata = lua
            .from_value::<ExtensionMetadata>(metadata_value)
            .map_err(|error| ExtensionError::Metadata(safe_lua_error(&error)))?;
        validate_metadata(&metadata)?;
        let inputs_value: LuaValue = module
            .get("inputs")
            .map_err(|error| ExtensionError::Metadata(safe_lua_error(&error)))?;
        let inputs = lua
            .from_value::<Vec<ExtensionInput>>(inputs_value)
            .map_err(|error| ExtensionError::Metadata(safe_lua_error(&error)))?;
        validate_inputs(&inputs)?;
        let render: Function = module
            .get("render")
            .map_err(|error| ExtensionError::Metadata(safe_lua_error(&error)))?;
        let render = lua
            .create_registry_value(render)
            .map_err(|error| ExtensionError::Lua(safe_lua_error(&error)))?;
        Ok(Self {
            metadata,
            inputs,
            lua,
            render,
            started,
        })
    }

    pub fn render_svg(&self, settings: &BTreeMap<String, Value>) -> Result<String, ExtensionError> {
        if self.started.elapsed() > MAX_LUA_TIME {
            return Err(ExtensionError::Lua("render time limit exceeded".to_owned()));
        }
        let function = self
            .lua
            .registry_value::<Function>(&self.render)
            .map_err(|error| ExtensionError::Lua(safe_lua_error(&error)))?;
        let context = self
            .lua
            .create_table()
            .map_err(|error| ExtensionError::Lua(safe_lua_error(&error)))?;
        context
            .set(
                "settings",
                self.lua
                    .to_value(settings)
                    .map_err(|error| ExtensionError::Lua(safe_lua_error(&error)))?,
            )
            .map_err(|error| ExtensionError::Lua(safe_lua_error(&error)))?;
        let svg = function
            .call::<mlua::String>(context)
            .map_err(|error| ExtensionError::Lua(safe_lua_error(&error)))?;
        if svg.as_bytes().len() > MAX_SVG_BYTES {
            return Err(ExtensionError::SvgTooLarge);
        }
        str::from_utf8(&svg.as_bytes())
            .map(str::to_owned)
            .map_err(|_| ExtensionError::SvgNotUtf8)
    }
}

fn validate_metadata(metadata: &ExtensionMetadata) -> Result<(), ExtensionError> {
    let valid_id = !metadata.id.is_empty()
        && metadata
            .id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    if !valid_id || metadata.name.trim().is_empty() || metadata.version == 0 {
        return Err(ExtensionError::Metadata(
            "id, name, and positive version are required".to_owned(),
        ));
    }
    Ok(())
}

fn validate_inputs(inputs: &[ExtensionInput]) -> Result<(), ExtensionError> {
    let mut keys = std::collections::BTreeSet::new();
    for input in inputs {
        if input.key.trim().is_empty()
            || input.label.trim().is_empty()
            || !keys.insert(input.key.as_str())
            || (input.kind == InputKind::Choice && input.choices.is_empty())
        {
            return Err(ExtensionError::Metadata(
                "input keys and labels must be non-empty and unique".to_owned(),
            ));
        }
    }
    Ok(())
}

fn install_render_limit(lua: &Lua, started: Instant) -> Result<(), ExtensionError> {
    lua.set_hook(
        HookTriggers::new().every_nth_instruction(10_000),
        move |_, _| {
            if started.elapsed() > MAX_LUA_TIME {
                Err(mlua::Error::RuntimeError(
                    "render time limit exceeded".to_owned(),
                ))
            } else {
                Ok(VmState::Continue)
            }
        },
    )
    .map_err(|error| ExtensionError::Lua(safe_lua_error(&error)))
}

fn install_host_api(lua: &Lua, host: Arc<dyn ExtensionHostApi>) -> Result<(), ExtensionError> {
    let api = lua
        .create_table()
        .map_err(|error| ExtensionError::Lua(safe_lua_error(&error)))?;
    let http_host = Arc::clone(&host);
    api.set(
        "http",
        lua.create_function(move |lua, request: Table| {
            let request = parse_http_request(&request)?;
            let response = http_host
                .http(request)
                .map_err(|error| mlua::Error::RuntimeError(error.to_string()))?;
            response_to_lua(lua, response)
        })
        .map_err(|error| ExtensionError::Lua(safe_lua_error(&error)))?,
    )
    .map_err(|error| ExtensionError::Lua(safe_lua_error(&error)))?;
    let clock_host = Arc::clone(&host);
    api.set(
        "clock",
        lua.create_function(move |lua, ()| {
            let clock = clock_host.clock();
            let result = lua.create_table()?;
            result.set("unix_seconds", clock.unix_seconds)?;
            result.set("utc_offset_minutes", clock.utc_offset_minutes)?;
            Ok(result)
        })
        .map_err(|error| ExtensionError::Lua(safe_lua_error(&error)))?,
    )
    .map_err(|error| ExtensionError::Lua(safe_lua_error(&error)))?;
    api.set(
        "asset",
        lua.create_function(move |_, relative: String| {
            let path = PathBuf::from(&relative);
            host.asset(&path)
                .map_err(|error| mlua::Error::RuntimeError(error.to_string()))?;
            Ok(relative.replace('\\', "/"))
        })
        .map_err(|error| ExtensionError::Lua(safe_lua_error(&error)))?,
    )
    .map_err(|error| ExtensionError::Lua(safe_lua_error(&error)))?;
    lua.globals()
        .set("wireterm", api)
        .map_err(|error| ExtensionError::Lua(safe_lua_error(&error)))
}

fn parse_http_request(table: &Table) -> mlua::Result<HostHttpRequest> {
    let method = table
        .get::<Option<String>>("method")?
        .unwrap_or_else(|| "GET".to_owned())
        .to_ascii_uppercase();
    let url = table.get::<String>("url")?;
    let headers = lua_string_map(table.get::<Option<Table>>("headers")?)?;
    let secret_headers = lua_string_map(table.get::<Option<Table>>("secret_headers")?)?;
    let body = table
        .get::<Option<mlua::String>>("body")?
        .map_or_else(Vec::new, |value| value.as_bytes().to_vec());
    let timeout_ms = table.get::<Option<u64>>("timeout_ms")?.unwrap_or(15_000);
    let timeout = Duration::from_millis(timeout_ms).min(MAX_HTTP_TIMEOUT);
    Ok(HostHttpRequest {
        method,
        url,
        headers,
        secret_headers,
        body,
        timeout,
    })
}

fn lua_string_map(table: Option<Table>) -> mlua::Result<BTreeMap<String, String>> {
    let Some(table) = table else {
        return Ok(BTreeMap::new());
    };
    table.pairs::<String, String>().collect()
}

fn response_to_lua(lua: &Lua, response: HostHttpResponse) -> mlua::Result<Table> {
    if response.body.len() > MAX_RESPONSE_BYTES {
        return Err(mlua::Error::RuntimeError(
            HostApiError::ResponseTooLarge.to_string(),
        ));
    }
    let result = lua.create_table()?;
    result.set("status", response.status)?;
    let headers = lua.create_table()?;
    for (name, value) in response.headers {
        headers.set(name, value)?;
    }
    result.set("headers", headers)?;
    result.set("body", lua.create_string(response.body)?)?;
    Ok(result)
}

fn safe_lua_error(error: &mlua::Error) -> String {
    let text = error.to_string();
    if text.chars().count() > 500 {
        format!("{}…", text.chars().take(500).collect::<String>())
    } else {
        text
    }
}

/// Load one self-describing Lua script through a deterministic fixture host,
/// render its final SVG, and convert it to a display-ready panel frame.
pub fn render_local_fixture(
    script_path: &Path,
    settings: &BTreeMap<String, Value>,
    host: Arc<dyn ExtensionHostApi>,
) -> Result<(ExtensionMetadata, Vec<ExtensionInput>, PanelFrame), ExtensionError> {
    let extension = LoadedExtension::load(script_path, host)?;
    let svg = extension.render_svg(settings)?;
    let root = script_path.parent().ok_or(ExtensionError::ReadScript)?;
    let frame = render_svg_to_panel(&svg, root)?;
    Ok((extension.metadata, extension.inputs, frame))
}

/// Pure-Rust fixed SVG renderer with hybrid conversion.
///
/// Relative PNG/JPEG assets are Floyd–Steinberg dithered before `resvg`
/// composition. The composed vector/text result is quantized directly to the
/// panel palette without whole-frame error diffusion.
pub fn render_svg_to_panel(svg: &str, extension_root: &Path) -> Result<PanelFrame, ExtensionError> {
    if svg.len() > MAX_SVG_BYTES {
        return Err(ExtensionError::SvgTooLarge);
    }
    validate_svg_canvas(svg)?;
    let prepared_assets = prepare_svg_assets(svg, extension_root)?;
    let image_href_resolver = resvg::usvg::ImageHrefResolver {
        resolve_data: Box::new(|_, _, _| None),
        resolve_string: Box::new(move |href, _| {
            prepared_assets
                .get(href)
                .cloned()
                .map(resvg::usvg::ImageKind::PNG)
        }),
    };
    let mut options = resvg::usvg::Options {
        resources_dir: Some(extension_root.to_path_buf()),
        image_href_resolver,
        ..Default::default()
    };
    load_windows_ui_font(&mut options);
    let tree = resvg::usvg::Tree::from_str(svg, &options)
        .map_err(|error| ExtensionError::Svg(error.to_string()))?;
    let size = tree.size().to_int_size();
    if size.width() != 800 || size.height() != 480 {
        return Err(ExtensionError::WrongCanvas);
    }
    let mut pixmap = resvg::tiny_skia::Pixmap::new(800, 480)
        .ok_or_else(|| ExtensionError::Svg("could not allocate SVG canvas".to_owned()))?;
    pixmap.fill(resvg::tiny_skia::Color::WHITE);
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    let pixels = pixmap
        .pixels()
        .iter()
        .map(|pixel| nearest_panel_color(pixel.red(), pixel.green(), pixel.blue()))
        .collect::<Vec<_>>();
    debug_assert_eq!(pixels.len(), PIXEL_COUNT);
    PanelFrame::from_palette_pixels(&pixels).map_err(Into::into)
}

fn load_windows_ui_font(options: &mut resvg::usvg::Options<'_>) {
    let windows_directory =
        std::env::var_os("WINDIR").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
    if let Ok(bytes) = std::fs::read(windows_directory.join("Fonts").join("segoeui.ttf")) {
        options.fontdb_mut().load_font_data(bytes);
        "Segoe UI".clone_into(&mut options.font_family);
    }
}

fn validate_svg_canvas(svg: &str) -> Result<(), ExtensionError> {
    let document =
        roxmltree::Document::parse(svg).map_err(|error| ExtensionError::Svg(error.to_string()))?;
    let has_remote_reference = document.descendants().any(|node| {
        node.attributes().any(|attribute| {
            attribute.name() == "href"
                && (attribute.value().starts_with("http://")
                    || attribute.value().starts_with("https://"))
        }) || (node.tag_name().name() == "style"
            && node
                .text()
                .is_some_and(|text| text.contains("url(http://") || text.contains("url(https://")))
    });
    if has_remote_reference {
        return Err(ExtensionError::UnsafeAsset);
    }
    let root = document.root_element();
    if root.tag_name().name() != "svg"
        || !dimension_is(root.attribute("width"), 800.0)
        || !dimension_is(root.attribute("height"), 480.0)
        || !view_box_is_fixed(root.attribute("viewBox"))
    {
        return Err(ExtensionError::WrongCanvas);
    }
    Ok(())
}

fn dimension_is(value: Option<&str>, expected: f32) -> bool {
    value
        .and_then(|value| value.strip_suffix("px").or(Some(value)))
        .and_then(|value| value.parse::<f32>().ok())
        .is_some_and(|value| (value - expected).abs() < f32::EPSILON)
}

fn view_box_is_fixed(value: Option<&str>) -> bool {
    let Some(value) = value else {
        return false;
    };
    let values = value
        .split(|character: char| character.is_ascii_whitespace() || character == ',')
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse::<f32>().ok())
        .collect::<Vec<_>>();
    values == [0.0, 0.0, 800.0, 480.0]
}

fn prepare_svg_assets(
    svg: &str,
    extension_root: &Path,
) -> Result<BTreeMap<String, Arc<Vec<u8>>>, ExtensionError> {
    let document =
        roxmltree::Document::parse(svg).map_err(|error| ExtensionError::Svg(error.to_string()))?;
    let mut prepared = BTreeMap::new();
    for node in document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "image")
    {
        let href = node
            .attribute("href")
            .or_else(|| node.attribute(("http://www.w3.org/1999/xlink", "href")))
            .ok_or(ExtensionError::UnsafeAsset)?;
        let relative = Path::new(href);
        let valid_extension = relative
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                matches!(
                    extension.to_ascii_lowercase().as_str(),
                    "png" | "jpg" | "jpeg"
                )
            });
        if !is_safe_relative_asset_path(relative) || !valid_extension {
            return Err(ExtensionError::UnsafeAsset);
        }
        let width = asset_dimension(node.attribute("width"), 800)?;
        let height = asset_dimension(node.attribute("height"), 480)?;
        if prepared.contains_key(href) {
            return Err(ExtensionError::Svg(
                "each raster asset reference may appear only once".to_owned(),
            ));
        }
        let source = image::open(extension_root.join(relative))
            .map_err(|_| ExtensionError::AssetDecode)?
            .to_rgb8();
        let placed = image::imageops::resize(
            &source,
            width,
            height,
            image::imageops::FilterType::Lanczos3,
        );
        let dithered = dither_raster_asset(&placed);
        let mut encoded = Cursor::new(Vec::new());
        DynamicImage::ImageRgb8(dithered)
            .write_to(&mut encoded, ImageFormat::Png)
            .map_err(|_| ExtensionError::AssetEncode)?;
        prepared.insert(href.to_owned(), Arc::new(encoded.into_inner()));
    }
    Ok(prepared)
}

fn asset_dimension(value: Option<&str>, maximum: u32) -> Result<u32, ExtensionError> {
    let dimension = value
        .and_then(|value| value.strip_suffix("px").or(Some(value)))
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| (1..=maximum).contains(value))
        .ok_or_else(|| {
            ExtensionError::Svg(
                "raster assets require whole-pixel width and height within the canvas".to_owned(),
            )
        })?;
    Ok(dimension)
}

fn nearest_panel_color(red: u8, green: u8, blue: u8) -> PanelColor {
    let sample = [i32::from(red), i32::from(green), i32::from(blue)];
    [PanelColor::Black, PanelColor::White, PanelColor::Red]
        .into_iter()
        .min_by_key(|color| {
            let rgb = color.rgb().map(i32::from);
            let red = sample[0] - rgb[0];
            let green = sample[1] - rgb[1];
            let blue = sample[2] - rgb[2];
            red * red * 30 + green * green * 59 + blue * blue * 11
        })
        .expect("panel palette is non-empty")
}

#[must_use]
pub fn system_fixture_clock() -> HostClock {
    HostClock {
        unix_seconds: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| {
                i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
            }),
        utc_offset_minutes: 0,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{Duration, SystemTime},
    };

    use image::{Rgb, RgbImage};

    use super::*;

    struct TestExtension(PathBuf);

    impl TestExtension {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::ZERO)
                .as_nanos();
            let path = std::env::temp_dir()
                .join(format!("wireterm-extension-{}-{nonce}", std::process::id()));
            fs::create_dir_all(path.join("assets")).expect("fixture directory");
            Self(path)
        }
    }

    impl Drop for TestExtension {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn self_describing_lua_handles_arbitrary_fixture_payload_and_secret_reference() {
        let fixture = TestExtension::new();
        let script = r##"
            local module = {
              metadata = {
                id = "fixture-demo",
                name = "Fixture demo",
                description = "Local conformance path",
                version = 1,
              },
              inputs = {
                { key = "title", label = "Title", kind = "text", required = true },
                { key = "token", label = "API token", kind = "named_secret", required = true },
              },
            }
            function module.render(context)
              local response = wireterm.http({
                method = "POST",
                url = "fixture://arbitrary",
                headers = { ["Content-Type"] = "application/octet-stream" },
                secret_headers = { Authorization = "token" },
                body = "request bytes",
              })
              local asset = wireterm.asset("assets/pixel.png")
              local clock = wireterm.clock()
              local width = #response.body + (clock.unix_seconds - clock.unix_seconds)
              return string.format(
                '<svg xmlns="http://www.w3.org/2000/svg" width="800" height="480" viewBox="0 0 800 480"><rect width="800" height="480" fill="white"/><rect width="%d" height="10" fill="#cd2323"/><image href="%s" x="20" y="20" width="2" height="2"/></svg>',
                width,
                asset
              )
            end
            return module
        "##;
        let script_path = fixture.0.join(EXTENSION_SCRIPT_NAME);
        fs::write(&script_path, script).expect("script");
        RgbImage::from_pixel(2, 2, Rgb([0, 0, 0]))
            .save(fixture.0.join("assets/pixel.png"))
            .expect("asset");
        let host = Arc::new(
            LocalFixtureHost::new(
                fixture.0.clone(),
                BTreeMap::from([("token".to_owned(), "github-production".to_owned())]),
                HostClock {
                    unix_seconds: 1_700_000_000,
                    utc_offset_minutes: 0,
                },
            )
            .with_response(
                "POST",
                "fixture://arbitrary",
                HostHttpResponse {
                    status: 200,
                    headers: BTreeMap::from([(
                        "Content-Type".to_owned(),
                        "application/octet-stream".to_owned(),
                    )]),
                    body: vec![0, 159, 146, 150, 255],
                },
            ),
        );
        let (metadata, inputs, frame) = render_local_fixture(
            &script_path,
            &BTreeMap::from([("title".to_owned(), Value::String("Hello".to_owned()))]),
            host.clone(),
        )
        .expect("fixture render");

        assert_eq!(metadata.id, "fixture-demo");
        assert_eq!(inputs.len(), 2);
        assert_eq!(frame.preview_rgb()[..3], PanelColor::Red.rgb());
        assert_eq!(
            host.requests()[0].secret_headers["Authorization"],
            "github-production"
        );
        assert_eq!(host.requests()[0].body, b"request bytes");
    }

    #[test]
    fn fixed_svg_rejects_remote_and_parent_assets() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="800" height="480" viewBox="0 0 800 480"><image href="../secret.png"/></svg>"#;
        assert!(matches!(
            render_svg_to_panel(svg, Path::new(".")),
            Err(ExtensionError::UnsafeAsset)
        ));
        let remote = r#"<svg xmlns="http://www.w3.org/2000/svg" width="800" height="480" viewBox="0 0 800 480"><image href="https://example.com/a.png"/></svg>"#;
        assert!(matches!(
            render_svg_to_panel(remote, Path::new(".")),
            Err(ExtensionError::UnsafeAsset)
        ));
    }

    #[test]
    fn raster_assets_require_a_whole_pixel_placement() {
        let fixture = TestExtension::new();
        RgbImage::from_pixel(2, 2, Rgb([0, 0, 0]))
            .save(fixture.0.join("assets/pixel.png"))
            .expect("asset");
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="800" height="480" viewBox="0 0 800 480"><image href="assets/pixel.png" x="0" y="0"/></svg>"#;
        assert!(matches!(
            render_svg_to_panel(svg, &fixture.0),
            Err(ExtensionError::Svg(_))
        ));
    }

    #[test]
    fn vector_composition_uses_direct_palette_quantization() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="800" height="480" viewBox="0 0 800 480"><rect width="400" height="480" fill="#cd2323"/><rect x="400" width="400" height="480" fill="black"/></svg>"##;
        let frame = render_svg_to_panel(svg, Path::new(".")).expect("render");
        assert_eq!(&frame.preview_rgb()[..3], &PanelColor::Red.rgb());
        let black = (400 * 3) as usize;
        assert_eq!(
            &frame.preview_rgb()[black..black + 3],
            &PanelColor::Black.rgb()
        );
    }
}
