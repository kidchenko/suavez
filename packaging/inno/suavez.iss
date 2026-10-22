; Inno Setup script for Sua Vez.
;   iscc /DAppVersion=x.y.z packaging\inno\suavez.iss
; Expects target\release\suavez.exe to already be built (build.ps1 does both).

#define AppName      "Sua Vez"
#ifndef AppVersion
  #define AppVersion "0.0.1"
#endif
#define AppPublisher "kidchenko"
#define AppURL       "https://github.com/kidchenko/suavez"
#define AppExe       "suavez.exe"
; Data folder and Run value name, without the space. Must match Config::dir
; and VALUE_NAME in src/autostart.rs.
#define AppKey       "SuaVez"

[Setup]
; Never reuse this GUID for another product: it is how Windows matches
; upgrades and uninstalls, and winget's ProductCode is derived from it.
AppId={{43DA7080-FEAA-4DD8-AC4C-96375FD1420D}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppPublisher}
AppPublisherURL={#AppURL}
AppSupportURL={#AppURL}/issues
AppUpdatesURL={#AppURL}/releases
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
DisableDirPage=auto
LicenseFile=..\..\LICENSE
OutputDir=..\..\dist
OutputBaseFilename=SuaVez-Setup-{#AppVersion}
SetupIconFile=..\..\assets\icon.ico
UninstallDisplayIcon={app}\{#AppExe}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; The app itself never needs elevation; only Program Files does.
PrivilegesRequired=admin
; Close a running instance on upgrade/uninstall instead of hitting a locked exe.
CloseApplications=yes
CloseApplicationsFilter=*.exe
; [Tasks], [UninstallRun] and [Code] touch HKCU and the user's profile on
; purpose; see the notes there.
UsedUserAreasWarning=no
MinVersion=10.0

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
; Ticked by default: a tray light that is not running after a reboot tells you
; nothing. Same HKCU value the app's own "Run at login" toggle uses.
Name: "autostart"; Description: "Start {#AppName} when I sign in";   GroupDescription: "Startup:"

[Files]
Source: "..\..\target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
; The AppUserModelID is what lets notifications show as "Sua Vez" with its
; icon: Windows drops toasts for an id with no shortcut behind it. Must match
; APP_ID in src/notify.rs.
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExe}"; AppUserModelID: "kidchenko.SuaVez"
Name: "{group}\Uninstall {#AppName}"; Filename: "{uninstallexe}"

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run";   ValueType: string; ValueName: "{#AppKey}"; ValueData: """{app}\{#AppExe}""";   Tasks: autostart; Flags: uninsdeletevalue

[Run]
; As the signed-in user, not the elevated installer: the first launch connects
; to the agents through that user's own settings files.
Filename: "{app}\{#AppExe}"; Description: "Launch {#AppName}";   Flags: nowait postinstall skipifsilent runasoriginaluser

[UninstallRun]
; Take the hooks out of every agent's settings first. Left behind, every
; agent turn would try to run an exe that no longer exists.
Filename: "{app}\{#AppExe}"; Parameters: "uninstall";   Flags: runhidden waituntilterminated; RunOnceId: "UnhookSuaVez"
Filename: "{sys}\taskkill.exe"; Parameters: "/f /im {#AppExe}";   Flags: runhidden; RunOnceId: "KillSuaVez"

[UninstallDelete]
Type: files; Name: "{userappdata}\{#AppKey}\suavez.log"
Type: files; Name: "{userappdata}\{#AppKey}\suavez.log.1"
Type: filesandordirs; Name: "{userappdata}\{#AppKey}\sessions"

[Code]
// The app's own "Run at login" toggle writes the same HKCU value; clear it
// even if the installer task was never ticked. Under UAC an admin elevating
// their own account keeps the same HKCU, which is the normal case.
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
  begin
    RegDeleteValue(HKEY_CURRENT_USER,
      'Software\Microsoft\Windows\CurrentVersion\Run', '{#AppKey}');
    // Only succeeds once config.json is gone: settings survive a reinstall.
    RemoveDir(ExpandConstant('{userappdata}\{#AppKey}'));
    RemoveDir(ExpandConstant('{app}'));
  end;
end;
