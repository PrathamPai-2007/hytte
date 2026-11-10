# Your open pull requests, as rows for Hytte. Needs the GitHub CLI (`gh`), signed in.
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false
$ErrorActionPreference = "Stop"
if (-not (Get-Command gh -ErrorAction SilentlyContinue)) {
    @{ rows = @(@{ text = "Install the GitHub CLI (gh)"; tone = "warn" }) } | ConvertTo-Json -Depth 4 -Compress
    exit 0
}
$prs = gh search prs --author "@me" --state open --limit 6 --json number,title,repository,isDraft | ConvertFrom-Json
if (-not $prs) {
    @{ rows = @(@{ text = "No open pull requests"; tone = "good" }) } | ConvertTo-Json -Depth 4 -Compress
    exit 0
}
$rows = foreach ($p in $prs) {
    $repo = ($p.repository.name)
    @{
        text   = "$repo #$($p.number)  $($p.title)"
        value  = if ($p.isDraft) { "draft" } else { "open" }
        tone   = if ($p.isDraft) { "warn" } else { "plain" }
        action = "open"
    }
}
@{ rows = @($rows) } | ConvertTo-Json -Depth 4 -Compress
