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

Copy-Item -LiteralPath $binaryPath -Destination (Join-Path $stagingRoot "wireterm.exe")
Copy-Item -LiteralPath (Join-Path $repoRoot "README.md") -Destination $stagingRoot
Copy-Item -LiteralPath (Join-Path $repoRoot "LICENSE") -Destination $stagingRoot
Copy-Item -LiteralPath (Join-Path $repoRoot "docs\extension-author-guide.md") -Destination (Join-Path $stagingRoot "docs")
Copy-Item -LiteralPath (Join-Path $repoRoot "docs\extension-contract.md") -Destination (Join-Path $stagingRoot "docs")
Copy-Item -LiteralPath (Join-Path $repoRoot "assets\fonts\Inter.ttf") -Destination (Join-Path $stagingRoot "fonts")
Copy-Item -LiteralPath (Join-Path $repoRoot "assets\fonts\OFL.txt") -Destination (Join-Path $stagingRoot "fonts\Inter-OFL.txt")
Copy-Item -LiteralPath (Join-Path $repoRoot "assets\fonts\README.md") -Destination (Join-Path $stagingRoot "fonts")
Copy-Item -LiteralPath (Join-Path $repoRoot "packaging\wireterm-data\README.md") -Destination (Join-Path $stagingRoot "wireterm-data")
Copy-Item -LiteralPath (Join-Path $repoRoot "examples\http-extension") -Destination (Join-Path $stagingRoot "wireterm-data\extensions") -Recurse

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
        "wireterm-data/README.md",
        "wireterm-data/extensions/http-extension/extension.lua",
        "wireterm-data/extensions/http-extension/assets/README.md"
    )
    $missing = @($required | Where-Object { $_ -notin $entries })
    if ($missing.Count -ne 0) {
        throw "portable archive is missing: $($missing -join ', ')"
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
