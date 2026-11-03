# Bloo AI - WP-05 smoke test (admin console).
#
#   scripts\smoke-wp05.ps1
#
# Exercises the admin API end to end against a real database:
#   A. sync creates 5 documents, and a second sync is a no-op
#   B. filters, detail and paginated chunks
#   C. metadata patch
#   D. publish -> searchable, unpublish -> not searchable
#   E. re-ingest
#   F. archive
#   G. delete guard while a job runs, then delete removes chunks + file
#   H. stats
#   I. admin page renders its JS and containers

[CmdletBinding()]
param([string]$BaseUrl = 'http://127.0.0.1:27810')

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

function Api([string]$path, [string]$method = 'GET', [string]$body = '') {
    if ($body) {
        return Invoke-RestMethod -Uri "$BaseUrl$path" -Method $method -ContentType 'application/json' -Body $body -TimeoutSec 60
    }
    return Invoke-RestMethod -Uri "$BaseUrl$path" -Method $method -TimeoutSec 60
}

function Status([string]$path, [string]$method = 'GET') {
    try {
        $r = Invoke-WebRequest -Uri "$BaseUrl$path" -Method $method -UseBasicParsing -TimeoutSec 30
        return [int]$r.StatusCode
    } catch {
        try { return [int]$_.Exception.Response.StatusCode } catch { return 0 }
    }
}

function Wait-Jobs([int]$Seconds = 120) {
    $deadline = (Get-Date).AddSeconds($Seconds)
    do {
        Start-Sleep -Seconds 2
        $pending = [int](Sql "select count(*) from jobs where state in ('queued','running');")
    } while ($pending -gt 0 -and (Get-Date) -lt $deadline)
    return $pending
}

Write-Host "`n== reset ==" -ForegroundColor Cyan
Sql "truncate table chunks, jobs, documents restart identity cascade;" | Out-Null
Get-ChildItem 'data\raw' -File -ErrorAction SilentlyContinue | Remove-Item -Force -ErrorAction SilentlyContinue
Get-Process bloo_api, worker -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 600

$api = Start-Process .\target\debug\bloo_api.exe -PassThru -NoNewWindow -RedirectStandardOutput (Join-Path $env:TEMP 'w5-api.log') -RedirectStandardError (Join-Path $env:TEMP 'w5-api.err')
$wk = Start-Process .\target\debug\worker.exe -PassThru -NoNewWindow -RedirectStandardOutput (Join-Path $env:TEMP 'w5-worker.log') -RedirectStandardError (Join-Path $env:TEMP 'w5-worker.err')

$deadline = (Get-Date).AddSeconds(60)
do {
    Start-Sleep -Milliseconds 800
    try { $h = Api '/health' } catch { $h = $null }
} while (-not $h -and (Get-Date) -lt $deadline)
if (-not $h) { throw 'API did not start' }

try {
    Write-Host "`n== A. sync ==" -ForegroundColor Cyan
    $s1 = Api '/api/admin/sync' 'POST'
    Check 'first sync finds 5 documents' ($s1.new -eq 5) "new=$($s1.new) queued=$($s1.queued)"
    $s2 = Api '/api/admin/sync' 'POST'
    Check 'second sync is a no-op' ($s2.new -eq 0 -and $s2.changed -eq 0) "new=$($s2.new) changed=$($s2.changed) unchanged=$($s2.unchanged)"

    $pending = Wait-Jobs
    $ready = [int](Sql "select count(*) from documents where status = 'ready';")
    Check 'all synced documents ready' ($pending -eq 0 -and $ready -eq 5) "pending=$pending ready=$ready"

    Write-Host "`n== B. list, detail, chunks ==" -ForegroundColor Cyan
    $all = Api '/api/admin/documents'
    Check 'list returns total' ($all.total -eq 5) "total=$($all.total)"
    $filtered = Api '/api/admin/documents?status=ready&q=hostel'
    Check 'filters narrow the list' ($filtered.total -eq 1 -and $filtered.documents[0].title -eq 'hostel.md') "total=$($filtered.total)"

    $feesId = (Sql "select id from documents where source_path like '%fees.md';")
    $detail = Api "/api/admin/documents/$feesId"
    Check 'detail exposes chunk count' ($detail.document.chunks -gt 0) "chunks=$($detail.document.chunks) jobs=$($detail.jobs.Count)"

    $page = Api "/api/admin/documents/$feesId/chunks?limit=3&offset=0"
    Check 'chunks are paginated' ($page.chunks.Count -eq 3 -and $page.total -gt 3) "returned=$($page.chunks.Count) total=$($page.total)"

    Write-Host "`n== C. metadata ==" -ForegroundColor Cyan
    $patched = Api "/api/admin/documents/$feesId" 'PATCH' '{"title":"Fees and Payments","category":"fees","valid_until":"2027-03-31"}'
    $row = Sql "select title || '|' || category || '|' || coalesce(valid_until::text,'') from documents where id = '$feesId';"
    Check 'patch updates metadata' ($row -eq 'Fees and Payments|fees|2027-03-31') "got=$row"

    Write-Host "`n== D. publish / unpublish ==" -ForegroundColor Cyan
    $p = Api "/api/admin/documents/$feesId/publish" 'POST'
    $hit = Api '/api/search?q=hostel%20fee%20per%20semester'
    $found = @($hit.hits | Where-Object { $_.document_id -eq $feesId }).Count -gt 0
    Check 'published document is searchable' $found "hits=$($hit.hits.Count)"

    $u = Api "/api/admin/documents/$feesId/unpublish" 'POST'
    $hit2 = Api '/api/search?q=hostel%20fee%20per%20semester'
    $found2 = @($hit2.hits | Where-Object { $_.document_id -eq $feesId }).Count -gt 0
    Check 'unpublished document is not searchable' (-not $found2) "status=$((Sql "select status from documents where id='$feesId';"))"

    Write-Host "`n== E. re-ingest ==" -ForegroundColor Cyan
    $r = Api "/api/admin/documents/$feesId/reingest" 'POST'
    $pending = Wait-Jobs
    $chunksAfter = [int](Sql "select count(*) from chunks where document_id = '$feesId';")
    Check 're-ingest rebuilds chunks' ($pending -eq 0 -and $chunksAfter -gt 0) "job=$($r.job_id) chunks=$chunksAfter"

    Write-Host "`n== F. archive ==" -ForegroundColor Cyan
    $a = Api "/api/admin/documents/$feesId/archive" 'POST'
    $status = Sql "select status from documents where id = '$feesId';"
    Check 'archive changes status' ($status -eq 'archived') "status=$status"

    $s3 = Api '/api/admin/sync' 'POST'
    Check 'archived file is re-added by sync' ($s3.new -ge 1) "new=$($s3.new) changed=$($s3.changed)"
    Wait-Jobs | Out-Null
    $live = [int](Sql "select count(*) from documents where source_path like '%fees.md' and status <> 'archived';")
    $maxVersion = [int](Sql "select coalesce(max(version),0) from documents where source_path like '%fees.md';")
    Check 're-added file is a new version' ($live -eq 1 -and $maxVersion -eq 2) "live=$live max_version=$maxVersion"

    Write-Host "`n== G. delete guard and delete ==" -ForegroundColor Cyan
    $libId = (Sql "select id from documents where source_path like '%library.md';")
    Sql "insert into jobs (kind, document_id, state, attempts) values ('ingest', '$libId', 'running', 1);" | Out-Null
    $guard = Status "/api/admin/documents/$libId" 'DELETE'
    Check 'delete refused while a job runs' ($guard -eq 409) "HTTP $guard"

    Sql "delete from jobs where document_id = '$libId' and state = 'running';" | Out-Null
    $filesBefore = (Get-ChildItem 'data\raw' -File).Count
    $del = Api "/api/admin/documents/$libId" 'DELETE'
    $left = [int](Sql "select count(*) from chunks where document_id = '$libId';")
    $filesAfter = (Get-ChildItem 'data\raw' -File).Count
    Check 'delete removes row and chunks' ($left -eq 0 -and (Sql "select count(*) from documents where id='$libId';") -eq '0') "chunks=$left"
    Check 'delete removes the stored file' ($filesAfter -lt $filesBefore) "files $filesBefore -> $filesAfter"

    Write-Host "`n== H. stats ==" -ForegroundColor Cyan
    $st = Api '/api/admin/stats'
    Check 'stats counts documents' ($st.documents -eq 5) "documents=$($st.documents) chunks=$($st.chunks) published=$($st.published)"

    Write-Host "`n== I. admin page ==" -ForegroundColor Cyan
    $page = Invoke-WebRequest "$BaseUrl/admin" -UseBasicParsing -TimeoutSec 15
    $html = $page.Content
    $hasJs = $html -match '/static/admin.js'
    $hasStats = $html -match 'id="stat-documents"'
    $hasChunks = $html -match 'id="chunk-list"'
    Check 'admin page renders' ($page.StatusCode -eq 200 -and $hasJs -and $hasStats -and $hasChunks) "js=$hasJs stats=$hasStats chunks=$hasChunks"
    $js = Invoke-WebRequest "$BaseUrl/static/admin.js" -UseBasicParsing -TimeoutSec 15
    Check 'admin.js is served' ($js.StatusCode -eq 200 -and $js.Content.Length -gt 5000) "bytes=$($js.Content.Length)"
}
catch {
    Check 'smoke script completed without an exception' $false $_.Exception.Message
}
finally {
    Stop-Process -Id $api.Id -Force -ErrorAction SilentlyContinue
    Stop-Process -Id $wk.Id -Force -ErrorAction SilentlyContinue
}

$failed = ($results | Where-Object { -not $_.Ok }).Count
Write-Host "`n== verdict ==" -ForegroundColor Cyan
$results | Format-Table -AutoSize | Out-String | Write-Host
if ($failed -eq 0) {
    Write-Host "WP-05 SMOKE TEST: PASS ($($results.Count)/$($results.Count))" -ForegroundColor Green
    exit 0
} else {
    Write-Host "WP-05 SMOKE TEST: FAIL ($failed of $($results.Count) checks)" -ForegroundColor Red
    exit 1
}


