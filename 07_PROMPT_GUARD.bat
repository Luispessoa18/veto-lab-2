@echo off
setlocal EnableExtensions
cd /d "%~dp0"
title VETO - Meta Prompt Guard 2
set "PY=.venv-lora\Scripts\python.exe"
set "MODEL=models\Llama-Prompt-Guard-2-86M"
if not exist "%PY%" (echo [ERRO] Ambiente .venv-lora ausente. Execute o conversor LoRA primeiro. & pause & exit /b 1)
if not exist "%MODEL%\model.safetensors" (echo [ERRO] Prompt Guard ausente em "%MODEL%". & pause & exit /b 1)
"%PY%" -m src.guard_server --model "%MODEL%" --host 127.0.0.1 --port 8090 --threshold 0.90
if errorlevel 1 echo [ERRO] Prompt Guard encerrado com erro.
pause
