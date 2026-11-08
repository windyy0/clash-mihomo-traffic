[Setup]
AppId={{ACF55E08-17D4-4F65-B135-FB96A4CEB535}
AppName=Mihomo Traffic
AppVersion={#AppVersion}
DefaultDirName={localappdata}\Programs\MihomoTraffic
DisableDirPage=auto
AppendDefaultDirName=no
DisableProgramGroupPage=yes
LanguageDetectionMethod=uilanguage
ShowLanguageDialog=no
UsePreviousLanguage=no
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
AppMutex=MihomoTrafficDesktop
SetupIconFile=..\assets\traffic.ico
UninstallDisplayIcon={app}\mihomo-traffic.exe
OutputBaseFilename=MihomoTraffic-{#AppVersion}-windows-x64-setup
Compression=lzma2
SolidCompression=yes

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
; Official translations from https://github.com/jrsoftware/issrc/tree/is-7_1_1/Files/Languages
Name: "zhcn"; MessagesFile: "installer-languages\ChineseSimplified.isl"
Name: "zhtw"; MessagesFile: "installer-languages\ChineseTraditional.isl"

[Files]
Source: "{#PayloadDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs

[Icons]
Name: "{userprograms}\Mihomo Traffic"; Filename: "{app}\mihomo-traffic.exe"; WorkingDir: "{app}"

[Run]
Filename: "{app}\mihomo-traffic.exe"; Description: "{cm:LaunchProgram,Mihomo Traffic}"; Flags: nowait postinstall skipifsilent

[Code]
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Command, Prefix: String;
begin
  if CurUninstallStep = usPostUninstall then
  begin
    Prefix := '"' + ExpandConstant('{app}\mihomo-traffic.exe') + '"';
    if RegQueryStringValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run',
      'MihomoTraffic', Command) and
      ((CompareText(Command, Prefix) = 0) or
       (CompareText(Copy(Command, 1, Length(Prefix) + 1), Prefix + ' ') = 0)) then
      RegDeleteValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run', 'MihomoTraffic');
  end;
end;
