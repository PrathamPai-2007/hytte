# Opens the repository's Actions page.
$repo = (Get-Content (Join-Path $PSScriptRoot "repo.txt") -TotalCount 1).Trim()
Start-Process "https://github.com/$repo/actions"
