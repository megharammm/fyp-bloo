# Bloo AI - verify the chat LLM endpoint and key in one call.
#
#   scripts\check-llm.ps1
#
# Reads LLM_BASE_URL / LLM_API_KEY / LLM_MODEL from .env, normalises the base URL,
# sends a 1-token "reply ok" request, and explains any failure.

[CmdletBinding()]
param([string]$EnvFile = '.env')

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

$base  = Get-EnvValue $EnvFile 'LLM_BASE_URL'
$key   = Get-EnvValue $EnvFile 'LLM_API_KEY'
$model = Get-EnvValue $EnvFile 'LLM_MODEL'

if ([string]::IsNullOrWhiteSpace($key)) {
    Write-Host 'LLM_API_KEY is empty.' -ForegroundColor Yellow
    Write-Host 'Bloo needs an OpenAI-compatible chat endpoint. For OpenCode Console:' -ForegroundColor DarkGray
    Write-Host '  LLM_BASE_URL=https://opencode.ai/inference/openai/v1' -ForegroundColor DarkGray
    Write-Host '  LLM_MODEL=deepseek-v4.1-flash' -ForegroundColor DarkGray
    Write-Host '  create a key at https://opencode.ai/console' -ForegroundColor DarkGray
    exit 1
}

if ([string]::IsNullOrWhiteSpace($base)) { $base = 'https://api.openai.com/v1' }
if ([string]::IsNullOrWhiteSpace($model)) { $model = 'gpt-4o-mini' }

$base = $base.TrimEnd('/')
if ($base -match '/chat/completions$') {
    Write-Host 'note: LLM_BASE_URL ends with /chat/completions - Bloo appends that itself.' -ForegroundColor Yellow
    $base = $base -replace '/chat/completions$', ''
    Write-Host "      use instead: $base" -ForegroundColor Yellow
}
$url = "$base/chat/completions"

Write-Host "POST $url" -ForegroundColor Cyan
Write-Host "  model  $model" -ForegroundColor DarkGray
Write-Host "  key    len=$($key.Length) prefix=$($key.Substring(0,[Math]::Min(6,$key.Length)))" -ForegroundColor DarkGray

$body = @{
    model      = $model
    messages   = @(@{ role = 'user'; content = 'Reply with exactly: ok' })
    max_tokens = 8
    stream     = $false
} | ConvertTo-Json -Depth 6

$sw = [Diagnostics.Stopwatch]::StartNew()
try {
    $r = Invoke-RestMethod -Uri $url -Method POST `
         -Headers @{ Authorization = "Bearer $key" } `
         -ContentType 'application/json' -Body $body -TimeoutSec 60
    $sw.Stop()
    Write-Host "  OK   HTTP 200 in $([math]::Round($sw.Elapsed.TotalSeconds,2)) s" -ForegroundColor Green
    Write-Host "  reply   $($r.choices[0].message.content)" -ForegroundColor DarkGray
    Write-Host "  served  $($r.model)" -ForegroundColor DarkGray
    if ($r.usage) {
        Write-Host "  tokens  in=$($r.usage.prompt_tokens) out=$($r.usage.completion_tokens)" -ForegroundColor DarkGray
    }
    exit 0
} catch {
    $sw.Stop()
    $resp = $_.Exception.Response
    if ($null -eq $resp) {
        Write-Host "  FAIL no response after $([math]::Round($sw.Elapsed.TotalSeconds,1)) s" -ForegroundColor Red
        Write-Host "  $($_.Exception.Message)" -ForegroundColor DarkGray
        Write-Host '  network blocked, DNS failure, or the provider is down' -ForegroundColor DarkGray
        exit 1
    }
    $code = [int]$resp.StatusCode
    Write-Host "  FAIL HTTP $code" -ForegroundColor Red
    switch ($code) {
        401 {
            Write-Host '  the key was rejected by the provider.' -ForegroundColor Yellow
            Write-Host '  For OpenCode: keys from the Go subscription are NOT API keys.' -ForegroundColor DarkGray
            Write-Host '  Create or re-copy one at https://opencode.ai/console -> API keys.' -ForegroundColor DarkGray
        }
        402 { Write-Host '  the account has no credits / quota left.' -ForegroundColor Yellow }
        403 { Write-Host '  the key is valid but this model is not allowed for it.' -ForegroundColor Yellow }
        404 {
            Write-Host '  the path does not exist - LLM_BASE_URL is probably wrong.' -ForegroundColor Yellow
            Write-Host "  expected shape: https://host/v1  ->  $base/chat/completions" -ForegroundColor DarkGray
        }
        429 { Write-Host '  rate limited. The key works; retry in a moment.' -ForegroundColor Yellow }
        default {
            try {
                $sr = New-Object System.IO.StreamReader($resp.GetResponseStream())
                Write-Host "  $($sr.ReadToEnd())" -ForegroundColor DarkGray
            } catch { }
        }
    }
    exit 1
}
