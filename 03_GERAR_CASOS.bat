@echo off
cd /d "%~dp0"
.venv\Scripts\python.exe -m src.main generate --count 1200
pause
