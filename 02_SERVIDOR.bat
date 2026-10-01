@echo off
setlocal EnableExtensions
cd /d "%~dp0"
title VETO - Servidor Gemma Q4_K_M com LoRA

set "SERVER=llama\llama-server.exe"
set "MODEL=models\gemma-3-4b-it-Q4_K_M.gguf"
set "LORA=models\VETO-Security-LoRA-F16.gguf"

if not exist "%SERVER%" (echo [ERRO] Faltando "%SERVER%". & pause & exit /b 1)
if not exist "%MODEL%" (echo [ERRO] Faltando a base Q4_K_M: "%MODEL%". & pause & exit /b 1)
if not exist "%LORA%" (echo [ERRO] Faltando o adaptador LoRA GGUF: "%LORA%". & pause & exit /b 1)

echo Modelo base: %MODEL%
echo LoRA:        %LORA%
echo Servidor:    http://127.0.0.1:8080
echo.
"%SERVER%" -m "%MODEL%" --lora "%LORA%" --host 127.0.0.1 --port 8080 -c 8192 -np 2 -ngl 99 --jinja
if errorlevel 1 echo [ERRO] O llama-server foi encerrado com erro.
pause
