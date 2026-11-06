; Instalador do Ferramentas 1Doc (Inno Setup 6)
;   iscc /DMyAppVersion=1.0.0 build\installer.iss
; Instala para o usuário atual, sem pedir administrador:
;   %LOCALAPPDATA%\Programs\Ferramentas 1Doc
; Sessão, caches e configurações ficam em %LOCALAPPDATA%\Ferramentas1Doc e
; são mantidos ao atualizar ou desinstalar.

#define MyAppName "Ferramentas 1Doc"
#ifndef MyAppVersion
  #define MyAppVersion "1.0.0"
#endif
#define MyAppExe "Ferramentas1Doc.exe"

[Setup]
AppId={{8F3A2C61-5B7E-4D2A-9C1F-1D0C20260001}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} {#MyAppVersion}
AppPublisher=Uso interno — Prefeitura de Montes Claros
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
DisableDirPage=auto
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
OutputDir=..\dist
OutputBaseFilename=Ferramentas1Doc-Setup-{#MyAppVersion}
SetupIconFile=..\ferramentas1doc\gui\icone.ico
UninstallDisplayIcon={app}\{#MyAppExe}
UninstallDisplayName={#MyAppName}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64
ArchitecturesInstallIn64BitMode=x64
CloseApplications=yes

[Languages]
Name: "brazilianportuguese"; MessagesFile: "compiler:Languages\BrazilianPortuguese.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"

[InstallDelete]
; bibliotecas da versão anterior (uma atualização não deve misturar as duas)
Type: filesandordirs; Name: "{app}\_internal"

[Files]
Source: "..\dist\Ferramentas1Doc\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\{#MyAppName}"; Filename: "{app}\{#MyAppExe}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExe}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExe}"; Description: "{cm:LaunchProgram,{#MyAppName}}"; Flags: nowait postinstall skipifsilent
