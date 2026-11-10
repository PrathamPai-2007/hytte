# Running containers, as rows for Hytte. Needs Docker (Desktop) running.
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false
$ErrorActionPreference = "Stop"
if (-not (Get-Command docker -ErrorAction SilentlyContinue)) {
    @{ rows = @(@{ text = "Docker isn't installed"; tone = "warn" }) } | ConvertTo-Json -Depth 4 -Compress
    exit 0
}
$lines = cmd /c 'docker ps --all --format "{{json .}}" 2>nul'
if ($LASTEXITCODE -ne 0) {
    @{ rows = @(@{ text = "Docker isn't running"; tone = "warn" }) } | ConvertTo-Json -Depth 4 -Compress
    exit 0
}
$list = @($lines | Where-Object { $_ } | ForEach-Object { $_ | ConvertFrom-Json })
if (-not $list) {
    @{ rows = @(@{ text = "No containers"; tone = "plain" }) } | ConvertTo-Json -Depth 4 -Compress
    exit 0
}
# Unhealthy and stopped ones first.
$rank = { if ($_.Status -match "unhealthy") { 0 } elseif ($_.State -ne "running") { 1 } else { 2 } }
$rows = $list | Sort-Object $rank | Select-Object -First 6 | ForEach-Object {
    $bad = $_.Status -match "unhealthy"
    $row = @{
        text  = $_.Names
        value = ($_.Status -replace "\s*\(.*\)", "")
        tone  = if ($bad) { "bad" } elseif ($_.State -eq "running") { "good" } else { "warn" }
    }
    if ($bad) { $row.action = "restart-unhealthy" }
    $row
}
@{ rows = @($rows) } | ConvertTo-Json -Depth 4 -Compress
