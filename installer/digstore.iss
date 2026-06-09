; Inno Setup script for the digstore CLI Windows installer.
;
; Built in CI on a version tag (see .github/workflows/release.yml):
;   ISCC.exe /DAppVersion=<x.y.z> installer\digstore.iss
;
; Installs digstore.exe per-user (no admin / UAC needed) and adds the install
; directory to the user's PATH, so `digstore` works from any terminal the way
; `git` does. The CLI itself operates on the current working directory (it
; discovers the nearest `.dig` walking up from where it is run), so no working
; directory is baked into the install.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#define MyAppName "digstore"
#define MyAppPublisher "DIG Network"
#define MyAppExe "digstore.exe"

[Setup]
; A stable AppId keeps upgrades/uninstall coherent across versions.
AppId={{8E5C9E4A-6D2B-4E3F-9A1C-D5F0C2A7B913}
AppName={#MyAppName}
AppVersion={#AppVersion}
AppPublisher={#MyAppPublisher}
VersionInfoVersion={#AppVersion}
; Per-user install: no administrator rights required.
PrivilegesRequired=lowest
DefaultDirName={localappdata}\Programs\Digstore
DefaultGroupName=digstore
DisableProgramGroupPage=yes
; PATH is modified, so Windows is told the environment changed.
ChangesEnvironment=yes
OutputDir=Output
OutputBaseFilename=digstore-{#AppVersion}-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesInstallIn64BitMode=x64compatible
UninstallDisplayName={#MyAppName} {#AppVersion}

[Files]
; The release binary produced by `cargo build -p digstore-cli --release`.
Source: "..\target\release\{#MyAppExe}"; DestDir: "{app}"; Flags: ignoreversion

[Registry]
; Append the install dir to the user's PATH (only if not already present).
Root: HKCU; Subkey: "Environment"; ValueType: expandsz; ValueName: "Path"; \
  ValueData: "{olddata};{app}"; Check: NeedsAddPath(ExpandConstant('{app}'))

[Code]
function NeedsAddPath(Param: string): Boolean;
var
  OrigPath: string;
begin
  if not RegQueryStringValue(HKEY_CURRENT_USER, 'Environment', 'Path', OrigPath) then
  begin
    Result := True;
    exit;
  end;
  // Match the exact directory bounded by ';' on both sides.
  Result := Pos(';' + Uppercase(Param) + ';', ';' + Uppercase(OrigPath) + ';') = 0;
end;

procedure RemoveFromPath(DirToRemove: string);
var
  OrigPath, Padded: string;
begin
  if not RegQueryStringValue(HKEY_CURRENT_USER, 'Environment', 'Path', OrigPath) then
    exit;
  Padded := ';' + OrigPath + ';';
  StringChangeEx(Padded, ';' + DirToRemove + ';', ';', True);
  // Strip the surrounding sentinels we added.
  if (Length(Padded) > 0) and (Padded[1] = ';') then
    Delete(Padded, 1, 1);
  if (Length(Padded) > 0) and (Padded[Length(Padded)] = ';') then
    Delete(Padded, Length(Padded), 1);
  RegWriteExpandStringValue(HKEY_CURRENT_USER, 'Environment', 'Path', Padded);
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
    RemoveFromPath(ExpandConstant('{app}'));
end;
