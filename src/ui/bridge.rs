use super::{
    updates::{CheckAction, CheckSnapshot, ReleaseChecks},
    window::WindowState,
    *,
};
use crate::{
    analysis::AnalysisRequest,
    backend::{AnalysisReply, CalendarReply},
    history::CalendarRequest,
    hotkey::Hotkey,
    overview_usage, permanent_control,
    speed_stats::{SecondsReply, SecondsRequest},
    storage_management::Request as StorageRequest,
    traffic_drill::{HoverReply, HoverRequest},
};
use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    io::{self, BufRead, BufReader, BufWriter, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

#[derive(Serialize, Deserialize)]
enum Request {
    Start,
    Poll(u64),
    CheckUpdates(CheckAction, u64),
    Send(Command),
    Query(u64, Query),
    QueryResults(Option<u64>, Option<u64>),
    QueryAnalysis(AnalysisRequest),
    AnalysisResult(AnalysisRequest, bool),
    QueryCalendar(CalendarRequest),
    CalendarResult(CalendarRequest),
    QuerySeconds(SecondsRequest),
    TakeSeconds,
    QueryHover(HoverRequest),
    TakeHover,
    QueryUsage(overview_usage::Request),
    TakeUsage,
    RefreshSubscriptions,
    RefreshStorage,
    PreviewStorage(StorageRequest),
    SubmitStorage(StorageRequest),
    SaveConfig(Box<WireConfig>),
    BackupDirectory(Option<PathBuf>),
    PrepareMigration(PathBuf),
    ApplyProcessMode(crate::config::ProcessControl),
    RetryProcessControl,
    Permanent(permanent_control::Request),
    Notice(String),
    HotkeyRecording(bool),
    ApplyHotkey(u32, Hotkey),
    TaskbarRescan,
    SaveWindow(WindowState),
    Open,
    Exit,
}

#[derive(Serialize, Deserialize)]
struct Poll {
    config: WireConfig,
    state: Shared,
    analysis_generation: u64,
    process_generation: u64,
    update: InterfaceUpdate,
}

#[derive(Default, Serialize, Deserialize)]
pub(super) struct InterfaceUpdate {
    pub(super) events: Vec<UiEvent>,
    pub(super) checks: Option<CheckSnapshot>,
}

// The disk format excludes plaintext and protected credentials. Both belong
// to the settings snapshot exchanged over the inherited private pipe.
#[derive(Serialize, Deserialize)]
struct WireConfig {
    settings: Config,
    secret: String,
    locked_secret: Option<toml::Value>,
}

impl From<Config> for WireConfig {
    fn from(mut settings: Config) -> Self {
        Self {
            secret: std::mem::take(&mut settings.secret),
            locked_secret: settings.locked_secret.take(),
            settings,
        }
    }
}

impl From<WireConfig> for Config {
    fn from(wire: WireConfig) -> Self {
        Config {
            secret: wire.secret,
            locked_secret: wire.locked_secret,
            ..wire.settings
        }
    }
}

#[derive(Default)]
pub(super) struct Session {
    pub(super) sequence: u64,
    pub(super) quit: AtomicBool,
    events: Mutex<Vec<UiEvent>>,
    pub(super) resume: Mutex<Option<WindowState>>,
    pub(super) ready: AtomicBool,
}

impl Session {
    pub(super) fn new(resume: Option<WindowState>) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self {
            sequence: NEXT.fetch_add(1, Ordering::Relaxed) << 32,
            resume: Mutex::new(resume),
            ..Self::default()
        }
    }

    pub(super) fn take_events(&self) -> Vec<UiEvent> {
        std::mem::take(&mut *self.events.lock().unwrap())
    }

    pub(super) fn post(&self, event: UiEvent) {
        let mut events = self.events.lock().unwrap();
        if matches!(event, UiEvent::Open | UiEvent::OpenOverview | UiEvent::Quit)
            && events
                .iter()
                .any(|queued| std::mem::discriminant(queued) == std::mem::discriminant(&event))
        {
            return;
        }
        events.push(event);
    }

    pub(super) fn serve(
        self: &Arc<Self>,
        backend: Backend,
        updates: Arc<Mutex<ReleaseChecks>>,
        child: &mut Child,
    ) -> Result<()> {
        let input = child.stdout.take().context("缺少界面输出管道")?;
        let output = child.stdin.take().context("缺少界面输入管道")?;
        let session = self.clone();
        std::thread::Builder::new()
            .name("desktop-interface".into())
            .spawn(move || {
                if let Err(error) = session.run(&backend, &updates, input, output)
                    && !error.downcast_ref::<io::Error>().is_some_and(|error| {
                        matches!(
                            error.kind(),
                            io::ErrorKind::BrokenPipe | io::ErrorKind::UnexpectedEof
                        )
                    })
                {
                    tracing::warn!(%error, "Desktop interface connection stopped");
                }
            })?;
        Ok(())
    }

    fn poll(backend: &Backend, update: InterfaceUpdate) -> Poll {
        Poll {
            config: backend.config_snapshot().into(),
            state: backend.snapshot(),
            analysis_generation: backend.analysis_generation(),
            process_generation: backend.process_generation.load(Ordering::Relaxed),
            update,
        }
    }

    fn run(
        &self,
        backend: &Backend,
        updates: &Mutex<ReleaseChecks>,
        input: ChildStdout,
        output: ChildStdin,
    ) -> Result<()> {
        let mut input = BufReader::new(input);
        let mut out = BufWriter::new(output);
        while let Some(request) = receive(&mut input)? {
            match request {
                Request::Start => {
                    let resume = *self.resume.lock().unwrap();
                    let checks = updates.lock().unwrap().snapshot(0);
                    send(
                        &mut out,
                        &(
                            &backend.dir,
                            self.sequence,
                            resume,
                            Self::poll(
                                backend,
                                InterfaceUpdate {
                                    checks,
                                    ..Default::default()
                                },
                            ),
                        ),
                    )?;
                }
                Request::Poll(known) => {
                    let checks = {
                        let mut updates = updates.lock().unwrap();
                        updates.tick(None);
                        updates.snapshot(known)
                    };
                    let update = InterfaceUpdate {
                        events: self.take_events(),
                        checks,
                    };
                    send(&mut out, &Self::poll(backend, update))?;
                    self.ready.store(true, Ordering::Release);
                }
                Request::CheckUpdates(action, known) => {
                    let snapshot = updates.lock().unwrap().request(action, known);
                    send(&mut out, &snapshot)?;
                }
                Request::Send(command) => send(&mut out, &backend.send(command))?,
                Request::Query(id, query) => send(&mut out, &backend.query(id, query))?,
                Request::QueryResults(report, detail) => {
                    send(&mut out, &backend.query_results(report, detail))?;
                }
                Request::QueryAnalysis(request) => {
                    send(&mut out, &backend.query_analysis(request))?;
                }
                Request::AnalysisResult(request, preview_seen) => {
                    let reply = backend.analysis_result().filter(|reply| {
                        reply.request == request && (reply.complete || !preview_seen)
                    });
                    send(&mut out, &reply)?;
                }
                Request::QueryCalendar(request) => {
                    send(&mut out, &backend.query_calendar(request))?;
                }
                Request::CalendarResult(request) => {
                    let reply = backend
                        .calendar_result()
                        .filter(|reply| reply.request == request);
                    send(&mut out, &reply)?;
                }
                Request::QuerySeconds(request) => {
                    send(&mut out, &backend.speeds.query(request))?;
                }
                Request::TakeSeconds => {
                    let reply = backend.speeds.reply.lock().unwrap().take();
                    send(&mut out, &reply)?;
                }
                Request::QueryHover(request) => send(&mut out, &backend.speeds.hover(request))?,
                Request::TakeHover => {
                    let reply = backend.speeds.hover_reply.lock().unwrap().take();
                    send(&mut out, &reply)?;
                }
                Request::QueryUsage(request) => {
                    backend.overview_usage.query(request);
                    send(&mut out, &())?;
                }
                Request::TakeUsage => {
                    let reply = backend.overview_usage.reply.lock().unwrap().take();
                    send(&mut out, &reply)?;
                }
                Request::RefreshSubscriptions => {
                    backend.subscriptions.refresh();
                    send(&mut out, &())?;
                }
                Request::RefreshStorage => send(&mut out, &backend.refresh_storage())?,
                Request::PreviewStorage(request) => {
                    send(&mut out, &backend.speeds.manage(request))?;
                }
                Request::SubmitStorage(request) => {
                    send(&mut out, &message(backend.submit_storage_job(request)))?;
                }
                Request::SaveConfig(next) => {
                    let result = backend.save_ui_config((*next).into());
                    send(
                        &mut out,
                        &(message(result), WireConfig::from(backend.config_snapshot())),
                    )?;
                }
                Request::BackupDirectory(path) => {
                    let result = backend.set_backup_directory(path);
                    send(
                        &mut out,
                        &(message(result), WireConfig::from(backend.config_snapshot())),
                    )?;
                }
                Request::PrepareMigration(path) => {
                    let result = backend.prepare_migration(&path);
                    if result.is_ok() {
                        self.quit.store(true, Ordering::Release);
                    }
                    send(&mut out, &message(result))?;
                }
                Request::ApplyProcessMode(mode) => {
                    let result = apply_process_mode(backend, mode);
                    send(
                        &mut out,
                        &(message(result), WireConfig::from(backend.config_snapshot())),
                    )?;
                }
                Request::RetryProcessControl => send(&mut out, &backend.retry_process_control())?,
                Request::Permanent(request) => {
                    backend.permanent_control.post(backend, request);
                    send(&mut out, &())?;
                }
                Request::Notice(notice) => send(&mut out, &backend.notice(notice))?,
                Request::HotkeyRecording(recording) => {
                    send(&mut out, &native::set_hotkey_recording(recording))?;
                }
                Request::ApplyHotkey(id, key) => {
                    send(&mut out, &message(native::apply_hotkey(id, key)))?;
                }
                Request::TaskbarRescan => send(&mut out, &native::request_taskbar_update(true))?,
                Request::SaveWindow(state) => {
                    *self.resume.lock().unwrap() = Some(state);
                    send(&mut out, &())?;
                }
                Request::Open => {
                    self.post(UiEvent::Open);
                    send(&mut out, &())?;
                }
                Request::Exit => {
                    self.quit.store(true, Ordering::Release);
                    send(&mut out, &())?;
                }
            }
        }
        Ok(())
    }
}

fn apply_process_mode(backend: &Backend, mode: crate::config::ProcessControl) -> Result<()> {
    backend
        .permanent_control
        .post(backend, permanent_control::Request::Cancel);
    let result = backend.apply_process_mode(mode);
    if let Err(error) = &result {
        let mut shared = backend.shared.lock().unwrap();
        shared.process_error = format!("设置保存失败：{error:#}");
        shared.process_status = ProcessControlStatus::Failed;
    } else {
        backend
            .permanent_control
            .paused
            .store(false, Ordering::Release);
    }
    result
}

fn message<T>(result: Result<T>) -> std::result::Result<T, String> {
    result.map_err(|error| format!("{error:#}"))
}

fn send(output: &mut impl Write, value: &impl Serialize) -> Result<()> {
    serde_json::to_writer(&mut *output, value)?;
    output.write_all(b"\n")?;
    output.flush()?;
    Ok(())
}

fn receive<T: DeserializeOwned>(input: &mut impl BufRead) -> Result<Option<T>> {
    let mut frame = Vec::new();
    if input.read_until(b'\n', &mut frame)? == 0 {
        return Ok(None);
    }
    Ok(Some(serde_json::from_slice(&frame)?))
}

struct Pipe {
    input: BufReader<io::Stdin>,
    output: BufWriter<io::Stdout>,
}

impl Pipe {
    fn call<T: DeserializeOwned>(&mut self, request: Request) -> Result<T> {
        send(&mut self.output, &request)?;
        receive(&mut self.input)?.context("后台已停止")
    }
}

#[derive(Clone)]
pub(crate) struct UiBackend {
    pub(super) dir: PathBuf,
    pub(super) sequence: u64,
    pub(crate) shared: Arc<Mutex<Shared>>,
    pub(crate) config: Arc<Mutex<Config>>,
    pub(crate) process_generation: Arc<AtomicU64>,
    analysis_generation: Arc<AtomicU64>,
    pipe: Option<Arc<Mutex<Pipe>>>,
    #[cfg(test)]
    local: Option<Backend>,
}

macro_rules! local {
    ($self:ident, $backend:ident => $value:expr) => {
        #[cfg(test)]
        if let Some($backend) = &$self.local {
            return $value;
        }
    };
}

impl UiBackend {
    pub(super) fn connect() -> Result<(Self, Option<WindowState>, Option<CheckSnapshot>)> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{FILE_TYPE_PIPE, GetFileType};
        let stdin = io::stdin();
        let stdout = io::stdout();
        anyhow::ensure!(
            unsafe {
                GetFileType(stdin.as_raw_handle()) == FILE_TYPE_PIPE
                    && GetFileType(stdout.as_raw_handle()) == FILE_TYPE_PIPE
            },
            "界面进程需要连接后台"
        );
        let mut pipe = Pipe {
            input: BufReader::new(stdin),
            output: BufWriter::new(stdout),
        };
        let (dir, sequence, resume, initial): (PathBuf, u64, Option<WindowState>, Poll) =
            pipe.call(Request::Start)?;
        Ok((
            Self {
                dir,
                sequence,
                shared: Arc::new(Mutex::new(initial.state)),
                config: Arc::new(Mutex::new(initial.config.into())),
                process_generation: Arc::new(AtomicU64::new(initial.process_generation)),
                analysis_generation: Arc::new(AtomicU64::new(initial.analysis_generation)),
                pipe: Some(Arc::new(Mutex::new(pipe))),
                #[cfg(test)]
                local: None,
            },
            resume,
            initial.update.checks,
        ))
    }

    #[cfg(test)]
    pub(super) fn local_backend(&self) -> Option<&Backend> {
        self.local.as_ref()
    }

    #[cfg(test)]
    pub(super) fn local(backend: Backend) -> Self {
        Self {
            dir: backend.dir.clone(),
            sequence: 0,
            shared: backend.shared.clone(),
            config: backend.config.clone(),
            process_generation: backend.process_generation.clone(),
            analysis_generation: Arc::new(AtomicU64::new(0)),
            pipe: None,
            local: Some(backend),
        }
    }

    pub(super) fn is_remote(&self) -> bool {
        self.pipe.is_some()
    }

    fn call<T: DeserializeOwned>(&self, request: Request) -> Result<T> {
        self.pipe
            .as_ref()
            .context("后台连接不可用")?
            .lock()
            .unwrap()
            .call(request)
    }

    fn action(&self, request: Request) {
        let _ = self.call::<()>(request);
    }

    fn result<T: DeserializeOwned>(&self, request: Request) -> Result<T> {
        self.call::<std::result::Result<T, String>>(request)?
            .map_err(anyhow::Error::msg)
    }

    fn change_config(&self, request: Request) -> Result<()> {
        let mut pipe = self
            .pipe
            .as_ref()
            .context("后台连接不可用")?
            .lock()
            .unwrap();
        let (result, config): (std::result::Result<(), String>, WireConfig) = pipe.call(request)?;
        *self.config.lock().unwrap() = config.into();
        result.map_err(anyhow::Error::msg)
    }

    pub(super) fn poll(&self, known: u64) -> Result<Option<InterfaceUpdate>> {
        let Ok(mut pipe) = self.pipe.as_ref().context("后台连接不可用")?.try_lock() else {
            return Ok(None);
        };
        let poll: Poll = pipe.call(Request::Poll(known))?;
        *self.shared.lock().unwrap() = poll.state;
        *self.config.lock().unwrap() = poll.config.into();
        self.analysis_generation
            .store(poll.analysis_generation, Ordering::Relaxed);
        self.process_generation
            .store(poll.process_generation, Ordering::Relaxed);
        Ok(Some(poll.update))
    }

    pub(super) fn check_updates(&self, action: CheckAction, known: u64) -> Option<CheckSnapshot> {
        self.call(Request::CheckUpdates(action, known))
            .ok()
            .flatten()
    }

    pub(super) fn config_snapshot(&self) -> Config {
        self.config.lock().unwrap().clone()
    }

    pub(super) fn snapshot(&self) -> Shared {
        self.shared.lock().unwrap().clone()
    }

    pub(super) fn storage_revision(&self) -> u64 {
        self.shared.lock().unwrap().storage_revision
    }

    pub(super) fn notice(&self, notice: impl Into<String>) {
        let notice = notice.into();
        local!(self, backend => backend.notice(notice));
        self.action(Request::Notice(notice));
    }

    pub(super) fn send(&self, command: Command) {
        local!(self, backend => backend.send(command));
        self.action(Request::Send(command));
    }

    pub(super) fn save_ui_config(&self, config: Config) -> Result<()> {
        local!(self, backend => backend.save_ui_config(config));
        self.change_config(Request::SaveConfig(Box::new(config.into())))
    }

    pub(super) fn set_backup_directory(&self, path: Option<PathBuf>) -> Result<()> {
        local!(self, backend => backend.set_backup_directory(path));
        self.change_config(Request::BackupDirectory(path))
    }

    pub(super) fn prepare_migration(&self, path: &Path) -> Result<()> {
        local!(self, backend => backend.prepare_migration(path));
        self.result(Request::PrepareMigration(path.into()))
    }

    pub(super) fn query(&self, id: u64, query: Query) -> bool {
        local!(self, backend => backend.query(id, query));
        self.call(Request::Query(id, query)).unwrap_or(false)
    }

    pub(super) fn query_results(
        &self,
        report: Option<u64>,
        detail: Option<u64>,
    ) -> (u64, Option<Report>, Option<Report>) {
        #[cfg(test)]
        if let Some(backend) = &self.local {
            let (revision, report, detail) = backend.query_results(report, detail);
            return (
                revision,
                report.map(Arc::unwrap_or_clone),
                detail.map(Arc::unwrap_or_clone),
            );
        }
        if report.is_none() && detail.is_none() {
            return (self.storage_revision(), None, None);
        }
        self.call(Request::QueryResults(report, detail))
            .unwrap_or_else(|_| (self.storage_revision(), None, None))
    }

    pub(super) fn query_analysis(&self, request: AnalysisRequest) -> bool {
        local!(self, backend => backend.query_analysis(request));
        self.call(Request::QueryAnalysis(request)).unwrap_or(false)
    }

    pub(super) fn analysis_result(
        &self,
        request: Option<&AnalysisRequest>,
        preview_seen: bool,
    ) -> Option<AnalysisReply> {
        let request = request?;
        local!(self, backend => backend.analysis_result());
        self.call(Request::AnalysisResult(request.clone(), preview_seen))
            .ok()
            .flatten()
    }

    pub(super) fn analysis_generation(&self) -> u64 {
        local!(self, backend => backend.analysis_generation());
        self.analysis_generation.load(Ordering::Relaxed)
    }

    pub(super) fn query_calendar(&self, request: CalendarRequest) -> bool {
        local!(self, backend => backend.query_calendar(request));
        self.call(Request::QueryCalendar(request)).unwrap_or(false)
    }

    pub(super) fn calendar_result(
        &self,
        request: Option<&CalendarRequest>,
    ) -> Option<CalendarReply> {
        let request = request?;
        local!(self, backend => backend.calendar_result());
        self.call(Request::CalendarResult(request.clone()))
            .ok()
            .flatten()
    }

    pub(super) fn query_seconds(&self, request: SecondsRequest) -> bool {
        local!(self, backend => backend.speeds.query(request));
        self.call(Request::QuerySeconds(request)).unwrap_or(false)
    }

    pub(super) fn take_seconds(&self) -> Option<SecondsReply> {
        local!(self, backend => backend.speeds.reply.lock().unwrap().take());
        self.call(Request::TakeSeconds).ok().flatten()
    }

    pub(super) fn query_hover(&self, request: HoverRequest) -> bool {
        local!(self, backend => backend.speeds.hover(request));
        self.call(Request::QueryHover(request)).unwrap_or(false)
    }

    pub(super) fn take_hover(&self) -> Option<HoverReply> {
        local!(self, backend => backend.speeds.hover_reply.lock().unwrap().take());
        self.call(Request::TakeHover).ok().flatten()
    }

    pub(super) fn query_usage(&self, request: overview_usage::Request) {
        local!(self, backend => backend.overview_usage.query(request));
        self.action(Request::QueryUsage(request));
    }

    pub(super) fn take_usage(&self) -> Option<overview_usage::Reply> {
        local!(self, backend => backend.overview_usage.reply.lock().unwrap().take());
        self.call(Request::TakeUsage).ok().flatten()
    }

    pub(super) fn refresh_subscriptions(&self) {
        local!(self, backend => backend.subscriptions.refresh());
        self.action(Request::RefreshSubscriptions);
    }

    pub(super) fn refresh_storage(&self) {
        local!(self, backend => backend.refresh_storage());
        self.action(Request::RefreshStorage);
    }

    pub(super) fn preview_storage(&self, request: StorageRequest) -> bool {
        local!(self, backend => backend.speeds.manage(request));
        self.call(Request::PreviewStorage(request)).unwrap_or(false)
    }

    pub(super) fn submit_storage_job(&self, request: StorageRequest) -> Result<bool> {
        local!(self, backend => backend.submit_storage_job(request));
        self.result(Request::SubmitStorage(request))
    }

    pub(super) fn apply_process_mode(&self, mode: crate::config::ProcessControl) -> Result<()> {
        local!(self, backend => apply_process_mode(backend, mode));
        self.change_config(Request::ApplyProcessMode(mode))
    }

    pub(super) fn retry_process_control(&self) {
        local!(self, backend => backend.retry_process_control());
        self.action(Request::RetryProcessControl);
    }

    pub(super) fn permanent(&self, request: permanent_control::Request) {
        local!(self, backend => backend.permanent_control.post(backend, request));
        self.action(Request::Permanent(request));
    }

    pub(super) fn set_hotkey_recording(&self, recording: bool) {
        if self.is_remote() {
            self.action(Request::HotkeyRecording(recording));
        } else {
            native::set_hotkey_recording(recording);
        }
    }

    pub(super) fn apply_hotkey(&self, id: u32, key: Hotkey) -> Result<()> {
        if self.is_remote() {
            self.result(Request::ApplyHotkey(id, key))
        } else {
            native::apply_hotkey(id, key)
        }
    }

    pub(super) fn taskbar_rescan(&self) {
        if self.is_remote() {
            self.action(Request::TaskbarRescan);
        } else {
            native::request_taskbar_update(true);
        }
    }

    pub(super) fn open(&self) {
        if self.is_remote() {
            self.action(Request::Open);
        } else {
            native::request_app_open();
        }
    }

    pub(super) fn save_window(&self, state: WindowState) {
        if self.is_remote() {
            self.action(Request::SaveWindow(state));
        }
    }

    pub(super) fn exit(&self) {
        if self.is_remote() {
            self.action(Request::Exit);
        }
    }
}

#[cfg(test)]
impl std::ops::Deref for UiBackend {
    type Target = Backend;
    fn deref(&self) -> &Self::Target {
        self.local
            .as_ref()
            .expect("fixture requires an in-process backend")
    }
}

#[cfg(test)]
impl std::ops::DerefMut for UiBackend {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.local
            .as_mut()
            .expect("fixture requires an in-process backend")
    }
}

#[cfg(test)]
mod tests {
    use super::{InterfaceUpdate, Poll, receive, send};
    use crate::{
        app_activity::application_activity,
        attribution::{Attribution, Owner},
        config::Config,
        model::{Bytes, Connection, Metadata, Shared},
    };
    use std::sync::Arc;

    #[test]
    fn poll_preserves_live_application_attribution() {
        let process = r"C:\Runtime\msedgewebview2.exe";
        let host = r"C:\Windows\SystemApps\SearchHost.exe";
        let owner = Owner {
            key: host.into(),
            label: "SearchHost.exe".into(),
            icon: host.into(),
        };
        let rate = Bytes { up: 8, down: 16 };
        let poll = Poll {
            config: Config::default().into(),
            state: Shared {
                connected: true,
                connections: Arc::new(vec![Connection {
                    metadata: Metadata {
                        process_path: process.into(),
                        ..Default::default()
                    },
                    attribution: Some(Attribution {
                        owner: owner.clone(),
                        pid: 42,
                    }),
                    ..Default::default()
                }]),
                app_rates: [(process.into(), rate)].into(),
                application_rates: [(host.into(), rate)].into(),
                identities: [(host.into(), owner.clone())].into(),
                ..Default::default()
            },
            analysis_generation: 0,
            process_generation: 0,
            update: InterfaceUpdate::default(),
        };

        let mut frame = Vec::new();
        send(&mut frame, &poll).unwrap();
        let received: Poll = receive(&mut frame.as_slice()).unwrap().unwrap();

        let apps = application_activity(&received.state);
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].key, host);
        assert_eq!(apps[0].connections, 1);
        assert_eq!(apps[0].rate, rate);
        let connection = &received.state.connections[0];
        assert_eq!(connection.app(), process);
        let attribution = connection.attribution.as_ref().unwrap();
        assert_eq!(attribution.owner, owner);
        assert_eq!(attribution.pid, 42);
    }
}
