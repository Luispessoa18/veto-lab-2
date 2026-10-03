@echo off
setlocal EnableExtensions
cd /d "%~dp0"
title VETO - Baixar modelos

set "GEMMA=models\gemma-3-4b-it-Q4_K_M.gguf"
set "GEMMA_URL=https://huggingface.co/ggml-org/gemma-3-4b-it-GGUF/resolve/main/gemma-3-4b-it-Q4_K_M.gguf?download=true"
set "GUARD=models\Llama-Prompt-Guard-2-86M"
set "GUARD_REPO=meta-llama/Llama-Prompt-Guard-2-86M"
set "PIGUARD=models\PIGuard"
set "PIGUARD_REPO=leolee99/PIGuard"

if not exist models mkdir models

if exist "%GEMMA%" (
  echo [OK] Gemma ja existe: %GEMMA%
) else (
  echo [1/3] Baixando Gemma 3 4B IT Q4_K_M ^(~2.5 GB^)...
  curl.exe -fL --retry 5 -C - -o "%GEMMA%" "%GEMMA_URL%" || (echo [ERRO] Falha ao baixar o Gemma. & pause & exit /b 1)
)

if exist "%GUARD%\model.safetensors" (
  echo [OK] Prompt Guard ja existe: %GUARD%
  goto :done
)
echo [2/3] Baixando Prompt Guard 2 86M ^(repositorio restrito da Meta^).
echo Aceite a licenca em https://huggingface.co/%GUARD_REPO% e defina HF_TOKEN antes de rodar.
if "%HF_TOKEN%"=="" (
  echo [AVISO] HF_TOKEN nao definido; pulando Prompt Guard.
  echo Exemplo: set HF_TOKEN=hf_xxx ^&^& BAIXAR_MODELOS.bat
  goto :done
)
if not exist "%GUARD%" mkdir "%GUARD%"
for %%F in (config.json model.safetensors special_tokens_map.json tokenizer.json tokenizer_config.json) do (
  echo   - %%F
  curl.exe -fL --retry 5 -C - -H "Authorization: Bearer %HF_TOKEN%" -o "%GUARD%\%%F" "https://huggingface.co/%GUARD_REPO%/resolve/main/%%F?download=true" || (echo [ERRO] Falha ao baixar %%F. & pause & exit /b 1)
)

:done
if exist "%PIGUARD%\model.safetensors" (
  echo [OK] PIGuard ja existe: %PIGUARD%
) else (
  echo [3/3] Baixando PIGuard ^(segundo detector anti-injection, MIT, ~740 MB^)...
  if not exist "%PIGUARD%" mkdir "%PIGUARD%"
  rem So pesos, tokenizer e configuracao: o modeling_piguard.py do Hub NAO e baixado nem executado;
  rem o VETO carrega o modelo com uma reimplementacao local ^(src\guard_server.py^).
  for %%F in (config.json model.safetensors tokenizer.json tokenizer_config.json special_tokens_map.json added_tokens.json) do (
    echo   - %%F
    curl.exe -fL --retry 5 -C - -o "%PIGUARD%\%%F" "https://huggingface.co/%PIGUARD_REPO%/resolve/main/%%F?download=true" || (echo [ERRO] Falha ao baixar %%F. & pause & exit /b 1)
  )
)
echo.
echo Pronto. O LoRA ja vem no repositorio: models\VETO-Security-LoRA-F16.gguf
pause
