[CmdletBinding()]
param(
    [switch]$SkipBuild,
    [string]$Target = ""
)

$ErrorActionPreference = "Stop"
$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$distRoot = Join-Path $repoRoot "dist"
$stagingRoot = Join-Path $distRoot "wireterm-windows-x86_64-portable"
$archivePath = "$stagingRoot.zip"

if (-not $SkipBuild) {
    $cargoCommand = Get-Command cargo -ErrorAction SilentlyContinue
    if ($null -eq $cargoCommand) {
        $cargoExecutable = Join-Path $env:USERPROFILE ".cargo\bin\cargo.exe"
        if (-not (Test-Path -LiteralPath $cargoExecutable -PathType Leaf)) {
            throw "cargo was not found"
        }
    } else {
        $cargoExecutable = $cargoCommand.Source
    }
    $arguments = @("build", "--release")
    if ($Target) {
        $arguments += @("--target", $Target)
    }
    & $cargoExecutable @arguments
    if ($LASTEXITCODE -ne 0) {
        throw "release build failed"
    }
}

$binaryRoot = if ($Target) {
    Join-Path $repoRoot "target\$Target\release"
} else {
    Join-Path $repoRoot "target\release"
}
$binaryPath = Join-Path $binaryRoot "wireterm.exe"
if (-not (Test-Path -LiteralPath $binaryPath -PathType Leaf)) {
    throw "release wireterm.exe was not found"
}

foreach ($candidate in @($stagingRoot, $archivePath)) {
    $resolvedCandidate = [System.IO.Path]::GetFullPath($candidate)
    if (-not $resolvedCandidate.StartsWith(
        [System.IO.Path]::GetFullPath($distRoot) + [System.IO.Path]::DirectorySeparatorChar,
        [System.StringComparison]::OrdinalIgnoreCase
    )) {
        throw "refusing to replace output outside dist"
    }
    if (Test-Path -LiteralPath $resolvedCandidate) {
        Remove-Item -LiteralPath $resolvedCandidate -Recurse -Force
    }
}

New-Item -ItemType Directory -Force -Path $stagingRoot | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $stagingRoot "fonts") | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $stagingRoot "docs") | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $stagingRoot "wireterm-data\extensions") | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $stagingRoot "wireterm-data\images") | Out-Null

$defaultImageSource = Join-Path $repoRoot "assets\default-playlist"
$defaultImages = @(Get-ChildItem -LiteralPath $defaultImageSource -File)
if ($defaultImages.Count -ne 18) {
    throw "default Playlist image folder must contain exactly 18 files"
}
$unsupportedDefaultImages = @($defaultImages | Where-Object {
    $_.Extension.ToLowerInvariant() -ne ".jpg"
})
if ($unsupportedDefaultImages.Count -ne 0) {
    throw "default Playlist image folder must contain only JPEG files"
}
$nestedDefaultEntries = @(Get-ChildItem -LiteralPath $defaultImageSource -Directory -Recurse)
if ($nestedDefaultEntries.Count -ne 0) {
    throw "default Playlist image folder must be non-recursive"
}
Add-Type -AssemblyName System.Drawing
foreach ($defaultImage in $defaultImages) {
    $decoded = $null
    try {
        $decoded = [System.Drawing.Image]::FromFile($defaultImage.FullName)
        if ($decoded.Width -ne 800 -or $decoded.Height -ne 480) {
            throw "default Playlist image must be exactly 800x480: $($defaultImage.Name)"
        }
        $decoded.Dispose()
        $decoded = $null
    } catch {
        if ($null -ne $decoded) {
            $decoded.Dispose()
        }
        throw "default Playlist image is invalid: $($defaultImage.Name): $($_.Exception.Message)"
    }
}

Copy-Item -LiteralPath $binaryPath -Destination (Join-Path $stagingRoot "wireterm.exe")
Copy-Item -LiteralPath (Join-Path $repoRoot "README.md") -Destination $stagingRoot
Copy-Item -LiteralPath (Join-Path $repoRoot "LICENSE") -Destination $stagingRoot
Copy-Item -LiteralPath (Join-Path $repoRoot "docs\extension-author-guide.md") -Destination (Join-Path $stagingRoot "docs")
Copy-Item -LiteralPath (Join-Path $repoRoot "docs\extension-contract.md") -Destination (Join-Path $stagingRoot "docs")
Copy-Item -LiteralPath (Join-Path $repoRoot "docs\default-playlist-attribution.md") -Destination (Join-Path $stagingRoot "docs")
Copy-Item -LiteralPath (Join-Path $repoRoot "assets\fonts\Inter.ttf") -Destination (Join-Path $stagingRoot "fonts")
Copy-Item -LiteralPath (Join-Path $repoRoot "assets\fonts\OFL.txt") -Destination (Join-Path $stagingRoot "fonts\Inter-OFL.txt")
Copy-Item -LiteralPath (Join-Path $repoRoot "assets\fonts\README.md") -Destination (Join-Path $stagingRoot "fonts")
Copy-Item -LiteralPath (Join-Path $repoRoot "packaging\wireterm-data\README.md") -Destination (Join-Path $stagingRoot "wireterm-data")
Copy-Item -LiteralPath (Join-Path $repoRoot "packaging\wireterm-data\default-playlist.json") -Destination (Join-Path $stagingRoot "wireterm-data")
Copy-Item -LiteralPath (Join-Path $repoRoot "examples\http-extension") -Destination (Join-Path $stagingRoot "wireterm-data\extensions") -Recurse
Copy-Item -LiteralPath $defaultImageSource -Destination (Join-Path $stagingRoot "wireterm-data\images") -Recurse

$stagingUri = [System.Uri]::new($stagingRoot.TrimEnd("\") + "\")
$hashLines = Get-ChildItem -LiteralPath $stagingRoot -Recurse -File |
    Sort-Object FullName |
    ForEach-Object {
        $relative = [System.Uri]::UnescapeDataString(
            $stagingUri.MakeRelativeUri([System.Uri]::new($_.FullName)).ToString()
        )
        $hash = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        "$hash  $relative"
    }
$hashLines | Set-Content -LiteralPath (Join-Path $stagingRoot "SHA256SUMS.txt") -Encoding utf8

Compress-Archive -Path (Join-Path $stagingRoot "*") -DestinationPath $archivePath -CompressionLevel Optimal

Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive = [System.IO.Compression.ZipFile]::OpenRead($archivePath)
try {
    $entries = @($archive.Entries | ForEach-Object { $_.FullName.Replace("\", "/") })
    $required = @(
        "wireterm.exe",
        "README.md",
        "LICENSE",
        "SHA256SUMS.txt",
        "fonts/Inter.ttf",
        "fonts/Inter-OFL.txt",
        "fonts/README.md",
        "docs/extension-author-guide.md",
        "docs/extension-contract.md",
        "docs/default-playlist-attribution.md",
        "wireterm-data/README.md",
        "wireterm-data/default-playlist.json",
        "wireterm-data/extensions/http-extension/extension.lua",
        "wireterm-data/extensions/http-extension/assets/README.md",
        "wireterm-data/images/default-playlist/abu-sayed-mohammad-tamanna-0Nrk3XNOWkc-unsplash.jpg"
    )
    $missing = @($required | Where-Object { $_ -notin $entries })
    if ($missing.Count -ne 0) {
        throw "portable archive is missing: $($missing -join ', ')"
    }
    $packagedDefaultImages = @($entries | Where-Object {
        $_ -match '^wireterm-data/images/default-playlist/[^/]+\.(png|jpe?g)$'
    })
    if ($packagedDefaultImages.Count -ne $defaultImages.Count) {
        throw "portable archive default Playlist image count differs from source"
    }
    $checksumEntry = $archive.GetEntry("SHA256SUMS.txt")
    $checksumReader = [System.IO.StreamReader]::new($checksumEntry.Open())
    try {
        $checksums = $checksumReader.ReadToEnd()
    } finally {
        $checksumReader.Dispose()
    }
    foreach ($packagedDefaultImage in $packagedDefaultImages) {
        if ($checksums -notmatch [regex]::Escape("  $packagedDefaultImage")) {
            throw "portable archive checksum manifest omits: $packagedDefaultImage"
        }
    }
} finally {
    $archive.Dispose()
}

$entryCount = $entries.Count
Remove-Item -LiteralPath $stagingRoot -Recurse -Force
$archiveHash = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
Write-Output "Archive: $archivePath"
Write-Output "SHA256: $archiveHash"
Write-Output "Entries: $entryCount"
