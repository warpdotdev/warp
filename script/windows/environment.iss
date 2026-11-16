[Code]
{ Adapted from https://stackoverflow.com/a/46609047 }

const
  SystemEnvironmentKey = 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment';
  UserEnvironmentKey = 'Environment';

{ Get the appropriate registry key and root based on install mode. }
procedure GetEnvironmentKeyInfo(var RootKey: Integer; var SubKey: string);
begin
  if IsAdminInstallMode then begin
    RootKey := HKEY_LOCAL_MACHINE;
    SubKey := SystemEnvironmentKey;
  end else begin
    RootKey := HKEY_CURRENT_USER;
    SubKey := UserEnvironmentKey;
  end;
end;

{ Add path to environment PATH variable. }
procedure EnvAddPath(Path: string);
var
    Paths: string;
    RootKey: Integer;
    SubKey: string;
begin
    { Get the appropriate registry location }
    GetEnvironmentKeyInfo(RootKey, SubKey);
    
    { Retrieve current path (use empty string if entry not exists) }
    if not RegQueryStringValue(RootKey, SubKey, 'Path', Paths)
    then Paths := '';

    { Skip if string already found in path }
    if Pos(';' + Uppercase(Path) + ';', ';' + Uppercase(Paths) + ';') > 0 then exit;

    { Append string to the end of the path variable }
    Paths := Paths + ';'+ Path +';'

    { Overwrite (or create if missing) path environment variable }
    if RegWriteStringValue(RootKey, SubKey, 'Path', Paths)
    then Log(Format('Added [%s] to PATH: [%s]', [Path, Paths]))
    else Log(Format('Error adding [%s] to PATH: [%s]', [Path, Paths]));
end;

{ Remove path from environment PATH variable. }
procedure EnvRemovePath(Path: string);
var
    Paths: string;
    P: Integer;
    RootKey: Integer;
    SubKey: string;
begin
    { Get the appropriate registry location }
    GetEnvironmentKeyInfo(RootKey, SubKey);
    
    { Skip if registry entry not exists }
    if not RegQueryStringValue(RootKey, SubKey, 'Path', Paths) then
        exit;

    { Skip if string not found in path }
    P := Pos(';' + Uppercase(Path) + ';', ';' + Uppercase(Paths) + ';');
    if P = 0 then exit;

    { Update path variable }
    Delete(Paths, P - 1, Length(Path) + 1);

    { Overwrite path environment variable }
    if RegWriteStringValue(RootKey, SubKey, 'Path', Paths)
    then Log(Format('Removed [%s] from PATH: [%s]', [Path, Paths]))
    else Log(Format('Error removing [%s] from PATH: [%s]', [Path, Paths]));
end;

