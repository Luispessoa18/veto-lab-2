@echo off
setlocal EnableExtensions
cd /d "%~dp0"
title VETO - Servidor de IA

rem ===== CONFIGURACAO =====
rem USAR_GPU=1 roda o LLM na GPU (CUDA). USAR_GPU=0 roda so na CPU.
if not defined USAR_GPU set "USAR_GPU=1"
rem MODELO=gemma usa Gemma 3 4B + LoRA VETO. MODELO=lfm usa LFM2.5 350M Q8 (LoRA embutido).
if not defined MODELO set "MODELO=gemma"
rem =========================

set "SERVER=llama\llama-server.exe"
if /i "%MODELO%"=="gemma" (
  if not defined VETO_MODEL set "VETO_MODEL=models\gemma-3-4b-it-Q4_K_M.gguf"
  if not defined VETO_LORA set "VETO_LORA=models\VETO-Security-LoRA-F16.gguf"
)
if not defined VETO_MODEL set "VETO_MODEL=models\veto_lfm2_5_350m_aave_q8_0.gguf"
if not defined VETO_NGL if "%USAR_GPU%"=="1" (set "VETO_NGL=99") else (set "VETO_NGL=0")
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
"%SERVER%" -m "%MODEL%" %LORA_ARGS% --host 127.0.0.1 --port 18080 -c 8192 -np 2 -ngl %VETO_NGL% --jinja
if errorlevel 1 echo [ERRO] O llama-server foi encerrado com erro.
pause
