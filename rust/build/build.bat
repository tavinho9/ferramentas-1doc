@echo off
rem Compila a versão Rust e o instalador neste PC (alternativa ao GitHub Actions).
rem Precisa do Rust (https://rustup.rs). O Inno Setup é instalado pelo winget se faltar.
setlocal
cd /d "%~dp0.."

echo === Testes ===
cargo test || goto erro

echo.
echo === Compilando ===
cargo build --release || goto erro
for /f "tokens=3 delims= " %%v in ('findstr /b /c:"version = " Cargo.toml') do (set VERSAO=%%~v& goto achou)
:achou

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
echo Pronto: dist\Ferramentas1Doc-Rust-Setup-%VERSAO%.exe
pause
exit /b 0

:erro
echo.
echo *** A compilacao falhou; veja as mensagens acima. ***
pause
exit /b 1
