@echo off
rem Builds everything a release and local testing need, without clearing earlier builds first
rem (clean-build.bat clears them, then runs this):
rem   target\release\qm_winmm.dll, qm.exe, qm-launcher.exe, qm-launcher-legacy.exe, qm-site-builder.exe
rem   target\release\dist\  the installer, the portable zip and its .sha256 (tools\build-release.ps1)
rem The loader, CLI and launchers are also copied here, and the loader into the game folder as
rem winmm.dll. Set QM_GAME_DIR to use another game folder.
setlocal enabledelayedexpansion
set RUSTC_BOOTSTRAP=1
if not defined QM_GAME_DIR set "QM_GAME_DIR=D:\WindowsGames\SteamLibrary\steamapps\common\MudandBlood"

pushd "%~dp0"

if not exist launcher-ui\node_modules (
    echo [INFO] Installing launcher UI packages ^(npm ci^)...
    pushd launcher-ui
    call npm ci
    set NPM_ERR=!ERRORLEVEL!
    popd
    if !NPM_ERR! neq 0 (
        echo [ERROR] npm ci failed with exit code !NPM_ERR!
        popd
        exit /b !NPM_ERR!
    )
)

echo [INFO] Building loader (qm-win), CLI (qm-cli), legacy launcher (qm-app) and site builder (qm-site-builder)...
cargo build --release --package qm-win --package qm-cli --package qm-app --package qm-site-builder
if !ERRORLEVEL! neq 0 (
    set CARGO_ERR=!ERRORLEVEL!
    echo [ERROR] Cargo release build failed with exit code !CARGO_ERR!
    popd
    exit /b !CARGO_ERR!
)

echo [INFO] Building desktop launcher, installer and release files (tools\build-release.ps1)...
powershell -NoProfile -ExecutionPolicy Bypass -File tools\build-release.ps1
if !ERRORLEVEL! neq 0 (
    set REL_ERR=!ERRORLEVEL!
    echo [ERROR] Release build failed with exit code !REL_ERR!
    popd
    exit /b !REL_ERR!
)

echo [INFO] Copying build outputs...
for %%F in (qm_winmm.dll qm.exe qm-launcher.exe qm-launcher-legacy.exe) do (
    if exist target\release\%%F (
        copy /Y target\release\%%F . >nul
        if !ERRORLEVEL! neq 0 echo [WARN] Could not copy %%F here: is it running?
    )
)
set DEPLOYED=0
if exist "%QM_GAME_DIR%" (
    if exist target\release\qm_winmm.dll (
        copy /Y target\release\qm_winmm.dll "%QM_GAME_DIR%\winmm.dll" >nul
        if !ERRORLEVEL! equ 0 (set DEPLOYED=1) else (echo [WARN] Could not copy the loader into %QM_GAME_DIR%: is the game running?)
    )
) else (
    echo [WARN] Game folder not found, loader not deployed: %QM_GAME_DIR%
)

echo.
echo [OK] Build completed.
if exist qm_winmm.dll echo   - Loader: qm_winmm.dll
if exist qm.exe echo   - CLI: qm.exe
if exist qm-launcher.exe echo   - Launcher: qm-launcher.exe
if exist qm-launcher-legacy.exe echo   - Legacy launcher: qm-launcher-legacy.exe
if exist target\release\qm-site-builder.exe echo   - Site builder: target\release\qm-site-builder.exe
if exist target\release\dist (
    echo   - Release files in target\release\dist:
    for %%F in (target\release\dist\*) do echo       %%~nxF
)
if !DEPLOYED! equ 1 echo   - Deployed: %QM_GAME_DIR%\winmm.dll

popd
exit /b 0
