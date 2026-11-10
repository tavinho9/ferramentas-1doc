; Instalador do Ferramentas 1Doc, versão Rust (Inno Setup 6)
;   iscc /DMyAppVersion=1.1.0 rust\build\installer.iss
; Instala para o usuário atual, sem pedir administrador:
;   %LOCALAPPDATA%\Programs\Ferramentas 1Doc Rust
; Fica ao lado da versão Python (outro AppId, outra pasta, outro atalho).
; Sessão, caches e configurações são os mesmos da versão Python, em
; %LOCALAPPDATA%\Ferramentas1Doc, e são mantidos ao atualizar ou desinstalar.

#define MyAppName "Ferramentas 1Doc (Rust)"
#ifndef MyAppVersion
  #define MyAppVersion "1.1.0"
#endif
#define MyAppExe "Ferramentas1Doc.exe"

[Setup]
AppId={{3C5E9B27-7A41-4F0E-B6D2-1D0C20260002}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} {#MyAppVersion}
AppPublisher=Uso interno — Prefeitura de Montes Claros
DefaultDirName={autopf}\Ferramentas 1Doc Rust
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
DisableDirPage=auto
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
OutputDir=..\..\dist
OutputBaseFilename=Ferramentas1Doc-Rust-Setup-{#MyAppVersion}
SetupIconFile=..\..\ferramentas1doc\gui\icone.ico
UninstallDisplayIcon={app}\{#MyAppExe}
UninstallDisplayName={#MyAppName}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
CloseApplications=yes

[Languages]
Name: "brazilianportuguese"; MessagesFile: "compiler:Languages\BrazilianPortuguese.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "..\target\release\Ferramentas1Doc.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\target\release\ferramentas1doc-cli.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#MyAppName}"; Filename: "{app}\{#MyAppExe}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExe}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExe}"; Description: "{cm:LaunchProgram,{#MyAppName}}"; Flags: nowait postinstall skipifsilent
