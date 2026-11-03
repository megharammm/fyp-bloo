# Test the self-hosted THX-01 sidecar with a Bloo-shaped request.
#
#   scripts\test-thx01.ps1
#   scripts\test-thx01.ps1 -BaseUrl http://127.0.0.1:8095    # plain python server
#
# Answers three questions in one pass: campus topic, a stated fee, and a verbatim quote.

[CmdletBinding()]
param(
    [string]$BaseUrl = 'http://127.0.0.1:27812',
    [string]$ApiKey = 'bloo-local'
)

$ErrorActionPreference = 'Stop'

$doc = @"
BLOO INSTITUTE OF TECHNOLOGY - HOSTEL CIRCULAR 2026-27

The hostel fee for first year students is Rs 48,000 per semester, payable before 15 August 2026.
Refunds are pro-rata and require a written application to the hostel warden within 30 days.
Mess charges are billed separately at Rs 3,200 per month.
"@

$body = @{
    state = @{ document = $doc }
    questions = @{
        topic = @{
            type = 'choice'
            question = 'Which campus area does this document concern?'
            criteria = @{
                fees    = 'tuition, hostel fee, refund, payment, scholarship, fine'
                hostel  = 'rooms, wardens, mess, rules, vacating'
                exams   = 'timetable, results, revaluation, backlog'
                general = 'anything else campus related'
            }
        }
        fee = @{
            type = 'number'
            question = 'What is the hostel fee, in rupees?'
            cite = $true
        }
        quote = @{
            type = 'excerpt'
            question = 'What does a refund require?'
        }
        is_relevant = @{
            type = 'noul'
            question = 'Is this document relevant to student fee questions?'
        }
    }
} | ConvertTo-Json -Depth 8

Write-Host "POST $BaseUrl/v1/systemone" -ForegroundColor Cyan
$sw = [Diagnostics.Stopwatch]::StartNew()
try {
    $r = Invoke-RestMethod -Uri "$BaseUrl/v1/systemone" -Method POST `
         -Headers @{ Authorization = "Bearer $ApiKey" } `
         -ContentType 'application/json' -Body $body -TimeoutSec 300
} catch {
    Write-Host "  FAIL $($_.Exception.Message)" -ForegroundColor Red
    Write-Host '  Is the sidecar up?' -ForegroundColor DarkGray
    Write-Host '  docker compose -p blooai -f docker-compose.yml --profile thx01 up -d thx01' -ForegroundColor DarkGray
    exit 1
}
$sw.Stop()

$r | ConvertTo-Json -Depth 10

Write-Host ''
Write-Host "wall clock: $([math]::Round($sw.Elapsed.TotalMilliseconds)) ms (model reports $($r.latency_ms) ms)" -ForegroundColor Green
Write-Host "detected  : $($r.detected.language) / $($r.detected.script)" -ForegroundColor DarkGray

$t = $r.answers.topic
if ($t) { Write-Host "topic     : $($t.choice)  (p=$($t.probabilities.$($t.choice)))" -ForegroundColor DarkGray }
if ($r.answers.fee) {
    Write-Host "fee       : $($r.answers.fee.value)  p=$($r.answers.fee.probability)" -ForegroundColor DarkGray
    if ($r.answers.fee.citations) {
        Write-Host "citation  : $($r.answers.fee.citations[0].text)" -ForegroundColor DarkGray
    }
}
if ($r.answers.quote) { Write-Host "quote     : $($r.answers.quote.text)" -ForegroundColor DarkGray }

Write-Host ''
Write-Host 'If topic=fees, fee=48000 and the quote is verbatim, the sidecar is wired correctly.' -ForegroundColor Green
