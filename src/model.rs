use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, sync::Arc};

#[derive(Clone, Copy, Default, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Bytes {
    pub up: u64,
    pub down: u64,
}
impl Bytes {
    pub fn total(self) -> u64 {
        self.up.saturating_add(self.down)
    }
    pub fn delta(self, old: Self) -> Option<Self> {
        Some(Self {
            up: self.up.checked_sub(old.up)?,
            down: self.down.checked_sub(old.down)?,
        })
    }
    pub fn add(&mut self, v: Self) {
        self.up = self.up.saturating_add(v.up);
        self.down = self.down.saturating_add(v.down);
    }
}

#[derive(Clone, Default, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub upload_total: u64,
    pub download_total: u64,
    #[serde(default)]
    pub connections: Option<Vec<Connection>>,
}
impl Snapshot {
    pub fn bytes(&self) -> Bytes {
        Bytes {
            up: self.upload_total,
            down: self.download_total,
        }
    }
}

#[derive(Clone, Default, Deserialize, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    #[serde(default)]
    pub network: String,
    #[serde(default)]
    pub host: String,
    #[serde(default, rename = "destinationIP")]
    pub destination_ip: String,
    #[serde(default)]
    pub destination_port: String,
    #[serde(default, rename = "sourceIP")]
    pub source_ip: String,
    #[serde(default)]
    pub source_port: String,
    #[serde(default, alias = "pid", alias = "processPid", alias = "processPID")]
    pub process_id: Option<u32>,
    #[serde(default)]
    pub process: String,
    #[serde(default)]
    pub process_path: String,
    #[serde(default)]
    pub r#type: String,
}
#[derive(Clone, Default, Deserialize, Serialize, Debug)]
pub struct Connection {
    pub id: String,
    #[serde(default)]
    pub metadata: Metadata,
    #[serde(default)]
    pub upload: u64,
    #[serde(default)]
    pub download: u64,
    #[serde(default)]
    pub start: String,
    #[serde(default)]
    pub chains: Vec<String>,
    #[serde(default)]
    pub rule: String,
    #[serde(skip)]
    pub attribution: Option<crate::attribution::Attribution>,
}
impl Connection {
    pub fn application(&self) -> String {
        self.attribution
            .as_ref()
            .map(|value| value.owner.key.clone())
            .unwrap_or_else(|| self.app())
    }
    pub fn bytes(&self) -> Bytes {
        Bytes {
            up: self.upload,
            down: self.download,
        }
    }
    pub fn app(&self) -> String {
        if !self.metadata.process_path.is_empty() {
            self.metadata.process_path.clone()
        } else if !self.metadata.process.is_empty() {
            self.metadata.process.clone()
        } else {
            "未知进程".into()
        }
    }
    pub fn domain(&self) -> String {
        if self.metadata.host.is_empty() {
            self.metadata.destination_ip.clone()
        } else {
            self.metadata.host.clone()
        }
    }
    pub fn route(&self) -> String {
        self.chains
            .first()
            .cloned()
            .unwrap_or_else(|| "未知出站".into())
    }
    pub fn started(&self) -> i64 {
        chrono::DateTime::parse_from_rfc3339(&self.start)
            .map(|x| x.timestamp_millis())
            .unwrap_or(0)
    }
}
pub fn app_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}
pub fn human_bytes(v: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut n = v as f64;
    let mut i = 0;
    while n >= 1024. && i < 4 {
        n /= 1024.;
        i += 1;
    }
    if i == 0 {
        format!("{v} B")
    } else {
        format!("{n:.1} {}", units[i])
    }
}
pub fn speed(v: f64) -> String {
    format!("{}/s", human_bytes(v.max(0.) as u64))
}

#[derive(Clone, Default, Debug, serde::Serialize, serde::Deserialize)]
pub struct SeriesPoint {
    pub time: i64,
    pub up: f64,
    pub down: f64,
}
#[derive(Clone, Copy, Default, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LiveSample {
    pub from: i64,
    pub to: i64,
    pub up: f64,
    pub down: f64,
    pub valid: bool,
}
#[derive(Clone, Default, Debug, serde::Serialize, serde::Deserialize)]
pub struct Row {
    pub key: String,
    pub bytes: Bytes,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TrafficSort {
    #[default]
    Total,
    Download,
    Upload,
}

impl TrafficSort {
    pub const ALL: [Self; 3] = [Self::Total, Self::Download, Self::Upload];
    pub fn label(self) -> &'static str {
        match self {
            Self::Total => "总流量",
            Self::Download => "下载",
            Self::Upload => "上传",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Query {
    pub history: bool,
    pub history_context: Option<(i64, i64)>,
    pub group_apps: bool,
    pub group_domains: bool,
    pub from: i64,
    pub to: i64,
    pub dimension: String,
    pub search: String,
    pub app: String,
    pub page: usize,
    pub page_size: usize,
    pub ascending: bool,
    pub sort: TrafficSort,
}
impl Default for Query {
    fn default() -> Self {
        let (from, to) = crate::date_range::TimePeriod::Today
            .bounds(chrono::Local::now())
            .expect("valid local date")
            .expect("today is a preset");
        Self {
            history: false,
            history_context: None,
            group_apps: false,
            group_domains: false,
            from,
            to,
            dimension: "app".into(),
            search: String::new(),
            app: String::new(),
            page: 0,
            page_size: 50,
            ascending: false,
            sort: TrafficSort::Total,
        }
    }
}
impl Query {
    pub fn differs_only_by_page(&self, other: &Self) -> bool {
        self.page != other.page
            && *self
                == Self {
                    page: self.page,
                    ..other.clone()
                }
    }
}
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Report {
    pub row_speeds: std::collections::HashMap<String, crate::speed_stats::SpeedStats>,
    pub speeds: crate::speed_stats::SpeedStats,
    pub history: Option<std::sync::Arc<crate::history::Trend>>,
    pub histogram: crate::app_trend::Histogram,
    pub members: Vec<Row>,
    pub query: Option<Query>,
    pub total: Bytes,
    pub observed: Bytes,
    pub unknown: Bytes,
    pub recovered: Bytes,
    pub crossing_gaps: usize,
    pub gaps: usize,
    pub gap_rows: Vec<Gap>,
    pub rows: Vec<Row>,
    pub row_count: usize,
    pub app_count: usize,
    pub apps: Vec<Row>,
    pub domains: Vec<Row>,
    pub routes: Vec<Row>,
    pub sources: Vec<Row>,
    pub series: Vec<SeriesPoint>,
    pub archive_daily: bool,
    pub error: String,
}
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Gap {
    pub from: i64,
    pub to: i64,
    pub bytes: Bytes,
    pub reason: String,
}
#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct TaskbarGeometry {
    pub ready: bool,
    pub x: i32,
    pub max_x: i32,
    pub tray_start: i32,
    pub widget_width: i32,
    pub margin: i32,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ProcessControlStatus {
    #[default]
    Idle,
    Checking,
    Applying,
    Applied,
    Waiting,
    Failed,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Shared {
    pub subscriptions: crate::subscriptions::View,
    pub seconds_status: crate::speed_stats::SecondsStatus,
    pub application_rates: std::collections::HashMap<String, Bytes>,
    pub identities: std::collections::HashMap<String, crate::attribution::Owner>,
    pub connected: bool,
    pub status: String,
    pub endpoint: String,
    pub client_name: String,
    pub core_process_id: Option<u32>,
    pub core_identity: String,
    pub core_version: String,
    pub core_verified: bool,
    pub process_mode: String,
    pub process_status: ProcessControlStatus,
    pub process_error: String,
    pub permanent: crate::permanent_control::View,
    pub up: f64,
    pub down: f64,
    pub system: SystemMetrics,
    pub today: Bytes,
    pub month: Bytes,
    pub cumulative: Bytes,
    pub connections: Arc<Vec<Connection>>,
    pub live: VecDeque<LiveSample>,
    pub app_rates: std::collections::HashMap<String, Bytes>,
    #[serde(skip)]
    pub report: Arc<Report>,
    #[serde(skip)]
    pub report_id: u64,
    #[serde(skip)]
    pub detail: Arc<Report>,
    #[serde(skip)]
    pub detail_id: u64,
    pub taskbar_status: String,
    pub open_hotkey_active: String,
    pub open_hotkey_error: String,
    pub taskbar_geometry: TaskbarGeometry,
    pub notice: String,
    pub database_bytes: u64,
    pub storage_over_limit: bool,
    pub storage_revision: u64,
    pub storage_epoch: u64,
    pub storage_management: crate::storage_management::State,
}
impl Default for Shared {
    fn default() -> Self {
        Self {
            subscriptions: Default::default(),
            seconds_status: Default::default(),
            application_rates: Default::default(),
            identities: Default::default(),
            connected: false,
            status: "正在发现 Mihomo…".into(),
            endpoint: String::new(),
            client_name: String::new(),
            core_process_id: None,
            core_identity: String::new(),
            core_version: String::new(),
            core_verified: false,
            process_mode: String::new(),
            process_status: ProcessControlStatus::Idle,
            process_error: String::new(),
            permanent: Default::default(),
            up: 0.,
            down: 0.,
            system: SystemMetrics::default(),
            today: Bytes::default(),
            month: Bytes::default(),
            cumulative: Bytes::default(),
            connections: Default::default(),
            live: VecDeque::new(),
            app_rates: Default::default(),
            report: Default::default(),
            report_id: 0,
            detail: Default::default(),
            detail_id: 0,
            taskbar_status: "未启用".into(),
            open_hotkey_active: String::new(),
            open_hotkey_error: String::new(),
            taskbar_geometry: TaskbarGeometry::default(),
            notice: String::new(),
            database_bytes: 0,
            storage_over_limit: false,
            storage_revision: 0,
            storage_epoch: 0,
            storage_management: Default::default(),
        }
    }
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct SystemMetrics {
    pub cpu_percent: Option<f64>,
    /// Used and total physical memory available to Windows, in bytes.
    pub memory: Option<(u64, u64)>,
    pub gpus: Vec<GpuMetrics>,
    pub disks: Vec<DiskMetrics>,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct GpuMetrics {
    /// PCI adapter identity and enumeration index, independent of reboot LUIDs.
    pub id: String,
    pub name: String,
    pub percent: Option<f64>,
    pub memory: Option<(u64, u64)>,
    pub shared_memory: bool,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct DiskMetrics {
    pub id: String,
    pub name: String,
    pub percent: Option<f64>,
    pub read: Option<f64>,
    pub write: Option<f64>,
}
