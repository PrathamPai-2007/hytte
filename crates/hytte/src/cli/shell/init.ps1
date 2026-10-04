# Hytte / notch shell integration for PowerShell.
# Add to $PROFILE:   notch init pwsh | Out-String | Invoke-Expression
# Commands that run longer than the daemon's threshold (default 3 s) show up in the notch.

if (-not $global:__notch) {
    $global:__notch = @{ Seq = 0; Active = $false; Id = ''; Watch = $null }

    function global:__notch_send([hashtable]$msg) {
        try {
            $msg.v = 1
            $msg.source = 'shell'
            $msg.pid = $PID
            $json = $msg | ConvertTo-Json -Compress
            $c = New-Object System.IO.Pipes.NamedPipeClientStream('.', 'hytte', [System.IO.Pipes.PipeDirection]::Out)
            $c.Connect(40)
            $b = [Text.Encoding]::UTF8.GetBytes($json + "`n")
            $c.Write($b, 0, $b.Length)
            $c.Dispose()
        } catch { }   # the daemon being down must never affect the prompt
    }

    function global:__notch_start([string]$line) {
        $n = $global:__notch
        $n.Seq++
        $n.Id = "ps-$PID-$($n.Seq)"
        $n.Active = $true
        $n.Watch = [Diagnostics.Stopwatch]::StartNew()
        $label = if ($line.Length -gt 400) { $line.Substring(0, 400) } else { $line }
        __notch_send @{ task_id = $n.Id; event = 'Start'; label = $label; cwd = (Get-Location).Path }
    }

    # Wrap the prompt: it runs after every command, so it is where "finished" is known.
    $global:__notch_prompt = $function:prompt
    function global:prompt {
        $ok = $?
        $code = if ($ok) { 0 } elseif ($global:LASTEXITCODE -is [int] -and $global:LASTEXITCODE -ne 0) { $global:LASTEXITCODE } else { 1 }
        $n = $global:__notch
        if ($n.Active) {
            $n.Active = $false
            $ms = [int64]$n.Watch.ElapsedMilliseconds
            __notch_send @{
                task_id = $n.Id; event = $(if ($code -eq 0) { 'Done' } else { 'Failed' })
                label = ''; exit_code = $code; duration_ms = $ms
            }
        }
        if ($global:__notch_prompt) { & $global:__notch_prompt } else { "PS $($executionContext.SessionState.Path.CurrentLocation)> " }
    }

    # Start event as soon as Enter is pressed (needs PSReadLine, which is the default). Windows
    # PowerShell 5.1 has not loaded PSReadLine yet while the profile runs, so don't test for it:
    # calling Set-PSReadLineKeyHandler loads it on demand.
    try {
        Set-PSReadLineKeyHandler -Key Enter -BriefDescription 'NotchAcceptLine' -ScriptBlock {
            $line = $null; $cursor = $null
            [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref]$line, [ref]$cursor)
            if ($line -and $line.Trim()) { __notch_start $line }
            [Microsoft.PowerShell.PSConsoleReadLine]::AcceptLine()
        } -ErrorAction Stop
    } catch { }   # no PSReadLine: commands just aren't tracked
}
