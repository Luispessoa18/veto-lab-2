@echo off
cd /d "%~dp0"
.venv\Scripts\python.exe -m src.main catalog --fetch-rpc
pause
