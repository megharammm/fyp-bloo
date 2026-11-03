# Bloo AI — first run on a new laptop.
#
#   powershell -ExecutionPolicy Bypass -File scripts\setup.ps1
#
# Checks the toolchain, starts Docker, writes .env, creates the database,
# builds, launches, and opens the browser. Safe to run twice.

[CmdletBinding()]
param([switch]$SkipBuild)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo

function Ok($m)   { Write-Host "  OK   $m" -ForegroundColor Green }
function Warn($m) { Write-Host "  ..   $m" -ForegroundColor Yellow }
function Die($m)  { Write-Host "  FAIL $m" -ForegroundColor Red; exit 1 }
function Head($m) { Write-Host "`n$m" -ForegroundColor Cyan }

Write-Host @"
  ____  _                 _    ___
 | __ )| | ___   ___     / \  |_ _|
 |  _ \| |/ _ \ / _ \   / _ \  | |
 | |_) | | (_) | (_) | / ___ \ | |
 |____/|_|\___/ \___/ /_/   \_\___|
"@ -ForegroundColor DarkCyan

# ── 1. toolchain ────────────────────────────────────────────────────────────
Head '1/6  Checking toolchain'

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Die "Rust not found. Install it, then re-run:`n       winget install Rustlang.Rustup`n       (or https://rustup.rs)"
}

$rustOut = (rustc --version) -split ' '
$rustVer = $rustOut[1]
$parts = $rustVer.Split('.')
$major = [int]$parts[0]; $minor = [int]$parts[1]
if ($major -lt 1 -or ($major -eq 1 -and $minor -lt 85)) {
    Die "Rust $rustVer is too old (need >= 1.85 for edition 2024). Run: rustup update stable"
}
Ok "Rust $rustVer"

if (-not (Get-Command git -ErrorAction SilentlyContinue)) { Die 'git not found.' }
Ok "git $((git --version) -replace 'git version ','')"

if (Get-Command git-lfs -ErrorAction SilentlyContinue) {
    git lfs install --local 2>$null | Out-Null
    Ok 'git-lfs ready (PDFs in content/seed)'
} else {
    Warn 'git-lfs missing — large PDFs will not download. Install: winget install GitHub.GitLFS'
}

# ── 2. docker ───────────────────────────────────────────────────────────────
Head '2/6  Checking Docker'

if (-not (Get-Command docker -ErrorAction SilentlyContinue)) {
    Die "Docker not found. Install Docker Desktop: https://www.docker.com/products/docker-desktop/"
}

$daemon = $false
try { docker info 2>$null | Out-Null; $daemon = $true } catch { $daemon = $false }

if (-not $daemon) {
    $desktop = Join-Path $env:ProgramFiles 'Docker\Docker\Docker Desktop.exe'
    if (Test-Path $desktop) {
        Warn 'Docker daemon is down — starting Docker Desktop (this can take a minute)'
        Start-Process $desktop | Out-Null
        $deadline = (Get-Date).AddSeconds(180)
        do {
            Start-Sleep -Seconds 4
            try { docker info 2>$null | Out-Null; $daemon = $true } catch { $daemon = $false }
        } while (-not $daemon -and (Get-Date) -lt $deadline)
    }
}
if (-not $daemon) { Die 'Docker daemon is not running. Start Docker Desktop and re-run.' }
Ok 'Docker daemon running'

# ── 3. config ───────────────────────────────────────────────────────────────
Head '3/6  Writing .env'

function Set-EnvValue([string]$Path, [string]$Key, [string]$Value) {
    $lines = @(Get-Content -Path $Path)
    $found = $false
    $out = foreach ($line in $lines) {
        if ($line -match "^\s*$([regex]::Escape($Key))\s*=") {
            $found = $true
            "$Key=$Value"
        } else { $line }
    }
    if (-not $found) { $out = @($out) + "$Key=$Value" }
    Set-Content -Path $Path -Value $out -Encoding UTF8
}

if (-not (Test-Path '.env')) {
    Copy-Item '.env.example' '.env'
    Ok 'created .env from .env.example'
} else {
    Ok '.env already exists'
}

$halx = (Select-String -Path '.env' -Pattern '^\s*HALX_API_KEY\s*=\s*(.+)$').Matches.Groups[1].Value
if ([string]::IsNullOrWhiteSpace($halx)) {
    Write-Host '  HALX_API_KEY is empty. Paste it now (input is hidden), or press Enter to skip.' -ForegroundColor Yellow
    $k = Read-Host '  HALX_API_KEY' -AsSecureString
    $plain = [Runtime.InteropServices.Marshal]::PtrToStringAuto(
        [Runtime.InteropServices.Marshal]::SecureStringToBSTR($k))
    if (-not [string]::IsNullOrWhiteSpace($plain)) { Set-EnvValue '.env' 'HALX_API_KEY' $plain; Ok 'saved HALX_API_KEY' }
    else { Warn 'skipped — routing/rerank/citations will fail until you add it' }
} else { Ok 'HALX_API_KEY present' }

$llm = (Select-String -Path '.env' -Pattern '^\s*LLM_API_KEY\s*=\s*(.+)$').Matches.Groups[1].Value
if ([string]::IsNullOrWhiteSpace($llm)) {
    Write-Host '  LLM_API_KEY is empty. Paste it now (input is hidden), or press Enter to skip.' -ForegroundColor Yellow
    $k2 = Read-Host '  LLM_API_KEY' -AsSecureString
    $plain2 = [Runtime.InteropServices.Marshal]::PtrToStringAuto(
        [Runtime.InteropServices.Marshal]::SecureStringToBSTR($k2))
    if (-not [string]::IsNullOrWhiteSpace($plain2)) { Set-EnvValue '.env' 'LLM_API_KEY' $plain2; Ok 'saved LLM_API_KEY' }
    else { Warn 'skipped — chat answers will be unavailable until you add it' }
} else { Ok 'LLM_API_KEY present' }

# ── 4. isolation guard ──────────────────────────────────────────────────────
Head '4/6  Port isolation (27432 / 27810)'
& (Join-Path $PSScriptRoot 'check-ports.ps1') -AllowBound

# ── 5. database ─────────────────────────────────────────────────────────────
Head '5/6  Starting Postgres'
& docker compose -p blooai -f docker-compose.yml up -d db
if ($LASTEXITCODE -ne 0) { Die 'docker compose up db failed' }

$deadline = (Get-Date).AddSeconds(90)
do {
    Start-Sleep -Seconds 2
    $status = (& docker inspect --format '{{.State.Health.Status}}' blooai-db 2>$null)
} while ($status -ne 'healthy' -and (Get-Date) -lt $deadline)

if ($status -ne 'healthy') { Die 'Postgres did not become healthy. Run: docker logs blooai-db' }
Ok 'Postgres healthy on 27432 (volume blooai_pgdata)'

# ── 6. build + launch ───────────────────────────────────────────────────────
Head '6/6  Building and launching'

if ($SkipBuild) {
    Warn 'skipping build (-SkipBuild)'
} else {
    cargo build -p bloo_api
    if ($LASTEXITCODE -ne 0) { Die 'cargo build failed' }
    Ok 'binary built'
}

# open the browser once /health answers
$opener = Start-Job -ScriptBlock {
    for ($i = 0; $i -lt 60; $i++) {
        Start-Sleep -Milliseconds 800
        try {
            $r = Invoke-RestMethod 'http://127.0.0.1:27810/health' -TimeoutSec 2
            if ($r.status -eq 'ok') { Start-Process 'http://127.0.0.1:27810'; break }
        } catch { }
    }
} | Out-Null

Write-Host "`nBloo AI is starting." -ForegroundColor Green
Write-Host "  chat   http://127.0.0.1:27810/"      -ForegroundColor DarkGray
Write-Host "  admin  http://127.0.0.1:27810/admin" -ForegroundColor DarkGray
Write-Host "  stop   Ctrl+C (Postgres keeps running; scripts\stop.ps1 to stop it)`n" -ForegroundColor DarkGray

cargo run -p bloo_api
