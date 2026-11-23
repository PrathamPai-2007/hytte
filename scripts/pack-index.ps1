# Builds examples/panes/index.json, the registry `hytte pack` reads: one entry per pack folder
# with each file's size and SHA-256.
#
#   pack-index.ps1           write the index (a pack whose files changed gets a patch bump)
#   pack-index.ps1 -Check    fail unless the committed index matches the folders (CI runs this)
#
# Hashes are taken over the bytes with CR removed, which is what the repository stores
# (.gitattributes keeps these files LF), so they match what raw.githubusercontent.com serves
# whatever a Windows checkout did to the line endings.
# Works in Windows PowerShell 5.1 and PowerShell 7.
param([switch]$Check)
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
$dir = Join-Path $root "examples\panes"
$indexPath = Join-Path $dir "index.json"
$minHytte = "1.9.3"

function Get-Sha256([byte[]]$bytes) {
    $sha = [Security.Cryptography.SHA256]::Create()
    try { (($sha.ComputeHash($bytes) | ForEach-Object { $_.ToString("x2") }) -join "") } finally { $sha.Dispose() }
}

$old = @{}
if (Test-Path $indexPath) {
    foreach ($p in (Get-Content $indexPath -Raw | ConvertFrom-Json).packs) { $old[$p.name] = $p }
}

$packs = @()
foreach ($d in (Get-ChildItem $dir -Directory | Sort-Object Name)) {
    if (-not (Test-Path (Join-Path $d.FullName "pane.toml"))) { continue }
    $files = @()
    foreach ($f in (Get-ChildItem $d.FullName -File | Sort-Object Name)) {
        $raw = [IO.File]::ReadAllBytes($f.FullName)
        $bytes = [byte[]]@($raw | Where-Object { $_ -ne 13 })
        $files += [ordered]@{ path = $f.Name; sha256 = (Get-Sha256 $bytes); size = $bytes.Length }
    }
    $desc = ""
    $readme = Join-Path $d.FullName "README.md"
    if (Test-Path $readme) {
        $line = Get-Content $readme | Where-Object { $_.Trim() -ne "" -and -not $_.StartsWith("#") } | Select-Object -First 1
        if ($line) { $desc = ($line -replace '[`*]', '').Trim(); if ($desc.Length -gt 120) { $desc = $desc.Substring(0, 117) + "..." } }
    }
    $version = "1.0.0"
    if ($old.ContainsKey($d.Name)) {
        $o = $old[$d.Name]
        # Compared in ordinal order: Sort-Object is culture-aware, so a runner and a desktop list the files differently.
        $key = { param($l) $a = @($l | ForEach-Object { "$($_.path):$($_.sha256)" }); [Array]::Sort($a, [StringComparer]::Ordinal); $a -join "|" }
        $same = (& $key $o.files) -eq (& $key $files)
        $version = $o.version
        if (-not $same) {
            if ($Check) { $version = "CHANGED" }
            else { $v = [version]$o.version; $version = "{0}.{1}.{2}" -f $v.Major, $v.Minor, ($v.Build + 1) }
        }
    }
    $packs += [ordered]@{ name = $d.Name; description = $desc; version = $version; min_hytte = $minHytte; files = $files }
}

if ($Check) {
    $bad = @()
    $names = @($packs | ForEach-Object { $_.name })
    foreach ($p in $packs) {
        if (-not $old.ContainsKey($p.name)) { $bad += "$($p.name): not in index.json" }
        elseif ($p.version -eq "CHANGED") { $bad += "$($p.name): files changed since index.json was written" }
    }
    foreach ($n in $old.Keys) { if ($names -notcontains $n) { $bad += "${n}: in index.json but its folder is gone" } }
    if ($bad.Count) { $bad | ForEach-Object { Write-Host "  $_" }; Write-Host "Run scripts/pack-index.ps1 and commit examples/panes/index.json."; exit 1 }
    Write-Host "index.json matches examples/panes ($($packs.Count) packs)."
    exit 0
}

$json = [ordered]@{ version = 1; packs = $packs } | ConvertTo-Json -Depth 6
[IO.File]::WriteAllText($indexPath, ($json -replace "`r`n", "`n") + "`n", (New-Object Text.UTF8Encoding($false)))
Write-Host "Wrote $indexPath ($($packs.Count) packs)."
