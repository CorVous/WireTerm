local JSON_NULL = {}

local function decode_json(source)
  local position = 1
  local length = #source

  local function fail()
    error("GitHub returned invalid JSON")
  end

  local function skip_whitespace()
    while position <= length do
      local byte = source:byte(position)
      if byte ~= 0x20 and byte ~= 0x09 and byte ~= 0x0a and byte ~= 0x0d then
        break
      end
      position = position + 1
    end
  end

  local function is_digit(byte)
    return byte ~= nil and byte >= 0x30 and byte <= 0x39
  end

  local function parse_string()
    if source:byte(position) ~= 0x22 then
      fail()
    end
    position = position + 1
    local chunks = {}
    local chunk_start = position

    while position <= length do
      local byte = source:byte(position)
      if byte == 0x22 then
        chunks[#chunks + 1] = source:sub(chunk_start, position - 1)
        position = position + 1
        return table.concat(chunks)
      end
      if byte == 0x5c then
        chunks[#chunks + 1] = source:sub(chunk_start, position - 1)
        position = position + 1
        local escape = source:sub(position, position)
        local replacements = {
          ['"'] = '"',
          ["\\"] = "\\",
          ["/"] = "/",
          b = "\b",
          f = "\f",
          n = "\n",
          r = "\r",
          t = "\t",
        }
        if replacements[escape] ~= nil then
          chunks[#chunks + 1] = replacements[escape]
          position = position + 1
        elseif escape == "u" then
          local hex = source:sub(position + 1, position + 4)
          if #hex ~= 4 or not hex:match("^%x%x%x%x$") then
            fail()
          end
          local codepoint = tonumber(hex, 16)
          position = position + 5
          if codepoint >= 0xd800 and codepoint <= 0xdbff then
            if source:sub(position, position + 1) ~= "\\u" then
              fail()
            end
            local low_hex = source:sub(position + 2, position + 5)
            if #low_hex ~= 4 or not low_hex:match("^%x%x%x%x$") then
              fail()
            end
            local low = tonumber(low_hex, 16)
            if low < 0xdc00 or low > 0xdfff then
              fail()
            end
            codepoint = 0x10000 + (codepoint - 0xd800) * 0x400 + (low - 0xdc00)
            position = position + 6
          elseif codepoint >= 0xdc00 and codepoint <= 0xdfff then
            fail()
          end
          if codepoint > 0x10ffff then
            fail()
          end
          chunks[#chunks + 1] = utf8.char(codepoint)
        else
          fail()
        end
        chunk_start = position
      elseif byte < 0x20 then
        fail()
      else
        position = position + 1
      end
    end
    fail()
  end

  local function parse_number()
    local start = position
    if source:sub(position, position) == "-" then
      position = position + 1
    end
    local first = source:byte(position)
    if first == 0x30 then
      position = position + 1
      if is_digit(source:byte(position)) then
        fail()
      end
    elseif first ~= nil and first >= 0x31 and first <= 0x39 then
      repeat
        position = position + 1
      until not is_digit(source:byte(position))
    else
      fail()
    end
    if source:sub(position, position) == "." then
      position = position + 1
      if not is_digit(source:byte(position)) then
        fail()
      end
      repeat
        position = position + 1
      until not is_digit(source:byte(position))
    end
    local exponent = source:sub(position, position)
    if exponent == "e" or exponent == "E" then
      position = position + 1
      local sign = source:sub(position, position)
      if sign == "+" or sign == "-" then
        position = position + 1
      end
      if not is_digit(source:byte(position)) then
        fail()
      end
      repeat
        position = position + 1
      until not is_digit(source:byte(position))
    end
    local value = tonumber(source:sub(start, position - 1))
    if value == nil then
      fail()
    end
    return value
  end

  local parse_value

  local function parse_array(depth)
    position = position + 1
    skip_whitespace()
    local result = {}
    if source:sub(position, position) == "]" then
      position = position + 1
      return result
    end
    while true do
      result[#result + 1] = parse_value(depth + 1)
      skip_whitespace()
      local separator = source:sub(position, position)
      if separator == "]" then
        position = position + 1
        return result
      end
      if separator ~= "," then
        fail()
      end
      position = position + 1
      skip_whitespace()
    end
  end

  local function parse_object(depth)
    position = position + 1
    skip_whitespace()
    local result = {}
    if source:sub(position, position) == "}" then
      position = position + 1
      return result
    end
    while true do
      if source:sub(position, position) ~= '"' then
        fail()
      end
      local key = parse_string()
      skip_whitespace()
      if source:sub(position, position) ~= ":" then
        fail()
      end
      position = position + 1
      skip_whitespace()
      result[key] = parse_value(depth + 1)
      skip_whitespace()
      local separator = source:sub(position, position)
      if separator == "}" then
        position = position + 1
        return result
      end
      if separator ~= "," then
        fail()
      end
      position = position + 1
      skip_whitespace()
    end
  end

  parse_value = function(depth)
    if depth > 64 then
      fail()
    end
    skip_whitespace()
    local byte = source:byte(position)
    if byte == 0x22 then
      return parse_string()
    end
    if byte == 0x7b then
      return parse_object(depth)
    end
    if byte == 0x5b then
      return parse_array(depth)
    end
    if byte == 0x2d or is_digit(byte) then
      return parse_number()
    end
    if source:sub(position, position + 3) == "true" then
      position = position + 4
      return true
    end
    if source:sub(position, position + 4) == "false" then
      position = position + 5
      return false
    end
    if source:sub(position, position + 3) == "null" then
      position = position + 4
      return JSON_NULL
    end
    fail()
  end

  local result = parse_value(0)
  skip_whitespace()
  if position <= length then
    fail()
  end
  return result
end

local function escape_xml(value)
  return tostring(value)
    :gsub("&", "&amp;")
    :gsub("<", "&lt;")
    :gsub(">", "&gt;")
    :gsub('"', "&quot;")
    :gsub("'", "&apos;")
end

local function truncate_text(value, maximum)
  value = tostring(value)
  local length = utf8.len(value)
  if length == nil then
    return "[invalid text]"
  end
  if length <= maximum then
    return value
  end
  local boundary = utf8.offset(value, maximum + 1)
  return value:sub(1, boundary - 1) .. "..."
end

local function url_encode(value)
  return tostring(value):gsub("([^%w%-._~])", function(character)
    return string.format("%%%02X", character:byte())
  end)
end

local function days_from_civil(year, month, day)
  if month <= 2 then
    year = year - 1
  end
  local era = math.floor((year >= 0 and year or year - 399) / 400)
  local year_of_era = year - era * 400
  local shifted_month = month + (month > 2 and -3 or 9)
  local day_of_year = math.floor((153 * shifted_month + 2) / 5) + day - 1
  local day_of_era = year_of_era * 365 + math.floor(year_of_era / 4)
    - math.floor(year_of_era / 100) + day_of_year
  return era * 146097 + day_of_era - 719468
end

local function parse_github_time(value)
  if type(value) ~= "string" then
    return nil
  end
  local year, month, day, hour, minute, second = value:match(
    "^(%d%d%d%d)%-(%d%d)%-(%d%d)T(%d%d):(%d%d):(%d%d)Z$"
  )
  if year == nil then
    return nil
  end
  year, month, day = tonumber(year), tonumber(month), tonumber(day)
  hour, minute, second = tonumber(hour), tonumber(minute), tonumber(second)
  if month < 1 or month > 12 or day < 1 or day > 31
    or hour > 23 or minute > 59 or second > 60 then
    return nil
  end
  return days_from_civil(year, month, day) * 86400
    + hour * 3600 + minute * 60 + second
end

local function update_label(updated_at, now)
  if type(updated_at) ~= "string" then
    return "update unavailable"
  end
  local date = updated_at:match("^(%d%d%d%d%-%d%d%-%d%d)")
  local timestamp = parse_github_time(updated_at)
  if date == nil or timestamp == nil then
    return "update unavailable"
  end
  local elapsed = math.max(0, now - timestamp)
  local age
  if elapsed < 3600 then
    age = "just now"
  elseif elapsed < 86400 then
    age = string.format("%dh ago", math.floor(elapsed / 3600))
  elseif elapsed < 7 * 86400 then
    age = string.format("%dd ago", math.floor(elapsed / 86400))
  else
    age = string.format("%dw ago", math.floor(elapsed / (7 * 86400)))
  end
  return age .. " / " .. date
end

local function repository_name(item)
  if type(item.repository_url) == "string" then
    local name = item.repository_url:match("/repos/(.+)$")
    if name ~= nil and name ~= "" then
      return name
    end
  end
  if type(item.repository) == "table" and type(item.repository.full_name) == "string" then
    return item.repository.full_name
  end
  return "unknown/repository"
end

local function fail_for_status(response)
  if response.status == 401 then
    error("GitHub authentication failed; check the token")
  end
  if response.status == 403 or response.status == 429 then
    if response.status == 429
      or response.headers["x-ratelimit-remaining"] == "0"
      or response.headers["retry-after"] ~= nil then
      error("GitHub search rate limit reached; retry after the reset")
    end
    error("GitHub denied access; check token repository and SSO access")
  end
  if response.status == 422 then
    error("GitHub rejected the search; check the username")
  end
  error(string.format("GitHub request failed with HTTP %d", response.status))
end

local function render_empty(username)
  return string.format([[
<svg xmlns="http://www.w3.org/2000/svg" width="800" height="480" viewBox="0 0 800 480">
  <rect width="800" height="480" fill="#ffffff"/>
  <rect width="800" height="12" fill="#cd2323"/>
  <text x="32" y="58" font-family="Inter" font-size="32" font-weight="700" fill="#000000">Open pull requests</text>
  <text x="32" y="88" font-family="Inter" font-size="18" fill="#000000">@%s</text>
  <line x1="32" y1="112" x2="768" y2="112" stroke="#000000" stroke-width="2"/>
  <rect x="32" y="170" width="12" height="126" fill="#cd2323"/>
  <text x="70" y="220" font-family="Inter" font-size="34" font-weight="700" fill="#000000">No open pull requests</text>
  <text x="70" y="260" font-family="Inter" font-size="21" fill="#000000">Nothing authored by this account is waiting.</text>
</svg>]], escape_xml(username))
end

local extension = {
  metadata = {
    id = "github-open-prs",
    name = "GitHub open pull requests",
    description = "Shows recently updated open pull requests authored by a GitHub user.",
    version = 1,
  },
  inputs = {
    {
      key = "username",
      label = "GitHub username",
      kind = "text",
      required = true,
      default = "CorVous",
    },
    {
      key = "github_token",
      label = "GitHub token",
      kind = "secret",
      required = true,
    },
  },
}

function extension.render(context)
  local username = context.settings.username:match("^%s*(.-)%s*$")
  if username == "" or #username > 39
    or not username:match("^[A-Za-z0-9][A-Za-z0-9%-]*$")
    or username:sub(-1) == "-" then
    error("GitHub username is invalid")
  end

  local query = "type:pr state:open author:" .. username
  local url = "https://api.github.com/search/issues?q=" .. url_encode(query)
    .. "&sort=updated&order=desc&per_page=5"
  local response = wireterm.http({
    method = "GET",
    url = url,
    headers = {
      Accept = "application/vnd.github+json",
      ["User-Agent"] = "WireTerm-GitHub-Open-PRs/1.0",
      ["X-GitHub-Api-Version"] = "2022-11-28",
    },
    secret_headers = { Authorization = "Bearer " .. context.settings.github_token },
    timeout_ms = 15000,
    max_redirects = 0,
  })
  if response.status ~= 200 then
    fail_for_status(response)
  end

  local payload = decode_json(response.body)
  if type(payload) ~= "table" or type(payload.items) ~= "table" then
    error("GitHub response did not contain a pull request list")
  end
  if #payload.items == 0 then
    return render_empty(username)
  end

  local clock = wireterm.clock()
  local total = tonumber(payload.total_count) or #payload.items
  local count = math.min(5, #payload.items)
  local summary = string.format("@%s / showing %d of %d", username, count, total)
  if payload.incomplete_results == true then
    summary = summary .. " / partial results"
  end

  local svg = {
    '<svg xmlns="http://www.w3.org/2000/svg" width="800" height="480" viewBox="0 0 800 480">',
    '<rect width="800" height="480" fill="#ffffff"/>',
    '<rect width="800" height="12" fill="#cd2323"/>',
    '<text x="32" y="52" font-family="Inter" font-size="32" font-weight="700" fill="#000000">Open pull requests</text>',
    string.format('<text x="32" y="79" font-family="Inter" font-size="17" fill="#000000">%s</text>', escape_xml(summary)),
    '<line x1="32" y1="94" x2="768" y2="94" stroke="#000000" stroke-width="2"/>',
  }

  for index = 1, count do
    local item = payload.items[index]
    if type(item) ~= "table" then
      item = {}
    end
    local row_y = 96 + (index - 1) * 72
    local repo = truncate_text(repository_name(item), 42)
    local title = type(item.title) == "string" and item.title or "(untitled pull request)"
    title = truncate_text(title, 54)
    local number = tonumber(item.number)
    local number_label = number ~= nil and string.format("#%d", number) or "PR"
    local updated = update_label(item.updated_at, clock.unix_seconds)
    svg[#svg + 1] = string.format('<rect x="32" y="%d" width="7" height="48" fill="#cd2323"/>', row_y + 11)
    svg[#svg + 1] = string.format('<text x="52" y="%d" font-family="Inter" font-size="18" font-weight="700" fill="#000000">%s</text>', row_y + 27, escape_xml(repo))
    svg[#svg + 1] = string.format('<text x="768" y="%d" text-anchor="end" font-family="Inter" font-size="18" font-weight="700" fill="#cd2323">%s</text>', row_y + 27, escape_xml(number_label))
    svg[#svg + 1] = string.format('<text x="52" y="%d" font-family="Inter" font-size="21" fill="#000000">%s</text>', row_y + 54, escape_xml(title))
    svg[#svg + 1] = string.format('<text x="768" y="%d" text-anchor="end" font-family="Inter" font-size="14" fill="#000000">%s</text>', row_y + 54, escape_xml(updated))
    svg[#svg + 1] = string.format('<line x1="52" y1="%d" x2="768" y2="%d" stroke="#000000" stroke-width="1"/>', row_y + 68, row_y + 68)
  end
  svg[#svg + 1] = "</svg>"
  return table.concat(svg)
end

return extension
