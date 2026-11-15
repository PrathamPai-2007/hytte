# Uncommitted changes and unpushed commits in the repositories listed in repos.txt
# (one folder per line). Needs git.
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false
$file = Join-Path $PSScriptRoot "repos.txt"
if (-not (Test-Path $file)) {
    @{ rows = @(@{ text = "Write one folder per line in repos.txt"; tone = "warn" }) } | ConvertTo-Json -Depth 4 -Compress
    exit 0
}
$rows = Get-Content $file | Where-Object { $_.Trim() -and -not $_.StartsWith("#") } | Select-Object -First 6 | ForEach-Object {
    $dir = $_.Trim()
    $name = Split-Path $dir -Leaf
    if (-not (Test-Path (Join-Path $dir ".git"))) {
        @{ text = $name; value = "not a repo"; tone = "warn" }
    } else {
        $dirty = @(git -C $dir status --porcelain 2>$null).Count
        $ahead = 0
        $n = git -C $dir rev-list --count "@{u}..HEAD" 2>$null
        if ($LASTEXITCODE -eq 0) { $ahead = [int]$n }
        $parts = @()
        if ($dirty) { $parts += "$dirty changed" }
        if ($ahead) { $parts += "$ahead to push" }
        @{
            text  = $name
            value = if ($parts) { $parts -join ", " } else { "clean" }
            tone  = if ($dirty) { "warn" } elseif ($ahead) { "plain" } else { "good" }
        }
    }
}
@{ rows = @($rows) } | ConvertTo-Json -Depth 4 -Compress
