@echo off
rem Adds Leaf to this account's "Open with" lists (see install.ps1).
rem   install.cmd            add Leaf (again, after moving this folder)
rem   install.cmd --remove   take it out again
rem Windows doesn't run downloaded PowerShell scripts by default; this one
rem runs because you started it.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install.ps1" %*
set code=%errorlevel%
rem Opened with a double-click: keep the window up to read.
echo %cmdcmdline% | find /i "%~0" >nul && pause
exit /b %code%
