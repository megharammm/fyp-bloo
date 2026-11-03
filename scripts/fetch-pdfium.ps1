# Bloo AI - install the native PDFium library used by crates/bloo-parse.
#
#   scripts\fetch-pdfium.ps1
#
# Downloads the prebuilt Windows x64 build from bblanchon/pdfium-binaries into
# data/pdfium/pdfium.dll. The parser looks there automatically, or at PDFIUM_PATH.

[CmdletBinding()]
param([switch]$Force)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$dest = Join-Path $repo 'data\pdfium'
$dll = Join-Path $dest 'pdfium.dll'

if ((Test-Path $dll) -and -not $Force) {
    Write-Host "pdfium already present: $dll" -ForegroundColor Green
    exit 0
}

Write-Host 'resolving the latest pdfium-binaries release...' -ForegroundColor Cyan
$release = Invoke-RestMethod 'https://api.github.com/repos/bblanchon/pdfium-binaries/releases/latest' `
    -Headers @{ 'User-Agent' = 'BlooAI/0.1' } -TimeoutSec 60

$asset = $release.assets | Where-Object { $_.name -eq 'pdfium-win-x64.tgz' } | Select-Object -First 1
if (-not $asset) {
    throw "no pdfium-win-x64.tgz asset in release $($release.tag_name) - check https://github.com/bblanchon/pdfium-binaries/releases"
}

New-Item -ItemType Directory -Force -Path $dest | Out-Null
$tgz = Join-Path $env:TEMP 'pdfium-win-x64.tgz'
Write-Host "downloading $($asset.name) ($([math]::Round($asset.size/1MB,1)) MB)..." -ForegroundColor DarkGray
Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $tgz -TimeoutSec 300

Write-Host 'extracting...' -ForegroundColor DarkGray
& tar -xzf $tgz -C $dest
if ($LASTEXITCODE -ne 0) { throw 'tar failed to extract the archive' }

# The archive contains bin/pdfium.dll
$found = Get-ChildItem -Path $dest -Recurse -Filter 'pdfium.dll' | Select-Object -First 1
if (-not $found) { throw "pdfium.dll not found in the archive under $dest" }
if ($found.FullName -ne $dll) {
    Move-Item -Force $found.FullName $dll
}

Remove-Item $tgz -Force -ErrorAction SilentlyContinue

Write-Host "installed: $dll" -ForegroundColor Green
Write-Host 'crates/bloo-parse will pick it up automatically.' -ForegroundColor DarkGray
