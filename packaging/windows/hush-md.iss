#ifndef AppVersion
  #define AppVersion "0.1.2"
#endif
#ifndef AppSource
  #define AppSource "..\..\target\release"
#endif

[Setup]
AppId={{59DC98EE-A136-40C9-A7BD-EB0CC50E37C2}
AppName=hush.md
AppVersion={#AppVersion}
AppPublisher=hush.md
AppPublisherURL=https://github.com/dxer/hush-md
DefaultDirName={localappdata}\Programs\hush.md
DisableDirPage=no
DisableProgramGroupPage=yes
DisableWelcomePage=yes
DisableReadyPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.19041
OutputDir=..\..\dist\windows
OutputBaseFilename=hush-md-{#AppVersion}-windows-x64-setup
SetupIconFile=..\..\assets\branding\app-icon.ico
UninstallDisplayIcon={app}\hush_md.exe
WizardStyle=modern light
WizardSmallImageFile=wizard-logo.bmp
WizardSmallImageBackColor=$FFFFFF
WizardImageStretch=no
WizardSizePercent=105,105
Compression=lzma2
SolidCompression=yes
CloseApplications=yes
RestartApplications=no
SetupLogging=yes
#ifdef Signed
SignTool=hushmd
SignedUninstaller=yes
SignedUninstallerDir={#AppSource}
#endif

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Messages]
SelectDirLabel3=Choose an install folder. Your notes are stored separately.
SelectTasksLabel2=Make hush.md easier to reach.
FinishedLabel=hush.md is ready. A quiet place for your notes.

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked

[Files]
Source: "{#AppSource}\hush_md.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#AppSource}\vcruntime140.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"
Source: "..\..\THIRD_PARTY.md"; DestDir: "{app}"

[Icons]
Name: "{userprograms}\hush.md"; Filename: "{app}\hush_md.exe"; WorkingDir: "{app}"
Name: "{userdesktop}\hush.md"; Filename: "{app}\hush_md.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\hush_md.exe"; Description: "Launch hush.md"; Flags: nowait postinstall skipifsilent

[Code]
procedure InitializeWizard;
begin
  WizardForm.Caption := 'hush.md';
end;
