; Mothr's Windows installer (Inno Setup 6). Built by .github/workflows/release.yml through
; apps/desktop/windows/build-installer.ps1, which passes:
;   /DAppVersion=<x.y.z>   the release version
;   /DSourceDir=<dir>      the unpacked release folder (mothr.exe, LICENSE.txt, licenses\)
;
; Per user, no admin rights: installs to %LOCALAPPDATA%\Programs\Mothr, adds a Start menu entry
; and an Apps & features entry. install.json next to mothr.exe marks the copy as one that updates
; itself; Mothr then swaps in new versions on its own (crates/mothr-update/src/install.rs,
; Layout::InPlace), so this setup only runs to install, reinstall or repair.
; Chats and settings live in %APPDATA%\mothr and stay when Mothr is uninstalled.

#ifndef AppVersion
  #error Pass the version: /DAppVersion=x.y.z
#endif
#ifndef SourceDir
  #error Pass the release folder: /DSourceDir=<dir>
#endif

[Setup]
; Never change AppId: it's how Windows and the updater (WINDOWS_UNINSTALL_KEY in
; crates/mothr-update/src/install.rs) find the installed copy.
AppId={{0E591738-8A65-45C8-8C9B-61E1FE0ECC9E}
AppName=Mothr
AppVersion={#AppVersion}
AppVerName=Mothr {#AppVersion}
AppPublisher=Mothr contributors
AppPublisherURL=https://github.com/evolvedmesh/mothr
AppSupportURL=https://github.com/evolvedmesh/mothr/issues
AppUpdatesURL=https://github.com/evolvedmesh/mothr/releases
VersionInfoVersion={#AppVersion}
; Per user: {autopf} is %LOCALAPPDATA%\Programs without admin rights.
PrivilegesRequired=lowest
DefaultDirName={autopf}\Mothr
DisableProgramGroupPage=yes
UsePreviousAppDir=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; Windows 10 1903: GPUI's DirectWrite and ICU needs.
MinVersion=10.0.18362
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
OutputBaseFilename=mothr_{#AppVersion}_windows_x64_setup
UninstallDisplayName=Mothr
UninstallDisplayIcon={app}\mothr.exe
; Reinstalling while Mothr runs: ask to close it, and start it again afterwards.
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\Mothr"; Filename: "{app}\mothr.exe"; WorkingDir: "{app}"
Name: "{autodesktop}\Mothr"; Filename: "{app}\mothr.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\mothr.exe"; Description: "{cm:LaunchProgram,Mothr}"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
; What Mothr's own updates leave next to mothr.exe (the setup doesn't know these files).
Type: files; Name: "{app}\install.json"
Type: files; Name: "{app}\.install.json.new"
Type: files; Name: "{app}\*.old"
Type: filesandordirs; Name: "{app}\downloads"
Type: filesandordirs; Name: "{app}\.staging-*"
Type: dirifempty; Name: "{app}\licenses"
Type: dirifempty; Name: "{app}"

[Code]
procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
    // Marks a copy that updates itself; Mothr reads the version from it.
    if not SaveStringToFile(ExpandConstant('{app}\install.json'),
        '{"installer":"setup","version":"{#AppVersion}"}' + #13#10, False) then
      MsgBox('Couldn''t write install.json; Mothr won''t update itself.', mbError, MB_OK);
end;
