//! Self-describing Lua Extension and pure-Rust SVG-to-panel boundary.
//!
//! An Extension is one `extension.lua` script plus relative local assets. The
//! script returns metadata, its input schema, and a `render(context)` function.
//! There is no Liquid stage, separate transform, or host-declared URL.

use std::{
    collections::BTreeMap,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    str,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
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
const BUNDLED_FONT_BYTES: &[u8] = include_bytes!("../assets/fonts/Inter.ttf");
pub const BUNDLED_FONT_FAMILY: &str = "Inter";
const EXAMPLE_EXTENSION_LUA: &str = include_str!("../examples/http-extension/extension.lua");

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
    Secret,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostHttpRequest {
    pub method: String,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    /// Header name to a sensitive value supplied by the Extension. These
    /// values are never included in host logs or cross-origin redirects.
    pub secret_headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
    pub timeout: Duration,
    pub max_redirects: u8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostHttpResponse {
    pub status: u16,
    pub headers: BTreeMap<String, Vec<u8>>,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostClock {
    pub unix_seconds: i64,
    pub utc_offset_minutes: i16,
}

#[derive(Debug, Error)]
pub enum HostApiError {
    #[error("HTTP request is invalid")]
    InvalidHttp,
    #[error("HTTP request failed")]
    Http,
    #[error("host capability is unavailable while loading Extension inputs")]
    SchemaLoad,
    #[error("HTTP request timed out")]
    Timeout,
    #[error("Extension render was cancelled")]
    Cancelled,
    #[error("Extension render time limit exceeded")]
    TimeLimit,
    #[error("HTTP response exceeded the 5 MiB limit")]
    ResponseTooLarge,
    #[error("asset path must be local, relative, and contained by the Extension")]
    UnsafeAssetPath,
    #[error("local asset is unavailable")]
    MissingAsset,
}

/// Cooperative cancellation shared by the foreground app and render worker.
#[derive(Clone, Debug, Default)]
pub struct RenderCancellation(Arc<AtomicBool>);

impl RenderCancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
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
    fn is_cancelled(&self) -> bool {
        false
    }
}

struct SchemaExtensionHost {
    extension_root: PathBuf,
}

impl ExtensionHostApi for SchemaExtensionHost {
    fn http(&self, _: HostHttpRequest) -> Result<HostHttpResponse, HostApiError> {
        Err(HostApiError::SchemaLoad)
    }

    fn clock(&self) -> HostClock {
        HostClock {
            unix_seconds: 0,
            utc_offset_minutes: 0,
        }
    }

    fn asset(&self, relative_path: &Path) -> Result<PathBuf, HostApiError> {
        contained_asset(&self.extension_root, relative_path)
    }
}

/// Production host for one Extension render.
///
/// HTTP is deliberately synchronous because the whole Lua render already runs
/// on an app-owned worker thread. Every request is bounded by both its declared
/// timeout and the render's remaining overall budget.
pub struct LiveExtensionHost {
    extension_root: PathBuf,
    clock: HostClock,
    started: Instant,
    cancellation: RenderCancellation,
}

impl LiveExtensionHost {
    #[must_use]
    pub const fn new(
        extension_root: PathBuf,
        clock: HostClock,
        started: Instant,
        cancellation: RenderCancellation,
    ) -> Self {
        Self {
            extension_root,
            clock,
            started,
            cancellation,
        }
    }

    fn remaining(&self) -> Result<Duration, HostApiError> {
        if self.cancellation.is_cancelled() {
            return Err(HostApiError::Cancelled);
        }
        MAX_LUA_TIME
            .checked_sub(self.started.elapsed())
            .filter(|remaining| !remaining.is_zero())
            .ok_or(HostApiError::TimeLimit)
    }

    fn resolve_secret_headers(
        values: BTreeMap<String, String>,
    ) -> Result<Vec<(reqwest::header::HeaderName, reqwest::header::HeaderValue)>, HostApiError>
    {
        values
            .into_iter()
            .map(|(header_name, value)| {
                let name = reqwest::header::HeaderName::from_bytes(header_name.as_bytes())
                    .map_err(|_| HostApiError::InvalidHttp)?;
                let mut header = reqwest::header::HeaderValue::from_bytes(value.as_bytes())
                    .map_err(|_| HostApiError::InvalidHttp)?;
                header.set_sensitive(true);
                Ok((name, header))
            })
            .collect()
    }
}

impl ExtensionHostApi for LiveExtensionHost {
    fn http(&self, request: HostHttpRequest) -> Result<HostHttpResponse, HostApiError> {
        let timeout = request.timeout.min(MAX_HTTP_TIMEOUT).min(self.remaining()?);
        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|_| HostApiError::InvalidHttp)?;
        let url = reqwest::Url::parse(&request.url).map_err(|_| HostApiError::InvalidHttp)?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(HostApiError::InvalidHttp);
        }
        let has_secret_headers = !request.secret_headers.is_empty();
        let redirect = if request.max_redirects == 0 {
            reqwest::redirect::Policy::none()
        } else {
            let maximum = usize::from(request.max_redirects.min(10));
            reqwest::redirect::Policy::custom(move |attempt| {
                if attempt.previous().len() >= maximum {
                    attempt.error("redirect limit exceeded")
                } else if has_secret_headers
                    && attempt.previous().last().is_some_and(|previous| {
                        attempt.url().scheme() != previous.scheme()
                            || attempt.url().host_str() != previous.host_str()
                            || attempt.url().port_or_known_default()
                                != previous.port_or_known_default()
                    })
                {
                    attempt.stop()
                } else {
                    attempt.follow()
                }
            })
        };
        let client = reqwest::blocking::Client::builder()
            .redirect(redirect)
            .referer(false)
            .timeout(timeout)
            .user_agent(concat!("WireTerm/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| HostApiError::Http)?;
        let mut builder = client.request(method, url).body(request.body);
        for (name, value) in request.headers {
            let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| HostApiError::InvalidHttp)?;
            let value = reqwest::header::HeaderValue::from_str(&value)
                .map_err(|_| HostApiError::InvalidHttp)?;
            builder = builder.header(name, value);
        }
        for (name, value) in Self::resolve_secret_headers(request.secret_headers)? {
            builder = builder.header(name, value);
        }
        let mut response = builder.send().map_err(|error| {
            if error.is_timeout() {
                HostApiError::Timeout
            } else {
                HostApiError::Http
            }
        })?;
        if response
            .content_length()
            .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
        {
            return Err(HostApiError::ResponseTooLarge);
        }
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| (name.as_str().to_owned(), value.as_bytes().to_vec()))
            .collect();
        let mut body = Vec::new();
        response
            .by_ref()
            .take((MAX_RESPONSE_BYTES + 1) as u64)
            .read_to_end(&mut body)
            .map_err(|_| HostApiError::Http)?;
        if body.len() > MAX_RESPONSE_BYTES {
            return Err(HostApiError::ResponseTooLarge);
        }
        self.remaining()?;
        Ok(HostHttpResponse {
            status,
            headers,
            body,
        })
    }

    fn clock(&self) -> HostClock {
        self.clock.clone()
    }

    fn asset(&self, relative_path: &Path) -> Result<PathBuf, HostApiError> {
        contained_asset(&self.extension_root, relative_path)
    }

    fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct FixtureRequestKey {
    method: String,
    url: String,
}

/// Deterministic host used by local previews, fixtures, and conformance tests.
pub struct LocalFixtureHost {
    extension_root: PathBuf,
    responses: BTreeMap<FixtureRequestKey, HostHttpResponse>,
    clock: HostClock,
    requests: Mutex<Vec<HostHttpRequest>>,
}

impl LocalFixtureHost {
    #[must_use]
    pub const fn new(extension_root: PathBuf, clock: HostClock) -> Self {
        Self {
            extension_root,
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
    fn http(&self, request: HostHttpRequest) -> Result<HostHttpResponse, HostApiError> {
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
        contained_asset(&self.extension_root, relative_path)
    }
}

fn contained_asset(root: &Path, relative_path: &Path) -> Result<PathBuf, HostApiError> {
    if !is_safe_relative_asset_path(relative_path) {
        return Err(HostApiError::UnsafeAssetPath);
    }
    let root = root
        .canonicalize()
        .map_err(|_| HostApiError::MissingAsset)?;
    let path = root
        .join(relative_path)
        .canonicalize()
        .map_err(|_| HostApiError::MissingAsset)?;
    if path.is_file() && path.starts_with(root) {
        Ok(path)
    } else {
        Err(HostApiError::UnsafeAssetPath)
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
    #[error("Extension settings are invalid: {0}")]
    Configuration(String),
    #[error("Extension library could not be read")]
    LibraryRead,
    #[error("Extension could not be scaffolded")]
    Scaffold,
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
        let libraries =
            StdLib::COROUTINE | StdLib::TABLE | StdLib::STRING | StdLib::UTF8 | StdLib::MATH;
        let lua = Lua::new_with(libraries, LuaOptions::default())
            .map_err(|error| ExtensionError::Lua(safe_lua_error(&error)))?;
        for name in [
            "io", "os", "package", "debug", "dofile", "loadfile", "require",
        ] {
            lua.globals()
                .set(name, LuaValue::Nil)
                .map_err(|error| ExtensionError::Lua(safe_lua_error(&error)))?;
        }
        lua.set_memory_limit(MAX_LUA_MEMORY_BYTES)
            .map_err(|error| ExtensionError::Lua(safe_lua_error(&error)))?;
        let started = Instant::now();
        install_render_limit(&lua, started, Arc::clone(&host))?;
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

/// Load and validate an Extension's declared metadata and inputs without
/// calling its renderer or granting live HTTP access.
pub fn load_extension_schema(
    script_path: &Path,
) -> Result<(ExtensionMetadata, Vec<ExtensionInput>), ExtensionError> {
    let extension_root = script_path.parent().ok_or(ExtensionError::ReadScript)?;
    let host = Arc::new(SchemaExtensionHost {
        extension_root: extension_root.to_path_buf(),
    });
    let extension = LoadedExtension::load(script_path, host)?;
    Ok((extension.metadata, extension.inputs))
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
        if !input.default.is_null() && !value_matches_kind(&input.default, input.kind) {
            return Err(ExtensionError::Metadata(
                "input defaults must match their declared kinds".to_owned(),
            ));
        }
    }
    Ok(())
}

fn value_matches_kind(value: &Value, kind: InputKind) -> bool {
    match kind {
        InputKind::Text | InputKind::Choice | InputKind::Secret => value.is_string(),
        InputKind::Number => value.is_number(),
        InputKind::Checkbox => value.is_boolean(),
    }
}

/// Resolve defaults and validate all Extension-owned settings.
pub fn validate_extension_configuration(
    inputs: &[ExtensionInput],
    settings: &BTreeMap<String, Value>,
) -> Result<BTreeMap<String, Value>, ExtensionError> {
    let declared = inputs
        .iter()
        .map(|input| input.key.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    if settings.keys().any(|key| !declared.contains(key.as_str())) {
        return Err(ExtensionError::Configuration(
            "a saved setting is not declared by the Extension".to_owned(),
        ));
    }
    let mut resolved = BTreeMap::new();
    for input in inputs {
        let value = settings
            .get(&input.key)
            .filter(|value| !value.is_null())
            .cloned()
            .or_else(|| (!input.default.is_null()).then(|| input.default.clone()));
        let Some(value) = value else {
            if input.required {
                return Err(ExtensionError::Configuration(
                    "a required input has no value".to_owned(),
                ));
            }
            continue;
        };
        if (input.required
            && matches!(
                input.kind,
                InputKind::Text | InputKind::Choice | InputKind::Secret
            )
            && value.as_str().is_some_and(str::is_empty))
            || !value_matches_kind(&value, input.kind)
            || (input.kind == InputKind::Choice
                && value
                    .as_str()
                    .is_none_or(|choice| !input.choices.iter().any(|item| item == choice)))
        {
            return Err(ExtensionError::Configuration(
                "an input value does not match its declared schema".to_owned(),
            ));
        }
        resolved.insert(input.key.clone(), value);
    }
    Ok(resolved)
}

/// Discover direct child folders containing exactly one conventional script.
pub fn discover_extensions(data_dir: &Path) -> Result<Vec<PathBuf>, ExtensionError> {
    let library = data_dir.join("extensions");
    let entries = match std::fs::read_dir(library) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err(ExtensionError::LibraryRead),
    };
    let mut roots = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir() && path.join(EXTENSION_SCRIPT_NAME).is_file())
        .collect::<Vec<_>>();
    roots.sort_unstable();
    Ok(roots)
}

/// Create an editable HTTP example under the adjacent Extension library.
pub fn scaffold_extension(data_dir: &Path) -> Result<PathBuf, ExtensionError> {
    let library = data_dir.join("extensions");
    std::fs::create_dir_all(&library).map_err(|_| ExtensionError::Scaffold)?;
    let root = (1_u16..=999)
        .map(|suffix| {
            if suffix == 1 {
                library.join("http-extension")
            } else {
                library.join(format!("http-extension-{suffix}"))
            }
        })
        .find(|candidate| !candidate.exists())
        .ok_or(ExtensionError::Scaffold)?;
    std::fs::create_dir(&root).map_err(|_| ExtensionError::Scaffold)?;
    let temporary = root.join("extension.lua.tmp");
    std::fs::write(&temporary, EXAMPLE_EXTENSION_LUA).map_err(|_| ExtensionError::Scaffold)?;
    std::fs::rename(temporary, root.join(EXTENSION_SCRIPT_NAME))
        .map_err(|_| ExtensionError::Scaffold)?;
    Ok(root)
}

fn install_render_limit(
    lua: &Lua,
    started: Instant,
    host: Arc<dyn ExtensionHostApi>,
) -> Result<(), ExtensionError> {
    lua.set_hook(
        HookTriggers::new().every_nth_instruction(10_000),
        move |_, _| {
            if host.is_cancelled() {
                Err(mlua::Error::RuntimeError(
                    "Extension render was cancelled".to_owned(),
                ))
            } else if started.elapsed() > MAX_LUA_TIME {
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
    let timeout_ms = table
        .get::<Option<u64>>("timeout_ms")?
        .unwrap_or(15_000)
        .clamp(1, 60_000);
    let timeout = Duration::from_millis(timeout_ms).min(MAX_HTTP_TIMEOUT);
    let max_redirects = table
        .get::<Option<u8>>("max_redirects")?
        .unwrap_or_default()
        .min(10);
    Ok(HostHttpRequest {
        method,
        url,
        headers,
        secret_headers,
        body,
        timeout,
        max_redirects,
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
        headers.set(name, lua.create_string(value)?)?;
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
    load_bundled_font(&mut options);
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

fn load_bundled_font(options: &mut resvg::usvg::Options<'_>) {
    let database = options.fontdb_mut();
    database.load_font_data(BUNDLED_FONT_BYTES.to_vec());
    database.set_serif_family(BUNDLED_FONT_FAMILY);
    database.set_sans_serif_family(BUNDLED_FONT_FAMILY);
    database.set_cursive_family(BUNDLED_FONT_FAMILY);
    database.set_fantasy_family(BUNDLED_FONT_FAMILY);
    database.set_monospace_family(BUNDLED_FONT_FAMILY);
    BUNDLED_FONT_FAMILY.clone_into(&mut options.font_family);
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
        let asset_path =
            contained_asset(extension_root, relative).map_err(|_| ExtensionError::UnsafeAsset)?;
        let source = image::open(asset_path)
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
        io::{Read as _, Write as _},
        net::TcpListener,
        sync::mpsc,
        thread,
        time::{Duration, SystemTime},
    };

    use image::{Rgb, RgbImage};

    use super::*;

    const GITHUB_OPEN_PRS_LUA: &str =
        include_str!("../examples/extensions/github-open-prs/extension.lua");

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

    fn live_host(root: &Path, cancellation: RenderCancellation) -> LiveExtensionHost {
        LiveExtensionHost::new(
            root.to_path_buf(),
            HostClock {
                unix_seconds: 1_700_000_000,
                utc_offset_minutes: 0,
            },
            Instant::now(),
            cancellation,
        )
    }

    fn request(url: String) -> HostHttpRequest {
        HostHttpRequest {
            method: "GET".to_owned(),
            url,
            headers: BTreeMap::new(),
            secret_headers: BTreeMap::new(),
            body: Vec::new(),
            timeout: Duration::from_secs(2),
            max_redirects: 0,
        }
    }

    fn serve_responses(
        responses: Vec<Vec<u8>>,
    ) -> (String, mpsc::Receiver<Vec<Vec<u8>>>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
        let address = listener.local_addr().expect("fixture address");
        let (requests_receiver, handle) = serve_on(listener, responses);
        (format!("http://{address}"), requests_receiver, handle)
    }

    fn serve_on(
        listener: TcpListener,
        responses: Vec<Vec<u8>>,
    ) -> (mpsc::Receiver<Vec<Vec<u8>>>, thread::JoinHandle<()>) {
        let (requests_sender, requests_receiver) = mpsc::channel();
        let handle = thread::spawn(move || {
            let mut requests = Vec::new();
            for response in responses {
                let (mut stream, _) = listener.accept().expect("accept fixture request");
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .expect("read timeout");
                let mut bytes = Vec::new();
                let mut chunk = [0_u8; 4096];
                loop {
                    let count = stream.read(&mut chunk).unwrap_or_default();
                    if count == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&chunk[..count]);
                    if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                requests.push(bytes);
                stream.write_all(&response).expect("fixture response");
            }
            let _ = requests_sender.send(requests);
        });
        (requests_receiver, handle)
    }

    #[test]
    fn schema_loader_returns_every_declared_field_without_calling_render() {
        let fixture = TestExtension::new();
        let script = r#"
            return {
              metadata = { id = "schema-demo", name = "Schema demo", version = 1 },
              inputs = {
                { key = "title", label = "Title", kind = "text", required = true, default = "Hello" },
                { key = "count", label = "Count", kind = "number", required = false, default = 3 },
                { key = "enabled", label = "Enabled", kind = "checkbox", required = false, default = true },
                { key = "style", label = "Style", kind = "choice", required = true, default = "plain", choices = { "plain", "bold" } },
                { key = "token", label = "Token", kind = "secret", required = true },
              },
              render = function()
                error("schema loading must not call render")
              end,
            }
        "#;
        let script_path = fixture.0.join(EXTENSION_SCRIPT_NAME);
        fs::write(&script_path, script).expect("schema script");

        let (metadata, inputs) = load_extension_schema(&script_path).expect("load inputs");

        assert_eq!(metadata.id, "schema-demo");
        assert_eq!(inputs.len(), 5);
        assert_eq!(inputs[0].kind, InputKind::Text);
        assert_eq!(inputs[1].kind, InputKind::Number);
        assert_eq!(inputs[2].kind, InputKind::Checkbox);
        assert_eq!(inputs[3].kind, InputKind::Choice);
        assert_eq!(inputs[4].kind, InputKind::Secret);
    }

    #[test]
    fn schema_loader_does_not_grant_top_level_http_access() {
        let fixture = TestExtension::new();
        let script = r#"
            wireterm.http({ method = "GET", url = "https://example.invalid" })
            return {
              metadata = { id = "unsafe-schema", name = "Unsafe schema", version = 1 },
              inputs = {},
              render = function() return "<svg/>" end,
            }
        "#;
        let script_path = fixture.0.join(EXTENSION_SCRIPT_NAME);
        fs::write(&script_path, script).expect("schema script");

        let error = load_extension_schema(&script_path).expect_err("HTTP must be unavailable");

        assert!(
            error
                .to_string()
                .contains("host capability is unavailable while loading Extension inputs")
        );
    }

    #[test]
    fn self_describing_lua_handles_arbitrary_fixture_payload_and_secret_input() {
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
                { key = "token", label = "API token", kind = "secret", required = true },
              },
            }
            function module.render(context)
              local response = wireterm.http({
                method = "POST",
                url = "fixture://arbitrary",
                headers = { ["Content-Type"] = "application/octet-stream" },
                secret_headers = { Authorization = context.settings.token },
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
                        b"application/octet-stream".to_vec(),
                    )]),
                    body: vec![0, 159, 146, 150, 255],
                },
            ),
        );
        let (metadata, inputs, frame) = render_local_fixture(
            &script_path,
            &BTreeMap::from([
                ("title".to_owned(), Value::String("Hello".to_owned())),
                (
                    "token".to_owned(),
                    Value::String("github-production".to_owned()),
                ),
            ]),
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

    #[test]
    fn live_http_preserves_response_bytes_and_enforces_redirect_policy() {
        let fixture = TestExtension::new();
        let final_body = vec![0, 159, 146, 150, 255];
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
        let address = listener.local_addr().expect("fixture address");
        let location = format!("http://{address}/final");
        let redirect = format!(
            "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
        .into_bytes();
        let final_response = [
            b"HTTP/1.1 207 Multi-Status\r\nX-WireTerm: bytes\r\nContent-Length: 5\r\nConnection: close\r\n\r\n"
                .as_slice(),
            final_body.as_slice(),
        ]
        .concat();
        let (requests, server) =
            serve_on(listener, vec![redirect.clone(), redirect, final_response]);
        let base = format!("http://{address}");
        let host = live_host(&fixture.0, RenderCancellation::default());

        let denied = host
            .http(request(format!("{base}/redirect")))
            .expect("302 response");
        assert_eq!(denied.status, 302);
        let mut followed_request = request(format!("{base}/redirect"));
        followed_request.max_redirects = 2;
        let followed = host.http(followed_request).expect("followed response");
        assert_eq!(followed.status, 207);
        assert_eq!(followed.headers["x-wireterm"], b"bytes");
        assert_eq!(followed.body, final_body);
        server.join().expect("fixture server");
        assert_eq!(requests.recv().expect("requests").len(), 3);
    }

    #[test]
    fn live_http_caps_response_size_and_timeout() {
        let fixture = TestExtension::new();
        let oversized = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            MAX_RESPONSE_BYTES + 1
        )
        .into_bytes();
        let (base, _, server) = serve_responses(vec![oversized]);
        let host = live_host(&fixture.0, RenderCancellation::default());
        assert!(matches!(
            host.http(request(base)),
            Err(HostApiError::ResponseTooLarge)
        ));
        server.join().expect("oversize server");

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind timeout server");
        let address = listener.local_addr().expect("timeout address");
        let timeout_server = thread::spawn(move || {
            let (_stream, _) = listener.accept().expect("timeout accept");
            thread::sleep(Duration::from_millis(150));
        });
        let mut timed = request(format!("http://{address}/slow"));
        timed.timeout = Duration::from_millis(25);
        assert!(matches!(host.http(timed), Err(HostApiError::Timeout)));
        timeout_server.join().expect("timeout server");
    }

    #[test]
    fn sensitive_header_is_sent_without_appearing_in_errors() {
        let fixture = TestExtension::new();
        let (base, requests, server) = serve_responses(vec![
            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
        ]);
        let host = live_host(&fixture.0, RenderCancellation::default());
        let mut http_request = request(base);
        http_request.secret_headers =
            BTreeMap::from([("Authorization".to_owned(), "never-log-this".to_owned())]);
        host.http(http_request).expect("secret request");
        server.join().expect("secret server");
        let wire = requests.recv().expect("captured requests").remove(0);
        assert!(wire.windows(14).any(|window| window == b"never-log-this"));
        assert_eq!(HostApiError::Http.to_string(), "HTTP request failed");
    }

    #[test]
    fn lua_sandbox_exposes_only_the_narrow_host_capabilities_and_cancels() {
        let fixture = TestExtension::new();
        let script = r#"
          assert(io == nil and os == nil and package == nil and debug == nil)
          assert(dofile == nil and loadfile == nil and require == nil)
          return {
            metadata = { id = "sandbox", name = "Sandbox", version = 1 },
            inputs = {},
            render = function()
              while true do end
            end,
          }
        "#;
        let script_path = fixture.0.join(EXTENSION_SCRIPT_NAME);
        fs::write(&script_path, script).expect("sandbox script");
        let cancellation = RenderCancellation::default();
        cancellation.cancel();
        let host = Arc::new(live_host(&fixture.0, cancellation));
        let extension = LoadedExtension::load(&script_path, host).expect("load sandbox");
        let error = extension
            .render_svg(&BTreeMap::new())
            .expect_err("cancelled loop");
        assert!(error.to_string().contains("cancelled"));
    }

    #[test]
    fn configuration_resolves_defaults_and_extension_owned_secrets() {
        let inputs = vec![
            ExtensionInput {
                key: "title".to_owned(),
                label: "Title".to_owned(),
                kind: InputKind::Text,
                required: true,
                default: Value::String("Default title".to_owned()),
                choices: Vec::new(),
            },
            ExtensionInput {
                key: "token".to_owned(),
                label: "Token".to_owned(),
                kind: InputKind::Secret,
                required: true,
                default: Value::Null,
                choices: Vec::new(),
            },
        ];
        let settings = BTreeMap::from([(
            "token".to_owned(),
            Value::String("plain-local-secret".to_owned()),
        )]);
        let resolved =
            validate_extension_configuration(&inputs, &settings).expect("valid configuration");
        assert_eq!(resolved["title"], "Default title");
        assert_eq!(resolved["token"], "plain-local-secret");

        assert!(matches!(
            validate_extension_configuration(&inputs, &BTreeMap::new()),
            Err(ExtensionError::Configuration(_))
        ));
    }

    #[test]
    fn secret_header_redirect_does_not_cross_origins() {
        let fixture = TestExtension::new();
        let destination = TcpListener::bind("127.0.0.1:0").expect("destination");
        destination.set_nonblocking(true).expect("nonblocking");
        let destination_url = format!(
            "http://{}/target",
            destination.local_addr().expect("destination address")
        );
        let redirect = format!(
            "HTTP/1.1 302 Found\r\nLocation: {destination_url}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
        .into_bytes();
        let (origin, _, server) = serve_responses(vec![redirect]);
        let host = live_host(&fixture.0, RenderCancellation::default());
        let mut http_request = request(origin);
        http_request.max_redirects = 2;
        http_request.secret_headers =
            BTreeMap::from([("X-Api-Key".to_owned(), "never-forward".to_owned())]);
        assert_eq!(
            host.http(http_request).expect("stopped redirect").status,
            302
        );
        server.join().expect("origin server");
        assert_eq!(
            destination
                .accept()
                .expect_err("no cross-origin request")
                .kind(),
            std::io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn bundled_font_is_the_only_extension_font_and_renders_text() {
        let mut options = resvg::usvg::Options::default();
        load_bundled_font(&mut options);
        assert_eq!(options.fontdb.faces().count(), 1);
        assert_eq!(options.font_family, BUNDLED_FONT_FAMILY);
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="800" height="480" viewBox="0 0 800 480"><rect width="800" height="480" fill="white"/><text x="20" y="80" font-family="Missing System Font" font-size="48" fill="black">Bundled</text></svg>"#;
        let frame = render_svg_to_panel(svg, Path::new(".")).expect("font render");
        assert!(
            frame
                .preview_rgb()
                .chunks_exact(3)
                .any(|pixel| pixel == PanelColor::Black.rgb())
        );
    }

    #[test]
    fn github_open_prs_example_renders_fixture_with_extension_owned_token() {
        let fixture = TestExtension::new();
        let script_path = fixture.0.join(EXTENSION_SCRIPT_NAME);
        fs::write(&script_path, GITHUB_OPEN_PRS_LUA).expect("GitHub example script");
        let url = "https://api.github.com/search/issues?q=type%3Apr%20state%3Aopen%20author%3ACorVous&sort=updated&order=desc&per_page=5";
        let response = br#"{
          "total_count": 7,
          "incomplete_results": false,
          "items": [
            {
              "number": 42,
              "title": "Fix <parser> & \"quotes\" \ud83d\ude80",
              "updated_at": "2026-07-30T12:00:00Z",
              "repository_url": "https://api.github.com/repos/example/private-repo"
            },
            {
              "number": 7,
              "title": null,
              "updated_at": null,
              "repository_url": null
            }
          ]
        }"#;
        let secret_value = "fixture-github-authorization";
        let host = Arc::new(
            LocalFixtureHost::new(
                fixture.0.clone(),
                HostClock {
                    unix_seconds: 1_785_484_800,
                    utc_offset_minutes: 0,
                },
            )
            .with_response(
                "GET",
                url,
                HostHttpResponse {
                    status: 200,
                    headers: BTreeMap::from([("x-ratelimit-remaining".to_owned(), b"29".to_vec())]),
                    body: response.to_vec(),
                },
            ),
        );
        let extension =
            LoadedExtension::load(&script_path, host.clone()).expect("load GitHub example");
        let settings = BTreeMap::from([
            ("username".to_owned(), Value::String("CorVous".to_owned())),
            (
                "github_token".to_owned(),
                Value::String(secret_value.to_owned()),
            ),
        ]);
        let resolved = validate_extension_configuration(&extension.inputs, &settings)
            .expect("valid GitHub example settings");
        let svg = extension.render_svg(&resolved).expect("GitHub fixture SVG");

        assert!(svg.contains("example/private-repo"));
        assert!(svg.contains("#42"));
        assert!(svg.contains("Fix &lt;parser&gt; &amp; &quot;quotes&quot; 🚀"));
        assert!(svg.contains("(untitled pull request)"));
        assert!(!svg.contains(secret_value));
        assert_eq!(resolved["github_token"], secret_value);
        let frame = render_svg_to_panel(&svg, &fixture.0).expect("GitHub fixture frame");
        assert_eq!(frame.payload().len(), crate::frame::FRAME_BYTES);

        let requests = host.requests();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.method, "GET");
        assert_eq!(request.url, url);
        assert_eq!(request.headers["Accept"], "application/vnd.github+json");
        assert_eq!(request.headers["X-GitHub-Api-Version"], "2022-11-28");
        assert_eq!(
            request.headers["User-Agent"],
            "WireTerm-GitHub-Open-PRs/1.0"
        );
        assert_eq!(
            request.secret_headers["Authorization"],
            format!("Bearer {secret_value}")
        );
        assert!(!request.url.contains(secret_value));
    }

    #[test]
    fn github_open_prs_example_handles_empty_and_rate_limited_responses() {
        let fixture = TestExtension::new();
        let script_path = fixture.0.join(EXTENSION_SCRIPT_NAME);
        fs::write(&script_path, GITHUB_OPEN_PRS_LUA).expect("GitHub example script");
        let url = "https://api.github.com/search/issues?q=type%3Apr%20state%3Aopen%20author%3ACorVous&sort=updated&order=desc&per_page=5";
        let settings = BTreeMap::from([
            ("username".to_owned(), Value::String("CorVous".to_owned())),
            (
                "github_token".to_owned(),
                Value::String("fixture-github-authorization".to_owned()),
            ),
        ]);

        let empty_host = Arc::new(
            LocalFixtureHost::new(
                fixture.0.clone(),
                HostClock {
                    unix_seconds: 1_785_484_800,
                    utc_offset_minutes: 0,
                },
            )
            .with_response(
                "GET",
                url,
                HostHttpResponse {
                    status: 200,
                    headers: BTreeMap::new(),
                    body: br#"{"total_count":0,"incomplete_results":false,"items":[]}"#.to_vec(),
                },
            ),
        );
        let empty_extension =
            LoadedExtension::load(&script_path, empty_host).expect("load empty example");
        let empty_svg = empty_extension
            .render_svg(&settings)
            .expect("empty GitHub SVG");
        assert!(empty_svg.contains("No open pull requests"));
        render_svg_to_panel(&empty_svg, &fixture.0).expect("empty GitHub frame");

        let rate_host = Arc::new(
            LocalFixtureHost::new(
                fixture.0.clone(),
                HostClock {
                    unix_seconds: 1_785_484_800,
                    utc_offset_minutes: 0,
                },
            )
            .with_response(
                "GET",
                url,
                HostHttpResponse {
                    status: 403,
                    headers: BTreeMap::from([("x-ratelimit-remaining".to_owned(), b"0".to_vec())]),
                    body: br#"{"message":"must-not-appear"}"#.to_vec(),
                },
            ),
        );
        let rate_extension =
            LoadedExtension::load(&script_path, rate_host).expect("load rate-limit example");
        let error = rate_extension
            .render_svg(&settings)
            .expect_err("rate limit must fail safely")
            .to_string();
        assert!(error.contains("rate limit"));
        assert!(!error.contains("must-not-appear"));
        assert!(!error.contains("api.github.com"));
    }

    #[test]
    fn scaffold_discovery_and_example_reach_panel_frame_with_local_http() {
        let fixture = TestExtension::new();
        let root = scaffold_extension(&fixture.0).expect("scaffold");
        let discovered = discover_extensions(&fixture.0).expect("discover");
        assert_eq!(discovered.as_slice(), std::slice::from_ref(&root));
        let (base, _, server) = serve_responses(vec![
            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 29\r\nConnection: close\r\n\r\n{\"full_name\":\"local/fixture\"}".to_vec(),
        ]);
        let host = Arc::new(live_host(&root, RenderCancellation::default()));
        let extension =
            LoadedExtension::load(&root.join(EXTENSION_SCRIPT_NAME), host).expect("example load");
        let settings = BTreeMap::from([("endpoint".to_owned(), Value::String(base))]);
        let resolved = validate_extension_configuration(&extension.inputs, &settings)
            .expect("example settings");
        let svg = extension.render_svg(&resolved).expect("example svg");
        let frame = render_svg_to_panel(&svg, &root).expect("example frame");
        assert_eq!(frame.payload().len(), crate::frame::FRAME_BYTES);
        server.join().expect("example server");
    }
}
