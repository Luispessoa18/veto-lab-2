@echo off
setlocal EnableExtensions
cd /d "%~dp0"
title VETO - Inicializador completo

set "LLAMA=llama\llama-server.exe"
set "MODEL=models\gemma-3-4b-it-Q4_K_M.gguf"
set "LORA=models\VETO-Security-LoRA-F16.gguf"
set "PY=.venv-lora\Scripts\python.exe"
set "ANVIL_EXE="

if not exist "%LLAMA%" (echo [ERRO] Faltando %LLAMA% & goto :fail)
if not exist "%MODEL%" (echo [ERRO] Faltando %MODEL% & goto :fail)
if not exist "%LORA%" (echo [ERRO] Faltando %LORA% & goto :fail)
if not exist "%PY%" (echo [ERRO] Faltando .venv-lora. Execute 05_CONVERTER_LORA_GGUF.bat. & goto :fail)
if not exist "models\Llama-Prompt-Guard-2-86M\model.safetensors" (echo [ERRO] Modelo Prompt Guard ausente. & goto :fail)

for /f "delims=" %%I in ('where anvil.exe 2^>nul') do if not defined ANVIL_EXE set "ANVIL_EXE=%%I"
if not defined ANVIL_EXE if exist "%USERPROFILE%\.foundry\bin\anvil.exe" set "ANVIL_EXE=%USERPROFILE%\.foundry\bin\anvil.exe"
if not defined ANVIL_EXE (
  echo [ERRO] anvil.exe nao encontrado.
  echo Instale o Foundry para Windows e confirme que anvil esta no PATH.
  echo Depois execute este arquivo novamente.
  goto :fail
)

echo Iniciando llama.cpp na porta 8080...
start "VETO - IA" /min "%LLAMA%" -m "%MODEL%" --lora "%LORA%" --host 127.0.0.1 --port 8080 -c 8192 -np 2 -ngl 99 --jinja

echo Iniciando Anvil na porta 8545...
start "VETO - Anvil" /min "%ANVIL_EXE%" --host 127.0.0.1 --port 8545

echo Iniciando API e Prompt Guard na porta 8070...
start "VETO - API + Prompt Guard" /min "%PY%" -m src.unified_api --host 127.0.0.1 --port 8070

echo Aguardando a API carregar os modelos...
powershell -NoProfile -Command "$ok=$false; 1..60 | ForEach-Object { try { $r=Invoke-WebRequest -UseBasicParsing http://127.0.0.1:8070/health -TimeoutSec 2; if($r.StatusCode -eq 200){$ok=$true; break} } catch {}; Start-Sleep -Seconds 2 }; if(-not $ok){exit 1}"
if errorlevel 1 (
  echo [ERRO] A API nao respondeu em 120 segundos. Veja a janela VETO - API + Prompt Guard.
  goto :fail
)

echo.
echo Tudo pronto.
echo Swagger: http://127.0.0.1:8070/docs
echo OpenAPI: http://127.0.0.1:8070/openapi.json
start "" "http://127.0.0.1:8070/docs"
exit /b 0

:fail
echo.
pause
exit /b 1
