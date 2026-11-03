# Bloo AI - verify the chat LLM endpoint and key in one call.
#
#   scripts\check-llm.ps1
#   scripts\check-llm.ps1 -BaseUrl https://opencode.ai/inference/go/openai/v1
#
# Reads LLM_BASE_URL / LLM_API_KEY / LLM_MODEL from .env, normalises the base URL,
# sends a one-token request, and explains any failure.
#
# OpenCode has TWO incompatible base paths:
#   Go subscription : https://opencode.ai/inference/go/openai/v1
#   Console pay-as-you-go : https://opencode.ai/inference/openai/v1
# A Go key on the Console path authenticates but returns 402 (no PAYG balance).

[CmdletBinding()]
param(
    [string]$EnvFile = '.env',
    [string]$BaseUrl = ''
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

$key   = Get-EnvValue $EnvFile 'LLM_API_KEY'
$model = Get-EnvValue $EnvFile 'LLM_MODEL'
if ([string]::IsNullOrWhiteSpace($BaseUrl)) { $BaseUrl = Get-EnvValue $EnvFile 'LLM_BASE_URL' }

if ([string]::IsNullOrWhiteSpace($key)) {
    Write-Host 'LLM_API_KEY is empty.' -ForegroundColor Yellow
    Write-Host 'Create a key at https://opencode.ai/console, then set:' -ForegroundColor DarkGray
    Write-Host '  LLM_BASE_URL=https://opencode.ai/inference/go/openai/v1   (Go subscription)' -ForegroundColor DarkGray
    Write-Host '  LLM_BASE_URL=https://opencode.ai/inference/openai/v1      (Console pay-as-you-go)' -ForegroundColor DarkGray
    exit 1
}

if ([string]::IsNullOrWhiteSpace($BaseUrl)) { $BaseUrl = 'https://api.openai.com/v1' }
if ([string]::IsNullOrWhiteSpace($model)) { $model = 'gpt-4o-mini' }

$BaseUrl = $BaseUrl.TrimEnd('/')
if ($BaseUrl -match '/chat/completions$') {
    Write-Host 'note: LLM_BASE_URL ends with /chat/completions - Bloo appends that itself.' -ForegroundColor Yellow
    $BaseUrl = $BaseUrl -replace '/chat/completions$', ''
    Write-Host "      use instead: $BaseUrl" -ForegroundColor Yellow
}
$url = "$BaseUrl/chat/completions"
$isGo = $BaseUrl -match '/inference/go/'

# Go asks clients to identify themselves and send a stable session id.
$session = [guid]::NewGuid().ToString()
$headers = @{
    Authorization        = "Bearer $key"
    'User-Agent'         = 'BlooAI/0.1'
    'x-opencode-session' = $session
}

Write-Host "POST $url" -ForegroundColor Cyan
Write-Host "  model  $model" -ForegroundColor DarkGray
Write-Host "  key    len=$($key.Length) prefix=$($key.Substring(0,[Math]::Min(6,$key.Length)))" -ForegroundColor DarkGray
Write-Host "  mode   $(if ($isGo) { 'OpenCode Go' } else { 'generic / Console' })" -ForegroundColor DarkGray

$body = @{
    model      = $model
    messages   = @(@{ role = 'user'; content = 'Reply with exactly: ok' })
    max_tokens = 64
    stream     = $false
} | ConvertTo-Json -Depth 6

$sw = [Diagnostics.Stopwatch]::StartNew()
try {
    $r = Invoke-RestMethod -Uri $url -Method POST -Headers $headers `
         -ContentType 'application/json' -Body $body -TimeoutSec 90
    $sw.Stop()
    $text = $r.choices[0].message.content
    $finish = $r.choices[0].finish_reason
    Write-Host "  OK   HTTP 200 in $([math]::Round($sw.Elapsed.TotalSeconds,2)) s" -ForegroundColor Green
    Write-Host "  served  $($r.model)" -ForegroundColor DarkGray
    if ($r.usage) {
        Write-Host "  tokens  in=$($r.usage.prompt_tokens) out=$($r.usage.completion_tokens)" -ForegroundColor DarkGray
    }
    Write-Host "  finish  $finish" -ForegroundColor DarkGray
    if ([string]::IsNullOrWhiteSpace($text)) {
        Write-Host '  reply   <empty>' -ForegroundColor Yellow
        Write-Host '  reasoning models can spend the whole token budget before emitting text;' -ForegroundColor DarkGray
        Write-Host '  the endpoint and key are fine. Bloo uses max_tokens=500.' -ForegroundColor DarkGray
    } else {
        Write-Host "  reply   $text" -ForegroundColor DarkGray
    }
    exit 0
} catch {
    $sw.Stop()
    $resp = $_.Exception.Response
    if ($null -eq $resp) {
        Write-Host "  FAIL no response after $([math]::Round($sw.Elapsed.TotalSeconds,1)) s" -ForegroundColor Red
        Write-Host "  $($_.Exception.Message)" -ForegroundColor DarkGray
        exit 1
    }
    $code = [int]$resp.StatusCode
    Write-Host "  FAIL HTTP $code" -ForegroundColor Red
    switch ($code) {
        401 {
            Write-Host '  the key was rejected by the provider.' -ForegroundColor Yellow
            Write-Host '  Create or re-copy one at https://opencode.ai/console -> API keys.' -ForegroundColor DarkGray
        }
        402 {
            Write-Host '  the key authenticated, but this endpoint has no balance.' -ForegroundColor Yellow
            if (-not $isGo) {
                Write-Host '  If this is an OpenCode GO subscription, the base must be:' -ForegroundColor Yellow
                Write-Host '    https://opencode.ai/inference/go/openai/v1' -ForegroundColor DarkGray
            } else {
                Write-Host '  Go allowance exhausted, or the model is not included in your plan.' -ForegroundColor DarkGray
            }
        }
        403 { Write-Host '  the key is valid but this model is not allowed for it.' -ForegroundColor Yellow }
        404 {
            Write-Host '  the path does not exist - LLM_BASE_URL is probably wrong.' -ForegroundColor Yellow
            Write-Host "  expected shape: https://host/v1  ->  $BaseUrl/chat/completions" -ForegroundColor DarkGray
        }
        429 {
            Write-Host '  rate limited by the provider.' -ForegroundColor Yellow
            Write-Host '  Go: check your 5-hour allowance at https://opencode.ai/console' -ForegroundColor DarkGray
        }
        default {
            try {
                $sr = New-Object System.IO.StreamReader($resp.GetResponseStream())
                Write-Host "  $($sr.ReadToEnd())" -ForegroundColor DarkGray
            } catch { }
        }
    }
    exit 1
}
