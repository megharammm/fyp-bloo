# Bloo AI - WP-02 smoke test.
#
#   scripts\smoke-wp02.ps1
#
# Verifies the whole ingestion path against a real database and real PDFs:
#   A. 30 uploads -> 30 documents, chunks embedded, jobs done
#   B. duplicate upload is deduplicated
#   C. delete removes the row, its chunks and the stored file
#   D. a job orphaned by a worker crash is requeued and retried
#
# Prints a verdict and exits non-zero if anything fails.

[CmdletBinding()]
param(
    [string]$BaseUrl = 'http://127.0.0.1:27810',
    [string]$PdfDir = 'data\smoke',
    [int]$Count = 30,
    [switch]$KeepRunning
)

$ErrorActionPreference = 'Continue'
$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo

$results = New-Object System.Collections.ArrayList
function Check([string]$Name, [bool]$Ok, [string]$Detail) {
    [void]$results.Add([pscustomobject]@{ Check = $Name; Ok = $Ok; Detail = $Detail })
    $colour = if ($Ok) { 'Green' } else { 'Red' }
    Write-Host ("  {0,-46} {1}  {2}" -f $Name, $(if ($Ok) { 'PASS' } else { 'FAIL' }), $Detail) -ForegroundColor $colour
}

function Sql([string]$q) {
    # PowerShell 5.1 turns psql NOTICE lines on stderr into ErrorRecords; capture and inspect.
    $prev = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $out = & docker exec blooai-db psql -U bloo -d blooai -t -A -c $q 2>&1
    $code = $LASTEXITCODE
    $ErrorActionPreference = $prev
    if ($code -ne 0) { throw "psql failed: $out" }
    $clean = @($out | Where-Object { $_ -notmatch '^(NOTICE|WARNING|HINT):' -and $_ -notmatch '^(INSERT|UPDATE|DELETE|SELECT|TRUNCATE) \d' })
    return ($clean -join "`n").Trim()
}

function Wait-Health([int]$Seconds = 30) {
    $deadline = (Get-Date).AddSeconds($Seconds)
    do {
        Start-Sleep -Milliseconds 700
        try {
            $h = Invoke-RestMethod "$BaseUrl/health" -TimeoutSec 2
            if ($h.status -eq 'ok') { return }
        } catch { }
    } while ((Get-Date) -lt $deadline)
    throw "API did not become healthy at $BaseUrl"
}

function Start-Worker {
    $log = Join-Path $env:TEMP 'bloo-worker.log'
    $err = Join-Path $env:TEMP 'bloo-worker.err.log'
    $p = Start-Process -FilePath '.\target\debug\worker.exe' -PassThru -NoNewWindow `
        -RedirectStandardOutput $log -RedirectStandardError $err
    return @{ Proc = $p; Log = $log; Err = $err }
}

Write-Host "`n== preflight ==" -ForegroundColor Cyan

if (-not (Test-Path '.\target\debug\bloo_api.exe') -or -not (Test-Path '.\target\debug\worker.exe')) {
    throw 'binaries missing - run: cargo build -p bloo_api --bins'
}

& docker compose -p blooai -f docker-compose.yml up -d db 2>$null | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'docker compose up db failed' }
$deadline = (Get-Date).AddSeconds(60)
do {
    Start-Sleep -Seconds 1
    $s = (& docker inspect --format '{{.State.Health.Status}}' blooai-db 2>$null)
} while ($s -ne 'healthy' -and (Get-Date) -lt $deadline)
if ($s -ne 'healthy') { throw 'Postgres is not healthy' }

$pdfs = Get-ChildItem (Join-Path $repo $PdfDir) -Filter *.pdf | Sort-Object Name
if ($pdfs.Count -lt $Count) { throw "need $Count PDFs in $PdfDir, found $($pdfs.Count)" }

Write-Host "`n== reset ==" -ForegroundColor Cyan
Sql "truncate table chunks, jobs, documents restart identity cascade;" | Out-Null
Get-ChildItem 'data\raw' -File -ErrorAction SilentlyContinue | Remove-Item -Force -ErrorAction SilentlyContinue
Write-Host '  database and data/raw cleared' -ForegroundColor DarkGray

if (-not $KeepRunning) {
    Get-Process bloo_api, worker -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 500
}

Write-Host "`n== start ==" -ForegroundColor Cyan
$apiLog = Join-Path $env:TEMP 'bloo-api-smoke.log'
$apiErr = Join-Path $env:TEMP 'bloo-api-smoke.err.log'
$api = Start-Process -FilePath '.\target\debug\bloo_api.exe' -PassThru -NoNewWindow `
    -RedirectStandardOutput $apiLog -RedirectStandardError $apiErr
$worker = Start-Worker
Write-Host '  api pid ' $api.Id ', worker pid ' $worker.Proc.Id -ForegroundColor DarkGray
Wait-Health 40
Write-Host '  api healthy' -ForegroundColor Green

try {
    Write-Host "`n== A. upload $Count PDFs ==" -ForegroundColor Cyan
    $uploaded = 0
    $duplicates = 0
    foreach ($pdf in $pdfs | Select-Object -First $Count) {
        $raw = & curl.exe -s -X POST -F "file=@$($pdf.FullName)" "$BaseUrl/api/admin/documents"
        $j = $raw | ConvertFrom-Json
        if (-not $j.uploaded) { throw "upload failed for $($pdf.Name): $raw" }
        if ($j.uploaded[0].duplicate) { $duplicates++ } else { $uploaded++ }
    }
    Check 'uploads accepted' ($uploaded -eq $Count) "$uploaded new, $duplicates duplicates"

    Write-Host '  waiting for the worker...' -ForegroundColor DarkGray
    $deadline = (Get-Date).AddMinutes(6)
    do {
        Start-Sleep -Seconds 2
        $pending = [int](Sql "select count(*) from jobs where state in ('queued','running');")
    } while ($pending -gt 0 -and (Get-Date) -lt $deadline)
    Remove-Item $apiLog -Force -ErrorAction SilentlyContinue
    Check 'all jobs finished' ($pending -eq 0) "$pending pending"

    $docs = [int](Sql "select count(*) from documents;")
    $chunks = [int](Sql "select count(*) from chunks;")
    $ready = [int](Sql "select count(*) from documents where status = 'ready';")
    $vg = Sql "select round(avg(token_count))::int || ' / ' || max(token_count)::int from chunks;"
    Check 'documents stored' ($docs -eq $Count) "$docs rows"
    Check 'documents parsed' ($ready -eq $Count) "$ready ready"
    Check 'chunks written and embedded' ($chunks -gt 0) "$chunks chunks"
    $embedded = [int](Sql "select count(*) from chunks where embedding is not null;")
    Check 'every chunk has a vector' ($embedded -eq $chunks) "$embedded/$chunks non-null"
    Check 'chunk size within cap' ([int]($vg.Split('/')[1]) -le 600) "avg/max $vg tokens"

    Write-Host '  sample:' -ForegroundColor DarkGray
    Sql "select left(regexp_replace(text, E'[\n\r]+', ' ', 'g'), 90) from chunks order by ord limit 2;" |
        ForEach-Object { if ($_) { Write-Host "    $_" -ForegroundColor DarkGray } }

    Write-Host "`n== B. duplicate upload ==" -ForegroundColor Cyan
    $first = $pdfs[0]
    $raw = & curl.exe -s -X POST -F "file=@$($first.FullName)" "$BaseUrl/api/admin/documents"
    $j = $raw | ConvertFrom-Json
    $isDup = [bool]$j.uploaded[0].duplicate
    $docsNow = [int](Sql "select count(*) from documents;")
    Check 'duplicate is detected' $isDup "duplicate=$isDup"
    Check 'duplicate does not add a row' ($docsNow -eq $docs) "$docsNow documents"

    Write-Host "`n== C. delete ==" -ForegroundColor Cyan
    $docId = Sql "select id from documents order by created_at desc limit 1;"
    $chunksBefore = [int](Sql "select count(*) from chunks where document_id = '$docId';")
    $filesBefore = (Get-ChildItem 'data\raw' -File).Count
    & curl.exe -s -X DELETE "$BaseUrl/api/admin/documents/$docId" | Out-Null
    Start-Sleep -Seconds 1
    $docAfter = [int](Sql "select count(*) from documents where id = '$docId';")
    $chunksAfter = [int](Sql "select count(*) from chunks where document_id = '$docId';")
    $filesAfter = (Get-ChildItem 'data\raw' -File).Count
    Check 'document row deleted' ($docAfter -eq 0) "rows=$docAfter"
    Check 'chunks cascade' ($chunksAfter -eq 0) "before=$chunksBefore after=$chunksAfter"
    Check 'stored file removed' ($filesAfter -lt $filesBefore) "files $filesBefore -> $filesAfter"

    Write-Host "`n== D. crash recovery ==" -ForegroundColor Cyan
    $docId = Sql "select id from documents order by created_at limit 1;"
    $jobId = Sql "insert into jobs (kind, document_id, state, attempts, stage) values ('ingest', '$docId', 'running', 1, 'crash-test') returning id;"
    Write-Host "  orphaned job $jobId" -ForegroundColor DarkGray

    Stop-Process -Id $worker.Proc.Id -Force -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 1
    $worker = Start-Worker
    Write-Host "  worker restarted, pid $($worker.Proc.Id)" -ForegroundColor DarkGray

    $deadline = (Get-Date).AddMinutes(2)
    do {
        Start-Sleep -Seconds 2
        $row = Sql "select state || '|' || attempts from jobs where id = '$jobId';"
    } while (($row -notlike 'done|*') -and (Get-Date) -lt $deadline)

    $parts = $row.Split('|')
    Check 'orphaned job requeued and completed' ($parts[0] -eq 'done') "state=$($parts[0])"
    Check 'attempt count incremented' ([int]$parts[1] -ge 2) "attempts=$($parts[1])"

    Write-Host "`n== worker log (tail) ==" -ForegroundColor Cyan
    Get-Content $worker.Log -Tail 12 -ErrorAction SilentlyContinue | ForEach-Object {
        Write-Host "  $_" -ForegroundColor DarkGray
    }
}
finally {
    if (-not $KeepRunning) {
        Stop-Process -Id $api.Id -Force -ErrorAction SilentlyContinue
        if ($worker -and $worker.Proc) { Stop-Process -Id $worker.Proc.Id -Force -ErrorAction SilentlyContinue }
    }
}

$failed = ($results | Where-Object { -not $_.Ok }).Count
Write-Host "`n== verdict ==" -ForegroundColor Cyan
$results | Format-Table -AutoSize | Out-String | Write-Host
if ($failed -eq 0) {
    Write-Host "WP-02 SMOKE TEST: PASS ($($results.Count)/$($results.Count))" -ForegroundColor Green
    exit 0
} else {
    Write-Host "WP-02 SMOKE TEST: FAIL ($failed of $($results.Count) checks)" -ForegroundColor Red
    exit 1
}
