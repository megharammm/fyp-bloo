# Bloo AI - run THX-01 as a HOST process (recommended on this laptop).
#
#   scripts\start-thx01.ps1
#   scripts\start-thx01.ps1 -Port 8095 -ApiKey bloo-local
#
# Why host instead of Docker:
#   * the app already runs on the host in dev, so 127.0.0.1 is direct
#   * torch inside WSL2 duplicates ~2 GB of RAM and can wedge Docker Desktop
#   * no image rebuild when you change model settings
#
# First run creates .venv-thx01 and installs CPU-only torch (~200 MB).

[CmdletBinding()]
param(
    [int]$Port = 8095,
    [string]$ApiKey = 'bloo-local',
    [string]$Model = 'doofz/THX-01',
    [int]$Threads = 4
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo

function Ok($m)   { Write-Host "  OK   $m" -ForegroundColor Green }
function Warn($m) { Write-Host "  ..   $m" -ForegroundColor Yellow }
function Die($m)  { Write-Host "  FAIL $m" -ForegroundColor Red; exit 1 }

$venv = Join-Path $repo '.venv-thx01'
$py   = Join-Path $venv 'Scripts\python.exe'

if (-not (Test-Path $py)) {
    if (-not (Get-Command python -ErrorAction SilentlyContinue)) {
        Die 'python not found. Install Python 3.10+ (winget install Python.Python.3.12)'
    }
    Write-Host 'creating .venv-thx01 (one time)...' -ForegroundColor Cyan
    python -m venv $venv
    if ($LASTEXITCODE -ne 0) { Die 'could not create the virtualenv' }

    Ok 'venv created'
    Write-Host 'installing CPU-only torch (~200 MB)...' -ForegroundColor Cyan
    & $py -m pip install --upgrade pip | Out-Null
    & $py -m pip install torch --index-url https://download.pytorch.org/whl/cpu
    if ($LASTEXITCODE -ne 0) { Die 'torch install failed' }

    Write-Host 'installing thx01[server]...' -ForegroundColor Cyan
    & $py -m pip install "thx01[server]"
    if ($LASTEXITCODE -ne 0) { Die 'thx01 install failed' }
    Ok 'dependencies installed'
} else {
    Ok "using $venv"
}

# Threads: torch defaults to every core, which makes the whole laptop stutter.
$env:OMP_NUM_THREADS = "$Threads"
$env:MKL_NUM_THREADS = "$Threads"
$env:THX01_MODEL     = $Model
$env:THX01_DEVICE    = 'cpu'
$env:THX01_API_KEY   = $ApiKey

Write-Host ''
Write-Host "THX-01 starting on http://127.0.0.1:$Port" -ForegroundColor Green
Write-Host 'First start downloads ~645 MB of weights into your HuggingFace cache.' -ForegroundColor DarkGray
Write-Host ''
Write-Host 'Point Bloo at it - add these two lines to .env, then restart Bloo:' -ForegroundColor Cyan
Write-Host "  HALX_BASE_URL=http://127.0.0.1:$Port" -ForegroundColor DarkGray
Write-Host "  HALX_API_KEY=$ApiKey" -ForegroundColor DarkGray
Write-Host '  BLOO_DECIDER=thx' -ForegroundColor DarkGray
Write-Host ''
Write-Host "Verify from another terminal: scripts\test-thx01.ps1 -BaseUrl http://127.0.0.1:$Port" -ForegroundColor DarkGray
Write-Host 'Ctrl+C to stop.' -ForegroundColor DarkGray
Write-Host ''

& $py -m thx01.server --host 127.0.0.1 --port $Port
