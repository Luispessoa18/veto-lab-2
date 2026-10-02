@echo off
setlocal EnableExtensions
cd /d "%~dp0"
title VETO - Converter LoRA para GGUF

set "ROOT=%~dp0"
set "ADAPTER=%ROOT%models\VETO-Security-Gemma3-4B-LoRA"
set "LORA_GGUF=%ROOT%models\VETO-Security-LoRA-F16.gguf"
set "BASE_F16=%ROOT%models\gemma-3-4b-it-f16.gguf"
set "MERGED_F16=%ROOT%models\VETO-Security-Gemma3-4B-F16.gguf"
set "FINAL=%ROOT%models\VETO-Security-Gemma3-4B-Q5_K_M.gguf"
set "SOURCE=%ROOT%tools\llama.cpp"
set "VENV=%ROOT%.venv-lora"
set "SERVER=%ROOT%llama\llama-server.exe"
set "EXPORTER=%ROOT%llama\llama-export-lora.exe"
set "QUANTIZER=%ROOT%llama\llama-quantize.exe"
set "BASE_URL=https://huggingface.co/ggml-org/gemma-3-4b-it-GGUF/resolve/main/gemma-3-4b-it-f16.gguf?download=true"
set "BASE_CONFIG_URL=https://huggingface.co/unsloth/gemma-3-4b-it/resolve/main/config.json"

if not exist "%ADAPTER%\adapter_config.json" (
 echo [ERRO] Coloque adapter_config.json e adapter_model.safetensors em:
 echo "%ADAPTER%"
 goto :fail
)
if not exist "%ADAPTER%\adapter_model.safetensors" (
 echo [ERRO] adapter_model.safetensors ausente em "%ADAPTER%"
 goto :fail
)
if not exist "%SERVER%" (
 echo [ERRO] llama-server.exe nao encontrado em "%SERVER%"
 goto :fail
)
if not exist "%QUANTIZER%" (
 echo [ERRO] llama-quantize.exe nao encontrado em "%ROOT%llama".
 goto :fail
)
if not exist "%EXPORTER%" if exist "%SOURCE%\build-export\bin\llama-export-lora.exe" set "EXPORTER=%SOURCE%\build-export\bin\llama-export-lora.exe"
if not exist "%EXPORTER%" (
 echo [ERRO] llama-export-lora.exe nao foi incluido no pacote instalado.
 echo Baixe/extrai uma versao completa do llama.cpp em "%ROOT%llama".
 echo Esse executavel e necessario para embutir o LoRA no modelo antes do Q5_K_M.
 goto :fail
)
rem Verifica o modelo exato informado pelo adaptador. Nao baixar GGUF de uma arquitetura diferente.
findstr /i /c:"gemma-3-4b-it" "%ADAPTER%\adapter_config.json" >nul || (
 echo [ERRO] O adapter_config.json nao declara Gemma 3 4B IT como modelo base.
 goto :fail
)

if not exist "%SOURCE%\convert_lora_to_gguf.py" (
 echo [1/4] Baixando o conversor oficial do llama.cpp...
 if not exist "%ROOT%tools" mkdir "%ROOT%tools"
 curl.exe -fL --retry 5 -o "%ROOT%tools\llama.cpp.zip" "https://github.com/ggml-org/llama.cpp/archive/refs/heads/master.zip" || goto :fail
 tar -xf "%ROOT%tools\llama.cpp.zip" -C "%ROOT%tools" || goto :fail
 if not exist "%ROOT%tools\llama.cpp-master\convert_lora_to_gguf.py" goto :fail
 move "%ROOT%tools\llama.cpp-master" "%SOURCE%" || goto :fail
)

if exist "%LORA_GGUF%" (
 echo [INFO] Adaptador GGUF F16 ja existe: "%LORA_GGUF%"
) else (
 where python >nul 2>&1 || (echo [ERRO] Python nao encontrado no PATH. & goto :fail)
 if not exist "%VENV%\Scripts\python.exe" (
  echo [3/4] Criando ambiente isolado do conversor...
  python -m venv "%VENV%" || goto :fail
 )
 rem O llama-quantize.exe recebe GGUF e nao converte adapter_model.safetensors.
 rem Instala as dependencias oficiais somente na primeira vez em que faltarem.
 "%VENV%\Scripts\python.exe" -c "import gguf,numpy,safetensors,torch" >nul 2>&1
 if errorlevel 1 (
  echo [3/4] Instalando uma vez as dependencias oficiais do conversor LoRA...
  "%VENV%\Scripts\python.exe" -m pip install --disable-pip-version-check -r "%SOURCE%\requirements\requirements-convert_lora_to_gguf.txt" || goto :fail
 )

 rem O requirements atual fixa NumPy 2.2.6, que falha ao importar no Python 3.14/Windows.
 "%VENV%\Scripts\python.exe" -c "import numpy" >nul 2>&1
 if errorlevel 1 (
  echo [3/4] Corrigindo NumPy para Python 3.14...
  "%VENV%\Scripts\python.exe" -m pip install --disable-pip-version-check --upgrade "numpy^>=2.3.3,^<3" || goto :fail
 )
 "%VENV%\Scripts\python.exe" -c "import gguf,numpy,safetensors,torch,transformers" >nul 2>&1 || (
  echo [ERRO] As dependencias do conversor continuam incompativeis.
  goto :fail
 )
 echo [3/4] Dependencias do conversor prontas.

 if not exist "%ADAPTER%\config.json" (
  echo [3/4] Baixando configuracao pequena do modelo base...
  curl.exe -fL --retry 5 -o "%ADAPTER%\config.json" "%BASE_CONFIG_URL%" || goto :fail
 )

 echo [2/5] Convertendo o adaptador PEFT para LoRA GGUF F16...
 "%VENV%\Scripts\python.exe" "%SOURCE%\convert_lora_to_gguf.py" "%ADAPTER%" --base "%ADAPTER%" --outtype f16 --outfile "%LORA_GGUF%" || goto :fail
)
if not exist "%LORA_GGUF%" goto :fail
for %%F in ("%LORA_GGUF%") do if %%~zF LSS 1024 (echo [ERRO] LoRA GGUF incompleto. & goto :fail)

if exist "%BASE_F16%" goto :base_ready
echo [3/5] Baixando Gemma 3 4B IT F16; arquivo grande, pode levar tempo...
curl.exe -fL --retry 5 -C - -o "%BASE_F16%" "%BASE_URL%" || goto :fail

:base_ready
if not exist "%BASE_F16%" (
 echo [ERRO] A base F16 nao foi criada: "%BASE_F16%"
 goto :fail
)
for %%F in ("%BASE_F16%") do if %%~zF LSS 1048576 (echo [ERRO] A base F16 esta vazia ou incompleta. & goto :fail)
echo [3/5] Modelo base F16 confirmado.

if exist "%MERGED_F16%" goto :merged_ready
echo [4/5] Embutindo o LoRA no modelo base F16...
"%EXPORTER%" -m "%BASE_F16%" --lora "%LORA_GGUF%" -o "%MERGED_F16%" || goto :fail

:merged_ready
if not exist "%MERGED_F16%" (
 echo [ERRO] O exportador nao criou o modelo mesclado:
 echo "%MERGED_F16%"
 goto :fail
)
for %%F in ("%MERGED_F16%") do if %%~zF LSS 1048576 (echo [ERRO] O modelo mesclado esta vazio ou incompleto. & goto :fail)
echo [4/5] Modelo F16 com LoRA embutido confirmado.

if exist "%FINAL%" goto :final_ready
echo [5/5] Quantizando o modelo mesclado para Q5_K_M...
"%QUANTIZER%" "%MERGED_F16%" "%FINAL%" Q5_K_M || goto :fail

:final_ready
if not exist "%FINAL%" (
 echo [ERRO] O quantizador nao criou o GGUF Q5_K_M.
 goto :fail
)
for %%F in ("%FINAL%") do if %%~zF LSS 1048576 (echo [ERRO] GGUF Q5_K_M vazio ou incompleto. & goto :fail)
echo [5/5] Modelo final Q5_K_M confirmado.

echo.
echo [OK] Modelo final com LoRA embutido: "%FINAL%"
echo Iniciando o servidor com o GGUF Q5_K_M unico. CTRL+C para parar.
"%SERVER%" -m "%FINAL%" --host 127.0.0.1 --port 18080 -ngl 99 -c 2048
if errorlevel 1 goto :fail
pause
exit /b 0
:fail
echo.
echo [ERRO] Processo interrompido. Leia a mensagem acima. Seus arquivos originais foram preservados.
pause
exit /b 1
