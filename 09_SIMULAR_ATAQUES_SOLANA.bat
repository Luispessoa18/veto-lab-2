@echo off
setlocal EnableExtensions
cd /d "%~dp0"
title VETO - Simulador de ataques Solana

set "PY=.venv-lora\Scripts\python.exe"
if not exist "%PY%" (echo [ERRO] Ambiente .venv-lora ausente. & pause & exit /b 1)

powershell -NoProfile -Command "try { $null=Invoke-WebRequest -UseBasicParsing http://127.0.0.1:8070/health -TimeoutSec 3; exit 0 } catch { exit 1 }"
if errorlevel 1 (
  echo API indisponivel; iniciando IA, API e Prompt Guard automaticamente...
  start "VETO - IA" /min cmd /c "02_SERVIDOR.bat"
  start "VETO - API + Prompt Guard" /min cmd /c "08_API_UNICA.bat"
  powershell -NoProfile -Command "$ok=$false; for($i=0;$i -lt 90;$i++){ try { $api=Invoke-WebRequest -UseBasicParsing http://127.0.0.1:8070/health -TimeoutSec 2; $llm=Invoke-WebRequest -UseBasicParsing http://127.0.0.1:8080/health -TimeoutSec 2; if($api.StatusCode -eq 200 -and $llm.StatusCode -eq 200){$ok=$true; break} } catch {}; Start-Sleep -Seconds 2 }; if(-not $ok){exit 1}"
  if errorlevel 1 (
    echo [ERRO] A API/IA nao respondeu em 180 segundos. Veja as janelas VETO.
    pause
    exit /b 1
  )
)

echo Executando casos sinteticos. Nenhuma transacao sera enviada para a Solana.
"%PY%" -m src.solana_attack_simulator --api http://127.0.0.1:8070
if errorlevel 1 (echo [ERRO] O benchmark falhou. & pause & exit /b 1)
echo.
echo Resultado: results\solana_attack_simulation.jsonl
pause
