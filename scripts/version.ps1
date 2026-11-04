# Keeps the version in step everywhere it is written: both Cargo.toml files, the two Hytte
# entries in Cargo.lock, and the winget manifest (PackageVersion and the InstallerUrl).
#
#   version.ps1 check [vX.Y.Z]   fail unless they all agree (and match the tag, when given)
#   version.ps1 set X.Y.Z        write the version everywhere; the manifest's hash becomes a placeholder
#   version.ps1 hash <zip>       write that zip's SHA-256 into the manifest (the release job's step)
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
$placeholder = "0" * 64
$utf8 = New-Object System.Text.UTF8Encoding $false

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
    $u = [regex]::Match($m, '/download/v([^/]+)/hytte-([^-]+)-x64\.zip')
    $v["winget InstallerUrl (tag)"] = $u.Groups[1].Value
    $v["winget InstallerUrl (file)"] = $u.Groups[2].Value
    $v
}

switch ($Cmd) {
    "check" {
        $v = Get-Versions
        $want = $v["crates/hytte/Cargo.toml"]
        if ($Arg) { $want = $Arg.TrimStart("v") }
        $bad = @($v.GetEnumerator() | Where-Object { $_.Value -ne $want })
        $hash = [regex]::Match((Get-Raw $manifest), '(?m)^\s*InstallerSha256:\s*([^\r\n]+)').Groups[1].Value.Trim()
        if ($hash -notmatch '^[0-9A-Fa-f]{64}$') { $bad += [pscustomobject]@{ Name = "winget InstallerSha256"; Value = "'$hash' is not a SHA-256" } }
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
        $m = [regex]::Replace($m, '/download/v[^/]+/hytte-[^-]+-x64\.zip', "/download/v$new/hytte-$new-x64.zip")
        $m = [regex]::Replace($m, '(?m)^(\s*InstallerSha256:\s*)[^\r\n]+', "`${1}$placeholder")
        Set-Raw $manifest $m
        Write-Host "version set to $new (the winget hash is a placeholder until the release is built)"
    }
    "hash" {
        if (-not (Test-Path $Arg)) { throw "no such file: $Arg" }
        $name = Split-Path $Arg -Leaf
        $m = Get-Raw $manifest
        $ver = [regex]::Match($m, '(?m)^PackageVersion:\s*([^\r\n]+)').Groups[1].Value.Trim()
        if ($name -ne "hytte-$ver-x64.zip") { throw "the manifest is for version $ver but the file is $name" }
        $sha = (Get-FileHash $Arg -Algorithm SHA256).Hash
        Set-Raw $manifest ([regex]::Replace($m, '(?m)^(\s*InstallerSha256:\s*)[^\r\n]+', "`${1}$sha"))
        Write-Host "InstallerSha256 for $ver is $sha"
    }
}
