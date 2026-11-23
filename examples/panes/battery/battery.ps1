# Battery charge, state and time left. Red below 15 %, amber below 30 % when not charging.
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false
$b = Get-CimInstance Win32_Battery -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $b) {
    @{ rows = @(@{ text = "No battery (desktop PC)"; tone = "good" }) } | ConvertTo-Json -Depth 4 -Compress
    exit 0
}
$pct = [int]$b.EstimatedChargeRemaining
# BatteryStatus: 1 discharging, 2 on AC, 3 full, 4 low, 5 critical, 6 to 9 charging.
$st = [int]$b.BatteryStatus
$charging = $st -in 2, 3, 6, 7, 8, 9
$state = switch ($st) { 1 { "on battery" } 2 { "plugged in" } 3 { "full" } 4 { "low" } 5 { "critical" } default { "charging" } }
$rows = @(@{
        text     = "Charge"
        value    = "$pct %"
        progress = $pct
        tone     = if ($charging) { "good" } elseif ($pct -lt 15) { "bad" } elseif ($pct -lt 30) { "warn" } else { "good" }
    }, @{ text = "State"; value = $state })
# 71582788 means "unknown" (it is also what Windows reports while charging).
$min = [int64]$b.EstimatedRunTime
if (-not $charging -and $min -gt 0 -and $min -lt 71582788) {
    $rows += @{ text = "Time left"; value = ("{0} h {1:00} min" -f [math]::Floor($min / 60), ($min % 60)) }
}
@{ rows = @($rows) } | ConvertTo-Json -Depth 4 -Compress
