# One-command check for the QuarterMaster workspace.
$ErrorActionPreference = "Stop"
$root = $PSScriptRoot
$maxBytes = 24.5 * 1024 * 1024

function Invoke-Step {
    param([string]$Name, [string]$Dir, [scriptblock]$Action)
    Push-Location $Dir
    try {
        & $Action
        if ($LASTEXITCODE -ne 0) { throw "exit code $LASTEXITCODE" }
    } catch {
        Pop-Location
        Write-Host "[FAIL] $Name"
        Write-Host $_
        exit 1
    }
    Pop-Location
    Write-Host "[OK] $Name"
}

# fmt gates only the new crates; legacy crates are not refactored by this plan.
# clippy -D warnings gates the new crates and qm-schema (clean since Phase D).
Invoke-Step "cargo fmt (qm-service, qm-launcher, qm-site-builder)" $root { cargo fmt -p qm-service -p qm-launcher -p qm-site-builder -- --check }
Invoke-Step "cargo clippy (qm-service, qm-launcher, qm-site-builder, qm-schema)" $root { cargo clippy -p qm-service -p qm-launcher -p qm-site-builder -p qm-schema --all-targets --no-deps -- -D warnings }
# DLL-path crates: no -D warnings, so module-level deny attributes gate and legacy warnings do not.
Invoke-Step "cargo clippy (qm-core, qm-win; deny attributes)" $root { cargo clippy -p qm-core -p qm-win --no-deps }
Invoke-Step "cargo test --workspace" $root { cargo test --workspace }
# The site repository holds copies of the wiki, the changelog and the schemas; skipped when it is
# not next to this workspace.
Invoke-Step "site content is current (tools\sync-site.ps1)" $root { & "$root\tools\sync-site.ps1" -Check }
# The legacy app (ui/, qm-app): its tests read ui/app.js directly.
Invoke-Step "legacy app tests" $root { node ui/tests/run.js }
Invoke-Step "npm run check" "$root\launcher-ui" { npm run check }
Invoke-Step "npm test" "$root\launcher-ui" { npm test }
# Playwright builds the mock bundle into launcher-ui\dist-e2e and serves it on port 5173
# (vite preview); that port must be free.
# Visual baselines are valid for the Windows 11 reference environment (Segoe UI Variable);
# on a different setup, regenerate them with: npx playwright test --update-snapshots
Invoke-Step "npm run e2e" "$root\launcher-ui" { npm run e2e }
Invoke-Step "cargo build --release -p qm-win" $root { cargo build --release -p qm-win }
# Must run from qm-launcher: enables custom-protocol so the frontend is embedded.
Invoke-Step "cargo tauri build" "$root\crates\qm-launcher" { cargo tauri build }

# Svelte's server entry in the client bundle throws at runtime and no test catches it.
$assets = Join-Path $root "launcher-ui\dist\assets"
if (-not (Test-Path $assets)) {
    Write-Host "[FAIL] frontend bundle check: $assets is missing"
    exit 1
}
$scripts = @(Get-ChildItem $assets -Filter *.js)
if ($scripts.Count -eq 0) {
    Write-Host "[FAIL] frontend bundle check: no .js files in $assets"
    exit 1
}
$bad = $scripts |
    Where-Object { Select-String -Path $_.FullName -Pattern "is not available on the server" -SimpleMatch -Quiet }
if ($bad) {
    Write-Host "[FAIL] frontend bundle contains Svelte's server entry: $($bad.Name -join ', ')"
    exit 1
}
Write-Host "[OK] frontend bundle has no Svelte server entry"

$exe = Join-Path $root "target\release\qm-launcher.exe"
$dll = Join-Path $root "target\release\qm_winmm.dll"
if (-not (Test-Path $exe) -or -not (Test-Path $dll)) {
    Write-Host "[FAIL] release artifacts exist (qm-launcher.exe, qm_winmm.dll)"
    exit 1
}
Write-Host "[OK] release artifacts exist"

$size = (Get-Item $exe).Length
$mb = [math]::Round($size / 1MB, 2)
if ($size -gt $maxBytes) {
    Write-Host "[FAIL] size check: qm-launcher.exe is $mb MB (limit 24.5 MB)"
    exit 1
}
Write-Host "[OK] size check: qm-launcher.exe is $mb MB (limit 24.5 MB)"

Write-Host "[OK] All checks passed"
