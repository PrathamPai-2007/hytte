# Used space of each fixed drive, as rows with a bar. Red above 90 %, amber above 75 %.
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false
$rows = Get-CimInstance Win32_LogicalDisk -Filter "DriveType=3" | Select-Object -First 6 | ForEach-Object {
    $used = [int](100 * ($_.Size - $_.FreeSpace) / $_.Size)
    @{
        text     = $_.DeviceID
        value    = "{0:N0} GB free" -f ($_.FreeSpace / 1GB)
        progress = $used
        tone     = if ($used -ge 90) { "bad" } elseif ($used -ge 75) { "warn" } else { "good" }
    }
}
@{ rows = @($rows) } | ConvertTo-Json -Depth 4 -Compress
