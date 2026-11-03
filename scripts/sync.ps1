# Bloo AI - content sync helper (calls the running app).
# Re-scans content/ and data/uploads; only new or changed files are re-ingested.

[CmdletBinding()]
param([string]$BaseUrl = 'http://127.0.0.1:27810')

$ErrorActionPreference = 'Stop'

try {
    $r = Invoke-RestMethod -Method Post -Uri "$BaseUrl/api/admin/sync" -TimeoutSec 120
    $r | ConvertTo-Json -Depth 6
} catch {
    Write-Warning "sync failed: $($_.Exception.Message)"
    Write-Host 'Is the app running?  make dev   (or  make full )' -ForegroundColor DarkGray
    exit 1
}
