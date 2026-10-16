; EraSwitch - Inno Setup script
;
; Built by the release workflow with
;   ISCC.exe /DAppVersion=1.0.0 installer\eraswitch.iss
; Run `cargo build --release` (or the app's build) first so
; ..\target\release\EraSwitch.exe exists.
;
; Output: installer\output\eraswitch-<version>-setup-x64.exe

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

; Folder holding the built exe, relative to this script. CI passes
; /DBinDir=..\target\x86_64-pc-windows-msvc\release when it builds with --target.
#ifndef BinDir
  #define BinDir "..\target\release"
#endif

; VersionInfoVersion must be plain numeric (x.x.x[.x]); strip any prerelease
; suffix like "-rc1" so prerelease tags still compile.
#if Pos("-", AppVersion) > 0
  #define NumericVersion Copy(AppVersion, 1, Pos("-", AppVersion) - 1)
#else
  #define NumericVersion AppVersion
#endif

#define AppName      "EraSwitch"
#define AppPublisher "Londopy"
#define AppURL       "https://github.com/Londopy/EraSwitch"
#define AppExeName   "EraSwitch.exe"

[Setup]
; Unique GUID - do NOT reuse this for a different app.
; The doubled opening brace is Inno's escape for a literal "{".
AppId={{F8EB4DCC-7338-406C-A828-F005CB414C85}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppPublisher}
AppPublisherURL={#AppURL}
AppSupportURL={#AppURL}/issues
AppUpdatesURL={#AppURL}/releases

; Per-user install, no UAC prompt. The dialog still lets the user pick an
; all-users install if they want one.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog

; With lowest privileges {autopf} resolves to %LOCALAPPDATA%\Programs.
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
AllowNoIcons=yes

LicenseFile=..\LICENSE

OutputDir=output
OutputBaseFilename=eraswitch-{#AppVersion}-setup-x64
SetupIconFile=..\assets\icon.ico

; Compression
Compression=lzma2/ultra64
SolidCompression=yes

; Windows 10+ only (64-bit)
MinVersion=10.0
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

WizardStyle=modern

; Restart Manager closes a running copy before its files are replaced.
CloseApplications=yes
CloseApplicationsFilter=*.exe
RestartApplications=no

UninstallDisplayIcon={app}\{#AppExeName}
UninstallDisplayName={#AppName} {#AppVersion}
VersionInfoVersion={#NumericVersion}
VersionInfoDescription={#AppName} {#AppVersion} setup
VersionInfoProductName={#AppName}
VersionInfoCompany={#AppPublisher}

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; \
  Description: "{cm:CreateDesktopIcon}"; \
  GroupDescription: "{cm:AdditionalIcons}"
; Windows startup entry - unchecked by default; every extra integration is opt-in
Name: "startup"; \
  Description: "Launch {#AppName} at &Windows startup"; \
  GroupDescription: "Startup:"; \
  Flags: unchecked
; OPTIONAL (CLI apps): add {app} to PATH. Uncomment this task and the
; EnvAddPath/EnvRemovePath block in [Code], and set ChangesEnvironment=yes.
;Name: "addtopath"; \
;  Description: "Add {#AppName} to the PATH"; \
;  GroupDescription: "Environment:"

[Files]
; Main binary - built before running iscc
Source: "{#BinDir}\{#AppExeName}"; \
  DestDir: "{app}"; \
  Flags: ignoreversion

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExeName}"
Name: "{group}\{cm:UninstallProgram,{#AppName}}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#AppName}"; \
  Filename: "{app}\{#AppExeName}"; \
  Tasks: desktopicon

[Registry]
; HKCU Run entry - written only when the "startup" task is checked,
; removed on uninstall (uninsdeletevalue).
Root: HKCU; \
  Subkey: "SOFTWARE\Microsoft\Windows\CurrentVersion\Run"; \
  ValueType: string; \
  ValueName: "{#AppName}"; \
  ValueData: """{app}\{#AppExeName}"""; \
  Flags: uninsdeletevalue; \
  Tasks: startup

[Run]
; Offer to launch after install (user can uncheck)
Filename: "{app}\{#AppExeName}"; \
  Description: "{cm:LaunchProgram,{#StringChange(AppName, '&', '&&')}}"; \
  Flags: nowait postinstall skipifsilent

[UninstallRun]
; put Windows 11 back if the era was on: the taskkill above stops EraSwitch
; without letting it restore, and the snapshot it restores from lives in
; {userappdata}, which [UninstallDelete] removes right after
Filename: "{app}\{#AppExeName}"; \
  Parameters: "--restore"; \
  Flags: runhidden waituntilterminated; \
  RunOnceId: "RestoreWindows"
; .londo files: let the app drop its registration and hand the .londo
; double-click to another installed Londopy app (see londo-files.md).
; Runs before the files are removed. Delete if the app has no .londo files.
Filename: "{app}\{#AppExeName}"; \
  Parameters: "--londo-unregister"; \
  Flags: runhidden waituntilterminated; \
  RunOnceId: "LondoUnregister"

[UninstallDelete]
; Clean up config and logs left in AppData
Type: filesandordirs; Name: "{userappdata}\{#AppName}"

[Code]
// Kill the running instance before install/uninstall so the exe isn't locked
// (a tray app often survives the Restart Manager prompt).
procedure TerminateApp();
var
  ResultCode: Integer;
begin
  Exec('taskkill.exe', '/F /IM {#AppExeName}', '', SW_HIDE,
       ewWaitUntilTerminated, ResultCode);
  Sleep(500);
end;

// OPTIONAL PATH handling (from nexium): append or remove {app} in the user's
// or the machine's PATH, depending on the install mode chosen in the wizard.
// Uncomment together with the "addtopath" task.
//
// function EnvKey(): string;
// begin
//   if IsAdminInstallMode then
//     Result := 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment'
//   else
//     Result := 'Environment';
// end;
//
// function EnvRoot(): Integer;
// begin
//   if IsAdminInstallMode then
//     Result := HKEY_LOCAL_MACHINE
//   else
//     Result := HKEY_CURRENT_USER;
// end;
//
// procedure EnvAddPath(Dir: string);
// var
//   Paths: string;
// begin
//   if not RegQueryStringValue(EnvRoot(), EnvKey(), 'Path', Paths) then
//     Paths := '';
//   if Pos(';' + Uppercase(Dir) + ';', ';' + Uppercase(Paths) + ';') > 0 then
//     exit;
//   if (Paths <> '') and (Paths[Length(Paths)] <> ';') then
//     Paths := Paths + ';';
//   Paths := Paths + Dir;
//   RegWriteExpandStringValue(EnvRoot(), EnvKey(), 'Path', Paths);
// end;
//
// procedure EnvRemovePath(Dir: string);
// var
//   Paths: string;
//   P: Integer;
// begin
//   if not RegQueryStringValue(EnvRoot(), EnvKey(), 'Path', Paths) then
//     exit;
//   P := Pos(';' + Uppercase(Dir) + ';', ';' + Uppercase(Paths) + ';');
//   if P = 0 then
//     exit;
//   Delete(Paths, P - 1, Length(Dir) + 1);
//   RegWriteExpandStringValue(EnvRoot(), EnvKey(), 'Path', Paths);
// end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssInstall then
    TerminateApp();
  // OPTIONAL PATH: if (CurStep = ssPostInstall) and WizardIsTaskSelected('addtopath') then
  //   EnvAddPath(ExpandConstant('{app}'));
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
    TerminateApp();
  // OPTIONAL PATH: if CurUninstallStep = usPostUninstall then
  //   EnvRemovePath(ExpandConstant('{app}'));
end;
