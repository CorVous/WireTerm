# Extension contract

An MVP Extension is a folder containing one `extension.lua` file and optional
relative local assets. There is no manifest, Liquid template, or separate
transform script.

`extension.lua` returns one table:

```lua
local extension = {
  metadata = {
    id = "example-weather",
    name = "Example weather",
    description = "A self-describing example",
    version = 1,
  },
  inputs = {
    { key = "city", label = "City", kind = "text", required = true },
    {
      key = "token",
      label = "Weather token",
      kind = "named_secret",
      required = true,
    },
  },
}

function extension.render(context)
  local response = wireterm.http({
    method = "GET",
    url = "https://weather.example/current?city=" .. context.settings.city,
    secret_headers = { Authorization = "token" },
    timeout_ms = 15000,
    max_redirects = 0,
  })
  local icon = wireterm.asset("assets/weather.png")
  local clock = wireterm.clock()

  -- The script owns response interpretation. response.body is an arbitrary
  -- byte string; JSON is not required.
  return string.format(
    '<svg xmlns="http://www.w3.org/2000/svg" width="800" height="480" ' ..
    'viewBox="0 0 800 480"><rect width="800" height="480" fill="white"/>' ..
    '<image href="%s" x="32" y="32" width="128" height="128"/></svg>',
    icon
  )
end

return extension
```

Input kinds are `text`, `number`, `checkbox`, `choice`, and `named_secret`.
Choice inputs include a non-empty `choices` array. Named-secret inputs resolve
to app-owned references; their values are never placed in Lua settings.

## Host API

- `wireterm.http(request)` accepts a script-chosen method, URL, string headers,
  opaque `secret_headers` bindings, arbitrary byte-string body, timeout, and
  `max_redirects` from 0 to 10. Redirects default to denied. It returns status,
  byte-string headers, and an arbitrary byte-string body. TLS certificate and
  hostname validation are never disabled. Requests carrying a named-secret
  header never follow a redirect across scheme, host, or port boundaries.
- `wireterm.clock()` returns `unix_seconds` and `utc_offset_minutes`.
- `wireterm.asset(path)` validates a contained relative local path and returns
  its normalized SVG reference.

The sandbox allowlists only coroutine, table, string, UTF-8, and math helpers.
It removes direct filesystem, process, environment, package-loading, and
network capabilities; network is available only through `wireterm.http`.
Each response is capped at 5 MiB, individual request timeouts cap at 60
seconds, returned SVG caps at 2 MiB, and Lua execution caps at 30 seconds and
64 MiB. The overall Lua deadline also bounds HTTP, so a request may receive
less than its declared timeout when the render has less time remaining.

The SVG root must declare `width="800"`, `height="480"`, and
`viewBox="0 0 800 480"`. Remote assets and parent/absolute paths are rejected.
Relative PNG/JPEG assets are dithered before pure-Rust SVG composition.
Vector and text output is mapped directly to the panel palette without
whole-frame error diffusion. Each raster reference must appear once with
whole-pixel `width` and `height`; WireTerm resizes and dithers it at that painted
placement size before composition.

Extension text uses only the Inter font bundled with the locked WireTerm
build. System font discovery is disabled. Unknown family names fall back to
that bundled face, so output does not depend on fonts installed on Windows.

## Extension library and secrets

WireTerm discovers direct child folders containing `extension.lua` under the
adjacent `wireterm-data/extensions` library. The editor can add a discovered
folder, browse elsewhere, or scaffold an editable HTTP example. The shipped
example is also in `examples/http-extension` and is exercised against a local
deterministic HTTP fixture in tests.

Named-secret values are centrally created, updated, and removed in Advanced
details. Only their opaque names enter Playlist revisions. MVP values are
stored locally without encryption in immutable revisions under adjacent
`wireterm-data/secret-revisions`, read only for final HTTP-header injection,
marked sensitive at the HTTP boundary, and never returned to Lua or included
in WireTerm diagnostics. Protect access to the portable folder. Deleting it
removes every WireTerm-owned secret artifact.

`LocalFixtureHost` remains the deterministic unit-test implementation. The
host app uses `LiveExtensionHost` on its render worker, keeping the egui
thread responsive. Closing the app cooperatively cancels Lua execution; any
in-flight HTTP operation remains bounded by the shorter of its request timeout
and the overall render deadline.
