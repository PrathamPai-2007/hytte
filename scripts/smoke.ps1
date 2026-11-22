# Smoke test and idle-cost probe for a built hytte.exe.
#
#   smoke.ps1 [-Exe path] [-Out result.json] [-Baseline file] [-SkipPerf] [-Settle 45] [-Sample 30]
#
# Starts the exe as an isolated second copy (own %APPDATA%, own pipe and mutex through
# HYTTE_INSTANCE), so it is safe beside your own running Hytte and never touches your files, PATH
# or autostart. It then checks that the daemon starts, answers over the pipe, that the CLI
# commands reach it, and that nothing panicked. Unless -SkipPerf it measures idle cost (startup
# time, CPU over -Sample seconds after -Settle seconds, private memory) and compares it with
# the "limit" block of scripts/perf-baseline.json. Exit code 1 on any failed check or budget.
#
# Works in Windows PowerShell 5.1 and PowerShell 7. See docs/13-testing-and-release.md.
param(
    [string]$Exe = "",
    [string]$Out = "",
    [string]$Baseline = "",
    [switch]$SkipPerf,
    [int]$Settle = 45,
    [int]$Sample = 30
)
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
if (-not $Exe) { $Exe = Join-Path $root "target\release\hytte.exe" }
if (-not $Baseline) { $Baseline = Join-Path $PSScriptRoot "perf-baseline.json" }
$Exe = (Resolve-Path $Exe).Path

$failures = New-Object System.Collections.Generic.List[string]
function Check([string]$name, [bool]$ok, [string]$detail = "") {
    if ($ok) { Write-Host ("  ok    {0}" -f $name) }
    else { Write-Host ("  FAIL  {0}  {1}" -f $name, $detail); $failures.Add($name) }
}

# Run `hytte <args>` to the end; returns the exit code and what it printed.
function Invoke-Hytte([string[]]$ArgList) {
    $o = [IO.Path]::GetTempFileName(); $e = [IO.Path]::GetTempFileName()
    try {
        $p = Start-Process -FilePath $Exe -ArgumentList $ArgList -Wait -PassThru -WindowStyle Hidden `
            -RedirectStandardOutput $o -RedirectStandardError $e
        [pscustomobject]@{ Code = $p.ExitCode; Text = ((Get-Content $o -Raw) + (Get-Content $e -Raw)) }
    }
    finally { Remove-Item $o, $e -ErrorAction SilentlyContinue }
}

function Test-Pipe([string]$suffix) {
    $names = [IO.Directory]::GetFiles("\\.\pipe\") | ForEach-Object { $_.Substring($_.LastIndexOf('\') + 1) }
    [bool]($names | Where-Object { $_ -like "hytte-*$suffix" })
}

$instance = "smoke" + $PID
$dir = Join-Path ([IO.Path]::GetTempPath()) ("hytte-smoke-" + $PID)
$data = Join-Path $dir "Hytte"
New-Item -ItemType Directory -Force $data | Out-Null
# Everything that would touch the machine is off; a push-only pane gives `pane set` a target.
@'
[general]
autostart = false
add_to_path = false
check_updates = false
fullscreen = "off"

[[script_pane]]
name = "Smoke"
'@ | Set-Content -Encoding UTF8 (Join-Path $data "config.toml")

$saved = @{ APPDATA = $env:APPDATA; HYTTE_INSTANCE = $env:HYTTE_INSTANCE }
$env:APPDATA = $dir
$env:HYTTE_INSTANCE = $instance
$proc = $null
$result = [ordered]@{}
try {
    Write-Host "Hytte smoke test: $Exe"
    $v = Invoke-Hytte @("--version")
    Check "--version prints a version" ($v.Code -eq 0 -and $v.Text -match '\d+\.\d+\.\d+') $v.Text
    $result.version = ($v.Text -replace '[^\d\.]', '')

    $clock = [Diagnostics.Stopwatch]::StartNew()
    $proc = Start-Process -FilePath $Exe -PassThru -WindowStyle Hidden
    $up = $false
    while ($clock.Elapsed.TotalSeconds -lt 20) {
        if ($proc.HasExited) { break }
        if (Test-Pipe "-$instance") { $up = $true; break }
        Start-Sleep -Milliseconds 25
    }
    $result.startup_ms = [int]$clock.Elapsed.TotalMilliseconds
    Check "the daemon starts and opens its pipe" $up "exited=$($proc.HasExited)"
    if (-not $up) { throw "no daemon" }
    Write-Host ("        startup {0} ms" -f $result.startup_ms)

    # Measured first, on the quiet pill: the checks below leave cards (a pack waiting for Run)
    # that keep it drawing, which is not what "idle" means.
    if (-not $SkipPerf) {
        Write-Host ("  settling {0} s, then sampling CPU for {1} s ..." -f $Settle, $Sample)
        Start-Sleep -Seconds $Settle
        $proc.Refresh()
        $cpu0 = $proc.TotalProcessorTime.TotalSeconds
        $t0 = [Diagnostics.Stopwatch]::StartNew()
        Start-Sleep -Seconds $Sample
        $proc.Refresh()
        $cpu1 = $proc.TotalProcessorTime.TotalSeconds
        $result.idle_cpu_pct = [math]::Round(100 * ($cpu1 - $cpu0) / $t0.Elapsed.TotalSeconds, 3)
        $result.private_mb = [math]::Round($proc.PrivateMemorySize64 / 1MB, 1)
        $result.threads = $proc.Threads.Count
        $result.handles = $proc.HandleCount
        Write-Host ("        idle CPU {0} % of one core, private {1} MB, {2} threads, {3} handles" -f `
                $result.idle_cpu_pct, $result.private_mb, $result.threads, $result.handles)
        if (Test-Path $Baseline) {
            $b = Get-Content $Baseline -Raw | ConvertFrom-Json
            foreach ($k in "startup_ms", "idle_cpu_pct", "private_mb") {
                $limit = $b.limit.$k
                if ($null -ne $limit) {
                    Check ("{0} within budget ({1} <= {2})" -f $k, $result[$k], $limit) ($result[$k] -le $limit)
                }
            }
        }
    }

    $d = Invoke-Hytte @("doctor")
    Check "hytte doctor runs (no crash, a report)" ($d.Text -match 'Hytte doctor')
    Check "notify reaches the daemon" ((Invoke-Hytte @("notify", "smoke test")).Code -eq 0)
    Check "pane set reaches the daemon" ((Invoke-Hytte @("pane", "set", "Smoke", "--text", "hello")).Code -eq 0)
    Check "card reaches the daemon" ((Invoke-Hytte @("card", "--title", "smoke")).Code -eq 0)

    $marker = "smoke-note-" + [guid]::NewGuid().ToString("N").Substring(0, 8)
    Check "note command succeeds" ((Invoke-Hytte @("note", "--keep", $marker)).Code -eq 0)
    $jot = Join-Path $data "jot.json"
    $saw = $false
    for ($i = 0; $i -lt 100 -and -not $saw; $i++) {
        if ((Test-Path $jot) -and ((Get-Content $jot -Raw) -match $marker)) { $saw = $true } else { Start-Sleep -Milliseconds 100 }
    }
    Check "a note round-trips through the daemon to disk" $saw

    # `hytte mcp` end to end: the tool list, a read, and a pane made and removed as a pack.
    $rpc = @(
        '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}',
        '{"jsonrpc":"2.0","method":"notifications/initialized"}',
        '{"jsonrpc":"2.0","id":2,"method":"tools/list"}',
        '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"pane_list","arguments":{}}}',
        '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"pane_create","arguments":{"name":"SmokeAgent","command":"echo hi"}}}',
        '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"action_run","arguments":{"name":"no-such-action"}}}',
        '{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"pane_remove","arguments":{"name":"Smoke"}}}',
        '{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"pane_get","arguments":{"name":"Smoke"}}}',
        '{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"agent_status","arguments":{}}}',
        '{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"events","arguments":{}}}'
    )
    $in = [IO.Path]::GetTempFileName(); $outp = [IO.Path]::GetTempFileName()
    ($rpc -join "`n") | Set-Content -Encoding ASCII $in
    $m = Start-Process -FilePath $Exe -ArgumentList "mcp" -Wait -PassThru -WindowStyle Hidden `
        -RedirectStandardInput $in -RedirectStandardOutput $outp
    $mt = Get-Content $outp -Raw
    Remove-Item $in, $outp -ErrorAction SilentlyContinue
    Check "mcp lists the agent tools" ($mt -match 'action_run' -and $mt -match 'pane_create' -and $mt -match 'action_list')
    Check "mcp pane_list sees the config's pane" ($mt -match 'Smoke')
    Check "mcp pane_create writes an unapproved pack" (Test-Path (Join-Path $data "panes\smokeagent\pane.toml"))
    Check "mcp action_run refuses an unknown name" ($mt -match 'no action called')
    Check "mcp pane_get reads a pane back from the daemon" ($mt -match 'age_secs')
    Check "mcp agent_status and events answer" ($mt -match 'agents' -and $mt -match 'next')
    Check "mcp never removes a pane it did not make" ($mt -match 'made by an agent')

    # The agent's pane waits for a Run card on the pill; take it away so the idle measurement
    # below sees an idle pill, not a card being shown.
    Remove-Item (Join-Path $data "panes\smokeagent") -Recurse -Force -ErrorAction SilentlyContinue

    # `hytte pack` against the repository's own registry folder: verify, install, list, remove.
    $reg = Join-Path $root "examples\panes"
    $pi = Invoke-Hytte @("pack", "install", "disk-space", "--source", $reg)
    Check "pack install verifies and installs" ($pi.Code -eq 0 -and (Test-Path (Join-Path $data "panes\disk-space\pane.toml"))) $pi.Text
    Check "pack list shows it" ((Invoke-Hytte @("pack", "list")).Text -match 'disk-space')
    Check "pack update says up to date" ((Invoke-Hytte @("pack", "update", "--source", $reg)).Text -match 'up to date')
    Check "pack remove takes it away" ((Invoke-Hytte @("pack", "remove", "disk-space")).Code -eq 0 -and -not (Test-Path (Join-Path $data "panes\disk-space")))

    Check "still running after all of it" (-not $proc.HasExited)
}
catch { if ($_.Exception.Message -ne "no daemon") { Check "unexpected error" $false $_.Exception.Message } }
finally {
    if ($proc -and -not $proc.HasExited) { Stop-Process -Id $proc.Id -Force }
    $log = Join-Path $data "hytte.log"
    $text = if (Test-Path $log) { Get-Content $log -Raw } else { "" }
    Check "no panic in the log" ($text -notmatch 'panic in thread') $text
    $env:APPDATA = $saved.APPDATA
    $env:HYTTE_INSTANCE = $saved.HYTTE_INSTANCE
    Remove-Item $dir -Recurse -Force -ErrorAction SilentlyContinue
}
$result.failures = $failures.Count
if ($Out) { $result | ConvertTo-Json | Set-Content -Encoding UTF8 $Out }
if ($failures.Count) { Write-Host ("`n{0} check(s) failed." -f $failures.Count); exit 1 }
Write-Host "`nAll good."
