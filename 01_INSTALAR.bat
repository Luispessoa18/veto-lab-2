@echo off
cd /d "%~dp0"
where py >nul 2>nul
if errorlevel 1 (echo Instale Python 3.11+ e adicione ao PATH. & pause & exit /b 1)
py -3 -m venv .venv
if errorlevel 1 (pause & exit /b 1)
.venv\Scripts\python.exe -m pip install --upgrade pip
if errorlevel 1 (pause & exit /b 1)
.venv\Scripts\python.exe -m pip install -r requirements-guard.txt
if errorlevel 1 (pause & exit /b 1)
echo Pronto. Ambiente criado em .venv; os GGUF existentes serao usados diretamente.
pause
