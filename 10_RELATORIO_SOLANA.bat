@echo off
setlocal EnableExtensions
cd /d "%~dp0"
title VETO - Relatorio Solana

set "PY=.venv\Scripts\python.exe"
if not exist "%PY%" (echo [ERRO] Ambiente .venv ausente. Execute 01_INSTALAR.bat. & pause & exit /b 1)
if not exist "results\solana_attack_simulation.jsonl" (echo [ERRO] Sem resultados. Execute 09_SIMULAR_ATAQUES_SOLANA.bat. & pause & exit /b 1)

"%PY%" -m src.solana_html_report --open
if errorlevel 1 (echo [ERRO] Falha ao gerar o relatorio. & pause & exit /b 1)
echo Use o botao "Imprimir / salvar PDF" no topo da pagina.
pause
