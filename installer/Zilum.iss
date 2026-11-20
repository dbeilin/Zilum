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
; Inno's own first dialog asks who it is for: me only, the default, with no administrator and
; into %LOCALAPPDATA%\Programs\Zilum, or all users, with one and into C:\Program Files\Zilum.
; An upgrade takes the install's own mode and asks nothing (UsePreviousPrivileges, left at yes).
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
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
; Start with Windows for everyone is ticket 07's; an all-users install offers only the icon.
Name: startwithwindows; Description: "Start with Windows"; Check: not IsAdminInstallMode

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
; stays the person's. An all-users install writes none of them: they are one person's.
Root: HKCU; Subkey: "Software\Zilum"; ValueType: string; ValueName: "Install"; ValueData: "{code:InstallStamp}"; Flags: uninsdeletekey; Check: ForMeAtFirstInstall
Root: HKCU; Subkey: "Software\Zilum"; ValueType: dword; ValueName: "StartWithWindows"; ValueData: 1; Tasks: startwithwindows; Check: ForMeAtFirstInstall
Root: HKCU; Subkey: "Software\Zilum"; ValueType: dword; ValueName: "StartWithWindows"; ValueData: 0; Tasks: not startwithwindows; Check: ForMeAtFirstInstall
; The start at sign-in, set or cleared here so the next sign-in is right before Zilum has run. It
; is the Run entry Zilum's Settings sets, naming this exe with the sign-in flag.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Zilum"; ValueData: """{app}\Zilum.exe"" --startup"; Tasks: startwithwindows; Check: ForMeAtFirstInstall
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "Zilum"; Flags: deletevalue; Tasks: not startwithwindows; Check: ForMeAtFirstInstall

[Run]
; As the person at the PC, never as the administrator an all-users install asked for.
Filename: "{app}\Zilum.exe"; Description: "Launch Zilum"; Flags: nowait postinstall runasoriginaluser

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
// that folder only while UsePreviousAppDir is left at yes, and from the entry of the mode
// UsePreviousPrivileges took, the person's or the machine's.
function AlreadyInstalled: Boolean;
begin
  Result := WizardForm.PrevAppDir <> '';
end;

// Whether the install writes the person's own record and sign-in start: a first install for me
// only, never an upgrade or an all-users install.
function ForMeAtFirstInstall: Boolean;
begin
  Result := not AlreadyInstalled and not IsAdminInstallMode;
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

const
  // The start at sign-in, as src\startup.rs writes it: the quoted exe, then the sign-in flag.
  RUN_KEY = 'Software\Microsoft\Windows\CurrentVersion\Run';
  RUN_NAME = 'Zilum';

var
  // Whether the person uninstalling ticked Also remove my settings.
  RemoveSettings: Boolean;

// Asks whether the settings go too, with one checkbox, unticked. Comes after Inno's own "Are you
// sure" prompt, which cannot be left out, so it offers no second way back. True when ticked.
function AskRemoveSettings: Boolean;
var
  Form: TSetupForm;
  Prompt: TNewStaticText;
  SettingsBox: TNewCheckBox;
  Uninstall: TNewButton;
begin
  Form := CreateCustomForm(ScaleX(380), ScaleY(130), False, True);
  try
    Form.Caption := 'Uninstall Zilum';
    Form.BorderIcons := [];

    Prompt := TNewStaticText.Create(Form);
    Prompt.Parent := Form;
    Prompt.Left := ScaleX(16);
    Prompt.Top := ScaleY(16);
    Prompt.Width := Form.ClientWidth - ScaleX(32);
    Prompt.AutoSize := False;
    Prompt.WordWrap := True;
    Prompt.Height := ScaleY(30);
    Prompt.Caption := 'Zilum will be removed from this PC. Your captures stay where they are.';

    SettingsBox := TNewCheckBox.Create(Form);
    SettingsBox.Parent := Form;
    SettingsBox.Left := ScaleX(16);
    SettingsBox.Top := ScaleY(52);
    SettingsBox.Width := Form.ClientWidth - ScaleX(32);
    SettingsBox.Height := ScaleY(17);
    SettingsBox.Caption := 'Also remove my settings';
    SettingsBox.Checked := False;

    Uninstall := TNewButton.Create(Form);
    Uninstall.Parent := Form;
    Uninstall.Width := ScaleX(75);
    Uninstall.Height := ScaleY(23);
    Uninstall.Left := Form.ClientWidth - ScaleX(16) - Uninstall.Width;
    Uninstall.Top := Form.ClientHeight - ScaleY(12) - Uninstall.Height;
    Uninstall.Caption := 'Uninstall';
    Uninstall.ModalResult := mrOk;
    Uninstall.Default := True;

    Form.ActiveControl := Uninstall;
    Form.ShowModal;
    Result := SettingsBox.Checked;
  finally
    Form.Free;
  end;
end;

// Runs once the person has said yes to Inno's prompt, before anything is removed. A silent
// uninstall asks nothing and keeps the settings.
procedure InitializeUninstallProgressForm;
begin
  RemoveSettings := not UninstallSilent and AskRemoveSettings;
end;

// A running Zilum is closed as at an install, and the sign-in start goes whether the installer or
// Zilum's Settings wrote it, when it names this install's exe; a Portable Zilum's start is left.
// The settings go once everything else has.
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Command: String;
begin
  if CurUninstallStep = usUninstall then
  begin
    CloseRunningZilum;
    if RegQueryStringValue(HKCU, RUN_KEY, RUN_NAME, Command)
      and (Pos(Lowercase(ExpandConstant('"{app}\Zilum.exe"')), Lowercase(Command)) = 1) then
      RegDeleteValue(HKCU, RUN_KEY, RUN_NAME);
  end
  else if (CurUninstallStep = usPostUninstall) and RemoveSettings then
    DelTree(ExpandConstant('{userappdata}\Zilum'), True, True, True);
end;
