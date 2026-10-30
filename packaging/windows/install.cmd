@echo off
rem Adds leaf9000 to this account's "Open with" lists (see install.ps1).
rem   install.cmd            add leaf9000 (again, after moving this folder)
rem   install.cmd --remove   take it out again
rem Windows doesn't run downloaded PowerShell scripts by default; this one
rem runs because you started it.
set "LEAF_INSTALL=%~1"
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install.ps1"
set code=%errorlevel%
rem Opened with a double-click: keep the window up to read.
echo %cmdcmdline% | find /i "%~0" >nul && pause
exit /b %code%
