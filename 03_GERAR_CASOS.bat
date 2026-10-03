@echo off
cd /d "%~dp0"
rem ===== CONFIGURACAO =====
rem Quantidade de transacoes geradas (o 09_SIMULAR_ATAQUES_SOLANA.bat usa estes casos).
set "QTD_CASOS=200"
rem =========================
.venv\Scripts\python.exe -m src.main generate --count %QTD_CASOS%
if errorlevel 1 (pause & exit /b 1)
.venv\Scripts\python.exe -m src.solana_attack_simulator --prepare-only --count %QTD_CASOS%
pause
