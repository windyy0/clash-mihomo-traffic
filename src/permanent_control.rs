//! Durable client settings have their own worker; filesystem work never holds
//! the collector or shared UI locks.
use crate::{
    backend::Backend,
    client_config::{self, Kind, SavedMode, Target},
    config::{ProcessControl, ProcessMode},
    model::ProcessControlStatus,
};
use anyhow::{Result, ensure};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    Idle,
    WaitingExit,
    Saving,
    SavedPending,
    Saved,
    Applied,
    Restored,
    Failed,
}
#[derive(Clone, Debug, Default)]
pub struct View {
    pub available: bool,
    pub client: String,
    pub mode: SavedMode,
    pub source: String,
    pub note: String,
    pub error: String,
    pub status: Status,
    pub backup: String,
    pub launch_available: bool,
    pub restoring: bool,
}
#[derive(Clone)]
pub enum Request {
    Save(ProcessMode),
    Restore,
    Cancel,
    Refresh,
    SelectDirectory(PathBuf),
    Launch,
}
#[derive(Clone, PartialEq, Eq)]
struct Connection {
    endpoint: String,
    secret: String,
}
impl Connection {
    fn read(handle: &Backend) -> Self {
        let config = handle.config.lock().unwrap();
        Self {
            endpoint: config.endpoint.clone(),
            secret: config.secret.clone(),
        }
    }
}
struct Envelope {
    request: Request,
    connection: Connection,
    target: Option<Target>,
    kind: Option<Kind>,
    sequence: u64,
}
#[derive(Default)]
pub struct Control {
    request: Mutex<VecDeque<Envelope>>,
    target: Mutex<Option<Target>>,
    sequence: AtomicU64,
    pub paused: AtomicBool,
    #[cfg(test)]
    pub fixture: Mutex<Option<(Target, bool)>>,
}
impl Control {
    pub fn subscription_target(&self) -> Option<Target> {
        self.target.lock().unwrap().clone()
    }
    fn running(&self, target: &Target) -> Result<bool> {
        #[cfg(test)]
        if let Some((fixture, running)) = self.fixture.lock().unwrap().as_ref()
            && fixture == target
        {
            return Ok(*running);
        }
        target.running()
    }
    pub fn post(&self, handle: &Backend, request: Request) {
        let sequence = if matches!(request, Request::Refresh | Request::Launch) {
            self.sequence.load(Ordering::Acquire)
        } else {
            self.sequence.fetch_add(1, Ordering::AcqRel) + 1
        };
        let kind = Kind::from_name(&handle.shared.lock().unwrap().client_name);
        let envelope = Envelope {
            request,
            connection: Connection::read(handle),
            target: self.target.lock().unwrap().clone(),
            kind,
            sequence,
        };
        let mut queue = self.request.lock().unwrap();
        queue.retain(|old| {
            if matches!(envelope.request, Request::Save(_) | Request::Restore) {
                !matches!(old.request, Request::Save(_) | Request::Restore)
            } else {
                std::mem::discriminant(&old.request) != std::mem::discriminant(&envelope.request)
            }
        });
        queue.push_back(envelope);
    }
}
struct Pending {
    target: Target,
    mode: Option<ProcessMode>,
    connection: Connection,
    sequence: u64,
    control_endpoint: String,
}
impl Pending {
    fn current(&self, handle: &Backend) -> bool {
        !handle.stop.load(Ordering::Relaxed)
            && Connection::read(handle) == self.connection
            && handle.permanent_control.sequence.load(Ordering::Acquire) == self.sequence
    }
}
struct Pause(std::sync::Arc<Control>, bool);
impl Drop for Pause {
    fn drop(&mut self) {
        if self.1 {
            self.0.paused.store(false, Ordering::Release);
        }
    }
}

pub async fn run(handle: Backend) {
    let control = handle.permanent_control.clone();
    let mut connection = Connection::read(&handle);
    let mut target: Option<Target> = None;
    let mut owner = String::new();
    let mut last_endpoint = String::new();
    let mut pending: Option<Pending> = None;
    let mut saved: Option<ProcessMode> = None;
    let mut due = Instant::now();
    while !handle.stop.load(Ordering::Relaxed) {
        let next = Connection::read(&handle);
        if next != connection {
            connection = next;
            target = None;
            owner.clear();
            last_endpoint.clear();
            pending = None;
            saved = None;
            *control.target.lock().unwrap() = None;
            handle.shared.lock().unwrap().permanent = View::default();
            due = Instant::now();
        }
        let request = { control.request.lock().unwrap().pop_front() };
        if let Some(envelope) = request
            && envelope.connection == connection
        {
            match envelope.request {
                Request::Cancel => {
                    pending = None;
                    saved = None;
                    let mut shared = handle.shared.lock().unwrap();
                    shared.permanent.status = Status::Idle;
                    shared.permanent.error.clear();
                    shared.permanent.restoring = false;
                }
                Request::Refresh => {
                    due = Instant::now();
                    let mut shared = handle.shared.lock().unwrap();
                    if shared.permanent.status == Status::Failed {
                        shared.permanent.status = Status::Idle;
                        shared.permanent.error.clear();
                    }
                }
                Request::SelectDirectory(path) => {
                    pending = None;
                    saved = None;
                    handle.shared.lock().unwrap().permanent.restoring = false;
                    let connected = handle.shared.lock().unwrap().connected;
                    if envelope.kind.is_some() && connected {
                        let result = handle.writer.update(|config| {
                            config.client_config_dir = Some(path.to_string_lossy().into_owned());
                            true
                        });
                        let mut shared = handle.shared.lock().unwrap();
                        match result {
                            Err(error) => shared.permanent.error = error.to_string(),
                            Ok(_) => {
                                shared.permanent.status = Status::Idle;
                                shared.permanent.error.clear();
                            }
                        }
                        target = None;
                        due = Instant::now();
                    } else {
                        handle.shared.lock().unwrap().permanent.error =
                            "请先连接并识别受支持的客户端，再选择其配置目录".into();
                    }
                }
                request @ (Request::Save(_) | Request::Restore) => {
                    let mode = if let Request::Save(mode) = request {
                        Some(mode)
                    } else {
                        None
                    };
                    if let Some(selected) = envelope.target
                        && target.as_ref() == Some(&selected)
                    {
                        let mut shared = handle.shared.lock().unwrap();
                        saved = None;
                        pending = Some(Pending {
                            target: selected,
                            mode,
                            connection: connection.clone(),
                            sequence: envelope.sequence,
                            // A reconnect can publish its endpoint before this
                            // worker refreshes last_endpoint below. Bind the
                            // operation to the currently connected core.
                            control_endpoint: if shared.connected && !shared.endpoint.is_empty() {
                                shared.endpoint.clone()
                            } else {
                                last_endpoint.clone()
                            },
                        });
                        shared.permanent.error.clear();
                        shared.permanent.status = Status::Saving;
                        shared.permanent.restoring = mode.is_none();
                    } else {
                        let mut shared = handle.shared.lock().unwrap();
                        shared.permanent.status = Status::Failed;
                        shared.permanent.error = "请先核验当前客户端配置目录".into();
                    }
                }
                Request::Launch => {
                    if let Some(selected) = target.clone() {
                        let result = tokio::task::spawn_blocking(move || -> Result<()> {
                            ensure!(!selected.running()?, "客户端已经运行");
                            let path = selected
                                .executable
                                .as_ref()
                                .context("未记录客户端程序路径，请手动启动客户端")?;
                            ensure!(
                                path.is_file()
                                    && path.file_name().is_some_and(|name| name
                                        .to_string_lossy()
                                        .eq_ignore_ascii_case(selected.kind.executable())),
                                "客户端程序路径无效"
                            );
                            std::process::Command::new(path)
                                .current_dir(path.parent().unwrap())
                                .spawn()?;
                            Ok(())
                        })
                        .await;
                        if let Err(error) = result.unwrap_or_else(|e| Err(e.into())) {
                            handle.shared.lock().unwrap().permanent.error =
                                format!("启动失败：{error:#}");
                        }
                    }
                }
            }
        }
        let (connected, name, identity, endpoint, verified, core_pid) = {
            let shared = handle.shared.lock().unwrap();
            (
                shared.connected,
                shared.client_name.clone(),
                shared.core_identity.clone(),
                shared.endpoint.clone(),
                shared.core_verified,
                shared.core_process_id,
            )
        };
        if connected {
            last_endpoint = endpoint.clone();
        }
        if connected && !identity.is_empty() && identity != owner {
            // A new core of the same client is expected after restart. A
            // different client must not inherit a pending filesystem operation.
            if target
                .as_ref()
                .is_some_and(|t| Some(t.kind) != Kind::from_name(&name))
            {
                pending = None;
                saved = None;
                target = None;
                *control.target.lock().unwrap() = None;
                handle.shared.lock().unwrap().permanent = View::default();
            }
            owner = identity;
            due = Instant::now();
        }
        if Instant::now() >= due {
            let selected = handle.config.lock().unwrap().client_config_dir.clone();
            let kind = Kind::from_name(&name);
            let discovery = if connected && verified {
                if let Some(kind) = kind {
                    let endpoint = endpoint.clone();
                    Some(
                        tokio::task::spawn_blocking(move || {
                            client_config::discover(kind, selected.as_deref(), &endpoint, core_pid)
                        })
                        .await
                        .unwrap_or_else(|e| Err(e.into())),
                    )
                } else {
                    Some(Err(anyhow::anyhow!(
                        "此客户端尚未适配永久配置；可继续使用内核运行配置"
                    )))
                }
            } else {
                None
            };
            #[cfg(test)]
            let discovery = if connected {
                control
                    .fixture
                    .lock()
                    .unwrap()
                    .as_ref()
                    .map(|(target, _)| Ok(target.clone()))
                    .map(Some)
                    .unwrap_or(discovery)
            } else {
                discovery
            };
            if let Some(result) = discovery {
                match result {
                    Ok(mut found) => {
                        if pending.as_ref().is_some_and(|job| {
                            job.target.root != found.root || job.target.kind != found.kind
                        }) {
                            pending = None;
                            saved = None;
                        }
                        if let Some(previous) = &target
                            && previous.root == found.root
                            && previous.kind == found.kind
                            && found.executable.is_none()
                        {
                            found.executable = previous.executable.clone();
                        }
                        target = Some(found);
                    }
                    Err(error) => {
                        target = None;
                        pending = None;
                        let mut shared = handle.shared.lock().unwrap();
                        shared.permanent.available = false;
                        shared.permanent.client = name.clone();
                        shared.permanent.mode = SavedMode::Unavailable;
                        shared.permanent.error = format!("{error:#}");
                        shared.permanent.source.clear();
                    }
                }
            }
            *control.target.lock().unwrap() = target.clone();
            if let Some(selected) = target.clone() {
                let result = tokio::task::spawn_blocking(move || -> Result<_> {
                    Ok((
                        selected.read()?,
                        selected.executable.is_some() && !selected.running()?,
                    ))
                })
                .await
                .unwrap_or_else(|e| Err(e.into()));
                if Connection::read(&handle) == connection {
                    let mut shared = handle.shared.lock().unwrap();
                    match result {
                        Ok((snapshot, launch)) => {
                            shared.permanent.available = true;
                            shared.permanent.client = target.as_ref().unwrap().kind.name().into();
                            shared.permanent.mode = snapshot.mode;
                            shared.permanent.source = snapshot.source.display().to_string();
                            shared.permanent.note = snapshot.note;
                            shared.permanent.launch_available = launch;
                            if shared.permanent.status == Status::Idle {
                                shared.permanent.error.clear();
                            }
                        }
                        Err(error) => {
                            shared.permanent.available = false;
                            shared.permanent.mode = SavedMode::Unavailable;
                            shared.permanent.error = format!("读取失败：{error:#}");
                        }
                    }
                }
            } else if !connected && handle.shared.lock().unwrap().permanent.error.is_empty() {
                handle.shared.lock().unwrap().permanent.note =
                    "连接受支持的客户端后，自动核验其永久配置。".into();
            }
            due = Instant::now() + Duration::from_secs(10);
        }
        if let Some(job) = pending.take() {
            if !job.current(&handle) {
                continue;
            }
            let check_target = job.target.clone();
            let check_control = control.clone();
            let running = tokio::task::spawn_blocking(move || check_control.running(&check_target))
                .await
                .unwrap_or_else(|e| Err(e.into()));
            match running {
                Ok(true) if job.target.kind.needs_exit() => {
                    handle.shared.lock().unwrap().permanent.status = Status::WaitingExit;
                    pending = Some(job);
                }
                Err(error) => {
                    let mut shared = handle.shared.lock().unwrap();
                    shared.permanent.status = Status::Failed;
                    shared.permanent.error = format!("{error:#}");
                }
                _ => {
                    control.paused.store(true, Ordering::Release);
                    let mut pause = Pause(control.clone(), true);
                    handle.shared.lock().unwrap().permanent.status = Status::Saving;
                    let selected = job.target.clone();
                    let desired = job.mode;
                    let backup = handle.dir.join("client-config-backups");
                    let worker = handle.clone();
                    let sequence = job.sequence;
                    let scope = job.connection.clone();
                    let expected_endpoint = job.control_endpoint.clone();
                    let result = tokio::task::spawn_blocking(move || -> Result<_> {
                        let changes = match desired {
                            Some(mode) => selected.prepare(mode)?,
                            None => selected.prepare_restore()?,
                        };
                        let backup = changes.commit(&backup, || {
                            let same_client = {
                                let shared = worker.shared.lock().unwrap();
                                !shared.connected
                                    || (Kind::from_name(&shared.client_name) == Some(selected.kind)
                                        && client_config::same_control_target(
                                            &expected_endpoint,
                                            &shared.endpoint,
                                        ))
                            };
                            #[cfg(test)]
                            let same_client = same_client
                                || worker
                                    .permanent_control
                                    .fixture
                                    .lock()
                                    .unwrap()
                                    .as_ref()
                                    .is_some_and(|(fixture, _)| fixture == &selected);
                            same_client
                                && !worker.stop.load(Ordering::Relaxed)
                                && Connection::read(&worker) == scope
                                && worker.permanent_control.sequence.load(Ordering::Acquire)
                                    == sequence
                                && (!selected.kind.needs_exit()
                                    || matches!(
                                        worker.permanent_control.running(&selected),
                                        Ok(false)
                                    ))
                        })?;
                        let snapshot = selected.read()?;
                        ensure!(
                            snapshot.mode
                                == desired
                                    .map(SavedMode::Explicit)
                                    .unwrap_or(SavedMode::Inherited),
                            "文件已写入，但客户端保存值回读不一致"
                        );
                        Ok((backup, snapshot))
                    })
                    .await
                    .unwrap_or_else(|e| Err(e.into()));
                    if !job.current(&handle) {
                        drop(pause);
                        continue;
                    }
                    match result {
                        Ok((backup, snapshot)) => {
                            // Stop any previous maintenance before scheduling the
                            // one-time runtime application of the durable value.
                            let queued = handle.writer.update(|config| {
                                if config.endpoint != job.connection.endpoint
                                    || config.secret != job.connection.secret
                                    || control.sequence.load(Ordering::Acquire) != job.sequence
                                {
                                    return false;
                                }
                                config.process_control = job
                                    .mode
                                    .map(ProcessControl::ApplyOnce)
                                    .unwrap_or(ProcessControl::Unmanaged);
                                config.process_endpoint = (job.mode.is_some()
                                    && !job.control_endpoint.is_empty())
                                .then(|| job.control_endpoint.clone());
                                config.process_client =
                                    job.mode.map(|_| job.target.kind.name().into());
                                #[cfg(test)]
                                if control.fixture.lock().unwrap().is_some() {
                                    config.process_client = None;
                                }
                                true
                            });
                            if matches!(queued, Ok(None)) {
                                drop(pause);
                                continue;
                            }
                            let mut apply = queued.map(|_| ());
                            if apply.is_ok() {
                                handle.retry_process_control();
                                let writer = handle.writer.clone();
                                apply = tokio::task::spawn_blocking(move || {
                                    writer.flush(Duration::from_secs(5))
                                })
                                .await
                                .unwrap_or_else(|e| Err(e.into()));
                            }
                            if !job.current(&handle) {
                                drop(pause);
                                continue;
                            }
                            if apply.is_err() {
                                pause.1 = false;
                            }
                            let mut shared = handle.shared.lock().unwrap();
                            shared.permanent.mode = snapshot.mode;
                            shared.permanent.source = snapshot.source.display().to_string();
                            shared.permanent.status = if job.mode.is_some() {
                                Status::SavedPending
                            } else {
                                Status::Restored
                            };
                            shared.permanent.error = apply.err().map(|e| format!("文件已保存，但本程序设置保存失败，自动保持已暂停。请重试：{e:#}")).unwrap_or_default();
                            if let Some(backup) = backup {
                                shared.permanent.backup = backup.display().to_string();
                            }
                            saved = job.mode;
                        }
                        Err(error) => {
                            let mut shared = handle.shared.lock().unwrap();
                            shared.permanent.status = Status::Failed;
                            shared.permanent.error = format!("保存失败：{error:#}");
                        }
                    }
                    drop(pause);
                    due = Instant::now();
                }
            }
        }
        if let Some(mode) = saved {
            let policy = handle.config.lock().unwrap().process_control;
            let mut shared = handle.shared.lock().unwrap();
            if control.paused.load(Ordering::Acquire) {
                // Preserve the persistence error until an explicit retry.
            } else if shared.permanent.mode != SavedMode::Explicit(mode) {
                saved = None;
                shared.permanent.status = Status::Idle;
            } else if shared.connected
                && shared.process_mode == mode.as_str()
                && matches!(
                    shared.process_status,
                    ProcessControlStatus::Applied | ProcessControlStatus::Idle
                )
            {
                shared.permanent.status = Status::Applied;
                shared.permanent.error.clear();
            } else if policy == ProcessControl::ApplyOnce(mode) {
                shared.permanent.status = Status::SavedPending;
                shared.permanent.error = if shared.process_status == ProcessControlStatus::Failed {
                    format!("文件已保存，内核应用失败：{}", shared.process_error)
                } else {
                    String::new()
                };
            } else if shared.permanent.status != Status::Failed {
                shared.permanent.status = Status::Saved;
            }
        }
        tokio::time::sleep(Duration::from_millis(if pending.is_some() {
            500
        } else {
            200
        }))
        .await;
    }
}

use anyhow::Context;
