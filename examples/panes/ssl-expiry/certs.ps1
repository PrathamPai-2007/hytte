# Days until the TLS certificate of each host in hosts.txt expires.
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false
function Show($text, $tone) { @{ rows = @(@{ text = $text; tone = $tone }) } | ConvertTo-Json -Depth 4 -Compress; exit 0 }
$file = Join-Path $PSScriptRoot "hosts.txt"
if (-not (Test-Path $file)) { Show "Write one host per line in hosts.txt" "warn" }
$hosts = @(Get-Content $file | Where-Object { $_.Trim() -and -not $_.StartsWith("#") } | Select-Object -First 6 | ForEach-Object { $_.Trim() })
if (-not $hosts) { Show "hosts.txt has no hosts" "warn" }
# Connect to all of them at once, so one slow host cannot eat the pane's 10 seconds.
$jobs = foreach ($h in $hosts) {
    $name, $port = $h -split ":", 2
    $c = New-Object Net.Sockets.TcpClient
    $p = if ($port) { [int]$port } else { 443 }
    [pscustomobject]@{ Host = $h; Name = $name; Client = $c; Task = $c.ConnectAsync($name, $p) }
}
try { [void][Threading.Tasks.Task]::WaitAll([Threading.Tasks.Task[]]@($jobs.Task), 4000) } catch { }
$rows = foreach ($j in $jobs) {
    $left = $null
    if ($j.Task.Status -eq "RanToCompletion") {
        try {
            $ssl = New-Object Net.Security.SslStream($j.Client.GetStream(), $false, ([Net.Security.RemoteCertificateValidationCallback]{ $true }))
            $ssl.ReadTimeout = 3000
            $ssl.AuthenticateAsClient($j.Name)
            $cert = [Security.Cryptography.X509Certificates.X509Certificate2]$ssl.RemoteCertificate
            $left = [int][math]::Floor(($cert.NotAfter - (Get-Date)).TotalDays)
        } catch { }
    }
    $j.Client.Dispose()
    if ($null -eq $left) { @{ text = $j.Host; value = "no answer"; tone = "warn" } }
    else {
        @{
            text  = $j.Host
            value = if ($left -lt 0) { "expired" } else { "$left days" }
            tone  = if ($left -lt 7) { "bad" } elseif ($left -lt 30) { "warn" } else { "good" }
        }
    }
}
@{ rows = @($rows) } | ConvertTo-Json -Depth 4 -Compress
