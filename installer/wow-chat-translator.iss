; WoW Chat Translator - per-user installer (Inno Setup 6).
; Build with installer\build.ps1, or directly:
;   ISCC.exe installer\wow-chat-translator.iss
; Optional defines (ISCC /D...):
;   SourceExe     path of the release exe to package
;   AppMutexName  single-instance mutex the app creates (native/src/app/instance.rs)
;   RunValueName  HKCU Run value name of "Iniciar con Windows" (overridden only in tests)

#define AppName "WoW Chat Translator"
#define AppVersion "0.2.1"
#define AppPublisher "Jorge Luis Bustamante Jara"
#define AppExeName "wow-chat-translator.exe"
#ifndef SourceExe
  #define SourceExe "..\native\target\release\wow-chat-translator.exe"
#endif
#ifndef AppMutexName
  #define AppMutexName "Local\WoWChatTranslatorNative"
#endif
; Same value name the app writes for "Iniciar con Windows" (native/src/app/autostart.rs).
#ifndef RunValueName
  #define RunValueName "WoW Chat Translator"
#endif
; User config folder (native/src/app/win.rs). Kept by default on uninstall.
#define ConfigDirName "wow-chat-translator-native"

[Setup]
AppId={{6FE0FF5C-CEFC-4E1C-A0D1-93EABDD9A45A}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppPublisher}
AppCopyright=Copyright (c) 2026 {#AppPublisher}
VersionInfoVersion={#AppVersion}
VersionInfoProductName={#AppName}
VersionInfoCompany={#AppPublisher}
PrivilegesRequired=lowest
DefaultDirName={localappdata}\Programs\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
AllowNoIcons=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
SetupIconFile=..\native\assets\icon.ico
UninstallDisplayIcon={app}\{#AppExeName}
UninstallDisplayName={#AppName}
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
OutputDir=..\dist
OutputBaseFilename=WoW-Chat-Translator-Setup-{#AppVersion}
; The running instance is closed by [Code] below; never restart it afterwards.
CloseApplications=yes
RestartApplications=no
; Spanish is listed first: it is used unless the Windows UI language is English.
LanguageDetectionMethod=uilanguage
ShowLanguageDialog=auto

[Languages]
Name: "spanish"; MessagesFile: "compiler:Languages\Spanish.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
spanish.LaunchApp=Abrir {#AppName}
english.LaunchApp=Open {#AppName}
spanish.AppRunning={#AppName} está abierto. ¿Cerrarlo y continuar?
english.AppRunning={#AppName} is running. Close it and continue?
spanish.AppStillRunning=No se pudo cerrar {#AppName}. Ciérralo desde el icono de la bandeja y vuelve a intentarlo.
english.AppStillRunning=Could not close {#AppName}. Quit it from its tray icon and try again.
spanish.DeleteConfig=¿Borrar también tu configuración (%1)?%n%nSi vas a reinstalar, elige No para conservarla.
english.DeleteConfig=Also delete your settings (%1)?%n%nChoose No to keep them if you plan to reinstall.

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; DestName: "{#AppExeName}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion
Source: "..\THIRD_PARTY_NOTICES.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExeName}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExeName}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExeName}"; Description: "{cm:LaunchApp}"; Flags: postinstall nowait skipifsilent

[Code]
const
  RunKey = 'Software\Microsoft\Windows\CurrentVersion\Run';

function AppIsRunning(): Boolean;
begin
  Result := CheckForMutexes('{#AppMutexName}');
end;

{ Polls the single-instance mutex for up to Tries * 250 ms. }
function WaitForAppExit(Tries: Integer): Boolean;
var
  I: Integer;
begin
  for I := 1 to Tries do
  begin
    if not AppIsRunning() then
    begin
      Result := True;
      Exit;
    end;
    Sleep(250);
  end;
  Result := not AppIsRunning();
end;

{ Closes the running instance: a polite taskkill first, then a forced one. }
function CloseRunningApp(): Boolean;
var
  Code: Integer;
begin
  Result := True;
  if not AppIsRunning() then
    Exit;
  if SuppressibleMsgBox(CustomMessage('AppRunning'), mbConfirmation, MB_YESNO, IDYES) <> IDYES then
  begin
    Result := False;
    Exit;
  end;
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/IM {#AppExeName}', '', SW_HIDE, ewWaitUntilTerminated, Code);
  if not WaitForAppExit(12) then
    Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM {#AppExeName}', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Result := WaitForAppExit(20);
  if not Result then
    SuppressibleMsgBox(CustomMessage('AppStillRunning'), mbError, MB_OK, IDOK);
end;

function InitializeSetup(): Boolean;
begin
  Result := CloseRunningApp();
end;

function InitializeUninstall(): Boolean;
begin
  Result := CloseRunningApp();
end;

{ Removes the "Iniciar con Windows" value only when it launches this install. }
procedure RemoveAutostartIfOurs();
var
  Value, Exe: String;
begin
  if not RegQueryStringValue(HKCU, RunKey, '{#RunValueName}', Value) then
    Exit;
  Exe := Lowercase(ExpandConstant('{app}\{#AppExeName}'));
  if Pos(Exe, Lowercase(Value)) > 0 then
  begin
    if RegDeleteValue(HKCU, RunKey, '{#RunValueName}') then
      Log('Removed autostart value: ' + Value);
  end
  else
    Log('Autostart value kept (another install): ' + Value);
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  ConfigDir: String;
begin
  if CurUninstallStep = usUninstall then
    RemoveAutostartIfOurs();
  if CurUninstallStep = usPostUninstall then
  begin
    { The WoW addon is never touched. Settings are kept unless the user asks. }
    ConfigDir := ExpandConstant('{userappdata}\{#ConfigDirName}');
    if DirExists(ConfigDir) then
      if SuppressibleMsgBox(FmtMessage(CustomMessage('DeleteConfig'), [ConfigDir]),
           mbConfirmation, MB_YESNO or MB_DEFBUTTON2, IDNO) = IDYES then
        DelTree(ConfigDir, True, True, True);
  end;
end;
