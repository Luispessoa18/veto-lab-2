@echo off
setlocal EnableExtensions
cd /d "%~dp0"
title VETO - API Unica

if not exist ".venv-lora\Scripts\python.exe" (echo [ERRO] Ambiente .venv-lora ausente. Execute 05_CONVERTER_LORA_GGUF.bat primeiro. & pause & exit /b 1)

echo API publica: http://127.0.0.1:8070
echo Rotas: /v1/transactions/verify e /v1/agent/run
echo Backend esperado: llama.cpp em http://127.0.0.1:8080
echo Anvil esperado: http://127.0.0.1:8545
echo.
".venv-lora\Scripts\python.exe" -m src.unified_api --host 127.0.0.1 --port 8070
if errorlevel 1 echo [ERRO] A API foi encerrada com erro.
pause
