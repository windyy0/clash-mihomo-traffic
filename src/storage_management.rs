//! Database-only accounting and serial, bounded maintenance. No automatic VACUUM.
use crate::{config::Config, model::Shared, speed_stats};
use anyhow::{Context, Result};
use chrono::{Days, Local, NaiveDate, TimeZone};
use rusqlite::{Connection, params};
use std::{
    collections::{HashMap, VecDeque},
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    pub detail_days: Option<u32>,
    pub seconds_days: Option<u32>,
    pub max_bytes: Option<u64>,
}
impl From<&Config> for Policy {
    fn from(c: &Config) -> Self {
        Self {
            detail_days: c.retention_days_enabled.then_some(c.retention_days),
            seconds_days: c.seconds_days_enabled.then_some(c.seconds_days),
            max_bytes: c
                .storage_limit_enabled
                .then_some(c.storage_limit_mb.saturating_mul(1024 * 1024)),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Target {
    Seconds,
    Details,
}
impl Target {
    pub fn label(self) -> &'static str {
        match self {
            Self::Seconds => "秒级明细",
            Self::Details => "历史详细记录",
        }
    }
}
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Snapshot {
    pub categories: [u64; 4],
    pub physical: u64,
    pub wal: u64,
    pub reusable: u64,
    pub at: i64,
}
impl Snapshot {
    pub fn used(&self) -> u64 {
        self.categories.iter().sum()
    }
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Preview {
    pub id: u64,
    pub epoch: u64,
    pub target: Target,
    pub cutoff: Option<i64>,
    pub fence: i64,
    pub first: Option<i64>,
    pub last: Option<i64>,
    pub rows: u64,
    pub batches: u64,
}
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct State {
    pub snapshot: Option<Snapshot>,
    pub refreshing: bool,
    pub error: String,
    pub preview: Option<Preview>,
    pub preview_id: u64,
    pub preview_error: String,
    pub busy: bool,
    pub reclaiming: bool,
    pub job: String,
    pub job_error: String,
    pub over_limit: bool,
    pub limit_note: String,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum Request {
    Refresh {
        epoch: u64,
    },
    Preview {
        id: u64,
        epoch: u64,
        target: Target,
        cutoff: Option<i64>,
    },
    Clean(Preview),
    Reclaim {
        epoch: u64,
    },
}
pub fn midnight(date: NaiveDate) -> Result<i64> {
    Local
        .from_local_datetime(&date.and_hms_opt(0, 0, 0).unwrap())
        .earliest()
        .map(|v| v.timestamp_millis())
        .context("无法确定该日期的本地零点")
}
pub fn days_before(now: i64, days: u32) -> Result<i64> {
    let date = chrono::DateTime::from_timestamp_millis(now)
        .context("无效时间")?
        .with_timezone(&Local)
        .date_naive();
    midnight(
        date.checked_sub_days(Days::new(u64::from(days)))
            .context("日期超出范围")?,
    )
}

pub fn snapshot(db: &Connection, path: &Path, now: i64) -> Result<Snapshot> {
    let tx = db.unchecked_transaction()?;
    let owners: HashMap<String, String> = tx
        .prepare("SELECT name,tbl_name FROM sqlite_schema WHERE type IN ('table','index')")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut result = Snapshot {
        at: now,
        ..Default::default()
    };
    let mut stmt = tx.prepare("SELECT name,SUM(pgsize) FROM dbstat GROUP BY name")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)))?;
    for row in rows {
        let (name, bytes) = row?;
        let table = owners.get(&name).map(String::as_str).unwrap_or(&name);
        let category = match table {
            "seconds_samples" | "seconds_apps" => 0,
            "details" | "attribution_details" | "events" => 1,
            "totals"
            | "app_minutes"
            | "app_attribution_minutes"
            | "daily"
            | "attribution_daily"
            | "speed_minutes"
            | "speed_app_minutes" => 2,
            _ => 3,
        };
        result.categories[category] += bytes;
    }
    drop(stmt);
    let page_size: u64 = tx.query_row("PRAGMA page_size", [], |r| r.get(0))?;
    let free: u64 = tx.query_row("PRAGMA freelist_count", [], |r| r.get(0))?;
    result.reusable = free * page_size;
    tx.commit()?;
    result.wal = std::fs::metadata(format!("{}-wal", path.display())).map_or(0, |m| m.len());
    result.physical = std::fs::metadata(path).map_or(0, |m| m.len()) + result.wal;
    Ok(result)
}
pub fn preview(
    db: &Connection,
    id: u64,
    epoch: u64,
    target: Target,
    cutoff: Option<i64>,
    now: i64,
) -> Result<Preview> {
    let tx = db.unchecked_transaction()?;
    let cutoff = if target == Target::Details {
        let previous: String = tx.query_row(
            "SELECT COALESCE((SELECT value FROM meta WHERE key='detail_cutoff'),'0')",
            [],
            |r| r.get(0),
        )?;
        Some(
            cutoff
                .unwrap_or(i64::MAX)
                .max(previous.parse::<i64>()?)
                .min(days_before(now, 0)?),
        )
    } else {
        cutoff
    };
    let mut p = Preview {
        id,
        epoch,
        target,
        cutoff,
        fence: i64::MAX,
        first: None,
        last: None,
        rows: 0,
        batches: 0,
    };
    if target == Target::Seconds {
        p.fence = tx.query_row(
            "SELECT COALESCE(MAX(seq),0) FROM seconds_samples",
            [],
            |r| r.get(0),
        )?;
        (p.first,p.last,p.batches,p.rows) = tx.query_row(
            "SELECT MIN(started),MAX(ended),COUNT(*),COUNT(*)+(SELECT COUNT(*) FROM seconds_apps a JOIN seconds_samples b ON a.seq=b.seq WHERE b.seq<=?1 AND b.ended<=?2) FROM seconds_samples WHERE seq<=?1 AND ended<=?2",
            params![p.fence,cutoff.unwrap_or(i64::MAX)], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
    } else {
        (p.first,p.last,p.rows) = tx.query_row("SELECT MIN(time),MAX(time),COUNT(*) FROM (SELECT time FROM details UNION ALL SELECT time FROM attribution_details UNION ALL SELECT time FROM events) WHERE time<?1",[cutoff.unwrap()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
    }
    tx.commit()?;
    Ok(p)
}
fn fenced_preview(db: &Connection, p: &Preview, now: i64) -> Result<Preview> {
    if p.target == Target::Details {
        return preview(db, p.id, p.epoch, p.target, p.cutoff, now);
    }
    let tx = db.unchecked_transaction()?;
    let mut current = p.clone();
    (current.first,current.last,current.batches,current.rows)=tx.query_row(
        "SELECT MIN(started),MAX(ended),COUNT(*),COUNT(*)+(SELECT COUNT(*) FROM seconds_apps a JOIN seconds_samples b ON a.seq=b.seq WHERE b.seq<=?1 AND b.ended<=?2) FROM seconds_samples WHERE seq<=?1 AND ended<=?2",
        params![p.fence,p.cutoff.unwrap_or(i64::MAX)],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
    tx.commit()?;
    Ok(current)
}
#[derive(Default)]
pub struct Removed {
    pub rows: u64,
    pub batches: u64,
}
// Complete raw batches; cap a transaction at 512 batches / ~5000 app rows.
pub fn clean_seconds(db: &mut Connection, cutoff: Option<i64>, fence: i64) -> Result<Removed> {
    let tx = db.transaction()?;
    let items: Vec<(i64,u64)> = tx.prepare("SELECT s.seq,1+(SELECT COUNT(*) FROM seconds_apps a WHERE a.seq=s.seq) FROM seconds_samples s WHERE s.seq<=?1 AND s.ended<=?2 ORDER BY ended,seq LIMIT 512")?
        .query_map(params![fence,cutoff.unwrap_or(i64::MAX)],|r|Ok((r.get(0)?,r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    let mut result = Removed::default();
    for (seq, count) in items {
        if result.batches > 0 && result.rows + count > 5000 {
            break;
        }
        tx.execute("DELETE FROM seconds_apps WHERE seq=?1", [seq])?;
        tx.execute("DELETE FROM seconds_samples WHERE seq=?1", [seq])?;
        result.batches += 1;
        result.rows += count;
    }
    tx.commit()?;
    Ok(result)
}
// Publish the archive boundary atomically with the first chunk. Readers then
// use the already-maintained daily summaries, even while old pages are removed.
pub fn clean_details(db: &mut Connection, cutoff: i64) -> Result<Removed> {
    let tx = db.transaction()?;
    let previous: String = tx.query_row(
        "SELECT COALESCE((SELECT value FROM meta WHERE key='detail_cutoff'),'0')",
        [],
        |r| r.get(0),
    )?;
    let cutoff = cutoff.max(previous.parse::<i64>()?);
    tx.execute(
        "INSERT OR REPLACE INTO meta VALUES('detail_cutoff',?1)",
        [cutoff.to_string()],
    )?;
    let mut result = Removed::default();
    for table in ["details", "attribution_details", "events"] {
        let remaining = 5000 - result.rows;
        if remaining == 0 {
            break;
        }
        result.rows += tx.execute(&format!("DELETE FROM {table} WHERE rowid IN (SELECT rowid FROM {table} WHERE time<?1 LIMIT ?2)"),params![cutoff,remaining])? as u64;
    }
    tx.commit()?;
    Ok(result)
}
fn oldest_detail_cutoff(db: &Connection, now: i64) -> Result<Option<i64>> {
    let today = days_before(now, 0)?;
    let oldest: Option<i64> = db.query_row("SELECT MIN(time) FROM (SELECT time FROM details UNION ALL SELECT time FROM attribution_details UNION ALL SELECT time FROM events) WHERE time<?1",[today],|r|r.get(0))?;
    oldest
        .map(|v| {
            let date = chrono::DateTime::from_timestamp_millis(v)
                .context("无效时间")?
                .with_timezone(&Local)
                .date_naive()
                .succ_opt()
                .context("日期超出范围")?;
            Ok(midnight(date)?.min(today))
        })
        .transpose()
}
pub fn reclaim(db: &Connection) -> Result<()> {
    let busy: i64 = db.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))?;
    anyhow::ensure!(busy == 0, "数据库仍有任务占用，请稍后重试");
    db.execute_batch("VACUUM;")?;
    let busy: i64 = db.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))?;
    anyhow::ensure!(busy == 0, "整理已完成，写入日志暂未截断，请稍后刷新");
    Ok(())
}

enum Phase {
    Seconds(Option<i64>),
    Details(i64),
    Measure,
    CapacitySeconds(u64),
    CapacityDetails(i64),
}
struct Task {
    preview: Preview,
    removed: Removed,
}
pub struct Worker {
    path: std::path::PathBuf,
    pub shared: Arc<Mutex<Shared>>,
    previous: Option<Policy>,
    last: Instant,
    phases: VecDeque<Phase>,
    task: Option<Task>,
    pub refresh: bool,
    epoch: u64,
    retry_at: Option<Instant>,
    automatic: Removed,
}
impl Worker {
    pub fn new(path: std::path::PathBuf, shared: Arc<Mutex<Shared>>) -> Self {
        Self {
            path,
            shared,
            previous: None,
            last: Instant::now(),
            phases: VecDeque::new(),
            task: None,
            refresh: true,
            epoch: u64::MAX,
            retry_at: None,
            automatic: Removed::default(),
        }
    }
    pub fn read_step(&self) -> bool {
        self.task.is_none() && matches!(self.phases.front(), Some(Phase::Measure))
    }
    pub fn read_capacity_step(&mut self, db: &Connection, policy: Policy, now: i64) -> Result<()> {
        anyhow::ensure!(self.read_step(), "容量统计阶段已变化");
        self.phases.pop_front();
        self.capacity_measure(db, policy, now)
    }
    fn capacity_measure(&mut self, db: &Connection, policy: Policy, now: i64) -> Result<()> {
        self.refresh = true;
        let snap = snapshot(db, &self.path, now)?;
        let mut note = String::new();
        if let Some(limit) = policy.max_bytes
            && snap.used() > limit
        {
            let target = snap.categories[0].saturating_sub(snap.used() - limit);
            let budget = speed_stats::capacity_budget(
                db,
                speed_stats::SecondsPolicy {
                    enabled: false,
                    days: None,
                    max_bytes: Some(target),
                },
            )?;
            let raw: bool =
                db.query_row("SELECT EXISTS(SELECT 1 FROM seconds_samples)", [], |r| {
                    r.get(0)
                })?;
            if raw && budget > 0 {
                note = "正在按统一上限分批清理最旧秒级记录。".into();
                self.phases.push_back(Phase::CapacitySeconds(budget));
            } else if let Some(cutoff) = oldest_detail_cutoff(db, now)? {
                note = "正在按统一上限清理今天以前的历史详细记录。".into();
                self.phases.push_back(Phase::CapacityDetails(cutoff));
            } else {
                note="已无可清理记录：剩余为长期汇总、系统与归属数据、当天明细及必需的空表索引；不会删除汇总。".into();
            }
        }
        self.shared.lock().unwrap().storage_management.limit_note = note;
        Ok(())
    }
    pub fn reset(&mut self, epoch: u64) {
        if self.epoch != epoch {
            self.epoch = epoch;
            self.retry_at = None;
            self.automatic = Removed::default();
            self.previous = None;
            self.phases.clear();
            self.task = None;
            self.refresh = true;
            self.shared.lock().unwrap().storage_management = State::default();
        }
    }
    pub fn request(&mut self, db: &Connection, request: Request, now: i64) -> Result<()> {
        match request {
            Request::Refresh { epoch } => {
                anyhow::ensure!(epoch == self.epoch, "数据库已替换，请刷新");
                self.refresh = true;
            }
            Request::Preview {
                id,
                epoch,
                target,
                cutoff,
            } => {
                let result = if epoch == self.epoch {
                    preview(db, id, epoch, target, cutoff, now)
                } else {
                    anyhow::bail!("数据库已替换，请重新预览")
                };
                let mut s = self.shared.lock().unwrap();
                s.storage_management.preview_id = id;
                match result {
                    Ok(p) => {
                        s.storage_management.preview = Some(p);
                        s.storage_management.preview_error.clear();
                    }
                    Err(e) => {
                        s.storage_management.preview = None;
                        s.storage_management.preview_error = e.to_string();
                    }
                }
            }
            Request::Clean(p) => {
                anyhow::ensure!(p.epoch == self.epoch, "数据库已替换，请重新预览");
                anyhow::ensure!(self.task.is_none(), "清理任务仍在执行");
                anyhow::ensure!(p.rows > 0, "没有可清理数据");
                let current = fenced_preview(db, &p, now)?;
                // A new raw batch is outside the immutable preview fence. Historical
                // late writes require a new preview so the confirmation stays exact.
                anyhow::ensure!(current == p, "可清理记录已变化，请重新预览");
                self.phases.clear();
                self.retry_at = None;
                self.task = Some(Task {
                    preview: p,
                    removed: Removed::default(),
                });
                let mut s = self.shared.lock().unwrap();
                s.storage_management.busy = true;
                s.storage_management.job_error.clear();
                s.storage_management.job = "正在清理…".into();
            }
            Request::Reclaim { .. } => unreachable!(),
        }
        Ok(())
    }
    pub fn measure(&mut self, db: &Connection, policy: Policy, now: i64) -> Result<()> {
        let snap = snapshot(db, &self.path, now)?;
        let mut status = speed_stats::status(db)?;
        status.bytes = snap.categories[0];
        let mut s = self.shared.lock().unwrap();
        let revision = s.seconds_status.revision;
        let error = std::mem::take(&mut s.seconds_status.error);
        s.seconds_status = status;
        s.seconds_status.error = error;
        s.seconds_status.revision = revision;
        s.database_bytes = snap.physical;
        s.storage_over_limit = policy.max_bytes.is_some_and(|max| snap.used() > max);
        s.storage_management.over_limit = s.storage_over_limit;
        if !s.storage_over_limit {
            s.storage_management.limit_note.clear();
        }
        s.storage_management.snapshot = Some(snap);
        s.storage_management.refreshing = false;
        s.storage_management.error.clear();
        self.refresh = false;
        Ok(())
    }
    pub fn changed(&self, raw: bool) {
        let mut s = self.shared.lock().unwrap();
        s.storage_revision = s.storage_revision.wrapping_add(1);
        if raw {
            s.seconds_status.revision = s.seconds_status.revision.wrapping_add(1);
        }
    }
    pub fn failed(&mut self, error: &anyhow::Error) {
        let mut s = self.shared.lock().unwrap();
        if let Some(task) = self.task.take() {
            s.storage_management.job_error = format!(
                "清理未全部完成：{error}；已清理 {} 条记录、{} 个批次。可重新预览后重试。",
                task.removed.rows, task.removed.batches
            );
            s.storage_management.busy = false;
        } else {
            s.storage_management.error = format!(
                "存储维护未全部完成：{error}；已清理 {} 条记录、{} 个批次，稍后重试",
                self.automatic.rows, self.automatic.batches
            );
        }
        s.storage_management.refreshing = false;
        self.phases.clear();
        self.refresh = false;
        self.retry_at = Some(Instant::now() + Duration::from_secs(60));
    }
    // Caller holds the write-serialization mutex, for one bounded transaction only.
    pub fn step(&mut self, db: &mut Connection, policy: Policy, now: i64) -> Result<bool> {
        if let Some(task) = self.task.as_mut() {
            let removed = match task.preview.target {
                Target::Seconds => clean_seconds(db, task.preview.cutoff, task.preview.fence)?,
                Target::Details => clean_details(db, task.preview.cutoff.unwrap())?,
            };
            let done = removed.rows == 0;
            task.removed.rows += removed.rows;
            task.removed.batches += removed.batches;
            let message = format!(
                "{} · 已清理 {} 条记录、{} 个秒级批次",
                if done { "清理完成" } else { "正在清理" },
                task.removed.rows,
                task.removed.batches
            );
            let raw = task.preview.target == Target::Seconds;
            if done {
                self.task = None;
                self.previous = None;
                self.refresh = true;
            }
            {
                let mut s = self.shared.lock().unwrap();
                s.storage_management.job = message;
                s.storage_management.busy = !done;
            }
            if !done {
                self.changed(raw);
            }
            return Ok(!done);
        }
        if self.retry_at.is_some_and(|at| Instant::now() < at)
            && self.previous.is_none_or(|old| old == policy)
        {
            return Ok(false);
        }
        self.retry_at = None;
        if self.previous != Some(policy) || self.last.elapsed() >= Duration::from_secs(60) {
            self.phases.clear();
            self.automatic = Removed::default();
            if let Some(days) = policy.seconds_days {
                self.phases
                    .push_back(Phase::Seconds(Some(now - i64::from(days) * 86_400_000)));
            }
            // Resume unfinished physical deletion below the persisted archive boundary.
            self.phases.push_back(Phase::Details(
                policy
                    .detail_days
                    .map(|days| days_before(now, days))
                    .transpose()?
                    .unwrap_or(0),
            ));
            self.phases.push_back(Phase::Measure);
            self.previous = Some(policy);
            self.last = Instant::now();
        }
        let Some(phase) = self.phases.pop_front() else {
            return Ok(false);
        };
        match phase {
            Phase::Seconds(cutoff) => {
                let removed = clean_seconds(db, cutoff, i64::MAX)?;
                self.automatic.rows += removed.rows;
                self.automatic.batches += removed.batches;
                if removed.rows > 0 {
                    self.changed(true);
                    self.phases.push_front(Phase::Seconds(cutoff));
                }
            }
            Phase::Details(cutoff) => {
                let removed = clean_details(db, cutoff)?;
                self.automatic.rows += removed.rows;
                if removed.rows > 0 {
                    self.changed(false);
                    self.phases.push_front(Phase::Details(cutoff));
                }
            }
            Phase::Measure => self.capacity_measure(db, policy, now)?,
            Phase::CapacitySeconds(mut budget) => {
                let result = clean_seconds(db, None, i64::MAX)?;
                budget = budget.saturating_sub(result.rows);
                self.automatic.rows += result.rows;
                self.automatic.batches += result.batches;
                let removed = result.rows > 0;
                if removed {
                    self.changed(true);
                }
                self.phases.push_back(if removed && budget > 0 {
                    Phase::CapacitySeconds(budget)
                } else {
                    Phase::Measure
                });
            }
            Phase::CapacityDetails(cutoff) => {
                let result = clean_details(db, cutoff)?;
                self.automatic.rows += result.rows;
                let removed = result.rows > 0;
                if removed {
                    self.changed(false);
                }
                self.phases.push_back(if removed {
                    Phase::CapacityDetails(cutoff)
                } else {
                    Phase::Measure
                });
            }
        }
        if self.phases.is_empty() {
            self.refresh = true;
        }
        Ok(true)
    }
}

#[cfg(test)]
#[path = "storage_management_tests.rs"]
mod tests;
