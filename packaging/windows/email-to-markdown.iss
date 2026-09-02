#define AppName "Email to Markdown"
#define AppVersion "0.16.0"
#define AppPublisher "FX Guillois"
#define AppExeName "email-to-markdown.exe"

[Setup]
AppId={{8BC2AB85-9C79-4D83-A0AE-237D321B5EF6}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher={#AppPublisher}
DefaultDirName={localappdata}\Programs\Email to Markdown
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\..\target\installer
OutputBaseFilename=email-to-markdown-{#AppVersion}-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
Uninstallable=yes
CreateUninstallRegKey=yes
UninstallDisplayName={#AppName}
UninstallDisplayIcon={app}\email-to-markdown-mail.ico

[Files]
Source: "..\..\target\release\email-to-markdown.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\target\release\email-to-markdown-shell-extension-0.16.0.dll"; DestDir: "{app}"; Flags: ignoreversion restartreplace uninsrestartdelete
Source: "..\..\target\release\email-to-markdown-identity.msix"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\target\release\Assets\*"; DestDir: "{app}\Assets"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExeName}"; Parameters: "tray"; WorkingDir: "{app}"; IconFilename: "{app}\email-to-markdown-mail.ico"; Comment: "Lancer Email to Markdown dans la zone de notification"

[Run]
Filename: "{app}\{#AppExeName}"; Parameters: "shell install"; Flags: runhidden waituntilterminated; StatusMsg: "Intégration à l’Explorateur Windows…"
Filename: "{app}\{#AppExeName}"; Parameters: "tray"; Flags: runhidden nowait; StatusMsg: "Démarrage d’Email to Markdown…"

[UninstallRun]
Filename: "{app}\{#AppExeName}"; Parameters: "shell uninstall"; Flags: runhidden waituntilterminated; RunOnceId: "RemoveExplorerIntegration"

[UninstallDelete]
Type: files; Name: "{app}\email-to-markdown-mail.ico"
