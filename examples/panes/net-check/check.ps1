# Is the internet up, is DNS working, how slow is it. Two quick probes, three rows.
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false
$ProgressPreference = "SilentlyContinue"
$ping = $null
try { $ping = (Test-Connection 1.1.1.1 -Count 2 -ErrorAction Stop | Measure-Object ResponseTime -Average).Average } catch {}
$dns = $true
try { Resolve-DnsName example.com -Type A -DnsOnly -QuickTimeout -ErrorAction Stop | Out-Null } catch { $dns = $false }
$rows = @(
    @{ text = "Internet"; value = if ($null -ne $ping) { "up" } else { "down" }; tone = if ($null -ne $ping) { "good" } else { "bad" } },
    @{ text = "Latency"; value = if ($null -ne $ping) { "{0:N0} ms" -f $ping } else { "-" }; tone = if ($null -eq $ping) { "bad" } elseif ($ping -gt 150) { "warn" } else { "good" } },
    @{ text = "DNS"; value = if ($dns) { "ok" } else { "failing" }; tone = if ($dns) { "good" } else { "bad" } }
)
@{ rows = $rows } | ConvertTo-Json -Depth 4 -Compress
