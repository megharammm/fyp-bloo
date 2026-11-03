# Bloo AI — port isolation guard.
# Fails if 27432 or 27810 is taken by something that is not ours.
# This is the wall between Bloo and every other local project.

[CmdletBinding()]
param(
    [switch]$AllowBound
)

$ports = @(27432, 27810)

foreach ($port in $ports) {
    $conns = Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction SilentlyContinue
    if (-not $conns) {
        Write-Host "  port $port free" -ForegroundColor DarkGray
        continue
    }

    $pids = $conns | Select-Object -ExpandProperty OwningProcess -Unique
    $names = foreach ($p in $pids) {
        $proc = Get-Process -Id $p -ErrorAction SilentlyContinue
        if ($proc) { "$($proc.ProcessName) (pid $p)" } else { "pid $p" }
    }

    $ours = $conns | Where-Object { $_.OwningProcess -in $pids } | ForEach-Object {
        (& docker inspect --format '{{.Name}}' (& docker ps -q --filter "publish=$port") 2>$null)
    } | Where-Object { $_ -like '*blooai*' }

    if ($ours) {
        Write-Host "  port $port held by Bloo ($ours)" -ForegroundColor DarkGray
        continue
    }

    $msg = "port $port is already in use by: $($names -join ', ')"
    if ($AllowBound) {
        Write-Warning "$msg — continuing, but Bloo may fail to bind."
    } else {
        throw "$msg. Bloo must stay on 27432/27810. Stop the other process or change nothing in Bloo."
    }
}

Write-Host '  port check passed' -ForegroundColor Green
