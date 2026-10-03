; Build with ./build.sh windows or scripts/windows.ps1.
; SourceExe, AssetsDir and AppVersion come from that exact Cargo build.
#ifndef AppVersion
  #error AppVersion must be supplied by the build script
#endif

[Setup]
AppId=io.github.mny315.Carlitos
AppName=Carlitos
AppVersion={#AppVersion}
AppPublisher=mny315
AppPublisherURL=https://github.com/mny315/Carlitos
DefaultDirName={localappdata}\Programs\Carlitos
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
DisableProgramGroupPage=yes
DisableDirPage=auto
WizardStyle=modern
SetupIconFile={#AssetsDir}\carlitos.ico
UninstallDisplayName=Carlitos
UninstallDisplayIcon={app}\Carlitos.exe
OutputBaseFilename=Carlitos
Compression=lzma2
SolidCompression=yes
CloseApplications=yes
RestartApplications=no
SetupLogging=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "russian"; MessagesFile: "compiler:Languages\Russian.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; DestName: "Carlitos.exe"; Flags: ignoreversion
Source: "carlitos-installed"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#AssetsDir}\licenses.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Carlitos"; Filename: "{app}\Carlitos.exe"; WorkingDir: "{app}"; AppUserModelID: "io.github.mny315.Carlitos"
Name: "{autodesktop}\Carlitos"; Filename: "{app}\Carlitos.exe"; WorkingDir: "{app}"; AppUserModelID: "io.github.mny315.Carlitos"; Tasks: desktopicon

[Run]
Filename: "{app}\Carlitos.exe"; Description: "{cm:LaunchProgram,Carlitos}"; Flags: nowait postinstall skipifsilent

; No UninstallDelete: user data and audiobook files must survive uninstall.
