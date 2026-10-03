@echo off
setlocal EnableExtensions
cd /d "%~dp0"
title VETO - Simulador de ataques Solana

set "PY=.venv\Scripts\python.exe"
if not exist "%PY%" (echo [ERRO] Ambiente .venv ausente. Execute 01_INSTALAR.bat. & pause & exit /b 1)

rem Usa os casos gerados pelo 03_GERAR_CASOS.bat (quantidade definida la).
rem So gera aqui se ainda nao existirem, ou se VETO_NEW_CASES=1 (seed nova, VETO_CASES transacoes).
if not defined VETO_CASES set "VETO_CASES=600"
set "NEED_CASES="
if defined VETO_NEW_CASES set "NEED_CASES=1"
if not exist "results\solana_cases.jsonl" set "NEED_CASES=1"
if not exist "results\solana_answers.jsonl" set "NEED_CASES=1"
if defined NEED_CASES (
  echo Gerando %VETO_CASES% transacoes adversariais novas...
  "%PY%" -m src.solana_attack_simulator --prepare-only --count %VETO_CASES%
  if errorlevel 1 (echo [ERRO] Nao foi possivel gerar os casos Solana. & pause & exit /b 1)
) else (
  echo Usando os casos ja gerados em results\solana_cases.jsonl
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
echo Relatorio imprimivel: results\relatorio_solana.html
start "" "results\relatorio_solana.html"
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
