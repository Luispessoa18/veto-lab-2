@echo off
setlocal EnableExtensions
cd /d "%~dp0"
title VETO - Avaliacao anti-injection

set "PY=.venv\Scripts\python.exe"
if not exist "%PY%" (echo [ERRO] Ambiente .venv ausente. Execute 01_INSTALAR.bat. & pause & exit /b 1)

if not exist "fixtures\injection\neuralchemy_core_test.jsonl" (
  echo Baixando dataset neuralchemy...
  "%PY%" -m src.injection_datasets neuralchemy || (echo [ERRO] Falha ao baixar o neuralchemy. & pause & exit /b 1)
)
if not exist "external\datasets\necent_injection_sample.jsonl" (
  echo Tentando baixar o dataset Necent ^(gated: exige aceitar os termos no Hugging Face e HF_TOKEN^)...
  "%PY%" -m src.injection_datasets necent || echo [AVISO] Necent indisponivel; avaliando so o neuralchemy.
)

set "NEED_CALIBRATION="
if not exist "results\injection_calibration.json" set "NEED_CALIBRATION=1"
if exist "results\injection_calibration.json" findstr /c:"block_threshold" "results\injection_calibration.json" >nul || set "NEED_CALIBRATION=1"
if defined NEED_CALIBRATION (
  echo Calibrando os limites dos detectores no split de validacao...
  "%PY%" -m src.calibrate_injection || (echo [ERRO] Falha na calibracao. & pause & exit /b 1)
)
echo Avaliando regras e Prompt Guard no split de teste. Na CPU pode levar varios minutos.
"%PY%" -m src.eval_injection --open
if errorlevel 1 (echo [ERRO] Falha na avaliacao. & pause & exit /b 1)
pause
