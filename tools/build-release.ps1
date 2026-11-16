# Builds what a QuarterMaster release holds, into target\release\dist:
#   QuarterMaster_<version>_x64-setup.exe   the installer (recommended download)
#   QuarterMaster_<version>.zip             the portable copy (launcher, loader, qm.exe); launchers also update from it
#   QuarterMaster_<version>.zip.sha256      checked by the launcher before it updates
# Run check.ps1 first: this script only builds.
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
$launcher = Join-Path $root "crates\qm-launcher"
$release = Join-Path $root "target\release"
$dist = Join-Path $release "dist"

$conf = Get-Content (Join-Path $launcher "tauri.conf.json") -Raw | ConvertFrom-Json
$version = $conf.version

function Invoke-Native {
    param([string]$Name, [string]$Dir, [scriptblock]$Action)
    Push-Location $Dir
    try {
        & $Action
        if ($LASTEXITCODE -ne 0) { throw "$Name failed with exit code $LASTEXITCODE" }
    } finally {
        Pop-Location
    }
    Write-Host "[OK] $Name"
}

Invoke-Native "loader (qm_winmm.dll)" $root { cargo build --release -p qm-win }
Invoke-Native "command-line tool (qm.exe)" $root { cargo build --release -p qm-cli }
# Must run from qm-launcher: enables custom-protocol so the frontend is embedded.
Invoke-Native "launcher and installer" $launcher { cargo tauri build --config tauri.installer.conf.json }

$exe = Join-Path $release "qm-launcher.exe"
$dll = Join-Path $release "qm_winmm.dll"
$cli = Join-Path $release "qm.exe"
$portable = Join-Path $launcher "installer\portable.txt"
$setup = Get-ChildItem (Join-Path $release "bundle\nsis") -Filter "QuarterMaster_${version}_*-setup.exe" |
    Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $setup) { throw "The installer was not built (target\release\bundle\nsis)." }

if (Test-Path $dist) { Remove-Item $dist -Recurse -Force }
New-Item -ItemType Directory -Path $dist | Out-Null
Copy-Item $setup.FullName (Join-Path $dist $setup.Name)

$zip = Join-Path $dist "QuarterMaster_$version.zip"
Compress-Archive -Path $exe, $dll, $cli, $portable -DestinationPath $zip -CompressionLevel Optimal
$hash = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
# No BOM and no path: the launcher reads the first word.
[System.IO.File]::WriteAllText("$zip.sha256", "$hash  QuarterMaster_$version.zip`n")

Write-Host ""
Write-Host "Release files for v$version in ${dist}:"
Get-ChildItem $dist | ForEach-Object { Write-Host ("  {0}  ({1:N1} MB)" -f $_.Name, ($_.Length / 1MB)) }
