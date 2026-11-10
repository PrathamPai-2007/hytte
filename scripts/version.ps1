# Keeps the version in step everywhere it is written: both Cargo.toml files, the two Hytte
# entries in Cargo.lock, the winget manifest (PackageVersion and the InstallerUrls) and the Scoop
# manifest (bucket/hytte.json: version and the urls).
#
#   version.ps1 check [vX.Y.Z]   fail unless they all agree (and match the tag, when given)
#   version.ps1 set X.Y.Z        write the version everywhere; the manifests' hashes become placeholders
#   version.ps1 hash <zip>       write that zip's SHA-256 (hytte-<v>-x64.zip or -arm64.zip) into both
#                                manifests (the release job's step)
#
# Works in Windows PowerShell 5.1 and PowerShell 7.
param(
    [Parameter(Mandatory, Position = 0)][ValidateSet("check", "set", "hash")][string]$Cmd,
    [Parameter(Position = 1)][string]$Arg = ""
)
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
$cargoFiles = "crates/hytte/Cargo.toml", "crates/proto/Cargo.toml"
$lock = Join-Path $root "Cargo.lock"
$manifest = Join-Path $root "manifests/winget/Hytte.Hytte.yaml"
$bucket = Join-Path $root "bucket/hytte.json"
$placeholder = "0" * 64
$utf8 = New-Object System.Text.UTF8Encoding $false
# Starts with a digit so Scoop's `autoupdate` urls (`v$version`) are left alone.
$zipUrl = '/download/v(\d[^/]*)/hytte-(\d[^-]*)-(x64|arm64)\.zip'

function Get-Raw($path) { [IO.File]::ReadAllText($path) }
function Set-Raw($path, $text) { [IO.File]::WriteAllText($path, $text, $utf8) }

# `name = "<crate>"` followed by its `version = "..."` line, in Cargo.lock.
function Lock-Pattern($crate) { '(?m)(^name = "' + $crate + '"\r?\nversion = ")([^"]+)(")' }

function Get-Versions {
    $v = [ordered]@{}
    foreach ($f in $cargoFiles) {
        $m = [regex]::Match((Get-Raw (Join-Path $root $f)), '(?m)^version\s*=\s*"([^"]+)"')
        $v[$f] = $m.Groups[1].Value
    }
    $l = Get-Raw $lock
    foreach ($c in "hytte", "hytte-proto") {
        $v["Cargo.lock ($c)"] = [regex]::Match($l, (Lock-Pattern $c)).Groups[2].Value
    }
    $m = Get-Raw $manifest
    $v["winget PackageVersion"] = [regex]::Match($m, '(?m)^PackageVersion:\s*([^\r\n]+)').Groups[1].Value.Trim()
    foreach ($u in [regex]::Matches($m, $zipUrl)) {
        $v["winget $($u.Groups[3].Value) url (tag)"] = $u.Groups[1].Value
        $v["winget $($u.Groups[3].Value) url (file)"] = $u.Groups[2].Value
    }
    $b = Get-Raw $bucket
    $v["scoop version"] = [regex]::Match($b, '"version":\s*"([^"]+)"').Groups[1].Value
    foreach ($u in [regex]::Matches($b, $zipUrl)) {
        $v["scoop $($u.Groups[3].Value) url (tag)"] = $u.Groups[1].Value
        $v["scoop $($u.Groups[3].Value) url (file)"] = $u.Groups[2].Value
    }
    $v
}

switch ($Cmd) {
    "check" {
        $v = Get-Versions
        $want = $v["crates/hytte/Cargo.toml"]
        if ($Arg) { $want = $Arg.TrimStart("v") }
        $bad = @($v.GetEnumerator() | Where-Object { $_.Value -ne $want })
        $hashes = @()
        foreach ($h in [regex]::Matches((Get-Raw $manifest), '(?m)^\s*InstallerSha256:\s*([^\r\n]+)')) {
            $hashes += [pscustomobject]@{ Name = "winget InstallerSha256"; Value = $h.Groups[1].Value.Trim() }
        }
        foreach ($h in [regex]::Matches((Get-Raw $bucket), '"hash":\s*"([^"]*)"')) {
            $hashes += [pscustomobject]@{ Name = "scoop hash"; Value = $h.Groups[1].Value.Trim() }
        }
        foreach ($h in $hashes) {
            if ($h.Value -notmatch '^[0-9A-Fa-f]{64}$') { $bad += [pscustomobject]@{ Name = $h.Name; Value = "'$($h.Value)' is not a SHA-256" } }
        }
        if ($bad.Count) {
            $bad | ForEach-Object { Write-Host ("  {0}: {1}   (want {2})" -f $_.Name, $_.Value, $want) }
            throw "the version is not the same everywhere (see above); run scripts/version.ps1 set <version>"
        }
        Write-Host "version $want is consistent in $($v.Count) places"
    }
    "set" {
        $new = $Arg.TrimStart("v")
        if ($new -notmatch '^\d+\.\d+\.\d+$') { throw "give a version like 1.4.0" }
        foreach ($f in $cargoFiles) {
            $p = Join-Path $root $f
            Set-Raw $p ([regex]::Replace((Get-Raw $p), '(?m)^(version\s*=\s*")[^"]+(")', "`${1}$new`${2}", 1))
        }
        $l = Get-Raw $lock
        foreach ($c in "hytte", "hytte-proto") {
            $l = [regex]::Replace($l, (Lock-Pattern $c), "`${1}$new`${3}")
        }
        Set-Raw $lock $l
        $m = Get-Raw $manifest
        $m = [regex]::Replace($m, '(?m)^(PackageVersion:\s*)[^\r\n]+', "`${1}$new")
        $m = [regex]::Replace($m, $zipUrl, "/download/v$new/hytte-$new-`${3}.zip")
        $m = [regex]::Replace($m, '(?m)^(\s*InstallerSha256:\s*)[^\r\n]+', "`${1}$placeholder")
        Set-Raw $manifest $m
        $b = Get-Raw $bucket
        $b = [regex]::Replace($b, '("version":\s*")[^"]+(")', "`${1}$new`${2}", 1)
        $b = [regex]::Replace($b, $zipUrl, "/download/v$new/hytte-$new-`${3}.zip")
        $b = [regex]::Replace($b, '("hash":\s*")[^"]*(")', "`${1}$placeholder`${2}")
        Set-Raw $bucket $b
        Write-Host "version set to $new (the manifests' hashes are placeholders until the release is built)"
    }
    "hash" {
        if (-not (Test-Path $Arg)) { throw "no such file: $Arg" }
        $name = Split-Path $Arg -Leaf
        $m = Get-Raw $manifest
        $ver = [regex]::Match($m, '(?m)^PackageVersion:\s*([^\r\n]+)').Groups[1].Value.Trim()
        $arch = [regex]::Match($name, '^hytte-' + [regex]::Escape($ver) + '-(x64|arm64)\.zip$').Groups[1].Value
        if (-not $arch) { throw "the manifest is for version $ver but the file is $name" }
        $sha = (Get-FileHash $Arg -Algorithm SHA256).Hash
        Set-Raw $manifest ([regex]::Replace($m, '(?s)(Architecture:\s*' + $arch + '\b.*?InstallerSha256:\s*)[^\r\n]+', "`${1}$sha", 1))
        $key = if ($arch -eq "x64") { "64bit" } else { "arm64" }
        $b = Get-Raw $bucket
        Set-Raw $bucket ([regex]::Replace($b, '(?s)("' + $key + '":\s*\{.*?"hash":\s*")[^"]*', "`${1}$sha", 1))
        Write-Host "SHA-256 of $name is $sha"
    }
}
