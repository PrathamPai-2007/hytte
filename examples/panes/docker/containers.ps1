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
    $name = ($_.Names -split ",")[0]
    $row = @{
        text  = $name
        value = ($_.Status -replace "\s*\(.*\)", "")
        tone  = if ($bad) { "bad" } elseif ($_.State -eq "running") { "good" } else { "warn" }
    }
    # An unhealthy row restarts that one container: its name travels as the row's arg (HYTTE_ARG).
    if ($bad) { $row.action = "restart"; $row.arg = $name }
    $row
}
$pane = @{ rows = @($rows) }
# One button under the rows when something is unhealthy (a label is 16 characters at most).
if (@($list | Where-Object { $_.Status -match "unhealthy" }).Count) {
    $pane.buttons = @(@{ label = "Fix unhealthy"; action = "restart-unhealthy" })
}
$pane | ConvertTo-Json -Depth 4 -Compress
