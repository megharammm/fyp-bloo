# Bloo AI — seed the demo corpus.
# 1. start the stack, 2. sync content/seed into the index, 3. print what landed.

[CmdletBinding()]
param([string]$BaseUrl = 'http://127.0.0.1:27810')

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo

$seed = Join-Path $repo 'content/seed'
if (-not (Test-Path $seed)) { throw "Missing content/seed" }

$files = Get-ChildItem -Path $seed -Recurse -File -Include *.pdf, *.docx, *.html, *.md, *.txt
Write-Host "seed corpus: $($files.Count) files" -ForegroundColor Cyan

if ($files.Count -eq 0) {
    Write-Warning 'No seed files yet. Drop PDFs into content/seed and re-run.'
    exit 0
}

& (Join-Path $PSScriptRoot 'sync.ps1') -BaseUrl $BaseUrl
