@echo off
rem TokenMeter Windows 本机构建脚本
rem 前置：Node 22+、Rust（x86_64-pc-windows-msvc target）、VS Build Tools（VCTools workload）
rem 用法：scripts\win\build.cmd  （在仓库根目录执行）

cd /d "%~dp0..\.."

rem 配置 MSVC 环境（x64 目标；ARM64 机器同样适用）
call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvarsall.bat" amd64 >nul
if errorlevel 1 (
  echo [ERROR] vcvarsall.bat 初始化失败，请确认 VS Build Tools 已安装
  exit /b 1
)

echo [1/3] npm ci
call npm ci || exit /b 1

echo [2/3] 前端构建
call npm run build || exit /b 1

echo [3/3] cargo build --release --features custom-protocol
call cargo build --release --features custom-protocol --manifest-path src-tauri\Cargo.toml || exit /b 1

echo.
echo BUILD_OK: src-tauri\target\release\tokenmeter.exe
