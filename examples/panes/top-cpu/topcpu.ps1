# The five programs using the most CPU over one second, processes of one program added together.
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false
function Snap { $h = @{}; Get-Process | ForEach-Object { try { $h[$_.Id] = @($_.ProcessName, $_.TotalProcessorTime.TotalSeconds) } catch { } }; $h }
$a = Snap
Start-Sleep -Seconds 1
$b = Snap
$cores = [Environment]::ProcessorCount
$by = @{}
foreach ($id in $b.Keys) {
    if ($id -eq 0) { continue }
    $d = $b[$id][1] - $(if ($a.ContainsKey($id)) { $a[$id][1] } else { $b[$id][1] })
    if ($d -gt 0) { $by[$b[$id][0]] += $d }
}
$rows = $by.GetEnumerator() | Sort-Object Value -Descending | Select-Object -First 5 | ForEach-Object {
    $pct = [math]::Min(100, [int](100 * $_.Value / $cores))
    @{ text = $_.Key; value = "$pct %"; progress = $pct; tone = if ($pct -ge 75) { "bad" } elseif ($pct -ge 40) { "warn" } else { "good" } }
}
if (-not $rows) { $rows = @(@{ text = "Quiet"; tone = "good" }) }
@{ rows = @($rows) } | ConvertTo-Json -Depth 4 -Compress
