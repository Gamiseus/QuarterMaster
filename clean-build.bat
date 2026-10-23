rmdir /s /q .\target
cargo build --release --package qm-win
xcopy ".\target\release\winmm.dll" . /Y