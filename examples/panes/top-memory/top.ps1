# The five programs using the most memory (all their processes added up), as rows.
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false
$total = (Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory
$rows = Get-Process | Group-Object ProcessName |
    ForEach-Object { [pscustomobject]@{ Name = $_.Name; Bytes = ($_.Group | Measure-Object WorkingSet64 -Sum).Sum } } |
    Sort-Object Bytes -Descending | Select-Object -First 5 | ForEach-Object {
        $pct = [int](100 * $_.Bytes / $total)
        @{
            text     = $_.Name
            value    = if ($_.Bytes -ge 1GB) { "{0:N1} GB" -f ($_.Bytes / 1GB) } else { "{0:N0} MB" -f ($_.Bytes / 1MB) }
            progress = $pct
            tone     = if ($pct -ge 25) { "warn" } else { "plain" }
        }
    }
@{ rows = @($rows) } | ConvertTo-Json -Depth 4 -Compress
