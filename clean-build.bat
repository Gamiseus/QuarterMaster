@echo off
rem Clears every build output to free space, then rebuilds all of it with build.bat:
rem   target\                   all Rust builds (release, debug, test builds, the installer bundle,
rem                             release files in target\release\dist, the site preview)
rem   launcher-ui\dist\         the launcher's built web files
rem   launcher-ui\test-results\ and playwright-report\  e2e test output
rem   crates\qm-launcher\gen\   files Tauri writes on each build
rem   qm_winmm.dll, qm.exe, qm-launcher.exe, qm-launcher-legacy.exe here (copied back by build.bat)
rem launcher-ui\node_modules stays; build.bat installs it when it is missing.
setlocal enabledelayedexpansion

pushd "%~dp0"

echo [INFO] Starting full clean build...

for %%D in (target launcher-ui\dist launcher-ui\test-results launcher-ui\playwright-report crates\qm-launcher\gen) do (
    if exist %%D (
        echo [INFO] Removing %%D...
        rmdir /s /q %%D
        if exist %%D (
            echo [ERROR] Could not remove %%D: close any program using files in it and try again.
            popd
            exit /b 1
        )
    )
)
for %%F in (qm_winmm.dll qm.exe qm-launcher.exe qm-launcher-legacy.exe qm_launcher_legacy.exe) do (
    if exist %%F del /f /q %%F
)

call "%~dp0build.bat"
set ERR=!ERRORLEVEL!
popd
exit /b !ERR!
