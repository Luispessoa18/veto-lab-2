@echo off
setlocal EnableExtensions
cd /d "%~dp0"
title VETO - Inicializador completo

set "LLAMA=llama\llama-server.exe"
if not defined VETO_MODEL set "VETO_MODEL=models\veto_lfm2_5_350m_aave_f16.gguf"
if not defined VETO_NGL set "VETO_NGL=0"
set "MODEL=%VETO_MODEL%"
set "LORA_ARGS="
if defined VETO_LORA set "LORA_ARGS=--lora %VETO_LORA%"
set "PY=.venv\Scripts\python.exe"
set "ANVIL_EXE="

if not exist "%LLAMA%" (echo [ERRO] Faltando %LLAMA% & goto :fail)
if not exist "%MODEL%" (echo [ERRO] Faltando %MODEL% & goto :fail)
if defined VETO_LORA if not exist "%VETO_LORA%" (echo [ERRO] Faltando %VETO_LORA% & goto :fail)
if not exist "%PY%" (echo [ERRO] Faltando .venv. Execute 01_INSTALAR.bat. & goto :fail)
if not exist "models\Llama-Prompt-Guard-2-86M\model.safetensors" (echo [ERRO] Modelo Prompt Guard ausente. & goto :fail)

for /f "delims=" %%I in ('where anvil.exe 2^>nul') do if not defined ANVIL_EXE set "ANVIL_EXE=%%I"
if not defined ANVIL_EXE if exist "%USERPROFILE%\.foundry\bin\anvil.exe" set "ANVIL_EXE=%USERPROFILE%\.foundry\bin\anvil.exe"
if not defined ANVIL_EXE (
  echo [AVISO] anvil.exe nao encontrado. Testes EVM ficarao indisponiveis.
  echo O pipeline Solana continuara normalmente.
)

echo Iniciando llama.cpp na porta dedicada 18080...
echo Modelo: %MODEL% ^| GPU layers: %VETO_NGL%
start "VETO - IA" /min "%LLAMA%" -m "%MODEL%" %LORA_ARGS% --host 127.0.0.1 --port 18080 -c 4096 -np 2 -ngl %VETO_NGL% --jinja

if defined ANVIL_EXE (
  echo Iniciando Anvil opcional na porta 8545...
  start "VETO - Anvil" /min "%ANVIL_EXE%" --host 127.0.0.1 --port 8545
)

echo Iniciando API e Prompt Guard na porta 8070...
start "VETO - API + Prompt Guard" /min "%PY%" -m src.unified_api --host 127.0.0.1 --port 8070

echo Aguardando a API carregar os modelos...
call :wait_api
if errorlevel 1 (
  echo [ERRO] A API nao respondeu em 120 segundos. Veja a janela VETO - API + Prompt Guard.
  goto :fail
)

echo.
echo Tudo pronto.
echo Swagger: http://127.0.0.1:8070/docs
echo Painel admin: http://127.0.0.1:8070/admin
echo OpenAPI: http://127.0.0.1:8070/openapi.json
start "" "http://127.0.0.1:8070/docs"
start "" "http://127.0.0.1:8070/admin"
exit /b 0

:wait_api
set /a WAIT_COUNT=0
:wait_api_loop
curl.exe --fail --silent --max-time 2 http://127.0.0.1:8070/health 2>nul | findstr.exe /c:"18080" >nul
if errorlevel 1 goto :wait_api_retry
curl.exe --fail --silent --max-time 2 http://127.0.0.1:18080/health >nul 2>nul
if not errorlevel 1 exit /b 0
:wait_api_retry
set /a WAIT_COUNT+=1
if %WAIT_COUNT% GEQ 60 exit /b 1
timeout.exe /t 2 /nobreak >nul
goto :wait_api_loop

:fail
echo.
pause
exit /b 1
