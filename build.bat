@echo off
chcp 65001 >nul
REM ============================================================
REM  GroundControl (Tauri v2) 一键打包脚本  ——  Windows
REM  双击或在命令行运行即可产出安装器 exe。
REM  前置依赖（本机需先具备）：
REM    1. Rust (MSVC toolchain)        https://rustup.rs
REM    2. Node.js 18+                  https://nodejs.org
REM    3. VS2022 生成工具 (C++ 桌面开发) 含 MSVC + Windows SDK
REM    4. WebView2 Runtime（Win10 可能需手动装；Win11 通常自带）
REM    5. (本脚本会自动装) cargo-tauri CLI
REM ============================================================

setlocal EnableDelayedExpansion
cd /d %~dp0

echo [1/4] 检查/安装 cargo-tauri ...
where tauri >nul 2>&1
if errorlevel 1 (
    where cargo-tauri >nul 2>&1
    if errorlevel 1 (
        echo   未找到 tauri CLI，正在通过 cargo 安装（可能耗时数分钟）...
        cargo install tauri-cli --version "^2" --locked
        if errorlevel 1 (
            echo   [错误] cargo install tauri-cli 失败，请检查网络/rust 环境。
            pause & exit /b 1
        )
    )
)

echo [2/4] 安装前端依赖 ...
call npm --prefix ui-web install
if errorlevel 1 (
    echo   [错误] npm install 失败。
    pause & exit /b 1
)

echo [3/4] 编译前端 (vite build -> ui-web/dist) ...
call npm --prefix ui-web run build
if errorlevel 1 (
    echo   [错误] 前端构建失败。
    pause & exit /b 1
)

echo [4/4] 打包 Tauri (cargo tauri build) ...
call cargo tauri build
if errorlevel 1 (
    echo   [错误] cargo tauri build 失败。常见原因：缺少 MSVC 工具链或 WebView2。
    pause & exit /b 1
)

echo.
echo 构建完成！安装器位于：
echo   groundctrl-tauri\target\release\bundle\nsis\
for /f "delims=" %%f in ('dir /b /s groundctrl-tauri\target\release\bundle\nsis\*.exe 2^>nul') do echo     %%f
echo.
pause
