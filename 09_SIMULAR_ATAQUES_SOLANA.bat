@echo off
setlocal EnableExtensions
cd /d "%~dp0"
title VETO - Simulador de ataques Solana

set "PY=.venv\Scripts\python.exe"
if not exist "%PY%" (echo [ERRO] Ambiente .venv ausente. Execute 01_INSTALAR.bat. & pause & exit /b 1)

if not exist "results\synthetic_cases.jsonl" (
  echo Casos-base ausentes; gerando 600 casos...
  "%PY%" -m src.main generate --count 600
  if errorlevel 1 (echo [ERRO] Nao foi possivel gerar os casos-base. & pause & exit /b 1)
)
if not exist "results\solana_cases.jsonl" (
  echo Casos Solana ausentes; preparando dataset adversarial...
  "%PY%" -m src.solana_attack_simulator --prepare-only --limit 600
  if errorlevel 1 (echo [ERRO] Nao foi possivel preparar os casos Solana. & pause & exit /b 1)
)

curl.exe --fail --silent --max-time 3 http://127.0.0.1:8070/health 2>nul | findstr.exe /c:"18080" >nul
if not errorlevel 1 curl.exe --fail --silent --max-time 3 http://127.0.0.1:18080/health >nul 2>nul
if errorlevel 1 (
  echo API indisponivel; iniciando IA, API e Prompt Guard automaticamente...
  start "VETO - IA" /min cmd /c "02_SERVIDOR.bat"
  start "VETO - API + Prompt Guard" /min cmd /c "08_API_UNICA.bat"
  call :wait_services
  if errorlevel 1 (
    echo [ERRO] A API/IA nao respondeu em 180 segundos. Veja as janelas VETO.
    pause
    exit /b 1
  )
)

echo Executando casos sinteticos. Nenhuma transacao sera enviada para a Solana.
"%PY%" -m src.solana_attack_simulator --api http://127.0.0.1:8070 --prepared results\solana_cases.jsonl
if errorlevel 1 (echo [ERRO] O benchmark falhou. & pause & exit /b 1)
echo.
echo Resultado: results\solana_attack_simulation.jsonl
pause
exit /b 0

:wait_services
set /a WAIT_COUNT=0
:wait_services_loop
curl.exe --fail --silent --max-time 2 http://127.0.0.1:8070/health 2>nul | findstr.exe /c:"18080" >nul
if errorlevel 1 goto :wait_services_retry
curl.exe --fail --silent --max-time 2 http://127.0.0.1:18080/health >nul 2>nul
if not errorlevel 1 exit /b 0
:wait_services_retry
set /a WAIT_COUNT+=1
if %WAIT_COUNT% GEQ 90 exit /b 1
timeout.exe /t 2 /nobreak >nul
goto :wait_services_loop
