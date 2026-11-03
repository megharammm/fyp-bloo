# Bloo AI - verify a HAL-X key in five seconds.
#
#   scripts\check-halx.ps1
#
# GET /v1/models is the cheapest authenticated call:
#   200 -> key valid, lists the models you may use
#   401 -> key missing, unknown, revoked or expired
#   403 -> key exists but has no THX-01 grant, or thx-01 is not allowed for it

[CmdletBinding()]
param(
    [string]$EnvFile = '.env',
    [string]$BaseUrl = 'https://api.hal-x.ai'
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo

function Get-EnvValue([string]$Path, [string]$Key) {
    if (-not (Test-Path $Path)) { return '' }
    $pattern = '^\s*' + [regex]::Escape($Key) + '\s*=(.*)$'
    $line = Get-Content -Path $Path -Encoding UTF8 -ErrorAction SilentlyContinue |
            Where-Object { $_ -match $pattern } | Select-Object -First 1
    if ([string]::IsNullOrEmpty($line)) { return '' }
    return ([regex]::Match($line, $pattern)).Groups[1].Value.Trim().Trim('"').Trim("'")
}

if (-not (Test-Path $EnvFile)) {
    Write-Host "no $EnvFile - run scripts\setup.ps1 first" -ForegroundColor Red
    exit 1
}

$key = Get-EnvValue $EnvFile 'HALX_API_KEY'
if ([string]::IsNullOrWhiteSpace($key)) {
    Write-Host 'HALX_API_KEY is empty in .env.' -ForegroundColor Yellow
    Write-Host 'HAL-X has no public signup: request a key from the HAL-X team, with the' -ForegroundColor Yellow
    Write-Host 'THX-01 decisions grant (/v1/systemone) and model thx-01 allowed.' -ForegroundColor Yellow
    Write-Host 'Bloo still runs in local-decider mode without it - see PLAYBOOK.md 8.1.' -ForegroundColor DarkGray
    exit 1
}

Write-Host "checking $BaseUrl/v1/models ..." -ForegroundColor Cyan
try {
    $r = Invoke-WebRequest -Uri "$BaseUrl/v1/models" -Method GET `
         -Headers @{ Authorization = "Bearer $key" } -TimeoutSec 10 -UseBasicParsing
    Write-Host "  OK   key accepted (HTTP $($r.StatusCode))" -ForegroundColor Green
    $models = $r.Content | ConvertFrom-Json
    $models | ConvertTo-Json -Depth 6
    exit 0
} catch {
    $resp = $_.Exception.Response
    if ($null -eq $resp) {
        Write-Host "  FAIL no response: $($_.Exception.Message)" -ForegroundColor Red
        Write-Host '  network blocked, DNS failure, or the API is down' -ForegroundColor DarkGray
        exit 1
    }
    $code = [int]$resp.StatusCode
    switch ($code) {
        401 {
            Write-Host '  FAIL 401 api_key_invalid' -ForegroundColor Red
            Write-Host '  key is missing, unknown, revoked or expired' -ForegroundColor DarkGray
        }
        403 {
            Write-Host '  FAIL 403 forbidden' -ForegroundColor Red
            Write-Host '  key has no THX-01 grant, or thx-01 is not in its allowed models' -ForegroundColor DarkGray
        }
        429 {
            Write-Host '  WARN 429 concurrency limit - the key works, retry later' -ForegroundColor Yellow
        }
        default {
            Write-Host "  FAIL HTTP $code" -ForegroundColor Red
            try {
                $body = (New-Object System.IO.StreamReader($resp.GetResponseStream())).ReadToEnd()
                Write-Host "  $body" -ForegroundColor DarkGray
            } catch { }
        }
    }
    exit 1
}
