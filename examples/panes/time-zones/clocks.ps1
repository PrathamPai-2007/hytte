# The time in a few zones, from zones.txt (Label=Windows time zone id) or a default list.
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false
$file = Join-Path $PSScriptRoot "zones.txt"
$zones = if (Test-Path $file) {
    Get-Content $file | Where-Object { $_.Trim() -and -not $_.StartsWith("#") -and $_.Contains("=") }
} else {
    "UTC=UTC", "London=GMT Standard Time", "New York=Eastern Standard Time", "Tokyo=Tokyo Standard Time", "Sydney=AUS Eastern Standard Time"
}
$now = [DateTime]::UtcNow
$rows = $zones | Select-Object -First 6 | ForEach-Object {
    $label, $id = $_ -split "=", 2
    try {
        $tz = [TimeZoneInfo]::FindSystemTimeZoneById($id.Trim())
        $t = [TimeZoneInfo]::ConvertTimeFromUtc($now, $tz)
        @{ text = $label.Trim(); value = $t.ToString("HH:mm ddd", [Globalization.CultureInfo]::InvariantCulture) }
    } catch {
        @{ text = $label.Trim(); value = "unknown zone"; tone = "warn" }
    }
}
@{ rows = @($rows) } | ConvertTo-Json -Depth 4 -Compress
