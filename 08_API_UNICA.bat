@echo off
setlocal EnableExtensions
cd /d "%~dp0"
title VETO - API Unica

if not exist ".venv\Scripts\python.exe" (echo [ERRO] Ambiente .venv ausente. Execute 01_INSTALAR.bat primeiro. & pause & exit /b 1)

echo API publica: http://127.0.0.1:8070
echo Rotas: /v1/transactions/verify e /v1/agent/run
echo Backend esperado: llama.cpp em http://127.0.0.1:18080
echo Anvil esperado: http://127.0.0.1:8545
echo.
".venv\Scripts\python.exe" -m src.unified_api --host 127.0.0.1 --port 8070
if errorlevel 1 echo [ERRO] A API foi encerrada com erro.
pause
