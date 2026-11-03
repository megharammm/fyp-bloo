# Bloo AI — one-command upgrade.
#   git pull  ->  rebuild image  ->  migrate  ->  restart  ->  re-sync content
#
# Safe by construction: the compose project is pinned to `blooai`, so this can
# never touch another project's containers or volumes (e.g. Trivya-Data-Pipeline).

[CmdletBinding()]
param(
    [switch]$SkipBuild,
    [switch]$SkipContent
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo

if (-not (Test-Path (Join-Path $repo 'Cargo.toml'))) {
    throw "Not inside the Bloo AI repo (no Cargo.toml at $repo)."
}

$compose = @('compose', '-p', 'blooai', '-f', 'docker-compose.yml')

function Step($n, $msg) { Write-Host "`n[$n] $msg" -ForegroundColor Cyan }

# ── 0. sanity: is something else already using our ports? ────────────────────
Step 0 'Checking ports 27432 / 27810'
& (Join-Path $PSScriptRoot 'check-ports.ps1') -AllowBound

# ── 1. code ──────────────────────────────────────────────────────────────────
Step 1 'git pull --ff-only'
git pull --ff-only
if ($LASTEXITCODE -ne 0) { throw "git pull failed. Resolve the branch state manually." }
$commit = (git rev-parse --short HEAD).Trim()
Write-Host "    now at $commit" -ForegroundColor DarkGray

# ── 2. database ──────────────────────────────────────────────────────────────
Step 2 'Starting Postgres (blooai-db on 27432)'
& docker @compose up -d db
if ($LASTEXITCODE -ne 0) { throw "docker compose up db failed." }

Write-Host '    waiting for healthy…' -ForegroundColor DarkGray
$deadline = (Get-Date).AddSeconds(90)
do {
    Start-Sleep -Seconds 2
    $status = (& docker inspect --format '{{.State.Health.Status}}' blooai-db 2>$null)
} while ($status -ne 'healthy' -and (Get-Date) -lt $deadline)

if ($status -ne 'healthy') { throw "Postgres did not become healthy. Run: docker logs blooai-db" }
Write-Host '    healthy' -ForegroundColor Green

# ── 3. app image ─────────────────────────────────────────────────────────────
if (-not $SkipBuild) {
    Step 3 'Building blooai-app (release, cached)'
    & docker @compose --profile full build app
    if ($LASTEXITCODE -ne 0) { throw "docker build failed." }
} else {
    Step 3 'Skipping image build (-SkipBuild)'
}

# ── 4. run ───────────────────────────────────────────────────────────────────
Step 4 'Restarting app on 27810'
& docker @compose --profile full up -d app
if ($LASTEXITCODE -ne 0) { throw "docker compose up app failed." }

Write-Host '    waiting for /health…' -ForegroundColor DarkGray
$deadline = (Get-Date).AddSeconds(60)
$ok = $false
do {
    Start-Sleep -Seconds 2
    try {
        $h = Invoke-RestMethod -Uri 'http://127.0.0.1:27810/health' -TimeoutSec 3
        $ok = ($h.status -eq 'ok')
    } catch { $ok = $false }
} while (-not $ok -and (Get-Date) -lt $deadline)

if (-not $ok) {
    Write-Warning 'App is up but /health is not ok yet. Check: docker logs blooai-app'
} else {
    Write-Host '    healthy' -ForegroundColor Green
}

# ── 5. content ───────────────────────────────────────────────────────────────
if (-not $SkipContent) {
    Step 5 'Syncing content/ + data/uploads (changed files only)'
    try {
        $r = Invoke-RestMethod -Method Post -Uri 'http://127.0.0.1:27810/api/admin/sync' -TimeoutSec 60
        $r | ConvertTo-Json -Depth 5 | Write-Host
    } catch {
        Write-Warning "Content sync endpoint not ready yet (expected until P1): $($_.Exception.Message)"
    }
}

Write-Host "`nBloo AI is running -> http://127.0.0.1:27810" -ForegroundColor Green
Write-Host "  chat  : http://127.0.0.1:27810/" -ForegroundColor DarkGray
Write-Host "  admin : http://127.0.0.1:27810/admin" -ForegroundColor DarkGray
Write-Host "  commit: $commit" -ForegroundColor DarkGray
