@echo off
set RUSTC_BOOTSTRAP=1
if exist target rmdir /s /q target
cargo build --release --package qm-win --package qm-cli --package qm-app
rem qm-launcher is built by check.ps1 via `cargo tauri build` (custom-protocol), not here.
if exist target\release\qm_winmm.dll xcopy target\release\qm_winmm.dll . /Y
if exist target\release\qm.exe xcopy target\release\qm.exe . /Y
