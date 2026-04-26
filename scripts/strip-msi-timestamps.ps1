# strip-msi-timestamps.ps1 — strip embedded build timestamps from a Tauri-
# produced MSI so the same source tree produces a byte-identical bundle on
# every rebuild. Referenced by docs/REPRODUCIBLE-BUILDS.md.
#
# Usage:
#   pwsh scripts/strip-msi-timestamps.ps1 -MsiPath <path-to-msi>
#
# Approach:
#   1. dark.exe (WiX) decompiles the MSI to a .wxs source + extracted CABs.
#   2. light.exe (WiX) recompiles with -pdbout NUL (no .wixpdb) and
#      -out <new-msi>, dropping the build-time SummaryInformation stream
#      timestamps along the way.
#   3. lessmsi-based fallback path is left as a TODO for hosts that don't
#      have the WiX 3.x toolset on PATH.
#
# Currently a STUB: we exit 0 with a "TODO" notice if WiX isn't on PATH,
# so reproducible-builds CI doesn't fail-hard on hosts that haven't yet
# installed the toolset. Replace the stub block with the real dark/light
# pipeline once Nexus has WiX wired in (see docs/REPRODUCIBLE-BUILDS.md).

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string] $MsiPath
)

$ErrorActionPreference = 'Stop'

if (-not (Test-Path -LiteralPath $MsiPath)) {
    Write-Error "MSI not found: $MsiPath"
    exit 2
}

$dark  = Get-Command dark.exe  -ErrorAction SilentlyContinue
$light = Get-Command light.exe -ErrorAction SilentlyContinue

if (-not $dark -or -not $light) {
    Write-Host "TODO: WiX-based timestamp strip — dark.exe / light.exe not on PATH." -ForegroundColor Yellow
    Write-Host "      Install WiX 3.14 + add it to PATH, then re-run this script." -ForegroundColor Yellow
    Write-Host "      Skipping timestamp strip on this host (input MSI left unchanged): $MsiPath"
    exit 0
}

# Real pipeline (left in but unreached until WiX is on PATH on the build host):
$tmp = New-Item -ItemType Directory -Path (Join-Path $env:TEMP "msi-strip-$(Get-Random)")
try {
    $wxs = Join-Path $tmp.FullName 'extracted.wxs'
    & $dark.Source -x $tmp.FullName $MsiPath -o $wxs
    if ($LASTEXITCODE -ne 0) { throw "dark.exe failed with exit code $LASTEXITCODE" }

    $stripped = Join-Path $tmp.FullName 'stripped.msi'
    & $light.Source -nologo -pdbout NUL -out $stripped $wxs
    if ($LASTEXITCODE -ne 0) { throw "light.exe failed with exit code $LASTEXITCODE" }

    Move-Item -Force -LiteralPath $stripped -Destination $MsiPath
    Write-Host "Stripped build timestamps from: $MsiPath"
} finally {
    Remove-Item -Recurse -Force -LiteralPath $tmp.FullName -ErrorAction SilentlyContinue
}
