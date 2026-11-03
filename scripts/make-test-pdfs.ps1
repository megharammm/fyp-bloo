# Bloo AI - generate synthetic campus PDFs for the WP-02 smoke test.
#
#   scripts\make-test-pdfs.ps1 -Count 30 -OutDir data\smoke
#
# Each PDF has a 16 pt title (detected as a heading), a body at 10 pt, and a small
# three-column table, so the parser and chunker both get exercised.

[CmdletBinding()]
param(
    [int]$Count = 30,
    [string]$OutDir = 'data\smoke'
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo

$dir = Join-Path $repo $OutDir
New-Item -ItemType Directory -Force -Path $dir | Out-Null

function Esc([string]$s) {
    return $s.Replace('\', '\\').Replace('(', '\(').Replace(')', '\)')
}

$topics = @(
    @{ Title = 'Hostel Rules 2026';     Cat = 'hostel' },
    @{ Title = 'Fee Structure 2026-27'; Cat = 'fees' },
    @{ Title = 'Examination Schedule';  Cat = 'exams' },
    @{ Title = 'Library Membership';    Cat = 'library' },
    @{ Title = 'Bus Routes and Timings'; Cat = 'transport' }
)

for ($i = 1; $i -le $Count; $i++) {
    $topic = $topics[($i - 1) % $topics.Count]
    $title = "$($topic.Title) - Volume $i"

    $ops = New-Object System.Text.StringBuilder
    [void]$ops.Append("BT /F1 16 Tf 1 0 0 1 72 790 Tm (" + (Esc $title) + ") Tj ET`n")

    $y = 760
    $body = @(
        "This document describes $($topic.Cat) policy for the academic year 2026-27.",
        "Students must read this circular carefully before the registration deadline.",
        "The committee reviews every application within ten working days of submission.",
        "Late submissions are accepted only with the written approval of the dean.",
        "For assistance contact the administrative office between nine and five on weekdays.",
        "All amounts are quoted in Indian rupees and are inclusive of applicable taxes.",
        "This is synthetic text generated for ingestion testing and carries no policy weight."
    )

    for ($p = 0; $p -lt 10; $p++) {
        $sentence = $body[($i + $p) % $body.Count]
        $line = "$title section $($p + 1). $sentence $($body[($i + $p + 1) % $body.Count]) $($body[($i + $p + 2) % $body.Count]) $($body[($i + $p + 3) % $body.Count])"
        # wrap at ~95 characters so lines stay inside the page
        $words = $line.Split(' ')
        $buf = ''
        foreach ($w in $words) {
            if (($buf.Length + $w.Length + 1) -gt 95) {
                [void]$ops.Append("BT /F1 10 Tf 1 0 0 1 72 $y Tm (" + (Esc $buf) + ") Tj ET`n")
                $y -= 14
                $buf = $w
            } else {
                $buf = if ($buf.Length -eq 0) { $w } else { "$buf $w" }
            }
        }
        if ($buf.Length -gt 0) {
            [void]$ops.Append("BT /F1 10 Tf 1 0 0 1 72 $y Tm (" + (Esc $buf) + ") Tj ET`n")
            $y -= 14
        }
        $y -= 4
    }

    # three-column table
    $y -= 10
    $rows = @(
        @('Item', 'Fee', 'Deadline'),
        @("Hostel-$i", "$(48000 + $i)", '15 Aug'),
        @("Mess-$i", "$(3200 + $i)", 'Monthly')
    )
    foreach ($row in $rows) {
        [void]$ops.Append("BT /F1 10 Tf 1 0 0 1 72 $y Tm (" + (Esc $row[0]) + ") Tj ET`n")
        [void]$ops.Append("BT /F1 10 Tf 1 0 0 1 240 $y Tm (" + (Esc $row[1]) + ") Tj ET`n")
        [void]$ops.Append("BT /F1 10 Tf 1 0 0 1 360 $y Tm (" + (Esc $row[2]) + ") Tj ET`n")
        $y -= 14
    }

    $content = $ops.ToString()
    $len = [Text.Encoding]::ASCII.GetByteCount($content)

    $objects = @(
        '<< /Type /Catalog /Pages 2 0 R >>',
        '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
        '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>',
        "<< /Length $len >>`nstream`n$content`nendstream",
        '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>'
    )

    $sb = New-Object System.Text.StringBuilder
    [void]$sb.Append("%PDF-1.4`n")
    $offsets = @()
    for ($o = 0; $o -lt $objects.Count; $o++) {
        $offsets += [Text.Encoding]::ASCII.GetByteCount($sb.ToString())
        [void]$sb.Append("$($o + 1) 0 obj`n$($objects[$o])`nendobj`n")
    }
    $xref = [Text.Encoding]::ASCII.GetByteCount($sb.ToString())
    [void]$sb.Append("xref`n0 $($objects.Count + 1)`n")
    [void]$sb.Append("0000000000 65535 f `n")
    foreach ($off in $offsets) {
        [void]$sb.Append(("{0:D10} 00000 n `n" -f $off))
    }
    [void]$sb.Append("trailer`n<< /Size $($objects.Count + 1) /Root 1 0 R >>`nstartxref`n$xref`n%%EOF")

    $file = Join-Path $dir ("campus-{0:D2}.pdf" -f $i)
    [IO.File]::WriteAllText($file, $sb.ToString(), [Text.Encoding]::ASCII)
}

Write-Host "generated $Count PDFs in $dir" -ForegroundColor Green
