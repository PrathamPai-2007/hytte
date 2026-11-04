# Cut a release: bump the version everywhere, run the checks, commit "Release X", tag vX.
# Nothing is pushed unless you pass -Push (the tag is what starts the release workflow).
#
#   scripts/release.ps1 1.4.0            prepare it locally, then say what to push
#   scripts/release.ps1 1.4.0 -Push      ...and push main and the tag
#   scripts/release.ps1 1.4.0 -SkipChecks  skip fmt / clippy / test / release build (CI repeats them)
#
# After the workflow publishes, it commits the zip's SHA-256 into the winget manifest on main
# (scripts/version.ps1 hash), so pull before the next release.
param(
    [Parameter(Mandatory, Position = 0)][string]$Version,
    [switch]$SkipChecks,
    [switch]$Push
)
$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot -Parent)
$Version = $Version.TrimStart("v")

function Run {
    & $args[0] @($args | Select-Object -Skip 1)
    if ($LASTEXITCODE -ne 0) { throw "failed: $($args -join ' ')" }
}

if (git status --porcelain) { throw "the working tree isn't clean; commit or stash first" }
if ((git branch --show-current) -ne "main") { throw "release from main" }
Run git fetch origin main --quiet
if ((git rev-parse HEAD) -ne (git rev-parse origin/main)) { throw "main isn't level with origin/main; pull (or push) first" }
if (git tag -l "v$Version") { throw "tag v$Version already exists" }

& "$PSScriptRoot/version.ps1" set $Version
if (-not $SkipChecks) {
    Run cargo fmt --all --check
    Run cargo clippy --workspace --all-targets -- -D warnings
    Run cargo test --workspace
    Run cargo build --release --workspace
    $got = & target\release\hytte.exe --version
    if ($got -ne "hytte $Version") { throw "the built exe says '$got', not 'hytte $Version'" }
}
& "$PSScriptRoot/version.ps1" check "v$Version"

Run git add -A
Run git commit -m "Release $Version"
Run git tag "v$Version"
if ($Push) {
    Run git push origin main "v$Version"
    Write-Host "Pushed. Watch the release: gh run watch"
} else {
    Write-Host "Ready. To release: git push origin main v$Version"
}
