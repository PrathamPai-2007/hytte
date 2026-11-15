# How much is in the Recycle Bin, with a confirmed "empty" action on the row.
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false
$shell = New-Object -ComObject Shell.Application
$bin = $shell.NameSpace(0xA)
$items = @($bin.Items())
$bytes = 0
foreach ($i in $items) { $bytes += [int64]$i.ExtendedProperty("System.Size") }
if (-not $items.Count) {
    @{ rows = @(@{ text = "Recycle Bin"; value = "empty"; tone = "good" }) } | ConvertTo-Json -Depth 4 -Compress
    exit 0
}
$size = if ($bytes -ge 1GB) { "{0:N1} GB" -f ($bytes / 1GB) } else { "{0:N0} MB" -f ($bytes / 1MB) }
@{ rows = @(@{ text = "$($items.Count) items"; value = $size; tone = if ($bytes -ge 5GB) { "warn" } else { "plain" }; action = "empty" }) } | ConvertTo-Json -Depth 4 -Compress
