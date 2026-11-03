# Bloo AI - apply branch protection to `main`.
#
#   scripts\protect-main.ps1
#
# Requires `gh auth login` once. Idempotent: safe to run again.
# Rules applied (see protect-main.json):
#   - PR required, 1 approval, stale reviews dismissed
#   - status check `gate` must pass, branch must be up to date
#   - linear history (squash/rebase only), no force pushes, no deletions
#   - admins can bypass (enforce_admins=false) so the owner is never locked out

[CmdletBinding()]
param([string]$Repo = 'megharammm/fyp-bloo')

$ErrorActionPreference = 'Stop'
$json = Join-Path $PSScriptRoot 'protect-main.json'
if (-not (Test-Path $json)) { throw "missing $json" }

& gh auth status 2>$null | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'gh is not authenticated. Run: gh auth login' }

Write-Host "applying branch protection to ${Repo}:main" -ForegroundColor Cyan
& gh api --method PUT "repos/$Repo/branches/main/protection" --input $json
if ($LASTEXITCODE -ne 0) { throw 'branch protection request failed (see message above)' }

Write-Host "`ncurrent protection:" -ForegroundColor Green
& gh api "repos/$Repo/branches/main/protection" --jq '{pr: .required_pull_request_reviews.required_approving_review_count, checks: .required_status_checks.contexts, strict: .required_status_checks.strict, linear: .required_linear_history.enabled}'
