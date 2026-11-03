# Bloo AI - WP-03 smoke test (retrieval + answering).
#
#   scripts\smoke-wp03.ps1
#
# Rebuilds a small corpus from content/seed/*.md, publishes it, then checks:
#   A. hit@5 for six known questions
#   B. answers are grounded, cited, and citations are verbatim text from the corpus
#   C. four out-of-scope questions are refused
#   D. SSE streaming emits token + citations events
#   E. p95 latency

[CmdletBinding()]
param(
    [string]$BaseUrl = 'http://127.0.0.1:27810',
    [int]$P95BudgetMs = 12000
)

$ErrorActionPreference = 'Continue'
$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo

$results = New-Object System.Collections.ArrayList
function Check([string]$Name, [bool]$Ok, [string]$Detail) {
    [void]$results.Add([pscustomobject]@{ Check = $Name; Ok = $Ok; Detail = $Detail })
    $colour = if ($Ok) { 'Green' } else { 'Red' }
    Write-Host ("  {0,-44} {1}  {2}" -f $Name, $(if ($Ok) { 'PASS' } else { 'FAIL' }), $Detail) -ForegroundColor $colour
}

function Sql([string]$q) {
    $prev = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $out = & docker exec blooai-db psql -U bloo -d blooai -t -A -c $q 2>&1
    $code = $LASTEXITCODE
    $ErrorActionPreference = $prev
    if ($code -ne 0) { throw "psql failed: $out" }
    $clean = @($out | Where-Object {
        $_ -notmatch '^(NOTICE|WARNING|HINT):' -and $_ -notmatch '^(INSERT|UPDATE|DELETE|SELECT|TRUNCATE) \d'
    })
    return ($clean -join "`n").Trim()
}

function Normalize([string]$s) {
    $s = $s -replace '[\*#`_]', ''
    return (($s -split '\s+') -join ' ').Trim()
}

$questions = @(
    @{ Q = 'What is the hostel fee per semester?';                     Doc = 'fees.md' },
    @{ Q = 'How many books can an undergraduate borrow?';              Doc = 'library.md' },
    @{ Q = 'When does the revaluation window open?';                   Doc = 'exams.md' },
    @{ Q = 'Which bus route goes to the north campus?';                Doc = 'transport.md' },
    @{ Q = 'What are the mess timings?';                               Doc = 'hostel.md' },
    @{ Q = 'What is the deadline for the merit scholarship?';          Doc = 'fees.md' }
)

$outOfScope = @(
    'Who won the 2011 cricket world cup?',
    'What is the capital of France?',
    'How do I fix a diesel engine?',
    'What is the price of Bitcoin today?'
)

Write-Host "`n== reset and ingest ==" -ForegroundColor Cyan
Sql "truncate table chunks, jobs, documents restart identity cascade;" | Out-Null
Get-ChildItem 'data\raw' -File -ErrorAction SilentlyContinue | Remove-Item -Force -ErrorAction SilentlyContinue

Get-Process bloo_api, worker -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 600

$apiLog = Join-Path $env:TEMP 'bloo-api-wp03.log'
$apiErr = Join-Path $env:TEMP 'bloo-api-wp03.err'
$wkLog = Join-Path $env:TEMP 'bloo-worker-wp03.log'
$wkErr = Join-Path $env:TEMP 'bloo-worker-wp03.err'
$api = Start-Process .\target\debug\bloo_api.exe -PassThru -NoNewWindow -RedirectStandardOutput $apiLog -RedirectStandardError $apiErr
$wk = Start-Process .\target\debug\worker.exe -PassThru -NoNewWindow -RedirectStandardOutput $wkLog -RedirectStandardError $wkErr

$deadline = (Get-Date).AddSeconds(60)
do {
    Start-Sleep -Milliseconds 800
    try { $h = Invoke-RestMethod "$BaseUrl/health" -TimeoutSec 3 } catch { $h = $null }
} while (-not $h -and (Get-Date) -lt $deadline)
if (-not $h) { throw "API did not start; see $apiErr" }

$docs = Get-ChildItem content\seed\*.md | Where-Object { $_.Name -ne 'README.md' }
foreach ($doc in $docs) {
    & curl.exe -s -X POST -F "file=@$($doc.FullName)" "$BaseUrl/api/admin/documents" | Out-Null
}

$deadline = (Get-Date).AddMinutes(3)
do {
    Start-Sleep -Seconds 2
    $pending = [int](Sql "select count(*) from jobs where state in ('queued','running');")
} while ($pending -gt 0 -and (Get-Date) -lt $deadline)

Sql "update documents set status='published', updated_at=now() where status='ready';" | Out-Null
$chunks = [int](Sql "select count(*) from searchable_chunks;")
Write-Host "  published corpus: $($docs.Count) documents, $chunks chunks" -ForegroundColor DarkGray

$corpus = ($docs | ForEach-Object { Normalize (Get-Content $_.FullName -Raw) }) -join "`n"

try {
    Write-Host "`n== A. hit@5 ==" -ForegroundColor Cyan
    $hits = 0
    foreach ($item in $questions) {
        $q = [uri]::EscapeDataString($item.Q)
        $s = Invoke-RestMethod "$BaseUrl/api/search?q=$q" -TimeoutSec 60
        $top5 = @($s.hits | Select-Object -First 5)
        $found = @($top5 | Where-Object { $_.title -eq $item.Doc }).Count -gt 0
        if ($found) { $hits++ }
        Check "hit@5: $($item.Q)" $found "$($top5[0].title) (sim $([math]::Round($top5[0].similarity,3)))"
    }

    Write-Host "`n== B. answers and citations ==" -ForegroundColor Cyan
    $latencies = @()
    $allCited = $true
    $allVerbatim = $true
    $allNoPdfLinks = $true

    foreach ($item in $questions) {
        $body = @{ question = $item.Q; stream = $false } | ConvertTo-Json
        $sw = [Diagnostics.Stopwatch]::StartNew()
        $c = Invoke-RestMethod "$BaseUrl/api/chat" -Method POST -ContentType 'application/json' -Body $body -TimeoutSec 180
        $sw.Stop()
        $latencies += $sw.Elapsed.TotalMilliseconds

        if ($c.refused -or -not $c.answer) { $allCited = $false }

        foreach ($cit in @($c.citations)) {
            if ($cit.kind -ne 'html' -and $cit.url) { $allNoPdfLinks = $false }
            $nq = Normalize $cit.quote
            if ($nq.Length -lt 8 -or -not $corpus.Contains($nq)) { $allVerbatim = $false }
        }

        Check "answer: $($item.Q)" (-not $c.refused -and [bool]$c.answer) "$([math]::Round($sw.Elapsed.TotalSeconds,2))s, $($c.citations.Count) citations"
    }

    Check 'every citation is verbatim corpus text' $allVerbatim ''
    Check 'no link on non-web citations' $allNoPdfLinks ''

    Write-Host "`n== C. refusals ==" -ForegroundColor Cyan
    $refused = 0
    foreach ($q in $outOfScope) {
        $body = @{ question = $q; stream = $false } | ConvertTo-Json
        $c = Invoke-RestMethod "$BaseUrl/api/chat" -Method POST -ContentType 'application/json' -Body $body -TimeoutSec 180
        $ok = [bool]$c.refused
        if ($ok) { $refused++ }
        Check "refuse: $q" $ok "sim $([math]::Round($c.meta.best_similarity,3))"
    }

    Write-Host "`n== D. streaming ==" -ForegroundColor Cyan
    $jf = Join-Path $env:TEMP 'bloo-sse.json'
    '{"question":"What is the hostel fee per semester?","stream":true}' | Set-Content $jf -Encoding ASCII
    $raw = & curl.exe -s -N -X POST -H "Content-Type: application/json" --data-binary "@$jf" "$BaseUrl/api/chat"
    $joined = (@($raw) -join "`n")
    Check 'SSE emitted token events' ($joined -match 'event: ?token') ''
    Check 'SSE emitted citations' ($joined -match 'event: ?citations') ''
    Check 'SSE emitted done' ($joined -match 'event: ?done') ''

    Write-Host "`n== E. latency ==" -ForegroundColor Cyan
    $sorted = $latencies | Sort-Object
    $p95 = $sorted[[math]::Min($sorted.Count - 1, [int][math]::Ceiling($sorted.Count * 0.95) - 1)]
    $avg = ($latencies | Measure-Object -Average).Average
    Check 'p95 within budget' ($p95 -le $P95BudgetMs) "avg $([math]::Round($avg))ms, p95 $([math]::Round($p95))ms (budget ${P95BudgetMs}ms)"
}
finally {
    Stop-Process -Id $api.Id -Force -ErrorAction SilentlyContinue
    Stop-Process -Id $wk.Id -Force -ErrorAction SilentlyContinue
}

$failed = ($results | Where-Object { -not $_.Ok }).Count
Write-Host "`n== verdict ==" -ForegroundColor Cyan
$results | Format-Table -AutoSize | Out-String | Write-Host
if ($failed -eq 0) {
    Write-Host "WP-03 SMOKE TEST: PASS ($($results.Count)/$($results.Count))" -ForegroundColor Green
    exit 0
} else {
    Write-Host "WP-03 SMOKE TEST: FAIL ($failed of $($results.Count) checks)" -ForegroundColor Red
    exit 1
}

