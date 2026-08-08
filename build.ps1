# GroundControl (Tauri v2) 一键打包脚本 —— PowerShell
# 用法：在仓库根目录右键"使用 PowerShell 运行"，或：
#   powershell -ExecutionPolicy Bypass -File build.ps1
#
# 前置依赖（本机需先具备）：
#   1. Rust (MSVC toolchain)        https://rustup.rs
#   2. Node.js 18+                  https://nodejs.org
#   3. VS2022 生成工具 (C++ 桌面开发) 含 MSVC + Windows SDK
#   4. WebView2 Runtime（Win10 可能需手动装；Win11 通常自带）
# 脚本会自动安装 cargo-tauri CLI 与前端依赖。

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $root

function Check($name, $cmd) {
    if (Get-Command $cmd -ErrorAction SilentlyContinue) {
        Write-Host "[ok] $name" -ForegroundColor Green
        return $true
    } else {
        Write-Host "[缺失] $name ($cmd 未找到)" -ForegroundColor Yellow
        return $false
    }
}

Write-Host "=== 依赖自检 ==="
$hasNode   = Check "Node.js"      node
$hasCargo  = Check "Rust/cargo"   cargo
$hasTauri  = Check "tauri CLI"    tauri
if (-not $hasNode)  { Write-Host "请先安装 Node.js 18+"; Read-Host; exit 1 }
if (-not $hasCargo) { Write-Host "请先安装 Rust (MSVC)"; Read-Host; exit 1 }

# MSVC 链接器检测
$cl = Get-Command cl -ErrorAction SilentlyContinue
if (-not $cl) {
    Write-Host "[缺失] MSVC 链接器 (cl)。请安装 VS2022 生成工具 -> 工作负载'C++ 桌面开发'。" -ForegroundColor Red
    Write-Host "  winget install Microsoft.VisualStudio.2022.BuildTools --override `"--add Microsoft.VisualStudio.Workload.VCTools`"" -ForegroundColor Cyan
}
# WebView2 检测
$wv2 = Test-Path "C:\Windows\System32\WebView2Loader.dll"
if (-not $wv2) {
    Write-Host "[提示] 未检测到 WebView2。安装器已嵌入引导程序，目标机会自动补齐；" -ForegroundColor Yellow
    Write-Host "       若本地运行需 WebView2，可装 Microsoft.EdgeWebView2Runtime。" -ForegroundColor Yellow
}

Write-Host "=== [1/3] 安装 cargo-tauri ==="
if (-not $hasTauri) {
    cargo install tauri-cli --version "^2" --locked
}

Write-Host "=== [2/3] 前端依赖 + 构建 ==="
npm --prefix ui-web install
npm --prefix ui-web approve-scripts esbuild
npm --prefix ui-web run build

Write-Host "=== [3/3] cargo tauri build ==="
cargo tauri build

Write-Host ""
Write-Host "构建完成，安装器位于 groundctrl-tauri\target\release\bundle\nsis\"
Get-ChildItem groundctrl-tauri\target\release\bundle\nsis\*.exe -ErrorAction SilentlyContinue
Read-Host "按回车退出"
