; Inno Setup script for Audio Paint. Build it with installer\build_installer.ps1,
; which compiles the release executable first and passes the version from Cargo.toml.

#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif
#ifndef SourceExe
  #define SourceExe "..\target-dist\release\audio_paint.exe"
#endif

[Setup]
; The AppId identifies the app for upgrades and uninstall: never change it.
AppId={{348941AD-F705-4218-BC38-04C5E4EF00C8}
AppName=Audio Paint
AppVersion={#AppVersion}
AppVerName=Audio Paint {#AppVersion}
AppPublisher=mdab9999
VersionInfoVersion={#AppVersion}
VersionInfoDescription=Audio Paint Setup
DefaultDirName={autopf}\Audio Paint
DefaultGroupName=Audio Paint
DisableProgramGroupPage=yes
; Installs for the current user without admin rights; the user can choose all users instead.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\dist
OutputBaseFilename=AudioPaint-{#AppVersion}-setup
SetupIconFile=..\assets\icon\audio_paint.ico
UninstallDisplayIcon={app}\audio_paint.exe
UninstallDisplayName=Audio Paint
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ChangesAssociations=yes
CloseApplications=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "german"; MessagesFile: "compiler:Languages\German.isl"

[CustomMessages]
english.OpenWith=Add "Open with Audio Paint" for audio files (MP3, WAV, FLAC, OGG, M4A, AAC)
german.OpenWith=„Öffnen mit Audio Paint“ für Audiodateien hinzufügen (MP3, WAV, FLAC, OGG, M4A, AAC)
english.FileTypes=File types:
german.FileTypes=Dateitypen:

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "openwith"; Description: "{cm:OpenWith}"; GroupDescription: "{cm:FileTypes}"

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Audio Paint"; Filename: "{app}\audio_paint.exe"
Name: "{autodesktop}\Audio Paint"; Filename: "{app}\audio_paint.exe"; Tasks: desktopicon

[Registry]
; Registers the program for the "Open with" menu without taking over the default player.
Root: HKA; Subkey: "Software\Classes\Applications\audio_paint.exe"; ValueType: string; ValueName: "FriendlyAppName"; ValueData: "Audio Paint"; Flags: uninsdeletekey; Tasks: openwith
Root: HKA; Subkey: "Software\Classes\Applications\audio_paint.exe\DefaultIcon"; ValueType: string; ValueData: "{app}\audio_paint.exe,0"; Tasks: openwith
Root: HKA; Subkey: "Software\Classes\Applications\audio_paint.exe\shell\open\command"; ValueType: string; ValueData: """{app}\audio_paint.exe"" ""%1"""; Tasks: openwith
Root: HKA; Subkey: "Software\Classes\Applications\audio_paint.exe\SupportedTypes"; ValueType: string; ValueName: ".mp3"; ValueData: ""; Tasks: openwith
Root: HKA; Subkey: "Software\Classes\Applications\audio_paint.exe\SupportedTypes"; ValueType: string; ValueName: ".wav"; ValueData: ""; Tasks: openwith
Root: HKA; Subkey: "Software\Classes\Applications\audio_paint.exe\SupportedTypes"; ValueType: string; ValueName: ".flac"; ValueData: ""; Tasks: openwith
Root: HKA; Subkey: "Software\Classes\Applications\audio_paint.exe\SupportedTypes"; ValueType: string; ValueName: ".ogg"; ValueData: ""; Tasks: openwith
Root: HKA; Subkey: "Software\Classes\Applications\audio_paint.exe\SupportedTypes"; ValueType: string; ValueName: ".m4a"; ValueData: ""; Tasks: openwith
Root: HKA; Subkey: "Software\Classes\Applications\audio_paint.exe\SupportedTypes"; ValueType: string; ValueName: ".aac"; ValueData: ""; Tasks: openwith
Root: HKA; Subkey: "Software\Classes\.mp3\OpenWithList\audio_paint.exe"; Flags: uninsdeletekey; Tasks: openwith
Root: HKA; Subkey: "Software\Classes\.wav\OpenWithList\audio_paint.exe"; Flags: uninsdeletekey; Tasks: openwith
Root: HKA; Subkey: "Software\Classes\.flac\OpenWithList\audio_paint.exe"; Flags: uninsdeletekey; Tasks: openwith
Root: HKA; Subkey: "Software\Classes\.ogg\OpenWithList\audio_paint.exe"; Flags: uninsdeletekey; Tasks: openwith
Root: HKA; Subkey: "Software\Classes\.m4a\OpenWithList\audio_paint.exe"; Flags: uninsdeletekey; Tasks: openwith
Root: HKA; Subkey: "Software\Classes\.aac\OpenWithList\audio_paint.exe"; Flags: uninsdeletekey; Tasks: openwith

[Run]
Filename: "{app}\audio_paint.exe"; Description: "{cm:LaunchProgram,Audio Paint}"; Flags: nowait postinstall skipifsilent
