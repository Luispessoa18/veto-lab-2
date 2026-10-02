@echo off
cd /d "%~dp0"
.venv\Scripts\python.exe -m src.main generate --count 200
if errorlevel 1 (pause & exit /b 1)
.venv\Scripts\python.exe -m src.solana_attack_simulator --prepare-only --limit 200
pause
