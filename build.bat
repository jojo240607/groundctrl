@echo off
chcp 65001 >nul
REM ============================================================
REM  GroundControl (Tauri v2) one-click build script - Windows
REM  Double-click or run in CMD to produce the installer exe.
REM
REM  Prerequisites on the build machine:
REM    1. Rust (MSVC or GNU+MinGW toolchain)   https://rustup.rs
REM    2. Node.js 18+                          https://nodejs.org
REM    3. WebView2 Runtime (Win11 usually bundled; installer auto-fixes if missing)
REM    4. (this script auto-installs) cargo-tauri CLI
REM ============================================================

setlocal
cd /d %~dp0

echo [1/4] Check/install cargo-tauri ...
cargo tauri --version >nul 2>&1
if %errorlevel%==0 goto HAVE_TAURI
echo   tauri CLI not found, installing via cargo (may take several minutes)...
cargo install tauri-cli --version "2" --locked
if not %errorlevel%==0 (
  echo   [ERROR] cargo install tauri-cli failed. Check network / rust env.
  pause
  exit /b 1
)
:HAVE_TAURI

echo [2/4] Install frontend deps ...
call npm --prefix ui-web install
if not %errorlevel%==0 (
  echo   [ERROR] npm install failed.
  pause
  exit /b 1
)

echo [3/4] Build frontend (vite build -^> ui-web/dist) ...
call npm --prefix ui-web run build
if not %errorlevel%==0 (
  echo   [ERROR] frontend build failed.
  pause
  exit /b 1
)

echo [4/4] Package Tauri (cargo tauri build) ...
call cargo tauri build
if not %errorlevel%==0 (
  echo   [ERROR] cargo tauri build failed. Common cause: missing MSVC toolchain or WebView2.
  pause
  exit /b 1
)

echo.
echo Build done! Installer at:
echo   groundctrl-tauri\target\release\bundle\nsis\
for /f "delims=" %%f in ('dir /b /s groundctrl-tauri\target\release\bundle\nsis\*.exe 2^>nul') do echo     %%f
echo.
pause
