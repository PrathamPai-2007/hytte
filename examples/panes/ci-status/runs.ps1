# The latest GitHub Actions runs of one repository, as rows for Hytte. Needs `gh`, signed in.
# Put the repository in repo.txt next to this file, as owner/name.
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false
$ErrorActionPreference = "Stop"
$file = Join-Path $PSScriptRoot "repo.txt"
if (-not (Test-Path $file)) {
    @{ rows = @(@{ text = "Write owner/name in repo.txt"; tone = "warn" }) } | ConvertTo-Json -Depth 4 -Compress
    exit 0
}
$repo = (Get-Content $file -TotalCount 1).Trim()
$runs = gh run list -R $repo --limit 6 --json name,headBranch,status,conclusion | ConvertFrom-Json
if (-not $runs) {
    @{ rows = @(@{ text = "No runs in $repo" }) } | ConvertTo-Json -Depth 4 -Compress
    exit 0
}
$rows = foreach ($r in $runs) {
    $state = if ($r.status -ne "completed") { $r.status } else { $r.conclusion }
    @{
        text   = "$($r.name) - $($r.headBranch)"
        value  = $state
        tone   = switch ($state) { "success" { "good" } "failure" { "bad" } "cancelled" { "warn" } default { "plain" } }
        action = "open"
    }
}
@{ rows = @($rows) } | ConvertTo-Json -Depth 4 -Compress
