# Extension author guide

An Extension is one folder containing `extension.lua` and optional relative
PNG/JPEG assets. Put the folder directly under
`wireterm-data/extensions`, choose **+ Extension**, and select it from the
Extension library. **Scaffold Extension** creates an editable working example.

The script returns one table with `metadata`, `inputs`, and `render(context)`.
Metadata requires a lowercase letter/number/hyphen `id`, display `name`, and a
positive numeric `version`. Input kinds are `text`, `number`, `checkbox`,
`choice`, and `named_secret`; choices need a non-empty `choices` list. Defaults
must match their declared kind. `context.settings` contains validated non-secret
values with defaults resolved.

## Host APIs

```lua
local response = wireterm.http({
  method = "POST",                       -- default GET
  url = context.settings.endpoint,        -- http or https
  headers = { Accept = "application/json" },
  secret_headers = { ["X-API-Key"] = "token" },
  body = "arbitrary bytes",
  timeout_ms = 15000,                     -- 1..60000
  max_redirects = 0,                      -- 0..10; default deny
})
```

`response.status` is numeric. `response.headers` and `response.body` are Lua
byte strings; JSON is optional and parsing is the script's responsibility.
Responses are limited to 5 MiB. HTTP uses normal TLS certificate and hostname
validation. The 30-second overall render deadline can shorten a request.
When `secret_headers` is non-empty, redirects never cross scheme, host, or port
boundaries.

`secret_headers` maps an outgoing header name to a logical `named_secret` input
key. WireTerm resolves the Playlist Item's opaque binding and injects the value
only while constructing the HTTP request. Lua never receives the value. Create
or update values centrally under **Advanced details → Named secrets**.
MVP values are stored locally without encryption under adjacent data; protect
access to the portable folder.

`wireterm.clock()` returns `unix_seconds` and `utc_offset_minutes`.
`wireterm.asset("assets/picture.png")` validates a contained relative asset and
returns its normalized SVG reference. Absolute paths, parent traversal,
symlink escape, remote SVG assets, Lua filesystem/process/environment/package
access, and direct Lua networking are rejected.

## Render output

`render` must return UTF-8 SVG no larger than 2 MiB with exactly:

```xml
<svg xmlns="http://www.w3.org/2000/svg"
     width="800" height="480" viewBox="0 0 800 480">
```

Use `#000000`, `#ffffff`, and `#cd2323` for exact panel colors. Vectors and text
are composed then directly palette-mapped. Each relative PNG/JPEG `<image>`
must appear once with whole-pixel `width` and `height`; WireTerm resizes and
Floyd–Steinberg dithers it before composition. Text always uses the bundled
Inter font; system fonts are not discovered, and unknown family names fall
back to Inter.

Lua has a 64 MiB memory limit and 30-second overall deadline. A failed render,
request, asset, or validation is logged without replacing the last successful
panel frame; playback skips the failed item and continues.
