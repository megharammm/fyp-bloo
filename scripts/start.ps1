# Bloo AI - start the server. Everything else is already provisioned.
#
#   scripts\start.ps1            start (pulls nothing, keeps your local state)
#   scripts\start.ps1 -Pull      git pull --ff-only first
#   scripts\start.ps1 -Fast      skip the build step

[CmdletBinding()]
param(
    [switch]$Pull,
    [switch]$Fast
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo

if (-not (Test-Path '.env')) {
    Write-Host 'First run detected. Running setup instead...' -ForegroundColor Yellow
    & (Join-Path $PSScriptRoot 'setup.ps1')
    exit $LASTEXITCODE
}

if (-not (Select-String -Path '.env' -Pattern '^\s*HALX_API_KEY\s*=\s*\S' -Quiet)) {
    Write-Host '  note: HALX_API_KEY is empty - run scripts\setup.ps1 to add it' -ForegroundColor Yellow
}
if (-not (Select-String -Path '.env' -Pattern '^\s*LLM_API_KEY\s*=\s*\S' -Quiet)) {
    Write-Host '  note: LLM_API_KEY is empty - chat answers will be unavailable' -ForegroundColor Yellow
}

if ($Pull) {
    Write-Host 'pulling...' -ForegroundColor DarkGray
    git pull --ff-only
    if ($LASTEXITCODE -ne 0) { throw 'git pull failed - resolve the branch state first.' }
}

# Docker must be awake; start it if it is installed but asleep.
try { docker info 2>$null | Out-Null } catch {
    $desktop = Join-Path $env:ProgramFiles 'Docker\Docker\Docker Desktop.exe'
    if (Test-Path $desktop) {
        Write-Host 'starting Docker Desktop...' -ForegroundColor DarkGray
        Start-Process $desktop | Out-Null
        $deadline = (Get-Date).AddSeconds(180)
        $up = $false
        do {
            Start-Sleep -Seconds 4
            try { docker info 2>$null | Out-Null; $up = $true } catch { $up = $false }
        } while (-not $up -and (Get-Date) -lt $deadline)
        if (-not $up) { throw 'Docker daemon did not start. Open Docker Desktop manually.' }
    } else { throw 'Docker is not running and Docker Desktop was not found.' }
}

& (Join-Path $PSScriptRoot 'check-ports.ps1') -AllowBound

& docker compose -p blooai -f docker-compose.yml up -d db | Out-Null
$deadline = (Get-Date).AddSeconds(60)
do {
    Start-Sleep -Seconds 2
    $s = (& docker inspect --format '{{.State.Health.Status}}' blooai-db 2>$null)
} while ($s -ne 'healthy' -and (Get-Date) -lt $deadline)
if ($s -ne 'healthy') { throw 'Postgres did not become healthy. Run: docker logs blooai-db' }

if (-not $Fast) {
    cargo build -p bloo_api 2>&1 | Select-Object -Last 3
    if ($LASTEXITCODE -ne 0) { throw 'build failed' }
}

Start-Job -ScriptBlock {
    for ($i = 0; $i -lt 60; $i++) {
        Start-Sleep -Milliseconds 800
        try {
            $r = Invoke-RestMethod 'http://127.0.0.1:27810/health' -TimeoutSec 2
            if ($r.status -eq 'ok') { Start-Process 'http://127.0.0.1:27810'; break }
        } catch { }
    }
} | Out-Null

Write-Host "`nBloo AI live at http://127.0.0.1:27810  (Ctrl+C to stop)`n" -ForegroundColor Green
cargo run -p bloo_api
