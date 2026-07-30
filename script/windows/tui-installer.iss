#include "environment.iss"

#define MyAppPublisher "Denver Technologies, Inc."
#define MyAppURL "https://www.warp.dev/"
#ifndef MyAppName
  #define MyAppName "WarpAgentCLIDev"
#endif
#ifndef MyAppVersion
  #define MyAppVersion "0.1.0"
#endif
#ifndef MyAppExeName
  #define MyAppExeName "warp-tui-dev.exe"
#endif
#ifndef ReleaseChannel
  #define ReleaseChannel "dev"
#endif
#ifndef CLIName
  #define CLIName "warp-dev"
#endif
#ifndef InstallDirName
  #define InstallDirName "tui-dev"
#endif
#ifndef TargetProfileDir
  #define TargetProfileDir "target\rclida"
#endif
#ifndef WindowsAssetsDir
  #define WindowsAssetsDir "..\..\app\assets\windows\x64"
#endif

#define ProductRegistryKey "SOFTWARE\Warp.dev\WarpAgentCLI\" + ReleaseChannel

[Setup]
AppId=warp-agent-cli-{#ReleaseChannel}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} {#MyAppVersion}
UninstallDisplayName={#MyAppName} {#MyAppVersion}
UninstallDisplayIcon={app}\icon.ico
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}
DefaultDirName={code:GetDefaultInstallDir}
UsePreviousAppDir=yes
ArchitecturesAllowed={#Arch}
ArchitecturesInstallIn64BitMode={#Arch}
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=commandline dialog
DisableDirPage=yes
DisableProgramGroupPage=yes
DisableReadyPage=yes
DisableFinishedPage=yes
OutputBaseFilename={#OutputName}
Compression=lzma
SolidCompression=yes
WizardStyle=modern
WizardSmallImageFile="installer-images\warp-logo.bmp"
WizardImageFile="installer-images\warp-banner.bmp"
SetupIconFile="..\..\app\channels\{#ReleaseChannel}\icon\no-padding\icon.ico"
CloseApplications=no
RestartApplications=no
SetupMutex=Local\WarpAgentCLI{#ReleaseChannel}Setup
MinVersion=10.0.18362
ChangesEnvironment=true
RedirectionGuard=no
#ifdef SIGN_TOOL
SignTool=codesign
SignedUninstaller=yes
#endif

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Files]
Source: "{#TargetProfileDir}\{#MyAppExeName}"; DestDir: "{app}\versions\{#MyAppVersion}"; Check: ShouldInstallVersion
Source: "{#WindowsAssetsDir}\conpty.dll"; DestDir: "{app}\versions\{#MyAppVersion}"; Check: ShouldInstallVersion
Source: "{#WindowsAssetsDir}\OpenConsole.exe"; DestDir: "{app}\versions\{#MyAppVersion}\{#Arch}"; Check: ShouldInstallVersion
Source: "{#WindowsAssetsDir}\vcruntime140.dll"; DestDir: "{app}\versions\{#MyAppVersion}"; Check: ShouldInstallVersion
Source: "{#WindowsAssetsDir}\vcruntime140_1.dll"; DestDir: "{app}\versions\{#MyAppVersion}"; Check: ShouldInstallVersion
Source: "{#WindowsAssetsDir}\msvcp140.dll"; DestDir: "{app}\versions\{#MyAppVersion}"; Check: ShouldInstallVersion
Source: "{#TargetProfileDir}\resources\*"; DestDir: "{app}\versions\{#MyAppVersion}\resources"; Flags: recursesubdirs createallsubdirs; Check: ShouldInstallVersion
Source: "..\..\app\channels\{#ReleaseChannel}\icon\no-padding\icon.ico"; DestDir: "{app}"; Flags: ignoreversion

[Registry]
Root: HKA; Subkey: "{#ProductRegistryKey}"; ValueType: string; ValueName: "InstallRoot"; ValueData: "{app}"; Flags: uninsdeletekey
Root: HKA; Subkey: "{#ProductRegistryKey}"; ValueType: string; ValueName: "BinDir"; ValueData: "{code:GetBinDir}"; Flags: uninsdeletevalue

[UninstallDelete]
Type: files; Name: "{app}\current"
Type: files; Name: "{app}\previous"
Type: filesandordirs; Name: "{app}\version-leases"
Type: filesandordirs; Name: "{app}\versions"

[Code]
const
  MoveFileReplaceExisting = $1;
  MoveFileWriteThrough = $8;

var
  InstallVersionFiles: Boolean;
  PreviousBinDir: string;
  UninstallBinDir: string;

function MoveFileEx(
  ExistingFileName: string;
  NewFileName: string;
  Flags: Cardinal
): Boolean;
  external 'MoveFileExW@kernel32.dll stdcall';

function GetRegistryRoot(): Integer;
begin
  if IsAdminInstallMode then
    Result := HKEY_LOCAL_MACHINE
  else
    Result := HKEY_CURRENT_USER;
end;

function GetDefaultInstallDir(Param: string): string;
begin
  if IsAdminInstallMode then
    Result := ExpandConstant('{commonappdata}\Warp\{#InstallDirName}')
  else
    Result := ExpandConstant('{localappdata}\Warp\{#InstallDirName}');
end;

function GetDefaultBinDir(): string;
begin
  if IsAdminInstallMode then
    Result := ExpandConstant('{commonappdata}\Warp\bin')
  else
    Result := ExpandConstant('{localappdata}\Warp\bin');
end;
function GetRegisteredBinDir(var BinDir: string): Boolean;
begin
  Result := RegQueryStringValue(
    GetRegistryRoot(),
    '{#ProductRegistryKey}',
    'BinDir',
    BinDir
  );
end;

function GetBinDir(Param: string): string;
var
  RequestedBinDir: string;
begin
  RequestedBinDir := ExpandConstant('{param:WARP_BIN_DIR|}');
  if RequestedBinDir <> '' then
  begin
    Result := RequestedBinDir;
    exit;
  end;
  if GetRegisteredBinDir(Result) then
    exit;
  Result := GetDefaultBinDir();
end;

function SkipPathUpdate(): Boolean;
var
  Value: string;
begin
  Value := Lowercase(ExpandConstant('{param:SKIP_PATH_UPDATE|false}'));
  Result := (Value = '1') or (Value = 'true');
end;

function AllowDowngrade(): Boolean;
var
  Value: string;
begin
  Value := Lowercase(ExpandConstant('{param:ALLOW_DOWNGRADE|false}'));
  Result := (Value = '1') or (Value = 'true');
end;

function VersionDir(RootDir: string; Version: string): string;
begin
  Result := AddBackslash(RootDir) + 'versions\' + Version;
end;

function IsSafeVersionComponent(Value: string): Boolean;
var
  Index: Integer;
  Character: Char;
begin
  Result := (Value <> '') and (Pos('..', Value) = 0);
  if not Result then
    exit;
  for Index := 1 to Length(Value) do
  begin
    Character := Value[Index];
    if not (
      ((Character >= 'a') and (Character <= 'z')) or
      ((Character >= 'A') and (Character <= 'Z')) or
      ((Character >= '0') and (Character <= '9')) or
      (Character = '.') or
      (Character = '-') or
      (Character = '_')
    ) then
    begin
      Result := False;
      exit;
    end;
  end;
end;

function IsCompleteVersionDir(Path: string): Boolean;
begin
  Result :=
    FileExists(AddBackslash(Path) + '{#MyAppExeName}') and
    FileExists(AddBackslash(Path) + 'conpty.dll') and
    FileExists(AddBackslash(Path) + '{#Arch}\OpenConsole.exe') and
    FileExists(AddBackslash(Path) + 'vcruntime140.dll') and
    FileExists(AddBackslash(Path) + 'vcruntime140_1.dll') and
    FileExists(AddBackslash(Path) + 'msvcp140.dll') and
    DirExists(AddBackslash(Path) + 'resources');
end;

function ReadPointer(Path: string; var Value: string): Boolean;
var
  AnsiValue: AnsiString;
begin
  Result := LoadStringFromFile(Path, AnsiValue);
  if Result then
  begin
    Value := Trim(String(AnsiValue));
    Result := IsSafeVersionComponent(Value);
  end;
end;

procedure WriteAtomicTextFile(Path: string; Value: string);
var
  TemporaryPath: string;
begin
  TemporaryPath := Path + '.new-' + IntToStr(Random(1000000));
  if not SaveStringToFile(TemporaryPath, Value, False) then
    RaiseException('Failed to stage ' + Path);
  if not MoveFileEx(
    TemporaryPath,
    Path,
    MoveFileReplaceExisting or MoveFileWriteThrough
  ) then
  begin
    DeleteFile(TemporaryPath);
    RaiseException(
      'Failed to activate ' + Path + ': ' + SysErrorMessage(DLLGetLastError())
    );
  end;
end;

function EscapeBatchValue(Value: string): string;
begin
  Result := Value;
  StringChangeEx(Result, '^', '^^', True);
  StringChangeEx(Result, '%', '%%', True);
end;

procedure WriteLauncher(BinDir: string);
var
  LauncherPath: string;
  LauncherContents: string;
  ManagedRoot: string;
begin
  if not ForceDirectories(BinDir) then
    RaiseException('Failed to create command directory ' + BinDir);
  ManagedRoot := EscapeBatchValue(ExpandConstant('{app}'));
  LauncherPath := AddBackslash(BinDir) + '{#CLIName}.cmd';
  LauncherContents :=
    '@echo off' + #13#10 +
    'setlocal' + #13#10 +
    'set "WARP_TUI_MANAGED_ROOT=' + ManagedRoot + '"' + #13#10 +
    'set /p WARP_TUI_ACTIVE_VERSION=<"%WARP_TUI_MANAGED_ROOT%\current"' + #13#10 +
    '"%WARP_TUI_MANAGED_ROOT%\versions\%WARP_TUI_ACTIVE_VERSION%\{#MyAppExeName}" %*' + #13#10;
  WriteAtomicTextFile(LauncherPath, LauncherContents);
end;

function ShouldInstallVersion(): Boolean;
begin
  Result := InstallVersionFiles;
end;
function InitializeSetup(): Boolean;
begin
  InstallVersionFiles := True;
  Result := True;
end;

function PrepareToInstall(var NeedsRestart: Boolean): string;
var
  CurrentVersion: string;
  TargetVersionDir: string;
begin
  if not GetRegisteredBinDir(PreviousBinDir) then
    PreviousBinDir := '';
  if ReadPointer(AddBackslash(WizardDirValue()) + 'current', CurrentVersion) and
    (CompareText(CurrentVersion, '{#MyAppVersion}') > 0) and
    not AllowDowngrade() then
  begin
    Result :=
      'Warp Agent CLI ' + CurrentVersion +
      ' is newer than {#MyAppVersion}. Pass /ALLOW_DOWNGRADE=1 to install an older version.';
    exit;
  end;

  TargetVersionDir := VersionDir(WizardDirValue(), '{#MyAppVersion}');
  InstallVersionFiles := not IsCompleteVersionDir(TargetVersionDir);
  if InstallVersionFiles and DirExists(TargetVersionDir) and
    not DelTree(TargetVersionDir, True, True, True) then
  begin
    Result := 'Failed to remove incomplete version directory ' + TargetVersionDir;
    exit;
  end;
  Result := '';
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  BinDir: string;
  CurrentVersion: string;
  CurrentVersionDir: string;
  LeaseDir: string;
begin
  if CurStep <> ssPostInstall then
    exit;

  if not IsCompleteVersionDir(VersionDir(ExpandConstant('{app}'), '{#MyAppVersion}')) then
    RaiseException('The installed Warp Agent CLI payload is incomplete');

  BinDir := GetBinDir('');
  if (PreviousBinDir <> '') and
    (CompareText(PreviousBinDir, BinDir) <> 0) then
  begin
    DeleteFile(AddBackslash(PreviousBinDir) + '{#CLIName}.cmd');
    EnvRemovePath(PreviousBinDir);
  end;

  WriteLauncher(BinDir);
  if not SkipPathUpdate() then
    EnvAddPath(BinDir);

  if ReadPointer(AddBackslash(ExpandConstant('{app}')) + 'current', CurrentVersion) and
    (CompareText(CurrentVersion, '{#MyAppVersion}') <> 0) then
  begin
    CurrentVersionDir := VersionDir(ExpandConstant('{app}'), CurrentVersion);
    if IsCompleteVersionDir(CurrentVersionDir) then
      WriteAtomicTextFile(
        AddBackslash(ExpandConstant('{app}')) + 'previous',
        CurrentVersion
      );
  end;

  LeaseDir := AddBackslash(ExpandConstant('{app}')) + 'version-leases';
  if not ForceDirectories(LeaseDir) then
    RaiseException('Failed to create version lease directory ' + LeaseDir);
  if not FileExists(AddBackslash(LeaseDir) + '{#MyAppVersion}.lock') then
    SaveStringToFile(AddBackslash(LeaseDir) + '{#MyAppVersion}.lock', '', False);
  WriteAtomicTextFile(
    AddBackslash(ExpandConstant('{app}')) + 'current',
    '{#MyAppVersion}'
  );
end;

function InitializeUninstall(): Boolean;
begin
  if not GetRegisteredBinDir(UninstallBinDir) then
    UninstallBinDir := '';
  Result := True;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep <> usPostUninstall then
    exit;
  if UninstallBinDir <> '' then
  begin
    DeleteFile(AddBackslash(UninstallBinDir) + '{#CLIName}.cmd');
    EnvRemovePath(UninstallBinDir);
    RemoveDir(UninstallBinDir);
  end;
end;
