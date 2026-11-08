[Setup]
AppId={{ACF55E08-17D4-4F65-B135-FB96A4CEB535}
AppName=Mihomo Traffic
AppVersion={#AppVersion}
DefaultDirName={localappdata}\Programs\MihomoTraffic
DisableDirPage=no
AppendDefaultDirName=no
DisableProgramGroupPage=yes
LanguageDetectionMethod=uilanguage
ShowLanguageDialog=no
UsePreviousLanguage=no
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
SetupIconFile=..\assets\traffic.ico
UninstallDisplayName=Mihomo Traffic
UninstallDisplayIcon={app}\mihomo-traffic.exe
OutputBaseFilename=MihomoTraffic-{#AppVersion}-windows-x64-setup
Compression=lzma2
SolidCompression=yes

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
; Official translations from https://github.com/jrsoftware/issrc/tree/is-7_1_1/Files/Languages
Name: "zhcn"; MessagesFile: "installer-languages\ChineseSimplified.isl"
Name: "zhtw"; MessagesFile: "installer-languages\ChineseTraditional.isl"

[CustomMessages]
en.ConfirmMove=Install to a different folder?%n%nCurrent: %1%nNew: %2%n%nSetup will uninstall the old program first. Your settings and traffic history will be kept.
zhcn.ConfirmMove=要更换安装目录吗？%n%n原目录：%1%n新目录：%2%n%n安装程序会先卸载旧程序，保留设置和流量账本。
zhtw.ConfirmMove=要更換安裝資料夾嗎？%n%n原資料夾：%1%n新資料夾：%2%n%n安裝程式會先解除安裝舊程式，保留設定和流量帳本。
en.SilentMoveNeedsConsent=Changing the installation folder in silent mode requires /MOVEINSTALL=1.
zhcn.SilentMoveNeedsConsent=静默安装时更换目录需要指定 /MOVEINSTALL=1。
zhtw.SilentMoveNeedsConsent=靜默安裝時更換資料夾需要指定 /MOVEINSTALL=1。
en.StopFailed=Mihomo Traffic did not exit in time. Close it from the system tray and try again.
zhcn.StopFailed=Mihomo Traffic 未能及时退出。请从系统托盘退出后重试。
zhtw.StopFailed=Mihomo Traffic 未能及時結束。請從系統匣結束後再試。
en.UninstallFailed=Setup could not uninstall the previous version. Uninstall it in Windows Settings and try again.
zhcn.UninstallFailed=无法完成旧版卸载。请在 Windows 设置中卸载后重试。
zhtw.UninstallFailed=無法完成舊版解除安裝。請在 Windows 設定中解除安裝後再試。
en.PortableFolder=This folder contains the portable edition. Choose another folder to install Mihomo Traffic.
zhcn.PortableFolder=这个目录包含便携版，请为安装版选择另一个目录。
zhtw.PortableFolder=此資料夾包含可攜版，請為安裝版選擇其他資料夾。
en.UninstallOptions=Uninstall Mihomo Traffic
zhcn.UninstallOptions=卸载 Mihomo Traffic
zhtw.UninstallOptions=解除安裝 Mihomo Traffic
en.UninstallQuestion=Do you want to uninstall Mihomo Traffic?
zhcn.UninstallQuestion=确定卸载 Mihomo Traffic 吗？
zhtw.UninstallQuestion=確定解除安裝 Mihomo Traffic 嗎？
en.KeepPersonalData=Keep settings and traffic history
zhcn.KeepPersonalData=保留设置和流量账本
zhtw.KeepPersonalData=保留設定和流量帳本
en.UninstallDataHint=Clear the checkbox to remove settings, traffic history and logs from the current and default data folders. Backups and exports will be kept.
zhcn.UninstallDataHint=取消勾选会删除当前和默认数据目录中的设置、流量账本和日志。备份与导出文件会保留。
zhtw.UninstallDataHint=取消勾選會刪除目前和預設資料夾中的設定、流量帳本和記錄。備份與匯出檔案會保留。
en.UninstallButton=Uninstall
zhcn.UninstallButton=卸载
zhtw.UninstallButton=解除安裝
en.UninstallStartFailed=Could not start the uninstaller. Try uninstalling Mihomo Traffic from Windows Settings.
zhcn.UninstallStartFailed=无法继续卸载。请从 Windows 设置中卸载 Mihomo Traffic。
zhtw.UninstallStartFailed=無法繼續解除安裝。請從 Windows 設定中解除安裝 Mihomo Traffic。
en.DataRemovalFailed=Could not finish removing settings and traffic history. Retry after checking that the data folder is accessible and unused.
zhcn.DataRemovalFailed=未能完整清除设置和流量账本。请确认数据目录可访问且未被占用后重试。
zhtw.DataRemovalFailed=未能完整清除設定和流量帳本。請確認資料夾可存取且未被占用後再試。

[Files]
Source: "{#PayloadDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs

[Icons]
Name: "{userprograms}\Mihomo Traffic"; Filename: "{app}\mihomo-traffic.exe"; WorkingDir: "{app}"

[Run]
Filename: "{app}\mihomo-traffic.exe"; Description: "{cm:LaunchProgram,Mihomo Traffic}"; Flags: nowait postinstall skipifsilent

[Code]
const
  AppMutexName = 'Local\MihomoTrafficDesktop';
  RunKey = 'Software\Microsoft\Windows\CurrentVersion\Run';
  RunValue = 'MihomoTraffic';

var
  PreviousAppDir, AutorunAfterInstall: String;
  RemovePersonalData: Boolean;

function FindAppWindow(const ClassName, WindowName: String): HWND;
  external 'FindWindowW@user32.dll stdcall';

function CurrentProcessId: Cardinal;
  external 'GetCurrentProcessId@kernel32.dll stdcall';

function StopRunningApp: Boolean;
var
  Attempt: Integer;
  ShellWindow: HWND;
  ExitRequested: Boolean;
begin
  ExitRequested := False;
  for Attempt := 1 to 400 do
  begin
    if not CheckForMutexes(AppMutexName) then
      Break;
    if not ExitRequested then
    begin
      ShellWindow := FindAppWindow('MihomoTrafficShell', 'Mihomo Traffic Shell');
      { The same WM_COMMAND used by mihomo-traffic.exe --exit. }
      if ShellWindow <> 0 then
        ExitRequested := PostMessage(ShellWindow, $0111, 4, 0);
    end;
    Sleep(100);
  end;
  Result := not CheckForMutexes(AppMutexName);
end;

function UninstallKey: String;
begin
  Result := 'Software\Microsoft\Windows\CurrentVersion\Uninstall\' +
    ExpandConstant('{#SetupSetting("AppId")}') + '_is1';
end;

function RegisteredUninstaller: String;
var
  Command: String;
  EndQuote: Integer;
begin
  Result := '';
  if not RegQueryStringValue(HKCU, UninstallKey, 'UninstallString', Command) then
    Exit;
  Command := Trim(Command);
  if Copy(Command, 1, 1) <> '"' then
    Result := Command
  else
  begin
    Delete(Command, 1, 1);
    EndQuote := Pos('"', Command);
    if EndQuote > 1 then
      Result := Copy(Command, 1, EndQuote - 1);
  end;
end;

procedure InitializeWizard;
begin
  if not RegQueryStringValue(HKCU, UninstallKey, 'Inno Setup: App Path', PreviousAppDir) then
    RegQueryStringValue(HKCU, UninstallKey, 'InstallLocation', PreviousAppDir);
end;

function SameDirectory(const First, Second: String): Boolean;
begin
  Result := SameText(RemoveBackslashUnlessRoot(ExpandFileName(First)),
    RemoveBackslashUnlessRoot(ExpandFileName(Second)));
end;

function ChangingDirectory: Boolean;
begin
  Result := (PreviousAppDir <> '') and not SameDirectory(PreviousAppDir, WizardDirValue);
end;

function NextButtonClick(CurPageID: Integer): Boolean;
begin
  Result := True;
  if (CurPageID = wpSelectDir) and
    FileExists(AddBackslash(WizardDirValue) + 'portable.flag') then
  begin
    SuppressibleMsgBox(CustomMessage('PortableFolder'), mbError, MB_OK, IDOK);
    Result := False;
  end
  else if (CurPageID = wpReady) and not WizardSilent and ChangingDirectory then
    Result := MsgBox(FmtMessage(CustomMessage('ConfirmMove'), [PreviousAppDir, WizardDirValue]),
      mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES;
end;

function AutorunPrefix(const AppDir: String): String;
begin
  Result := '"' + AddBackslash(AppDir) + 'mihomo-traffic.exe"';
end;

function ReadAutorunArguments(const AppDir: String; var Arguments: String): Boolean;
var
  Command, Prefix: String;
begin
  Result := False;
  if not RegQueryStringValue(HKCU, RunKey, RunValue, Command) then
    Exit;
  Prefix := AutorunPrefix(AppDir);
  if SameText(Command, Prefix) or
    SameText(Copy(Command, 1, Length(Prefix) + 1), Prefix + ' ') then
  begin
    Arguments := Copy(Command, Length(Prefix) + 1, Length(Command));
    Result := True;
  end;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Uninstaller, Arguments: String;
  ExitCode, Attempt: Integer;
begin
  Result := '';
  if WizardSilent and ChangingDirectory and (ExpandConstant('{param:MOVEINSTALL|0}') <> '1') then
  begin
    Result := CustomMessage('SilentMoveNeedsConsent');
    Exit;
  end;
  if not StopRunningApp then
  begin
    Result := CustomMessage('StopFailed');
    Exit;
  end;
  if not ChangingDirectory then
    Exit;

  Result := CustomMessage('UninstallFailed');
  Uninstaller := RegisteredUninstaller;
  if Uninstaller = '' then
    Exit;
  if not SameDirectory(ExtractFileDir(Uninstaller), PreviousAppDir) or
    not WildcardMatch(ExtractFileName(Uninstaller), 'unins???.exe') or
    not FileExists(Uninstaller) or not FileExists(ChangeFileExt(Uninstaller, '.dat')) then
    Exit;

  if ReadAutorunArguments(PreviousAppDir, Arguments) then
    AutorunAfterInstall := AutorunPrefix(WizardDirValue) + Arguments;
  if not Exec(Uninstaller, '/VERYSILENT /SUPPRESSMSGBOXES /NORESTART',
    PreviousAppDir, SW_HIDE, ewWaitUntilTerminated, ExitCode) then
    Exit;
  if ExitCode <> 0 then
    Exit;

  { The uninstaller's temporary process removes its EXE and runs the
    old startup cleanup after the original process has exited. }
  for Attempt := 1 to 50 do
  begin
    if not FileExists(Uninstaller) and ((AutorunAfterInstall = '') or
      not ReadAutorunArguments(PreviousAppDir, Arguments)) then
      Break;
    Sleep(100);
  end;
  if FileExists(Uninstaller) or RegKeyExists(HKCU, UninstallKey) or
    FileExists(AddBackslash(PreviousAppDir) + 'mihomo-traffic.exe') then
    Exit;

  Result := '';
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  CurrentCommand, Arguments, Uninstaller: String;
begin
  if CurStep <> ssPostInstall then
    Exit;

  { Use the custom confirmation in Windows Settings. QuietUninstallString
    keeps Inno's ordinary unattended behavior. }
  Uninstaller := RegisteredUninstaller;
  if Uninstaller <> '' then
    if not RegWriteStringValue(HKCU, UninstallKey, 'UninstallString',
      '"' + Uninstaller + '" /SILENT /MIHOMOOPTIONS=1') then
      Log('Could not register the custom uninstall confirmation.');

  if AutorunAfterInstall <> '' then
  begin
    if RegQueryStringValue(HKCU, RunKey, RunValue, CurrentCommand) and
      not ReadAutorunArguments(PreviousAppDir, Arguments) then
      Exit;
    if not RegWriteStringValue(HKCU, RunKey, RunValue, AutorunAfterInstall) then
      Log('Could not restore the Mihomo Traffic startup entry.');
  end;
end;

function ConfirmUninstallOptions: Boolean;
var
  Form: TSetupForm;
  Content: TPanel;
  KeepData: TNewCheckBox;
  Question, HintText: TNewStaticText;
  UninstallButton, CancelButton: TNewButton;
begin
#if Ver >= EncodeVer(6, 6, 0, 0)
  Form := CreateCustomForm(380, 130, True, True);
#else
  Form := CreateCustomForm;
#endif
  try
    Form.Caption := CustomMessage('UninstallOptions');
    Form.BorderStyle := bsDialog;
    Form.Position := poScreenCenter;
    Form.ClientWidth := ScaleX(380);

    Content := TPanel.Create(Form);
    Content.Parent := Form;
    Content.Align := alTop;
    Content.BevelOuter := bvNone;
    Content.ParentBackground := False;
    Content.Color := clWindow;
    Content.Font.Color := clWindowText;

    Question := TNewStaticText.Create(Form);
    Question.Parent := Content;
    Question.AutoSize := False;
    Question.Color := clWindow;
    Question.WordWrap := True;
    Question.SetBounds(ScaleX(16), ScaleY(16), Form.ClientWidth - ScaleX(32), 0);
    Question.Caption := CustomMessage('UninstallQuestion');
    Question.Font.Style := [fsBold];
    Question.AdjustHeight;

    KeepData := TNewCheckBox.Create(Form);
    KeepData.Parent := Content;
    KeepData.SetBounds(Question.Left, Question.Top + Question.Height + ScaleY(9),
      Question.Width, ScaleY(20));
    KeepData.Caption := CustomMessage('KeepPersonalData');
    KeepData.Checked := True;

    HintText := TNewStaticText.Create(Form);
    HintText.Parent := Content;
    HintText.AutoSize := False;
    HintText.Color := clWindow;
    HintText.WordWrap := True;
    HintText.SetBounds(Question.Left, KeepData.Top + KeepData.Height + ScaleY(6), Question.Width, 0);
    HintText.Caption := CustomMessage('UninstallDataHint');
    HintText.AdjustHeight;

    Content.Height := HintText.Top + HintText.Height + ScaleY(16);
    Form.ClientHeight := Content.Height + ScaleY(48);

    UninstallButton := TNewButton.Create(Form);
    UninstallButton.Parent := Form;
    UninstallButton.SetBounds(Form.ClientWidth - ScaleX(188), Content.Height + ScaleY(11),
      ScaleX(82), ScaleY(26));
    UninstallButton.Caption := CustomMessage('UninstallButton');
    UninstallButton.ModalResult := mrOK;

    CancelButton := TNewButton.Create(Form);
    CancelButton.Parent := Form;
    CancelButton.SetBounds(Form.ClientWidth - ScaleX(98), UninstallButton.Top,
      UninstallButton.Width, UninstallButton.Height);
    CancelButton.Caption := SetupMessage(msgButtonCancel);
    CancelButton.ModalResult := mrCancel;
    CancelButton.Default := True;
    CancelButton.Cancel := True;

    Form.ActiveControl := KeepData;
    Result := Form.ShowModal = mrOK;
    RemovePersonalData := Result and not KeepData.Checked;
  finally
    Form.Free;
  end;
end;

function InitializeUninstall: Boolean;
var
  Choice: String;
  ExitCode: Integer;
begin
  if UninstallSilent then
  begin
    if ExpandConstant('{param:MIHOMOOPTIONS|0}') = '1' then
      Result := ConfirmUninstallOptions
    else
    begin
      RemovePersonalData := ExpandConstant('{param:MIHOMODELETEDATA|0}') = '1';
      Result := True;
    end;
    Exit;
  end;

  { Direct unins000.exe launches show the same dialog. Continue with a
    silent uninstaller after this instance releases its uninstall log. }
  Result := False;
  if not ConfirmUninstallOptions then
    Exit;
  Choice := 'keep';
  if RemovePersonalData then
    Choice := 'remove';
  if not Exec(ExpandConstant('{app}\mihomo-traffic.exe'),
    '--continue-uninstall ' + IntToStr(CurrentProcessId) + ' "' +
    ExtractFileName(ExpandConstant('{uninstallexe}')) + '" ' + Choice,
    ExpandConstant('{app}'), SW_HIDE, ewNoWait, ExitCode) then
    MsgBox(CustomMessage('UninstallStartFailed'), mbError, MB_OK);
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Arguments: String;
  ExitCode: Integer;
begin
  case CurUninstallStep of
    usUninstall:
      begin
        if not StopRunningApp then
          RaiseException(CustomMessage('StopFailed'));
        if RemovePersonalData then
          if not Exec(ExpandConstant('{app}\mihomo-traffic.exe'), '--purge-personal-data',
            ExpandConstant('{app}'), SW_HIDE, ewWaitUntilTerminated, ExitCode) or (ExitCode <> 0) then
            RaiseException(CustomMessage('DataRemovalFailed'));
      end;
    usPostUninstall:
      if ReadAutorunArguments(ExpandConstant('{app}'), Arguments) then
        RegDeleteValue(HKCU, RunKey, RunValue);
  end;
end;
