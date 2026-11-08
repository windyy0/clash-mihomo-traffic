use crate::{
    model::Shared,
    speed_stats::{self, Sample, SecondsPolicy, SecondsReply, SecondsRequest},
};
use anyhow::Result;
use rusqlite::{Connection, OpenFlags};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

struct ResumeOnDrop(Arc<AtomicBool>);
impl Drop for ResumeOnDrop {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

pub struct Service {
    pub storage_gate: Arc<Mutex<()>>,
    pub access: Arc<RwLock<()>>,
    pub pause: Arc<AtomicBool>,
    pub paused: Arc<AtomicBool>,
    interrupt: Arc<Mutex<Option<rusqlite::InterruptHandle>>>,
    #[cfg(test)]
    reclaim_fault: Arc<AtomicBool>,
    management_tx: mpsc::SyncSender<crate::storage_management::Request>,
    sample_tx: mpsc::SyncSender<Sample>,
    query_tx: mpsc::SyncSender<SecondsRequest>,
    pub reply: Arc<Mutex<Option<SecondsReply>>>,
    hover_tx: mpsc::SyncSender<crate::traffic_drill::HoverRequest>,
    pub hover_reply: Arc<Mutex<Option<crate::traffic_drill::HoverReply>>>,
    threads: Mutex<Vec<std::thread::JoinHandle<()>>>,
}

impl Service {
    pub fn start(
        path: PathBuf,
        shared: Arc<Mutex<Shared>>,
        stop: Arc<AtomicBool>,
        policy: Arc<Mutex<SecondsPolicy>>,
        management_policy: Arc<Mutex<crate::storage_management::Policy>>,
        ready: Arc<tokio::sync::Notify>,
    ) -> Result<Self> {
        let (management_tx, management_rx) =
            mpsc::sync_channel::<crate::storage_management::Request>(16);
        let access = Arc::new(RwLock::new(()));
        let pause = Arc::new(AtomicBool::new(false));
        let paused = Arc::new(AtomicBool::new(false));
        let interrupt = Arc::new(Mutex::new(None));
        let winterrupt = interrupt.clone();
        #[cfg(test)]
        let reclaim_fault = Arc::new(AtomicBool::new(false));
        #[cfg(test)]
        let writer_fault = reclaim_fault.clone();
        let qaccess = access.clone();
        let waccess = access.clone();
        let wpause = pause.clone();
        let wpaused = paused.clone();
        let (sample_tx, samples) = mpsc::sync_channel::<Sample>(128);
        let (query_tx, queries) = mpsc::sync_channel::<SecondsRequest>(4);
        let (hover_tx, hovers) = mpsc::sync_channel::<crate::traffic_drill::HoverRequest>(4);
        let hover_reply = Arc::new(Mutex::new(None));
        let hover_thread = start_hover_reader(
            path.clone(),
            shared.clone(),
            stop.clone(),
            access.clone(),
            ready.clone(),
            hovers,
            hover_reply.clone(),
        )?;
        let reply = Arc::new(Mutex::new(None));
        let storage_gate = Arc::new(Mutex::new(()));
        let writer_gate = storage_gate.clone();
        let qreply = reply.clone();
        let qshared = shared.clone();
        let qstop = stop.clone();
        let qready = ready.clone();
        let qpath = path.clone();
        let reader = std::thread::Builder::new()
            .name("seconds-query".into())
            .spawn(move || {
                let mut db = None;
                let mut epoch = u64::MAX;
                while !qstop.load(Ordering::Acquire) {
                    let Ok(mut request) = queries.recv_timeout(Duration::from_millis(250)) else {
                        continue;
                    };
                    while let Ok(newer) = queries.try_recv() {
                        request = newer;
                    }
                    let result = (|| -> Result<_> {
                        let _access = qaccess.read().unwrap();
                        let state = qshared.lock().unwrap();
                        let source = state.storage_epoch;
                        anyhow::ensure!(
                            request.generation == state.storage_revision
                                && request.revision == state.seconds_status.revision,
                            "秒级数据已变化，请刷新"
                        );
                        drop(state);
                        if db.is_none() || source != epoch {
                            epoch = source;
                            let conn = Connection::open_with_flags(
                                &qpath,
                                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
                            )?;
                            conn.busy_timeout(Duration::from_millis(500))?;
                            db = Some(conn);
                        }
                        let conn = db.as_mut().unwrap();
                        let tx = conn.transaction()?;
                        let result = speed_stats::read_seconds(&tx, &request)?;
                        tx.commit()?;
                        Ok(result)
                    })()
                    .map_err(|e| e.to_string());
                    if qshared.lock().unwrap().storage_revision == request.generation {
                        *qreply.lock().unwrap() = Some(SecondsReply { request, result });
                        qready.notify_one();
                    }
                }
            })?;
        let writer = std::thread::Builder::new()
            .name("seconds-store".into())
            .spawn(move || {
                let mut db = None;
                let mut generation = u64::MAX;
                let mut manager =
                    crate::storage_management::Worker::new(path.clone(), shared.clone());
                while !stop.load(Ordering::Acquire) {
                    let sample = samples.recv_timeout(Duration::from_millis(25)).ok();
                    let source = shared.lock().unwrap().storage_epoch;
                    let result = (|| -> Result<()> {
                        let _access = waccess.read().unwrap();
                        if db.is_none() || source != generation {
                            let conn = Connection::open(&path)?;
                            conn.busy_timeout(Duration::from_millis(500))?;
                            conn.execute_batch("PRAGMA foreign_keys=ON;")?;
                            *winterrupt.lock().unwrap() = Some(conn.get_interrupt_handle());
                            db = Some(conn);
                            generation = source;
                            manager.reset(source);
                        }
                        if shared.lock().unwrap().storage_epoch != source {
                            return Ok(());
                        }
                        let db = db.as_mut().unwrap();
                        if let Some(sample) = sample
                            && sample.generation == source
                        {
                            let _guard = writer_gate.lock().unwrap();
                            if policy.lock().unwrap().enabled {
                                match speed_stats::write_seconds(db, &sample) {
                                    Ok(_) => {
                                        let mut s = shared.lock().unwrap();
                                        s.seconds_status.first = Some(
                                            s.seconds_status
                                                .first
                                                .map_or(sample.from, |v| v.min(sample.from)),
                                        );
                                        s.seconds_status.last = Some(
                                            s.seconds_status
                                                .last
                                                .map_or(sample.to, |v| v.max(sample.to)),
                                        );
                                        s.seconds_status.error.clear();
                                    }
                                    Err(e) => {
                                        shared.lock().unwrap().seconds_status.error =
                                            format!("秒级明细：{e}");
                                        ready.notify_one();
                                    }
                                }
                            }
                        }
                        let current = *management_policy.lock().unwrap();
                        if let Ok(request) = management_rx.try_recv() {
                            if let crate::storage_management::Request::Reclaim { epoch } = request {
                                drop(_access);
                                let resume = ResumeOnDrop(wpause.clone());
                                let result = (|| -> Result<()> {
                                    anyhow::ensure!(
                                        epoch == shared.lock().unwrap().storage_epoch,
                                        "数据库已替换，请重新确认"
                                    );
                                    {
                                        let mut s = shared.lock().unwrap();
                                        s.storage_management.busy = true;
                                        s.storage_management.reclaiming = true;
                                        s.storage_management.job =
                                            "正在暂停采集并等待数据库任务…".into();
                                        s.storage_management.job_error.clear();
                                    }
                                    ready.notify_one();
                                    wpaused.store(false, Ordering::Release);
                                    wpause.store(true, Ordering::Release);
                                    let deadline = Instant::now() + Duration::from_secs(10);
                                    while !wpaused.load(Ordering::Acquire) {
                                        anyhow::ensure!(
                                            !stop.load(Ordering::Acquire),
                                            "程序正在退出"
                                        );
                                        anyhow::ensure!(
                                            Instant::now() < deadline,
                                            "等待采集暂停超时"
                                        );
                                        std::thread::sleep(Duration::from_millis(20));
                                    }
                                    {
                                        let _access = waccess.read().unwrap();
                                        let _guard = writer_gate.lock().unwrap();
                                        while let Ok(sample) = samples.try_recv() {
                                            if policy.lock().unwrap().enabled
                                                && sample.generation == epoch
                                            {
                                                speed_stats::write_seconds(db, &sample)?;
                                            }
                                        }
                                    }
                                    let deadline = Instant::now() + Duration::from_secs(10);
                                    let _exclusive = loop {
                                        if let Ok(guard) = waccess.try_write() {
                                            break guard;
                                        }
                                        anyhow::ensure!(
                                            !stop.load(Ordering::Acquire)
                                                && Instant::now() < deadline,
                                            "等待数据库查询退出超时"
                                        );
                                        std::thread::sleep(Duration::from_millis(20));
                                    };
                                    let _guard = writer_gate.lock().unwrap();
                                    anyhow::ensure!(
                                        epoch == shared.lock().unwrap().storage_epoch,
                                        "数据库已替换，请重新确认"
                                    );
                                    shared.lock().unwrap().storage_management.job =
                                        "正在回收文件空间…".into();
                                    #[cfg(test)]
                                    if writer_fault.load(Ordering::Acquire) {
                                        return Err(rusqlite::Error::SqliteFailure(
                                            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_FULL),
                                            None,
                                        )
                                        .into());
                                    }
                                    crate::storage_management::reclaim(db)?;
                                    manager.changed(false);
                                    Ok(())
                                })();
                                drop(resume);
                                {
                                    let mut s = shared.lock().unwrap();
                                    s.storage_management.busy = false;
                                    s.storage_management.reclaiming = false;
                                    match result {
                                        Ok(()) => {
                                            s.storage_management.job =
                                                "文件空间回收完成，采集正在恢复".into()
                                        }
                                        Err(e) => {
                                            s.storage_management.job = "采集正在恢复".into();
                                            s.storage_management.job_error =
                                                format!("文件空间回收失败：{e}");
                                        }
                                    }
                                }
                                manager.refresh = true;
                                ready.notify_one();
                                return Ok(());
                            }
                            let _guard = if matches!(
                                request,
                                crate::storage_management::Request::Clean(_)
                            ) {
                                Some(writer_gate.lock().unwrap())
                            } else {
                                None
                            };
                            if let Err(e) =
                                manager.request(db, request, chrono::Utc::now().timestamp_millis())
                            {
                                let mut state = shared.lock().unwrap();
                                state.storage_management.job_error = e.to_string();
                                state.storage_management.busy = false;
                            }
                            ready.notify_one();
                        }
                        let now = chrono::Utc::now().timestamp_millis();
                        if manager.read_step() {
                            manager.read_capacity_step(db, current, now)?;
                        } else {
                            let _guard = writer_gate.lock().unwrap();
                            let latest = *management_policy.lock().unwrap();
                            manager.step(db, latest, now)?;
                        }
                        if manager.refresh {
                            manager.measure(db, current, now)?;
                            ready.notify_one();
                        }
                        Ok(())
                    })();
                    if let Err(error) = result {
                        manager.failed(&error);
                        ready.notify_one();
                        std::thread::sleep(Duration::from_millis(250));
                    }
                }
                wpause.store(false, Ordering::Release);
                if let Some(db) = db.as_mut() {
                    let _access = waccess.read().unwrap();
                    let _guard = writer_gate.lock().unwrap();
                    while let Ok(sample) = samples.try_recv() {
                        if policy.lock().unwrap().enabled
                            && sample.generation == shared.lock().unwrap().storage_epoch
                        {
                            let _ = speed_stats::write_seconds(db, &sample);
                        }
                    }
                }
            })?;
        Ok(Self {
            storage_gate,
            access,
            pause,
            paused,
            interrupt,
            #[cfg(test)]
            reclaim_fault,
            management_tx,
            sample_tx,
            query_tx,
            reply,
            hover_tx,
            hover_reply,
            threads: Mutex::new(vec![reader, writer, hover_thread]),
        })
    }
    pub fn interrupt(&self) {
        if let Some(handle) = self.interrupt.lock().unwrap().as_ref() {
            handle.interrupt();
        }
    }
    pub fn manage(&self, request: crate::storage_management::Request) -> bool {
        self.management_tx.try_send(request).is_ok()
    }
    pub fn record(&self, sample: Sample) -> bool {
        self.sample_tx.try_send(sample).is_ok()
    }
    pub fn query(&self, request: SecondsRequest) -> bool {
        self.query_tx.try_send(request).is_ok()
    }
    pub fn hover(&self, request: crate::traffic_drill::HoverRequest) -> bool {
        self.hover_tx.try_send(request).is_ok()
    }
    pub fn finish(&self) {
        for thread in std::mem::take(&mut *self.threads.lock().unwrap()) {
            let _ = thread.join();
        }
    }
}

fn start_hover_reader(
    path: PathBuf,
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
    access: Arc<RwLock<()>>,
    ready: Arc<tokio::sync::Notify>,
    queries: mpsc::Receiver<crate::traffic_drill::HoverRequest>,
    reply: Arc<Mutex<Option<crate::traffic_drill::HoverReply>>>,
) -> Result<std::thread::JoinHandle<()>> {
    Ok(std::thread::Builder::new()
        .name("drill-hover-query".into())
        .spawn(move || {
            let mut db = None;
            let mut epoch = u64::MAX;
            while !stop.load(Ordering::Acquire) {
                let Ok(mut request) = queries.recv_timeout(Duration::from_millis(100)) else {
                    continue;
                };
                while let Ok(newer) = queries.try_recv() {
                    request = newer;
                }
                let result = (|| -> Result<_> {
                    let _access = access.read().unwrap();
                    let state = shared.lock().unwrap();
                    anyhow::ensure!(
                        request.window.generation == state.storage_revision
                            && request.window.revision == state.seconds_status.revision,
                        "数据已变化，请刷新"
                    );
                    let source = state.storage_epoch;
                    drop(state);
                    if db.is_none() || epoch != source {
                        let conn = Connection::open_with_flags(
                            &path,
                            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
                        )?;
                        conn.busy_timeout(Duration::from_millis(500))?;
                        db = Some(conn);
                        epoch = source;
                    }
                    let tx = db.as_mut().unwrap().transaction()?;
                    let result = crate::traffic_drill::read_hover(&tx, &request)?;
                    tx.commit()?;
                    Ok(result)
                })()
                .map_err(|e| e.to_string());
                let state = shared.lock().unwrap();
                if request.window.generation == state.storage_revision
                    && request.window.revision == state.seconds_status.revision
                {
                    *reply.lock().unwrap() =
                        Some(crate::traffic_drill::HoverReply { request, result });
                    ready.notify_one();
                }
            }
        })?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        accounting::{Batch, Ledger},
        model::{Bytes, Query},
        speed_stats::{Scope, SecondsRequest},
        storage::Storage,
    };
    fn wait(mut condition: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !condition() {
            assert!(
                Instant::now() < deadline,
                "seconds worker condition timed out"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    #[test]
    fn unified_reclaim_resumes_collection_on_success_busy_failure_and_rejects_old_epoch() {
        use crate::storage_management::{Policy, Request};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("traffic.sqlite3");
        let mut store = Storage::open(&path).unwrap();
        let shared = Arc::new(Mutex::new(Shared::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let service = Arc::new(
            Service::start(
                path.clone(),
                shared.clone(),
                stop.clone(),
                Arc::new(Mutex::new(SecondsPolicy {
                    enabled: false,
                    days: None,
                    max_bytes: None,
                })),
                Arc::new(Mutex::new(Policy {
                    detail_days: None,
                    seconds_days: None,
                    max_bytes: None,
                })),
                Arc::new(tokio::sync::Notify::new()),
            )
            .unwrap(),
        );
        let ack_service = service.clone();
        let ack_stop = stop.clone();
        let sampler = std::thread::spawn(move || {
            while !ack_stop.load(Ordering::Acquire) {
                ack_service
                    .paused
                    .store(ack_service.pause.load(Ordering::Acquire), Ordering::Release);
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        wait(|| shared.lock().unwrap().storage_management.snapshot.is_some());
        assert!(service.manage(Request::Reclaim { epoch: 0 }));
        wait(|| {
            shared
                .lock()
                .unwrap()
                .storage_management
                .job
                .contains("回收完成")
        });
        assert!(!service.pause.load(Ordering::Acquire));
        assert!(!shared.lock().unwrap().storage_management.busy);
        // An external pinned snapshot prevents checkpointing. It must fail and resume.
        let pinned = Connection::open(&path).unwrap();
        pinned.execute_batch("BEGIN;SELECT * FROM totals;").unwrap();
        store
            .commit(
                &Ledger {
                    sequence: 1,
                    time: 1000,
                    ..Default::default()
                },
                &Batch {
                    totals: vec![(0, Bytes { up: 1, down: 2 })],
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(service.manage(Request::Reclaim { epoch: 0 }));
        wait(|| {
            !shared
                .lock()
                .unwrap()
                .storage_management
                .job_error
                .is_empty()
        });
        assert!(!service.pause.load(Ordering::Acquire));
        assert!(!shared.lock().unwrap().storage_management.busy);
        pinned.execute_batch("ROLLBACK").unwrap();
        // Failure injection exercises the same pause/exclusive-lock/finally path
        // without filling the user's physical drive.
        service.reclaim_fault.store(true, Ordering::Release);
        shared.lock().unwrap().storage_management.job_error.clear();
        assert!(service.manage(Request::Reclaim { epoch: 0 }));
        wait(|| {
            shared
                .lock()
                .unwrap()
                .storage_management
                .job_error
                .to_lowercase()
                .contains("full")
        });
        assert!(!service.pause.load(Ordering::Acquire));
        assert!(!shared.lock().unwrap().storage_management.busy);
        service.reclaim_fault.store(false, Ordering::Release);
        {
            let mut s = shared.lock().unwrap();
            s.storage_epoch = 1;
            s.storage_management.job_error.clear();
        }
        assert!(service.manage(Request::Reclaim { epoch: 0 }));
        wait(|| {
            shared
                .lock()
                .unwrap()
                .storage_management
                .job_error
                .contains("数据库已替换")
        });
        assert!(!service.pause.load(Ordering::Acquire));
        stop.store(true, Ordering::Release);
        service.finish();
        sampler.join().unwrap();
        assert_eq!(
            store.total(i64::MIN, i64::MAX).unwrap(),
            Bytes { up: 1, down: 2 }
        );
    }
    #[test]
    fn independent_writer_failure_policy_changes_disabled_cleanup_and_stale_queries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("traffic.sqlite3");
        let mut store = Storage::open(&path).unwrap();
        let shared = Arc::new(Mutex::new(Shared::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let policy = Arc::new(Mutex::new(SecondsPolicy {
            enabled: false,
            days: Some(30),
            max_bytes: None,
        }));
        let maintenance = Arc::new(Mutex::new(crate::storage_management::Policy {
            detail_days: None,
            seconds_days: None,
            max_bytes: None,
        }));
        let service = Service::start(
            path.clone(),
            shared.clone(),
            stop.clone(),
            policy.clone(),
            maintenance.clone(),
            Arc::new(tokio::sync::Notify::new()),
        )
        .unwrap();
        let mut reader = Connection::open(&path).unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        let sample = |seq, from| Sample {
            generation: 0,
            seq,
            from,
            to: from + 1000,
            duration_ms: 1000,
            bytes: Bytes {
                down: 1000,
                up: 500,
            },
            apps: vec![],
        };
        assert!(service.record(sample(1, now - 6000)));
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(
            reader
                .query_row("SELECT COUNT(*) FROM seconds_samples", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        policy.lock().unwrap().enabled = true;
        assert!(service.record(sample(2, now - 5000)));
        wait(|| shared.lock().unwrap().seconds_status.last == Some(now - 4000));
        reader.execute_batch("CREATE TRIGGER fail_raw BEFORE INSERT ON seconds_samples BEGIN SELECT RAISE(FAIL,'forced independent failure'); END;").unwrap();
        assert!(service.record(sample(3, now - 4000)));
        wait(|| {
            shared
                .lock()
                .unwrap()
                .seconds_status
                .error
                .contains("forced independent failure")
        });
        let b = Batch {
            sample_from: Some(now - 4000),
            rate: Bytes { down: 10, up: 5 },
            totals: crate::accounting::split(now - 4000, now - 3000, Bytes { down: 10, up: 5 }),
            ..Default::default()
        };
        assert!(
            store
                .commit(
                    &Ledger {
                        time: now - 3000,
                        sequence: 1,
                        ..Default::default()
                    },
                    &b
                )
                .unwrap()
        );
        reader.execute_batch("DROP TRIGGER fail_raw;").unwrap();
        assert!(service.record(sample(4, now - 3000)));
        wait(|| shared.lock().unwrap().seconds_status.last == Some(now - 2000));
        assert_eq!(
            reader
                .query_row(
                    "SELECT COUNT(*) FROM seconds_samples WHERE seq=3",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        policy.lock().unwrap().enabled = false;
        {
            let _guard = service.storage_gate.lock().unwrap();
            policy.lock().unwrap().days = None;
            maintenance.lock().unwrap().seconds_days = None;
            speed_stats::write_seconds(&mut reader, &sample(6, now - 31 * 86_400_000)).unwrap();
        }
        assert!(service.manage(crate::storage_management::Request::Refresh { epoch: 0 }));
        wait(|| shared.lock().unwrap().seconds_status.first == Some(now - 31 * 86_400_000));
        assert_eq!(
            reader
                .query_row(
                    "SELECT COUNT(*) FROM seconds_samples WHERE seq=6",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
        {
            let guard = service.storage_gate.lock().unwrap();
            maintenance.lock().unwrap().seconds_days = Some(1);
            std::thread::sleep(Duration::from_millis(100));
            maintenance.lock().unwrap().seconds_days = None;
            drop(guard);
            assert!(service.manage(crate::storage_management::Request::Refresh { epoch: 0 }));
        }
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(
            reader
                .query_row(
                    "SELECT COUNT(*) FROM seconds_samples WHERE seq=6",
                    [],
                    |r| r.get::<_, u64>(0)
                )
                .unwrap(),
            1,
            "used a stale expiry policy after waiting for the writer gate"
        );
        policy.lock().unwrap().days = Some(1);
        maintenance.lock().unwrap().seconds_days = Some(1);
        wait(|| {
            reader
                .query_row(
                    "SELECT COUNT(*) FROM seconds_samples WHERE seq=6",
                    [],
                    |r| r.get::<_, i64>(0),
                )
                .unwrap()
                == 0
        });
        assert_eq!(
            store
                .query_snapshot(&Query {
                    from: now - 60_000,
                    to: now,
                    ..Default::default()
                })
                .unwrap()
                .speeds
                .bytes
                .down,
            10
        );
        shared.lock().unwrap().storage_revision = 1;
        let mut req = SecondsRequest {
            id: 1,
            generation: 0,
            revision: shared.lock().unwrap().seconds_status.revision,
            view: None,
            scope: Scope {
                from: now - 60_000,
                to: now,
                app: String::new(),
                grouped: true,
            },
            from: Some(now - 60_000),
        };
        assert!(service.query(req.clone()));
        std::thread::sleep(Duration::from_millis(300));
        assert!(service.reply.lock().unwrap().is_none());
        req.id = 2;
        req.generation = 1;
        assert!(service.query(req));
        wait(|| service.reply.lock().unwrap().is_some());
        assert_eq!(
            service
                .reply
                .lock()
                .unwrap()
                .take()
                .unwrap()
                .result
                .unwrap()
                .samples
                .len(),
            2
        );
        // A cleanup revision invalidates queries, never valid queued samples.
        policy.lock().unwrap().enabled = true;
        assert!(service.record(sample(10, now - 1000)));
        wait(|| shared.lock().unwrap().seconds_status.last == Some(now));
        shared.lock().unwrap().storage_epoch = 1;
        assert!(service.record(sample(11, now)));
        let mut replacement = sample(12, now + 1000);
        replacement.generation = 1;
        assert!(service.record(replacement));
        wait(|| shared.lock().unwrap().seconds_status.last == Some(now + 2000));
        assert_eq!(
            reader
                .query_row(
                    "SELECT COUNT(*) FROM seconds_samples WHERE seq=11",
                    [],
                    |r| r.get::<_, u64>(0)
                )
                .unwrap(),
            0
        );
        assert_eq!(
            reader
                .query_row(
                    "SELECT COUNT(*) FROM seconds_samples WHERE seq=10",
                    [],
                    |r| r.get::<_, u64>(0)
                )
                .unwrap(),
            1
        );
        stop.store(true, Ordering::Release);
        service.finish();
    }
}
