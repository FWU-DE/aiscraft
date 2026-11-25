<#
.SYNOPSIS
  Build, sign and package AIS.ChatCraft for Windows.

.DESCRIPTION
  Produces, in $env:DIST (default: dist/release):
    aischatcraft-<version>-windows-<arch>.msi            per-machine installer (WiX v5)
    aischatcraft-<version>-windows-<arch>-portable.zip   the programs with portable.txt

  Called by craft-common-actions' release.yml (job "Windows"), which installs WiX (install-wix).
  The binaries link the C runtime statically (+crt-static), so neither package needs the Visual
  C++ redistributable. aiscraft-preview needs the WebView2 runtime (part of Windows 10/11).
  Signing is delegated to sign.ps1 (skipped with a warning when no signing secrets are set).

.EXAMPLE
  pwsh packaging/windows/package.ps1 -Arch x64
  pwsh packaging/windows/package.ps1 -Arch arm64 -SkipBuild
#>
param(
  [ValidateSet('x64', 'arm64')] [string] $Arch = 'x64',
  [switch] $SkipBuild
)
$ErrorActionPreference = 'Stop'
$Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path

function Invoke-Native([string] $What, [scriptblock] $Block) {
  Write-Output "==> $What"
  & $Block
  if ($LASTEXITCODE -ne 0) { throw "$What failed with exit code $LASTEXITCODE" }
}

# The version lives in one place: [workspace.package] version in the root Cargo.toml.
$Version = $env:AISCHATCRAFT_VERSION
if (-not $Version) {
  $inPkg = $false
  foreach ($line in Get-Content (Join-Path $Root 'Cargo.toml')) {
    if ($line -match '^\s*\[') { $inPkg = ($line.Trim() -eq '[workspace.package]'); continue }
    if ($inPkg -and $line -match '^\s*version\s*=\s*"([^"]+)"') { $Version = $Matches[1]; break }
  }
}
if (-not $Version) { throw 'could not read [workspace.package] version from Cargo.toml' }
# MSI ProductVersion is numeric (major.minor.build); pre-release tags are dropped there.
$MsiVersion = ($Version -split '-')[0]

$Target = switch ($Arch) { 'x64' { 'x86_64-pc-windows-msvc' } 'arm64' { 'aarch64-pc-windows-msvc' } }
$Dist = if ($env:DIST) { $env:DIST } else { Join-Path $Root 'dist\release' }
$TargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $Root 'target' }
New-Item -ItemType Directory -Force -Path $Dist | Out-Null
$Programs = 'aiscraft.exe', 'aiscraft-cli.exe', 'aiscraft-preview.exe', 'aiscraft-collab-server.exe'

Write-Output "AIS.ChatCraft $Version for Windows $Arch ($Target)"

if (-not $SkipBuild) {
  # Static CRT, scoped to the target so build scripts and proc-macros are unaffected.
  $flagVar = 'CARGO_TARGET_' + ($Target.ToUpper() -replace '-', '_') + '_RUSTFLAGS'
  [Environment]::SetEnvironmentVariable($flagVar, '-C target-feature=+crt-static')
  Invoke-Native "cargo build ($Target)" {
    cargo build --release --locked -p aiscraft -p aiscraft-cli -p aiscraft-preview -p aiscraft-collab-server --target $Target
  }
}

$Bin = Join-Path $TargetDir "$Target\release"

# Machine (COFF header) must match -Arch, so an x64 build never ships labelled arm64. Subsystem:
# 2 = Windows GUI (the app opens no console window), 3 = console (the CLI's output reaches the
# terminal).
function Get-PeHeader([string] $Path) {
  $bytes = [System.IO.File]::ReadAllBytes($Path)
  $pe = [BitConverter]::ToInt32($bytes, 0x3C)
  return @{ Machine = [BitConverter]::ToUInt16($bytes, $pe + 4); Subsystem = [BitConverter]::ToUInt16($bytes, $pe + 0x5C) }
}
$Machine = switch ($Arch) { 'x64' { 0x8664 } 'arm64' { 0xAA64 } }
foreach ($exe in $Programs) {
  $h = Get-PeHeader (Join-Path $Bin $exe)
  if ($h.Machine -ne $Machine) { throw "$exe is for machine 0x$('{0:X}' -f $h.Machine), expected 0x$('{0:X}' -f $Machine) ($Arch)" }
  Write-Output "ok ${exe}: $Arch, PE subsystem $($h.Subsystem)"
}
if ((Get-PeHeader (Join-Path $Bin 'aiscraft.exe')).Subsystem -ne 2) { throw 'aiscraft.exe must be a Windows GUI program' }
if ((Get-PeHeader (Join-Path $Bin 'aiscraft-cli.exe')).Subsystem -ne 3) { throw 'aiscraft-cli.exe must be a console program' }

$Stage = Join-Path $TargetDir "windows-package\$Arch"
Remove-Item -Recurse -Force $Stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $Stage | Out-Null
foreach ($exe in $Programs) { Copy-Item (Join-Path $Bin $exe) $Stage }

& (Join-Path $PSScriptRoot 'sign.ps1') ($Programs | ForEach-Object { Join-Path $Stage $_ })

# ---- MSI ---------------------------------------------------------------------------------------
$Msi = Join-Path $Dist "aischatcraft-$Version-windows-$Arch.msi"
Invoke-Native 'WiX UI extension' { wix extension add -g WixToolset.UI.wixext/5.0.2 }
Invoke-Native 'wix build' {
  wix build (Join-Path $PSScriptRoot 'aischatcraft.wxs') -arch $Arch -ext WixToolset.UI.wixext -culture de-DE `
    -d "Version=$MsiVersion" -d "BinDir=$Stage" -d "IconPath=$(Join-Path $Root 'packaging\icons\aischatcraft.ico')" `
    -o $Msi
}
# wix writes its debug symbols (.wixpdb) next to the MSI; keep them out of the release assets.
Remove-Item -Force -ErrorAction SilentlyContinue ([IO.Path]::ChangeExtension($Msi, '.wixpdb'))
& (Join-Path $PSScriptRoot 'sign.ps1') $Msi

# ---- portable zip ------------------------------------------------------------------------------
# The programs at the top level of the zip, as craft-update's portable update expects.
$Portable = Join-Path $TargetDir "windows-package\aischatcraft-$Version-windows-$Arch-portable"
Remove-Item -Recurse -Force $Portable -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $Portable | Out-Null
Copy-Item (Join-Path $Stage '*.exe') $Portable
Set-Content -LiteralPath (Join-Path $Portable 'portable.txt') -Encoding Ascii -Value 'AIS.ChatCraft portable.'
foreach ($f in 'README.md', 'LICENSE-MIT', 'LICENSE-APACHE') {
  $p = Join-Path $Root $f
  if (Test-Path $p) { Copy-Item $p $Portable }
}
$Zip = Join-Path $Dist "aischatcraft-$Version-windows-$Arch-portable.zip"
Remove-Item -Force $Zip -ErrorAction SilentlyContinue
Compress-Archive -Path (Join-Path $Portable '*') -DestinationPath $Zip

Invoke-Native 'aiscraft-cli --version' { & (Join-Path $Stage 'aiscraft-cli.exe') --version }
Get-Item $Msi, $Zip | Format-Table Name, Length
