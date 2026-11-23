# Unread GitHub notifications, as rows for Hytte. Needs the GitHub CLI (`gh`), signed in.
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false
function Show($text, $tone) { @{ rows = @(@{ text = $text; tone = $tone }) } | ConvertTo-Json -Depth 4 -Compress; exit 0 }
if (-not (Get-Command gh -ErrorAction SilentlyContinue)) { Show "Install the GitHub CLI (gh)" "warn" }
$raw = gh api "notifications?per_page=6" 2>$null
if ($LASTEXITCODE -ne 0) { Show "gh is not signed in (gh auth login)" "warn" }
# Two steps: Windows PowerShell 5.1 hands the whole JSON array on as one object.
$items = ($raw -join "`n") | ConvertFrom-Json
$items = @($items)
if (-not $items) { Show "Inbox zero" "good" }
$rows = foreach ($n in $items) {
    @{
        text   = "$($n.repository.name)  $($n.subject.title)"
        value  = ($n.reason -replace "_", " ")
        tone   = if ($n.reason -in "review_requested", "mention", "assign") { "warn" } else { "plain" }
        action = "open"
    }
}
@{ rows = @($rows) } | ConvertTo-Json -Depth 4 -Compress
