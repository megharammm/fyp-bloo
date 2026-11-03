@echo off
title Bloo AI
cd /d "%~dp0"
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\start.ps1"
echo.
echo Bloo AI stopped. Press any key to close.
pause >nul
