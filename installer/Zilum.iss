; The installer, Zilum-Setup.exe, made from the release exe by Inno Setup 6. From the repo's root,
; after `cargo build --release`:
;
;   & "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe" installer\Zilum.iss
;
; It writes target\release\Zilum-Setup.exe beside the exe it carries.

#define ReleaseExe "..\target\release\Zilum.exe"
#if !FileExists(AddBackslash(SourcePath) + ReleaseExe)
  #error Build the release exe first: cargo build --release
#endif
; The version the exe itself carries, so the installer and Installed apps say the same.
#define AppVersion GetStringFileInfo(AddBackslash(SourcePath) + ReleaseExe, PRODUCT_VERSION)

[Setup]
; Names this install in Installed apps and to every later installer. Never change it.
AppId={{4D134D07-969E-4B64-9267-8461C277C777}
AppName=Zilum
AppVersion={#AppVersion}
VersionInfoVersion={#AppVersion}
; For the person running it, with no administrator: %LOCALAPPDATA%\Programs\Zilum.
PrivilegesRequired=lowest
DefaultDirName={autopf}\Zilum
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; The folder page, then the options page, whose button installs. An upgrade shows neither and
; installs into the install's own folder (see AlreadyInstalled).
DisableWelcomePage=yes
DisableDirPage=no
DisableProgramGroupPage=yes
DisableReadyPage=yes
ShowLanguageDialog=no
; A running Zilum is closed in [Code], as Quit closes it, never through Windows' page of
; applications in use.
CloseApplications=no
UninstallDisplayName=Zilum
UninstallDisplayIcon={app}\Zilum.exe
SetupIconFile=..\assets\icon.ico
WizardStyle=modern dynamic
; Zilum's icon in the corner of every page and on the last one, in Windows' light and dark
; modes alike, at its own size for the screen's scale rather than stretched.
WizardImageStretch=no
WizardSmallImageFile=icon-48.png,icon-64.png,icon-96.png
WizardSmallImageFileDynamicDark=icon-48.png,icon-64.png,icon-96.png
WizardImageFile=icon-128.png
WizardImageFileDynamicDark=icon-128.png
OutputDir=..\target\release
OutputBaseFilename=Zilum-Setup
SolidCompression=yes

[Messages]
; The options page's button installs, there being no Ready to Install page after it.
SelectTasksLabel2=Select the additional tasks you would like Setup to perform while installing [name], then click Install.

[Tasks]
Name: desktopicon; Description: "Create a desktop icon"; Flags: unchecked
Name: startwithwindows; Description: "Start with Windows"

[Files]
Source: "{#ReleaseExe}"; DestDir: "{app}"; Flags: ignoreversion

; Made at the first install only: an upgrade leaves the shortcuts as the person has them since.
[Icons]
Name: "{autoprograms}\Zilum"; Filename: "{app}\Zilum.exe"; Check: not AlreadyInstalled
Name: "{autodesktop}\Zilum"; Filename: "{app}\Zilum.exe"; Tasks: desktopicon; Check: not AlreadyInstalled

[Registry]
; Start with Windows as the options page left it, for Zilum to make the setting at its next start,
; once for this install (src\startup.rs reads these names). Install is new at each first install;
; an upgrade writes none of these rows, so Zilum finds the stamp it already took and the setting
; stays the person's.
Root: HKCU; Subkey: "Software\Zilum"; ValueType: string; ValueName: "Install"; ValueData: "{code:InstallStamp}"; Flags: uninsdeletekey; Check: not AlreadyInstalled
Root: HKCU; Subkey: "Software\Zilum"; ValueType: dword; ValueName: "StartWithWindows"; ValueData: 1; Tasks: startwithwindows; Check: not AlreadyInstalled
Root: HKCU; Subkey: "Software\Zilum"; ValueType: dword; ValueName: "StartWithWindows"; ValueData: 0; Tasks: not startwithwindows; Check: not AlreadyInstalled
; The start at sign-in, set or cleared here so the next sign-in is right before Zilum has run. It
; is the Run entry Zilum's Settings sets, naming this exe with the sign-in flag.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Zilum"; ValueData: """{app}\Zilum.exe"" --startup"; Tasks: startwithwindows; Check: not AlreadyInstalled
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "Zilum"; Flags: deletevalue; Tasks: not startwithwindows; Check: not AlreadyInstalled

[Run]
Filename: "{app}\Zilum.exe"; Description: "Launch Zilum"; Flags: nowait postinstall

[Code]
const
  WM_CLOSE = $0010;
  BM_CLICK = $00F5;
  SYNCHRONIZE = $00100000;
  // How long a running Zilum has to end once asked, as a newer Zilum gives it (CLOSE_MS in
  // src\instance.rs).
  CLOSE_MS = 5000;

function GetWindowThreadProcessId(Wnd: HWND; var ProcessId: DWORD): DWORD;
  external 'GetWindowThreadProcessId@user32.dll stdcall';
function OpenProcess(Access: DWORD; Inherit: BOOL; ProcessId: DWORD): THandle;
  external 'OpenProcess@kernel32.dll stdcall';
function WaitForSingleObject(Handle: THandle; Milliseconds: DWORD): DWORD;
  external 'WaitForSingleObject@kernel32.dll stdcall';
function CloseHandle(Handle: THandle): BOOL;
  external 'CloseHandle@kernel32.dll stdcall';

// Closes the Zilum running for this person, installed, portable or loose, the way a newer
// Zilum closes an older one: WM_CLOSE to its tray window. Waits for its process to end, so its
// exe can be replaced.
procedure CloseRunningZilum;
var
  Tray: HWND;
  ProcessId: DWORD;
  Process: THandle;
begin
  // The tray window's class, as src\main.rs registers it.
  Tray := FindWindowByClassName('Zilum.Tray');
  if Tray = 0 then
    Exit;
  GetWindowThreadProcessId(Tray, ProcessId);
  // Opened before asking, so the process waited on is the one asked.
  Process := OpenProcess(SYNCHRONIZE, False, ProcessId);
  PostMessage(Tray, WM_CLOSE, 0, 0);
  if Process <> 0 then
  begin
    WaitForSingleObject(Process, CLOSE_MS);
    CloseHandle(Process);
  end;
end;

// Whether Zilum is already installed here, by the install's own entry in Installed apps, whose
// folder Setup read at its start: a loose or portable exe is not an install. An upgrade, a repair
// or a going back alike asks nothing and changes none of the first install's choices. Setup reads
// that folder only while UsePreviousAppDir is left at yes.
function AlreadyInstalled: Boolean;
begin
  Result := WizardForm.PrevAppDir <> '';
end;

// An upgrade skips the folder and options pages.
function ShouldSkipPage(PageID: Integer): Boolean;
begin
  Result := AlreadyInstalled and ((PageID = wpSelectDir) or (PageID = wpSelectTasks));
end;

// This install's stamp: the time to the millisecond, so no two installs share one.
function InstallStamp(Param: String): String;
begin
  Result := GetDateTimeString('yyyymmddhhnnsszzz', #0, #0);
end;

// Names the options page's button Install, since the install starts from it. An upgrade, having
// skipped every page before it, lands on the Ready page, which Setup shows whenever no page came
// before it; the click on its Install is posted, so it is handled before the page is painted and
// the first page an upgrade shows is the installing one. A silent run moves on by itself.
procedure CurPageChanged(CurPageID: Integer);
begin
  if CurPageID = wpSelectTasks then
    WizardForm.NextButton.Caption := SetupMessage(msgButtonInstall)
  else if (CurPageID = wpReady) and AlreadyInstalled and not WizardSilent then
    PostMessage(WizardForm.NextButton.Handle, BM_CLICK, 0, 0);
end;

// Runs once Install is clicked, before anything is copied.
function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  CloseRunningZilum;
  Result := '';
end;
