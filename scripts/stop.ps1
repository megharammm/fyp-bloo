# Bloo AI - stop everything Bloo owns. Never touches other projects.
# Data is preserved: the Postgres volume `blooai_pgdata` is not removed.

[CmdletBinding()]
param([switch]$KeepDatabase)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo

# Stop the app process if it was started from this repo.
Get-Process -Name 'bloo_api' -ErrorAction SilentlyContinue | ForEach-Object {
    Write-Host "stopping bloo_api (pid $($_.Id))" -ForegroundColor DarkGray
    Stop-Process -Id $_.Id -Force
}

if (-not $KeepDatabase) {
    Write-Host 'stopping blooai-db' -ForegroundColor DarkGray
    & docker compose -p blooai -f docker-compose.yml stop db
}

Write-Host "`nBloo AI stopped. Data kept in volume blooai_pgdata." -ForegroundColor Green
