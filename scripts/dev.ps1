# Bloo AI - dev loop. Postgres in Docker, API on the host (fast rebuilds).
# Frontend and backend are one Rust binary; there is no Node process anywhere.

[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo

if (-not (Test-Path (Join-Path $repo '.env'))) {
    Copy-Item (Join-Path $repo '.env.example') (Join-Path $repo '.env')
    Write-Host 'created .env from .env.example - add HALX_API_KEY and LLM_API_KEY' -ForegroundColor Yellow
}

& (Join-Path $PSScriptRoot 'check-ports.ps1') -AllowBound

Write-Host '`nstarting postgres...' -ForegroundColor Cyan
& docker compose -p blooai -f docker-compose.yml up -d db

Write-Host 'waiting for healthy...' -ForegroundColor DarkGray
$deadline = (Get-Date).AddSeconds(60)
do {
    Start-Sleep -Seconds 2
    $s = (& docker inspect --format '{{.State.Health.Status}}' blooai-db 2>$null)
} while ($s -ne 'healthy' -and (Get-Date) -lt $deadline)

if ($s -ne 'healthy') { throw 'Postgres did not start. Run: docker logs blooai-db' }

Write-Host "`npostgres ready on 27432" -ForegroundColor Green
Write-Host 'running API on http://127.0.0.1:27810' -ForegroundColor Green
$ErrorActionPreference = 'Continue'
& cargo run --quiet -p bloo_api --bin bloo_api
