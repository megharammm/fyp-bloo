# Bloo AI - reset the local database to a clean slate.
#
#   scripts\reset-db.ps1          asks for confirmation
#   scripts\reset-db.ps1 -Force   no prompt
#
# Use this if sqlx reports "migration N was previously applied but has been modified"
# (happens when line endings of a migration file change after it was applied), or when
# you simply want a clean demo. It deletes documents, chunks, chat history and uploads.

[CmdletBinding()]
param([switch]$Force)

$ErrorActionPreference = 'Continue'
$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo

if (-not $Force) {
    Write-Host 'This deletes ALL Bloo data: documents, chunks, chat history and data/raw.' -ForegroundColor Yellow
    $answer = Read-Host 'Type RESET to continue'
    if ($answer -ne 'RESET') {
        Write-Host 'aborted' -ForegroundColor DarkGray
        exit 1
    }
}

Get-Process bloo_api, worker -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue

& docker exec blooai-db psql -U bloo -d blooai -v ON_ERROR_STOP=1 `
    -c "drop schema public cascade;" -c "create schema public;" 2>&1 | Select-Object -Last 3
if ($LASTEXITCODE -ne 0) {
    Write-Host 'database reset failed - is blooai-db running on 27432?' -ForegroundColor Red
    exit 1
}

Get-ChildItem 'data\raw' -File -ErrorAction SilentlyContinue | Remove-Item -Force -ErrorAction SilentlyContinue

Write-Host ''
Write-Host 'database and data/raw cleared.' -ForegroundColor Green
Write-Host 'Start the app to re-run migrations, then press "Sync content folder" on /admin.' -ForegroundColor DarkGray
