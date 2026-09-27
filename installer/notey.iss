#define MyAppName "Notey"
#define MyAppVersion "0.3.0"
#define MyAppPublisher "Notey"
#define MyAppExeName "notey.exe"
; extensions offered in the Open With menu
; script types are deliberately excluded: an extra Open With ProgID leaves them
; without a default handler, so double-click prompts and "Run as administrator" breaks
#define Exts ".txt .log .md .markdown .ini .cfg .conf .json .xml .csv .tsv .yaml .yml .toml"
; registered by 0.2.0 and earlier; removed on install and uninstall
#define RetiredExts ".bat .cmd .ps1"

[Setup]
AppId={{8B1F1C5A-9C7D-4E2B-A1D3-6F0E2C9B7A41}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={autopf}\{#MyAppName}
PrivilegesRequired=lowest
DisableProgramGroupPage=yes
DisableDirPage=no
OutputDir={#SourcePath}\..\installer_out
OutputBaseFilename=NoteySetup
SetupIconFile={#SourcePath}\..\notey_icon.ico
UninstallDisplayIcon={app}\{#MyAppExeName}
UninstallDisplayName={#MyAppName}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ChangesAssociations=yes
ChangesEnvironment=yes

[Tasks]
Name: "openwith"; Description: "Register Notey in the ""Open with"" menu for text files"
Name: "contextmenu"; Description: "Add ""Open with Notey"" to the right-click menu for all files"
Name: "addtopath"; Description: "Add Notey to the user PATH"; Flags: checkedonce
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked

[Files]
Source: "{#SourcePath}\..\target\release\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{userprograms}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{userdesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Registry]
; ---- application registration (drives the "Open with" list) ----
Root: HKCU; Subkey: "Software\Classes\Applications\{#MyAppExeName}"; ValueType: string; ValueName: "FriendlyAppName"; ValueData: "{#MyAppName}"; Tasks: openwith; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\Applications\{#MyAppExeName}\DefaultIcon"; ValueType: string; ValueData: "{app}\{#MyAppExeName},0"; Tasks: openwith
Root: HKCU; Subkey: "Software\Classes\Applications\{#MyAppExeName}\shell\open\command"; ValueType: string; ValueData: """{app}\{#MyAppExeName}"" ""%1"""; Tasks: openwith

; ---- ProgID used by the per-extension Open With entries ----
Root: HKCU; Subkey: "Software\Classes\Notey.Document"; ValueType: string; ValueData: "Text Document"; Tasks: openwith; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\Notey.Document\DefaultIcon"; ValueType: string; ValueData: "{app}\{#MyAppExeName},0"; Tasks: openwith
Root: HKCU; Subkey: "Software\Classes\Notey.Document\shell\open\command"; ValueType: string; ValueData: """{app}\{#MyAppExeName}"" ""%1"""; Tasks: openwith

; ---- App Paths so "notey" works from Run / Open With browse ----
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\App Paths\{#MyAppExeName}"; ValueType: string; ValueData: "{app}\{#MyAppExeName}"; Tasks: openwith; Flags: uninsdeletekey

; ---- optional: classic context menu entries on every file ----
; opens as a tab in the running Notey window (or starts one)
Root: HKCU; Subkey: "Software\Classes\*\shell\NoteyOpen"; ValueType: string; ValueData: "Open with {#MyAppName}"; Tasks: contextmenu; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\*\shell\NoteyOpen"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#MyAppExeName},0"; Tasks: contextmenu
Root: HKCU; Subkey: "Software\Classes\*\shell\NoteyOpen\command"; ValueType: string; ValueData: """{app}\{#MyAppExeName}"" ""%1"""; Tasks: contextmenu
; always opens a separate window
Root: HKCU; Subkey: "Software\Classes\*\shell\NoteyOpenNew"; ValueType: string; ValueData: "Open in new {#MyAppName} window"; Tasks: contextmenu; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\*\shell\NoteyOpenNew"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#MyAppExeName},0"; Tasks: contextmenu
Root: HKCU; Subkey: "Software\Classes\*\shell\NoteyOpenNew\command"; ValueType: string; ValueData: """{app}\{#MyAppExeName}"" --new-window ""%1"""; Tasks: contextmenu

[Code]
const
  Extensions = '{#Exts}';
  RetiredExtensions = '{#RetiredExts}';
  InstallerStateKey = 'Software\Notey\Installer';

function NormalizePathEntry(Value: String): String;
begin
  Result := Lowercase(Trim(Value));
  if (Length(Result) >= 2) and (Result[1] = '"') and
     (Result[Length(Result)] = '"') then
  begin
    Delete(Result, Length(Result), 1);
    Delete(Result, 1, 1);
  end;
  while (Length(Result) > 3) and (Result[Length(Result)] = '\') do
    Delete(Result, Length(Result), 1);
end;

function PathContains(Entries, Directory: String): Boolean;
var
  Entry, Rest: String;
  P: Integer;
begin
  Result := False;
  Rest := Entries;
  while Rest <> '' do
  begin
    P := Pos(';', Rest);
    if P > 0 then
    begin
      Entry := Copy(Rest, 1, P - 1);
      Rest := Copy(Rest, P + 1, MaxInt);
    end
    else
    begin
      Entry := Rest;
      Rest := '';
    end;
    if NormalizePathEntry(Entry) = NormalizePathEntry(Directory) then
    begin
      Result := True;
      exit;
    end;
  end;
end;

procedure AddToUserPath();
var
  AppDir, UserPath: String;
begin
  if not WizardIsTaskSelected('addtopath') then
    exit;

  AppDir := ExpandConstant('{app}');
  if not RegQueryStringValue(HKCU, 'Environment', 'Path', UserPath) then
    UserPath := '';
  if PathContains(UserPath, AppDir) then
    exit;

  if (UserPath <> '') and (UserPath[Length(UserPath)] <> ';') then
    UserPath := UserPath + ';';
  if RegWriteExpandStringValue(HKCU, 'Environment', 'Path', UserPath + AppDir) then
    RegWriteDWordValue(HKCU, InstallerStateKey, 'AddedToUserPath', 1);
end;

procedure RemoveFromUserPath();
var
  AppDir, Entry, NewPath, UserPath: String;
  AddedToUserPath: Cardinal;
  P: Integer;
begin
  if not RegQueryDWordValue(HKCU, InstallerStateKey, 'AddedToUserPath',
     AddedToUserPath) or (AddedToUserPath <> 1) then
    exit;

  AppDir := ExpandConstant('{app}');
  if not RegQueryStringValue(HKCU, 'Environment', 'Path', UserPath) then
    exit;

  NewPath := '';
  while UserPath <> '' do
  begin
    P := Pos(';', UserPath);
    if P > 0 then
    begin
      Entry := Copy(UserPath, 1, P - 1);
      UserPath := Copy(UserPath, P + 1, MaxInt);
    end
    else
    begin
      Entry := UserPath;
      UserPath := '';
    end;

    if (Trim(Entry) <> '') and
       (NormalizePathEntry(Entry) <> NormalizePathEntry(AppDir)) then
    begin
      if NewPath <> '' then
        NewPath := NewPath + ';';
      NewPath := NewPath + Entry;
    end;
  end;

  RegWriteExpandStringValue(HKCU, 'Environment', 'Path', NewPath);
  RegDeleteValue(HKCU, InstallerStateKey, 'AddedToUserPath');
  RegDeleteKeyIfEmpty(HKCU, InstallerStateKey);
end;

procedure RegisterExtensions();
var
  Ext, Rest: String;
  P: Integer;
begin
  if not WizardIsTaskSelected('openwith') then
    exit;
  Rest := Extensions;
  while Rest <> '' do
  begin
    P := Pos(' ', Rest);
    if P > 0 then
    begin
      Ext := Copy(Rest, 1, P - 1);
      Rest := Copy(Rest, P + 1, MaxInt);
    end
    else
    begin
      Ext := Rest;
      Rest := '';
    end;
    if Ext <> '' then
    begin
      RegWriteStringValue(HKCU, 'Software\Classes\' + Ext + '\OpenWithProgids',
        'Notey.Document', '');
      RegWriteStringValue(HKCU,
        'Software\Classes\Applications\{#MyAppExeName}\SupportedTypes', Ext, '');
    end;
  end;
end;

procedure UnregisterExtensions(List: String);
var
  Ext, Rest: String;
  P: Integer;
begin
  Rest := List;
  while Rest <> '' do
  begin
    P := Pos(' ', Rest);
    if P > 0 then
    begin
      Ext := Copy(Rest, 1, P - 1);
      Rest := Copy(Rest, P + 1, MaxInt);
    end
    else
    begin
      Ext := Rest;
      Rest := '';
    end;
    if Ext <> '' then
    begin
      RegDeleteValue(HKCU, 'Software\Classes\' + Ext + '\OpenWithProgids',
        'Notey.Document');
      RegDeleteValue(HKCU,
        'Software\Classes\Applications\{#MyAppExeName}\SupportedTypes', Ext);
    end;
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
  begin
    UnregisterExtensions(RetiredExtensions);
    RegisterExtensions();
    AddToUserPath();
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
  begin
    UnregisterExtensions(Extensions);
    UnregisterExtensions(RetiredExtensions);
    RemoveFromUserPath();
  end;
end;

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "Launch {#MyAppName}"; Flags: nowait postinstall skipifsilent
