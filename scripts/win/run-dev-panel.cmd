@echo off
rem Diagnostic: 启动即自动打开面板并进入"添加供应商"视图（配合 TOKENMETER_AUTO_PANEL）
set TOKENMETER_AUTO_PANEL=1
set TOKENMETER_LOG_FILE=C:\Users\hangbits\tokenmeter\tokenmeter.log
C:\Users\hangbits\tokenmeter\repo\src-tauri\target\x86_64-pc-windows-msvc\release\tokenmeter.exe
