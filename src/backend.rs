mod collector;
mod queries;
mod settings;

use crate::{
    analysis::{AnalysisRequest, SampleWindow},
    config::Config,
    config_writer::ConfigWriter,
    mihomo::Client,
    model::*,
    storage::Storage,
    storage_management::Request as StorageRequest,
};
use anyhow::{Context, Result};
pub use queries::{AnalysisReply, CalendarReply};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

#[derive(serde::Serialize, serde::Deserialize)]
pub enum Command {
    Reconnect,
    Backup,
    Restore(PathBuf),
    Export(Query),
}
#[cfg(test)]
mod analysis_backend_tests;

#[derive(Clone)]
pub struct Backend {
    pub shared: Arc<Mutex<Shared>>,
    pub config: Arc<Mutex<Config>>,
    pub stop: Arc<AtomicBool>,
    pub done: Arc<AtomicBool>,
    pub writer: ConfigWriter,
    pub process_generation: Arc<AtomicU64>,
    pub process_wake: Arc<tokio::sync::Notify>,
    pub query_ready: Arc<tokio::sync::Notify>,
    pub permanent_control: Arc<crate::permanent_control::Control>,
    pub subscriptions: Arc<crate::subscriptions::Control>,
    pub overview_usage: Arc<crate::overview_usage::Control>,
    _data_lock: Arc<std::fs::File>,
    pub dir: PathBuf,
    pub seconds_policy: Arc<Mutex<crate::speed_stats::SecondsPolicy>>,
    #[cfg(test)]
    pub storage_policy: Arc<Mutex<crate::storage_management::Policy>>,
    pub speeds: Arc<crate::speed_worker::Service>,
    tx: mpsc::SyncSender<Command>,
    query_tx: mpsc::SyncSender<(u64, Query)>,
    analysis_tx: mpsc::SyncSender<AnalysisRequest>,
    pub analysis_result: Arc<Mutex<Option<AnalysisReply>>>,
    analysis_generation: Arc<AtomicU64>,
    calendar_tx: mpsc::SyncSender<crate::history::CalendarRequest>,
    pub calendar_result: Arc<Mutex<Option<CalendarReply>>>,
    analysis_cache: Arc<Mutex<SampleWindow>>,
}
impl Backend {
    pub fn start(dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&dir)?;
        std::fs::create_dir_all(dir.join("backups"))?;
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(dir.join("collector.lock"))?;
        lock.try_lock().context("这个数据目录已被另一实例使用")?;
        let (config, notice) = Config::load_for_start(&dir)?;
        let config = Arc::new(Mutex::new(config));
        let seconds_policy = Arc::new(Mutex::new(crate::speed_stats::SecondsPolicy::from(
            &*config.lock().unwrap(),
        )));
        let storage_policy = Arc::new(Mutex::new(crate::storage_management::Policy::from(
            &*config.lock().unwrap(),
        )));
        let db = Storage::open(&dir.join("traffic.sqlite3"))?;
        let ledger = db.checkpoint()?;
        let (today, month, cumulative) =
            db.dashboard_totals(chrono::Utc::now().timestamp_millis())?;
        let shared = Arc::new(Mutex::new(Shared {
            identities: db.identities()?,
            today,
            month,
            cumulative,
            notice,
            ..Default::default()
        }));
        let writer = settings::start_writer(
            dir.clone(),
            shared.clone(),
            config.clone(),
            seconds_policy.clone(),
            storage_policy.clone(),
        )?;
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::sync_channel(32);
        let (qtx, qrx) = mpsc::sync_channel::<(u64, Query)>(4);
        let (atx, arx) = mpsc::sync_channel::<AnalysisRequest>(4);
        let (ctx, crx) = mpsc::sync_channel::<crate::history::CalendarRequest>(2);
        let query_ready = Arc::new(tokio::sync::Notify::new());
        let speeds = Arc::new(crate::speed_worker::Service::start(
            dir.join("traffic.sqlite3"),
            shared.clone(),
            stop.clone(),
            seconds_policy.clone(),
            storage_policy.clone(),
            query_ready.clone(),
        )?);
        let handle = Self {
            query_ready,
            shared: shared.clone(),
            config,
            stop: stop.clone(),
            done: Arc::new(AtomicBool::new(false)),
            writer,
            process_generation: Arc::new(AtomicU64::new(0)),
            process_wake: Arc::new(tokio::sync::Notify::new()),
            permanent_control: Arc::new(crate::permanent_control::Control::default()),
            subscriptions: Arc::new(crate::subscriptions::Control::default()),
            overview_usage: Arc::new(crate::overview_usage::Control::default()),
            _data_lock: Arc::new(lock),
            dir,
            seconds_policy,
            #[cfg(test)]
            storage_policy,
            speeds,
            tx,
            query_tx: qtx,
            analysis_tx: atx,
            analysis_result: Arc::new(Mutex::new(None)),
            analysis_generation: Arc::new(AtomicU64::new(0)),
            calendar_tx: ctx,
            calendar_result: Arc::new(Mutex::new(None)),
            analysis_cache: Arc::new(Mutex::new(SampleWindow::default())),
        };
        queries::start(&handle, qrx, arx, crx)?;
        crate::subscriptions::start(handle.clone())?;
        crate::overview_usage::start(handle.clone())?;
        crate::system_metrics::start(shared, stop)?;
        collector::start(handle.clone(), db, ledger, rx)?;
        Ok(handle)
    }
    pub fn send(&self, cmd: Command) {
        if matches!(cmd, Command::Restore(_)) && self.shared.lock().unwrap().storage_management.busy
        {
            self.notice("存储任务正在执行，请完成后再恢复数据库");
            return;
        }
        if matches!(cmd, Command::Reconnect) {
            self.retry_process_control();
        }
        if self.tx.try_send(cmd).is_err() {
            self.notice("操作队列已满，请稍后重试");
        }
    }
    pub fn request_shutdown(&self) -> Result<()> {
        let saved = self.writer.close(settings::SAVE_TIMEOUT);
        self.stop.store(true, Ordering::Relaxed);
        self.speeds.interrupt();
        self.query_ready.notify_waiters();
        saved
    }
    pub fn shutdown(&self) -> Result<()> {
        let saved = self.request_shutdown();
        let deadline = Instant::now() + Duration::from_secs(30);
        while !self.done.load(Ordering::Acquire) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        anyhow::ensure!(self.done.load(Ordering::Acquire), "等待采集停止超时");
        saved
    }
    pub fn snapshot(&self) -> Shared {
        self.shared.lock().unwrap().clone()
    }
    pub fn clear_interface_results(&self) {
        {
            let mut state = self.shared.lock().unwrap();
            state.report = Default::default();
            state.report_id = 0;
            state.detail = Default::default();
            state.detail_id = 0;
        }
        self.analysis_result.lock().unwrap().take();
        self.calendar_result.lock().unwrap().take();
        self.speeds.reply.lock().unwrap().take();
        self.speeds.hover_reply.lock().unwrap().take();
        self.overview_usage.reply.lock().unwrap().take();
    }
    pub fn notice(&self, msg: impl Into<String>) {
        self.shared.lock().unwrap().notice = msg.into();
    }
    pub fn refresh_storage(&self) {
        let mut state = self.shared.lock().unwrap();
        if self.speeds.manage(StorageRequest::Refresh {
            epoch: state.storage_epoch,
        }) {
            state.storage_management.refreshing = true;
        } else {
            state.storage_management.error = "存储任务队列忙，请稍后刷新".into();
        }
    }
    pub fn submit_storage_job(&self, request: StorageRequest) -> Result<bool> {
        let epoch = match &request {
            StorageRequest::Clean(preview) => preview.epoch,
            StorageRequest::Reclaim { epoch } => *epoch,
            _ => anyhow::bail!("需要清理或回收请求"),
        };
        let mut state = self.shared.lock().unwrap();
        if state.storage_management.busy || state.storage_epoch != epoch {
            return Ok(false);
        }
        anyhow::ensure!(self.speeds.manage(request), "任务队列忙，请稍后重试");
        state.storage_management.busy = true;
        state.storage_management.job = "任务已提交…".into();
        state.storage_management.job_error.clear();
        Ok(true)
    }
    #[cfg(test)]
    pub fn set_process_control(&self, enabled: bool) -> Result<()> {
        self.apply_process_mode(if enabled {
            crate::config::ProcessControl::KeepAlways
        } else {
            crate::config::ProcessControl::RestoreStrict
        })
    }
    pub fn apply_process_mode(&self, policy: crate::config::ProcessControl) -> Result<()> {
        let (endpoint, client) = {
            let shared = self.shared.lock().unwrap();
            (shared.endpoint.clone(), shared.client_name.clone())
        };
        self.writer.update(|config| {
            config.process_control = policy;
            if policy == crate::config::ProcessControl::Unmanaged {
                config.process_endpoint = None;
                config.process_client = None;
            } else {
                if !endpoint.is_empty() {
                    config.process_endpoint = Some(endpoint);
                }
                if !client.is_empty() {
                    config.process_client = Some(client);
                }
            }
            true
        })?;
        self.retry_process_control();
        Ok(())
    }
    pub fn retry_process_control(&self) {
        self.process_generation.fetch_add(1, Ordering::Relaxed);
        self.process_wake.notify_one();
    }
}
pub(crate) fn log(dir: &std::path::Path, msg: &str) {
    use std::io::Write;
    let p = dir.join("traffic.log");
    if std::fs::metadata(&p).is_ok_and(|m| m.len() > 2 * 1024 * 1024) {
        let _ = crate::native::replace_file(&p, &dir.join("traffic.previous.log"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(p)
    {
        let _ = writeln!(f, "{} {}", chrono::Local::now().to_rfc3339(), msg);
    }
}
pub fn probe() -> Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let mut client = Client::new(&Config::default())?;
        let snapshot = client.snapshot().await?;
        let connections = snapshot.connections.as_deref().unwrap_or_default();
        let identified = connections
            .iter()
            .filter(|connection| connection.app() != "未知进程")
            .count();
        println!(
            "Mihomo OK; active connections: {}; process identities present: {identified}; totals: up={} down={}; core identity verified={}",
            connections.len(),
            snapshot.upload_total,
            snapshot.download_total,
            client.verified,
        );
        Ok(())
    })
}

#[cfg(test)]
#[tokio::test]
#[ignore = "requires the locally running Mihomo core; reads process and connection metadata"]
async fn inspect_live_application_attribution() {
    let stop = Arc::new(AtomicBool::new(false));
    let mut resolver = crate::attribution::Resolver::new(stop.clone());
    let mut client = Client::new(&Config::default()).unwrap();
    let mut snapshot = client.snapshot().await.unwrap();
    resolver.annotate(
        &mut snapshot,
        &client.identity,
        client.verified,
        chrono::Utc::now().timestamp_millis(),
    );
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let mut snapshot = client.snapshot().await.unwrap();
    resolver.annotate(
        &mut snapshot,
        &client.identity,
        client.verified,
        chrono::Utc::now().timestamp_millis(),
    );
    stop.store(true, Ordering::Relaxed);
    let connections = snapshot.connections.unwrap_or_default();
    println!(
        "local_core={} connections={} source_ports={} resolved={}",
        client.verified,
        connections.len(),
        connections
            .iter()
            .filter(|connection| !connection.metadata.source_port.is_empty())
            .count(),
        connections
            .iter()
            .filter(|connection| connection.attribution.is_some())
            .count()
    );
    let mut owners = std::collections::BTreeMap::<(String, String), usize>::new();
    for connection in connections {
        *owners
            .entry((
                app_name(&connection.app()).into(),
                connection
                    .attribution
                    .as_ref()
                    .map(|attribution| attribution.owner.label.clone())
                    .unwrap_or_else(|| "原进程".into()),
            ))
            .or_default() += 1;
    }
    for ((process, owner), count) in owners {
        println!("{process} -> {owner}: {count}");
    }
}
