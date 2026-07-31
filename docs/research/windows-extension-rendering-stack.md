# Windows extension rendering stack

> Historical research, superseded by ADRs 0002 and 0003. The accepted MVP uses
> one self-describing Lua script returning fixed SVG through a pure-Rust
> renderer; it has no Liquid, HTML, WebView2, or child-process transform stage.

## Decision

Use a host-side Rust pipeline with these components:

1. Evaluate extension templates with [`liquid`](https://github.com/cobalt-org/liquid-rust) (`ParserBuilder::with_stdlib()`), adding only explicitly documented WireTerm filters.
2. Render the resulting HTML/CSS in the Evergreen Microsoft Edge WebView2 Runtime through a small Windows-only adapter built on [`webview2-com`](https://docs.rs/webview2-com/latest/webview2_com/).
3. Serve generated HTML from memory with `WebResourceRequested`; map extension assets and WireTerm's bundled fonts to separate HTTPS virtual hosts with `SetVirtualHostNameToFolderMapping`.
4. Capture an exactly 800 × 480 PNG with WebView2 `CapturePreview`, verify its dimensions, decode it, and pass its pixels to the B/W/R frame conversion boundary.
5. Treat filesystem notifications as invalidations, then re-read and revalidate the whole extension on the next preview or playlist refresh.
6. Run an optional transform as a trusted, explicitly configured child process using a bounded JSON-on-stdin/JSON-on-stdout protocol. Do not embed Python, Node, Ruby, or another scripting runtime in WireTerm.

Use the shared Evergreen runtime in production, not a bundled Fixed Version runtime and not an installed Chrome executable. This is the best operational tradeoff for an installed Windows application: Windows 11 includes Evergreen WebView2, most Windows 10 systems already have it, and Microsoft supplies a roughly 2 MB bootstrapper for the remaining online installs. Evergreen is shared and automatically serviced. By contrast, Microsoft's Fixed Version binaries add more than 250 MB and make the application owner responsible for shipping runtime updates. ([Microsoft: WebView2 distribution](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution))

“Deterministic” for the MVP should mean that the same data, extension files, bundled fonts, WireTerm version, WebView2 runtime version, and rendering settings yield the same frame. It should not promise a bit-identical PNG across future Evergreen runtime versions. Record the WebView2 runtime version in diagnostics and render-error logs.

## Why this stack

### Liquid belongs in Rust

The upstream `liquid-rust` implementation aims to conform to strict Shopify Liquid, provides a standard-library parser, and supports application-defined filters, tags, and blocks. Its direct Rust API avoids installing or embedding Ruby merely to evaluate templates. ([liquid-rust README](https://github.com/cobalt-org/liquid-rust#readme))

WireTerm should:

- parse JSON into the Liquid object model;
- build one parser at process startup with the standard library and a versioned list of WireTerm additions;
- compile a template when its source changes and cache the compiled form by extension revision;
- render with only fetched/transformed data and non-secret item settings;
- report syntax and render failures with the extension path and Liquid source location where available.

This is WireTerm-native Liquid, not a promise of complete TRMNL compatibility. TRMNL maintains its own Liquid extensions, and `liquid-rust` deliberately allows language variants, so every WireTerm-specific filter must be named, versioned, tested, and included in the authoring guide. ([TRMNL custom filters](https://help.usetrmnl.com/en/articles/10347358-custom-plugin-filters), [liquid-rust customization](https://github.com/cobalt-org/liquid-rust#customizing-liquid))

### WebView2 is the deployable HTML/CSS engine already native to the target

WebView2 hosts current Edge/Chromium web technology in a native application and exposes the operations this feature needs: navigation, request interception, script execution for host synchronization, raw-pixel controller sizing, and PNG capture. `CapturePreview` writes what the WebView is displaying to a supplied stream and supports PNG. It must only be called after the new page has begun loading. ([WebView2 `ICoreWebView2`](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2), [capture contract](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2#capturepreview))

Use a dedicated single-threaded apartment (STA) renderer thread with a Win32 message pump and one reusable, non-interactive WebView2 controller. The rest of WireTerm communicates with it by request/result channels. This keeps COM and window lifetime rules out of playlist logic and lets background playback render while the editor window is closed.

Put the low-level COM work in a small Windows-only adapter crate. The current repository forbids unsafe code; generated COM bindings may require narrow unsafe calls. Keeping those calls in a separate adapter crate lets the main application retain `unsafe_code = "forbid"` and gives the boundary focused tests and review.

`webview2-com` is preferable to a general UI wrapper for this component because WireTerm needs low-level controller and capture APIs even when its editor UI is absent. The crate exposes current WebView2 COM interfaces and completion-handler implementations, including `CapturePreview`. ([`webview2-com` API](https://docs.rs/webview2-com/latest/webview2_com/))

### Evergreen is the right production distribution mode

The installer must detect WebView2 before first launch and install the Evergreen bootstrapper if it is absent. Microsoft recommends checking at install/update time, includes Evergreen in Windows 11, and documents both online bootstrapper and offline standalone installer paths. ([Microsoft: deploy Evergreen](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution#deploying-the-evergreen-webview2-runtime))

Pin the Rust crate versions and WebView2 SDK bindings in `Cargo.lock`, but feature-detect COM interfaces at runtime because an enterprise administrator can delay Evergreen updates. Microsoft explicitly requires feature detection for recent APIs under Evergreen. ([Microsoft: feature detection under Evergreen](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution#feature-detect-when-using-recent-apis))

A Fixed Version runtime remains a later option for installations that truly require cross-machine pixel reproducibility. It is not justified for the MVP because the runtime is over 250 MB, cannot update itself, and requires WireTerm releases to carry browser servicing. ([Microsoft: Fixed Version mode](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution#the-fixed-version-runtime-distribution-mode))

## Render transaction

Each render is one bounded transaction:

1. Resolve the playlist item, fetch its data on the host, and obtain a JSON value.
2. If configured, send that JSON through the transform contract described below.
3. Render the Liquid template to HTML.
4. Assign an unguessable render ID and map two local folders:
   - `https://extension-<id>.wireterm.invalid/` → the extension root;
   - `https://app-<id>.wireterm.invalid/` → WireTerm's versioned design assets and bundled fonts.
5. Intercept `https://render-<id>.wireterm.invalid/index.html` and return the generated HTML from memory with `Content-Type: text/html; charset=utf-8` and a restrictive Content Security Policy.
6. Navigate, wait for successful navigation, wait for images and fonts, force final layout, capture PNG, and enforce the render deadline.
7. Decode the PNG and reject it unless it is exactly 800 × 480 before handing pixels to the existing frame conversion boundary.
8. Clear mappings and per-render handlers even on cancellation or failure.

WebView2 documents four local-content approaches. `NavigateToString` supports dynamic HTML but has a 2 MB limit and does not support referenced CSS, images, scripts, or fonts. Virtual-host mapping supports relative local resources but only serves static files. `WebResourceRequested` can supply dynamic response content, while virtual-host mapping efficiently resolves local subresources inside WebView2. The combination above therefore fits generated HTML plus extension assets without a localhost server or temporary rendered HTML file. ([Microsoft: using local content](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/working-with-local-content))

Use `COREWEBVIEW2_HOST_RESOURCE_ACCESS_KIND_DENY_CORS` for each mapping. Give every render a new host name so cache and origin storage cannot leak between extension instances. Do not permit the template to choose a filesystem root.

## Deterministic 800 × 480 capture

Configure the controller before navigation:

- bounds mode: `COREWEBVIEW2_BOUNDS_MODE_USE_RAW_PIXELS`;
- bounds: 800 × 480;
- rasterization scale: 1.0;
- automatic monitor-scale detection: false;
- zoom factor: 1.0;
- preferred color scheme: fixed light;
- transparent/default background: fixed white;
- browser JavaScript, dialogs, context menus, downloads, permission prompts, autofill, and DevTools UI: disabled for extension content.

WebView2's raw-pixel bounds mode makes controller bounds independent of monitor rasterization scale, and disabling monitor-scale detection prevents the runtime from changing the chosen scale as the host window moves between displays. ([Microsoft: controller rasterization scale](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2controller3), [bounds modes](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/winrt/microsoft_web_webview2_core/corewebview2boundsmode))

The host's HTML envelope should set a UTF-8 document, a fixed viewport, `color-scheme: light`, zero body margin, 800 × 480 root/body dimensions, hidden overflow, and a mandatory final stylesheet that disables CSS animations, transitions, smooth scrolling, carets, and selection UI. Extension markup may not override the output dimensions.

Before capture, host-injected script should:

1. wait for `window.load`;
2. await `decode()` for every document image, treating a decode failure as a render failure;
3. await `document.fonts.ready`;
4. force layout by reading the root bounding rectangle;
5. post a render-ID-tagged ready message to the host.

The CSS Font Loading specification defines `document.fonts.ready` as resolving only when font loading and dependent layout operations are done, which is the required synchronization point before capture. ([W3C CSS Font Loading](https://www.w3.org/TR/css-font-loading/#font-face-set-ready))

The host synchronization script is application code executed through WebView2, not extension JavaScript. Keep extension JavaScript disabled in the MVP. Data access and computation belong in host fetches and the transform stage; allowing browser script would add nondeterministic timers, storage, and network behavior without being needed for Liquid-rendered screens.

Apply a single deadline to navigation, local resource loading, readiness, and capture (recommended default: 10 seconds). On timeout or any resource error, fail the item and leave the currently displayed panel frame unchanged, matching the playlist failure policy.

### Local assets and fonts

Virtual-host mapping gives local files HTTPS-like URLs, relative URL resolution, and browser-process loading. Microsoft recommends a unique virtual host per folder when several local folders are in use. ([Microsoft: `SetVirtualHostNameToFolderMapping`](https://learn.microsoft.com/en-us/dotnet/api/microsoft.web.webview2.core.corewebview2.setvirtualhostnametofoldermapping))

Ship fonts as versioned application assets and publish generated `@font-face` rules from the app asset host. The required authoring CSS should end in a bundled font family, not a Windows-installed font. Extension-local fonts may be allowed only from the extension host. Remote fonts and remote images should fail validation or be blocked at render time.

To prevent hidden network access:

- use a Content Security Policy with `default-src 'none'`;
- allow images, styles, and fonts only from the two per-render virtual hosts and narrowly required `data:` image URLs;
- set `connect-src 'none'`, `script-src 'none'`, `frame-src 'none'`, and `object-src 'none'`;
- register a catch-all `WebResourceRequested` handler and return a blocked response for every non-render HTTP(S) request.

WebView2 permits the host to intercept and block network requests. Virtual-host mappings do not fire `WebResourceRequested`, so the allowlisted local files remain fast while unmatched network resources are denied. ([Microsoft: custom management of network requests](https://learn.microsoft.com/en-us/microsoft-edge/webview2/how-to/webresourcerequested))

## Author file reload

Use [`notify`](https://docs.rs/notify/latest/notify/) with its recommended Windows watcher and a small debounce window (recommended: 300 ms). Watch the configured extensions directory and the parent of each loaded extension, not individual files. Editors commonly save by creating and renaming replacement files, and `notify` warns that precise event kinds vary by editor and that behavior can be surprising when a watched path is renamed or removed. ([`notify` known problems and watcher contract](https://docs.rs/notify/latest/notify/))

Events are only “something may have changed” signals:

- mark the affected extension dirty;
- after the debounce interval, re-read its manifest, template, CSS, and asset metadata from disk;
- build a fresh validated extension revision and atomically replace the previous valid revision;
- preserve the previous valid revision if reload fails and surface the new validation error in the GUI;
- always re-check the revision at scheduled refresh, so missed notifications cannot make playlist playback permanently stale.

Use [`notify-debouncer-mini`](https://docs.rs/notify-debouncer-mini/latest/notify_debouncer_mini/) unless later requirements need rename correlation. It emits at most one event per file per debounce interval, which is sufficient because WireTerm performs a full extension re-read rather than reconstructing filesystem history.

## Optional transform contract

Do not define a language-specific plugin API. The manifest declares an executable and argument array. At installation/configuration time WireTerm resolves the executable, displays that arbitrary trusted code will run as the user, and requires confirmation.

Protocol:

- working directory: extension root;
- stdin: one UTF-8 JSON value, then EOF;
- stdout: exactly one UTF-8 JSON value;
- stderr: diagnostic text shown with the item error;
- success: exit code 0 and valid JSON output;
- default timeout: 5 seconds; manifest may request up to 30 seconds;
- output limits: 4 MiB stdout and 256 KiB stderr;
- no shell: pass the program and each argument separately;
- environment: clear inherited variables and add only a documented allowlist plus item settings explicitly marked for transforms;
- secrets: never include secret values in stdin, arguments, environment, logs, or template globals.

Rust's `std::process::Command` provides explicit program/argument construction and piped standard streams; it otherwise inherits the parent environment and working directory, so WireTerm must deliberately clear and rebuild both. ([Rust `Command`](https://doc.rust-lang.org/stable/std/process/struct.Command.html))

On Windows, put every transform process into a Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, and terminate the job on timeout or cancellation so descendants do not survive. Windows Job Objects manage a process group as a unit, normally include child processes, and can terminate the entire group. ([Microsoft: Job Objects](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects), [TerminateJobObject](https://learn.microsoft.com/en-us/windows/win32/api/jobapi2/nf-jobapi2-terminatejobobject))

This is resource containment, not a security sandbox. A transform runs with the user's authority and must be treated like any other program the user launches. Windows AppContainer can restrict filesystem, network, process, credential, and device access, but adding a useful capability and file-broker model is a separate security feature, not an MVP detail. ([Microsoft: AppContainer](https://learn.microsoft.com/en-us/windows/win32/secauthz/implementing-an-appcontainer))

## Rejected alternatives

### Bundled Fixed Version WebView2

It offers control over browser update timing, but costs more than 250 MB, cannot self-update, and adds a browser security servicing obligation. Keep it as an enterprise/reproducibility option only if visual regressions from Evergreen become a measured problem.

### Playwright or a headless Chrome executable

This reproduces TRMNL-style server tooling, but it either requires a separately installed browser (operationally fragile) or another large browser payload. Chrome DevTools Protocol's tip-of-tree API changes frequently and provides no backwards-compatibility guarantee. ([Chrome DevTools Protocol](https://chromedevtools.github.io/devtools-protocol/))

### `NavigateToString` alone

It is simple but capped at 2 MB, produces a null origin, and cannot reference extension CSS, images, or fonts. It cannot satisfy the bundle model without inlining every asset. ([Microsoft: `NavigateToString` limitations](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/working-with-local-content#loading-local-content-by-navigating-to-an-html-string))

### Local HTTP server

It makes dynamic HTML and assets straightforward but adds port selection, firewall/loopback behavior, server lifetime, and an unnecessary listening surface. WebView2 request interception and virtual hosts provide the same local resource model without a socket.

### Embedded Python, Node, or Ruby

Choosing one transform language makes WireTerm responsible for distributing and patching another runtime and excludes authors who prefer a different language. A process protocol keeps the host stable and lets authors use any executable already available to them.

## Implementation slices and acceptance checks

1. **Liquid adapter**
   - Golden tests cover nested GitHub-like JSON, escaping, loops, missing values, and every WireTerm filter.
   - Templates never receive credential values.
2. **WebView2 adapter**
   - A test fixture with bundled font, CSS, PNG/JPEG/WebP images, and non-ASCII text captures at exactly 800 × 480 at Windows display scales 100%, 125%, and 200%.
   - A fixture containing remote CSS, image, font, `fetch`, iframe, and WebSocket attempts causes no network traffic.
   - Missing assets, failed image decode, font failure, navigation error, capture error, and timeout return typed errors.
3. **Frame boundary**
   - Captured PNG decodes to the pixel type consumed by the B/W/R converter.
   - The same capture can be previewed without rerendering and converted without disk round-trips.
4. **Reload**
   - In-place writes, atomic-save rename, extension-directory rename, deletion, and rapid multi-file saves each produce one eventual valid revision or a surfaced validation error.
5. **Transforms**
   - Success, nonzero exit, malformed JSON, oversize output, timeout, and a spawned child process are covered; timeout leaves no descendant running.
6. **Deployment**
   - Clean Windows 11 works without a bundled browser.
   - A Windows 10 test image without WebView2 takes the installer bootstrap path and can render after installation.
   - Diagnostics report WebView2 runtime, SDK binding, Liquid crate, design-assets, and extension-format versions.

## Remaining decisions and fog

These do not change the stack choice, but should be resolved before implementation begins:

- **Prototype required:** prove that `CapturePreview` is reliable from the tray-only background state using a hidden or off-screen Win32 host window. If a hidden controller does not paint reliably, choose between a non-activating off-screen window and `ICoreWebView2CompositionController`; do not discover this after playlist work is built.
- **Product decision:** confirm that extension JavaScript is out of MVP. The recommendation is yes: Liquid plus an optional external transform supplies authored code without making the renderer an application runtime.
- **Policy decision:** define whether extension-local fonts are accepted in MVP or only WireTerm-bundled fonts are allowed. Bundled-only is more reproducible; extension-local fonts are more expressive and still offline.
- **Compatibility decision:** publish the initial WireTerm Liquid filter/tag list. Do not imply TRMNL filter compatibility until compatibility fixtures exist.
- **Security follow-up:** if extensions will later be downloaded from other people rather than authored locally, design signing/trust and AppContainer isolation before treating transforms as installable plugins.
