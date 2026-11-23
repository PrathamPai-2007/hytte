# Uptime, CPU load and RAM in use. Amber above 75 %, red above 90 %.
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false
$os = Get-CimInstance Win32_OperatingSystem
$up = (Get-Date) - $os.LastBootUpTime
$cpu = [int](Get-CimInstance Win32_Processor | Measure-Object -Property LoadPercentage -Average).Average
$ram = [int](100 * ($os.TotalVisibleMemorySize - $os.FreePhysicalMemory) / $os.TotalVisibleMemorySize)
function Tone([int]$p) { if ($p -ge 90) { "bad" } elseif ($p -ge 75) { "warn" } else { "good" } }
$days = [int][math]::Floor($up.TotalDays)
@{
    rows = @(
        @{ text = "Up for"; value = if ($days -ge 1) { "{0} d {1} h" -f $days, $up.Hours } else { "{0} h {1:00} min" -f [int][math]::Floor($up.TotalHours), $up.Minutes } },
        @{ text = "CPU"; value = "$cpu %"; progress = $cpu; tone = (Tone $cpu) },
        @{ text = "Memory"; value = "$ram % of {0:N0} GB" -f ($os.TotalVisibleMemorySize / 1MB); progress = $ram; tone = (Tone $ram) }
    )
} | ConvertTo-Json -Depth 4 -Compress
