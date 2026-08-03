@echo off
rem TokenMeter dev 实例启动包装（Windows 本机构建产物）
rem 走宿主机 Mac 的代理（HK 出口）：绕过 OpenAI 对国内 IP 的地区拦截
set HTTP_PROXY=http://192.168.68.1:7890
set HTTPS_PROXY=http://192.168.68.1:7890
set TOKENMETER_LOG_FILE=C:\Users\hangbits\tokenmeter\tokenmeter.log
C:\Users\hangbits\tokenmeter\repo\src-tauri\target\x86_64-pc-windows-msvc\release\tokenmeter.exe
