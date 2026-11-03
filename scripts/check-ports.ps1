# Bloo AI - port isolation guard.
# Fails if 27432 or 27810 is taken by something that is not ours.
# This is the wall between Bloo and every other local project.

[CmdletBinding()]
param(
    [switch]$AllowBound
)

$ports = @(27432, 27810, 27812)

foreach ($port in $ports) {
    $conns = Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction SilentlyContinue
    if (-not $conns) {
        Write-Host "  port $port free" -ForegroundColor DarkGray
        continue
    }

    # Which containers publish this port?
    $containers = @()
    try {
        $containers = @(& docker ps --format '{{.Names}}' --filter "publish=$port" 2>$null)
    } catch { $containers = @() }

    $isBloo = $false
    foreach ($c in $containers) { if ($c -like '*blooai*') { $isBloo = $true } }

    if ($isBloo) {
        Write-Host "  port $port held by Bloo ($($containers -join ', '))" -ForegroundColor DarkGray
        continue
    }

    # Human-readable owner of the socket.
    $names = foreach ($p in ($conns | Select-Object -ExpandProperty OwningProcess -Unique)) {
        $proc = Get-Process -Id $p -ErrorAction SilentlyContinue
        if ($proc) { "$($proc.ProcessName) (pid $p)" } else { "pid $p" }
    }

    $msg = "port $port is in use by: $($names -join ', ')"
    if ($AllowBound) {
        Write-Warning "$msg - continuing, but Bloo may fail to bind."
    } else {
        throw "$msg. Bloo must stay on 27432/27810. Stop the other process; do not change Bloo."
    }
}

Write-Host '  port check passed' -ForegroundColor Green
