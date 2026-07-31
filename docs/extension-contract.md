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
  opaque `secret_headers` bindings, arbitrary byte-string body, and timeout.
  It returns status, headers, and an arbitrary byte-string body.
- `wireterm.clock()` returns `unix_seconds` and `utc_offset_minutes`.
- `wireterm.asset(path)` validates a contained relative local path and returns
  its normalized SVG reference.

The sandbox exposes safe Lua libraries but no direct filesystem, process,
environment, package loading, or network access. Each response is capped at
5 MiB, individual request timeouts cap at 60 seconds, returned SVG caps at
2 MiB, and Lua execution caps at 30 seconds and 64 MiB.

The SVG root must declare `width="800"`, `height="480"`, and
`viewBox="0 0 800 480"`. Remote assets and parent/absolute paths are rejected.
Relative PNG/JPEG assets are dithered before pure-Rust SVG composition.
Vector and text output is mapped directly to the panel palette without
whole-frame error diffusion. Each raster reference must appear once with
whole-pixel `width` and `height`; WireTerm resizes and dithers it at that painted
placement size before composition.

## Current slice

`LocalFixtureHost` is the complete deterministic implementation used by tests
and local fixture callers. It matches script-chosen method/URL pairs, supplies
arbitrary response bytes and a fixed clock, records requests, validates named
secret bindings, and resolves local assets.

Live HTTP transport and central secret-value storage/injection remain
deliberately unimplemented. Until both are added with their bounds and
redaction guarantees intact, the foreground GUI supplies an empty fixture host:
offline-only Extensions render, while HTTP-dependent Extensions fail safely,
log the failure, and leave the current panel Frame unchanged.
