; Dictaship installer — Inno Setup 6
; https://jrsoftware.org/isinfo.php
;
; Build steps:
;   1. cargo build --release
;   2. iscc installer.iss          (or: .\build-installer.ps1)
; Output: installer\DictashipSetup.exe

#define AppName "Dictaship"
; AppVersion is passed by build-installer.ps1 via /DAppVersion=<version>
#define AppExe  "dictaship.exe"

[Setup]
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher=Evgenii Grebeniuk
; User-level install: no UAC prompt, goes to %LocalAppData%
DefaultDirName={localappdata}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
OutputDir=installer
OutputBaseFilename=DictashipSetup
Compression=lzma2
SolidCompression=yes
PrivilegesRequired=lowest
WizardStyle=modern
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: startup; Description: "Start {#AppName} automatically when Windows starts"; Flags: checked

[Files]
; Binary: always overwritten so upgrades take effect immediately
Source: "target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
; Config: installed only on first install; never removed on uninstall (preserves user settings)
Source: "config.toml"; DestDir: "{app}"; Flags: onlyifdoesntexist uninsneveruninstall
; Third-party license attribution (Apache-2.0 distribution requirement)
Source: "THIRD_PARTY_LICENSES.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#AppName}";           Filename: "{app}\{#AppExe}"; WorkingDir: "{app}"
Name: "{group}\Uninstall {#AppName}"; Filename: "{uninstallexe}"

[Registry]
; WorkingDir is not stored in the registry for startup entries — wrap in quotes only
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "{#AppName}"; ValueData: """{app}\{#AppExe}"""; Tasks: startup; Flags: uninsdeletevalue

[Run]
; Offer to launch immediately after installation
Filename: "{app}\{#AppExe}"; Description: "Launch {#AppName} now"; WorkingDir: "{app}"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
; Remove the install directory only if it is empty (it won't be if config.toml remains)
Type: dirifempty; Name: "{app}"
