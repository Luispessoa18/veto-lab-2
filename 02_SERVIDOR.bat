@echo off
setlocal EnableExtensions
cd /d "%~dp0"
title VETO - Servidor de IA

set "SERVER=llama\llama-server.exe"
if not defined VETO_MODEL set "VETO_MODEL=models\veto_lfm2_5_350m_aave_f16.gguf"
if not defined VETO_NGL set "VETO_NGL=0"
set "MODEL=%VETO_MODEL%"
set "LORA_ARGS="
if defined VETO_LORA set "LORA_ARGS=--lora %VETO_LORA%"

if not exist "%SERVER%" (echo [ERRO] Faltando "%SERVER%". & pause & exit /b 1)
if not exist "%MODEL%" (echo [ERRO] Modelo ausente: "%MODEL%". & pause & exit /b 1)
if defined VETO_LORA if not exist "%VETO_LORA%" (echo [ERRO] LoRA ausente: "%VETO_LORA%". & pause & exit /b 1)

echo Modelo base: %MODEL%
if defined VETO_LORA (echo LoRA externo: %VETO_LORA%) else (echo LoRA: embutido no modelo)
echo GPU layers:  %VETO_NGL% ^(0 = CPU^)
echo Servidor:    http://127.0.0.1:18080
echo.
"%SERVER%" -m "%MODEL%" %LORA_ARGS% --host 127.0.0.1 --port 18080 -c 4096 -np 2 -ngl %VETO_NGL% --jinja
if errorlevel 1 echo [ERRO] O llama-server foi encerrado com erro.
pause
