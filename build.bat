@echo off
rem Compila o instalador neste PC (alternativa ao GitHub Actions).
rem Precisa de Python 3.10+ instalado. O Inno Setup é instalado pelo winget se faltar.
setlocal
cd /d "%~dp0"

if not exist .venv (
  py -3 -m venv .venv 2>nul || python -m venv .venv || goto erro
)
call .venv\Scripts\activate.bat || goto erro
python -m pip install --upgrade pip
pip install -r requirements.txt -r requirements-build.txt || goto erro

echo.
echo === Testes ===
python -m pytest -q tests || goto erro

echo.
echo === Empacotando ===
pyinstaller --noconfirm --clean build\Ferramentas1Doc.spec --distpath dist --workpath build\tmp || goto erro
for /f %%v in ('python -c "import ferramentas1doc; print(ferramentas1doc.VERSAO)"') do set VERSAO=%%v

set "ISCC=%ProgramFiles(x86)%\Inno Setup 6\ISCC.exe"
if not exist "%ISCC%" set "ISCC=%LOCALAPPDATA%\Programs\Inno Setup 6\ISCC.exe"
if not exist "%ISCC%" (
  echo Inno Setup nao encontrado; instalando pelo winget...
  winget install -e --id JRSoftware.InnoSetup --scope user --accept-package-agreements --accept-source-agreements
)
if not exist "%ISCC%" set "ISCC=%ProgramFiles(x86)%\Inno Setup 6\ISCC.exe"
if not exist "%ISCC%" (
  echo Instale o Inno Setup 6 ^(https://jrsoftware.org/isdl.php^) e rode de novo.
  goto erro
)
"%ISCC%" /DMyAppVersion=%VERSAO% build\installer.iss || goto erro

echo.
echo Pronto: dist\Ferramentas1Doc-Setup-%VERSAO%.exe
pause
exit /b 0

:erro
echo.
echo *** A compilacao falhou; veja as mensagens acima. ***
pause
exit /b 1
