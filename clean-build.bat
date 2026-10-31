@echo off
set RUSTC_BOOTSTRAP=1
if exist target rmdir /s /q target
cargo build --release --package qm-win --package qm-cli --package qm-app
if exist target\release\winmm.dll xcopy target\release\winmm.dll . /Y
if exist target\release\qm.exe xcopy target\release\qm.exe . /Y
if exist target\release\qm-launcher.exe xcopy target\release\qm-launcher.exe . /Y
