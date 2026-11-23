# Wi-Fi name, signal and speed from `netsh wlan show interfaces` (English labels).
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false
function Show($text, $tone) { @{ rows = @(@{ text = $text; tone = $tone }) } | ConvertTo-Json -Depth 4 -Compress; exit 0 }
$out = (netsh wlan show interfaces 2>&1) -join "`n"
function Field($label) { if ($out -match "(?m)^\s*$label\s*:\s*(.+?)\s*$") { $Matches[1] } }
$ssid = Field "SSID"
$state = Field "State"
if (-not $state) { Show "No Wi-Fi adapter (or not an English Windows)" "warn" }
if ($state -ne "connected" -or -not $ssid) { Show "Not connected" "warn" }
$sig = [int]((Field "Signal") -replace "[^\d]", "")
$rate = Field "Receive rate \(Mbps\)"
$rows = @(
    @{ text = $ssid; value = "connected"; tone = "good" },
    @{ text = "Signal"; value = "$sig %"; progress = $sig; tone = if ($sig -lt 25) { "bad" } elseif ($sig -lt 50) { "warn" } else { "good" } }
)
if ($rate) { $rows += @{ text = "Speed"; value = "$rate Mbps" } }
@{ rows = @($rows) } | ConvertTo-Json -Depth 4 -Compress
