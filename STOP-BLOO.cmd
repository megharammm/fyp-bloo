@echo off
title Bloo AI - stop
cd /d "%~dp0"
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\stop.ps1"
timeout /t 3 >nul
