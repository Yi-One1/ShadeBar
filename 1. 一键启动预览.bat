@echo off
setlocal
cd /d "%~dp0"

echo [1/2] Building latest ShadeBar ...
cargo build --release
if errorlevel 1 (
    echo Build FAILED. Preview not started.
    exit /b 1
)

echo [2/2] Starting preview ...
start "" "%~dp0target\x86_64-pc-windows-msvc\release\shadebar-app.exe"
