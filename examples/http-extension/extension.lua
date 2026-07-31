local function escape_xml(value)
  return tostring(value)
    :gsub("&", "&amp;")
    :gsub("<", "&lt;")
    :gsub(">", "&gt;")
    :gsub('"', "&quot;")
    :gsub("'", "&apos;")
end

return {
  metadata = {
    id = "wireterm-http-example",
    name = "HTTP repository status",
    description = "Fetches a real JSON response and renders it directly to fixed SVG.",
    version = 1,
  },
  inputs = {
    {
      key = "endpoint",
      label = "JSON endpoint",
      kind = "text",
      required = true,
      default = "https://api.github.com/repos/CorVous/WireTerm",
    },
  },
  render = function(context)
    local response = wireterm.http({
      method = "GET",
      url = context.settings.endpoint,
      headers = { Accept = "application/vnd.github+json" },
      timeout_ms = 10000,
      max_redirects = 2,
    })
    local full_name = response.body:match('"full_name"%s*:%s*"([^"]+)"')
      or response.body:match('"name"%s*:%s*"([^"]+)"')
      or "HTTP response received"
    local clock = wireterm.clock()
    local accent = response.status >= 200 and response.status < 300 and "#cd2323" or "#000000"
    return string.format([[
<svg xmlns="http://www.w3.org/2000/svg" width="800" height="480" viewBox="0 0 800 480">
  <rect width="800" height="480" fill="#ffffff"/>
  <rect width="18" height="480" fill="%s"/>
  <text x="58" y="112" font-family="Inter" font-size="34" font-weight="700" fill="#000000">WireTerm HTTP Extension</text>
  <text x="58" y="204" font-family="Inter" font-size="46" font-weight="600" fill="#000000">%s</text>
  <text x="58" y="294" font-family="Inter" font-size="28" fill="#cd2323">HTTP %d</text>
  <text x="58" y="358" font-family="Inter" font-size="22" fill="#000000">Rendered at Unix time %d</text>
</svg>]], accent, escape_xml(full_name), response.status, clock.unix_seconds)
  end,
}
