# NVIDIA GPU load, memory and temperature via nvidia-smi.
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false
$smi = Get-Command nvidia-smi -ErrorAction SilentlyContinue
if (-not $smi) {
    @{ rows = @(@{ text = "No NVIDIA GPU (nvidia-smi not found)"; tone = "warn" }) } | ConvertTo-Json -Depth 4 -Compress
    exit 0
}
function Tone([int]$p) { if ($p -ge 90) { "bad" } elseif ($p -ge 75) { "warn" } else { "good" } }
$rows = @()
foreach ($line in @(& $smi.Source --query-gpu=name,utilization.gpu,memory.used,memory.total,temperature.gpu --format=csv,noheader,nounits | Select-Object -First 2)) {
    $f = $line -split ",\s*"
    if ($f.Count -lt 5) { continue }
    $name = ($f[0] -replace "NVIDIA |GeForce ", "")
    $util = [int]$f[1]
    $mem = [int](100 * [double]$f[2] / [double]$f[3])
    $rows += @{ text = $name; value = ("{0} C" -f [int][double]$f[4]) }
    $rows += @{ text = "  load"; value = "$util %"; progress = $util; tone = (Tone $util) }
    $rows += @{ text = "  memory"; value = "{0:N1} of {1:N0} GB" -f ([double]$f[2] / 1024), ([double]$f[3] / 1024); progress = $mem; tone = (Tone $mem) }
}
if (-not $rows) { $rows = @(@{ text = "nvidia-smi gave no GPU"; tone = "warn" }) }
@{ rows = @($rows | Select-Object -First 6) } | ConvertTo-Json -Depth 4 -Compress
