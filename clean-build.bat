@echo off
setlocal enabledelayedexpansion
set RUSTC_BOOTSTRAP=1

echo [INFO] Starting full clean build...

if exist target (
    echo [INFO] Cleaning target directory...
    rmdir /s /q target
)

echo [INFO] Building loader (qm-win), CLI (qm-cli), and legacy launcher (qm-app)...
cargo build --release --package qm-win --package qm-cli --package qm-app
if %ERRORLEVEL% neq 0 (
    echo [ERROR] Cargo release build failed with exit code %ERRORLEVEL%
    exit /b %ERRORLEVEL%
)

echo [INFO] Building desktop launcher (qm-launcher via cargo tauri build)...
pushd crates\qm-launcher
cargo tauri build
set TAURI_ERR=%ERRORLEVEL%
popd

if %TAURI_ERR% neq 0 (
    echo [ERROR] Tauri build failed with exit code %TAURI_ERR%
    exit /b %TAURI_ERR%
)

echo [INFO] Copying release artifacts...
if exist target\release\qm_winmm.dll copy /Y target\release\qm_winmm.dll . >nul
if exist target\release\qm.exe copy /Y target\release\qm.exe . >nul
if exist target\release\qm-launcher.exe copy /Y target\release\qm-launcher.exe . >nul
if exist target\release\qm_launcher_legacy.exe copy /Y target\release\qm_launcher_legacy.exe . >nul
if exist target\release\qm-launcher-legacy.exe copy /Y target\release\qm-launcher-legacy.exe . >nul
if exist "D:\WindowsGames\SteamLibrary\steamapps\common\MudandBlood" (
    if exist target\release\qm_winmm.dll copy /Y target\release\qm_winmm.dll "D:\WindowsGames\SteamLibrary\steamapps\common\MudandBlood\winmm.dll" >nul
)

echo [OK] Full build completed successfully.
if exist qm_winmm.dll echo   - Loader: qm_winmm.dll
if exist qm.exe echo   - CLI: qm.exe
if exist qm-launcher.exe echo   - Launcher: qm-launcher.exe
if exist qm_launcher_legacy.exe echo   - Legacy Launcher: qm_launcher_legacy.exe
if exist qm-launcher-legacy.exe echo   - Legacy Launcher: qm-launcher-legacy.exe
if exist "D:\WindowsGames\SteamLibrary\steamapps\common\MudandBlood\winmm.dll" echo   - Deployed: Steam Mud and Blood winmm.dll

