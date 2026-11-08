use super::{Backend, Command, log};
use crate::{
    accounting::Ledger,
    mihomo::Client,
    model::{Bytes, LiveSample},
    storage::Storage,
};
use anyhow::Result;
use std::{
    sync::{Arc, atomic::Ordering, mpsc},
    time::{Duration, Instant},
};

pub(super) fn start(
    handle: Backend,
    db: Storage,
    ledger: Ledger,
    rx: mpsc::Receiver<Command>,
) -> Result<()> {
    std::thread::Builder::new()
        .name("mihomo-collector".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("Tokio runtime");
            rt.block_on(collect(handle, db, ledger, rx));
        })?;
    Ok(())
}

async fn collect(
    handle: Backend,
    mut db: Storage,
    mut ledger: Ledger,
    rx: mpsc::Receiver<Command>,
) {
    let process_task = tokio::spawn(crate::process_control::run(handle.clone()));
    let permanent_task = tokio::spawn(crate::permanent_control::run(handle.clone()));
    let mut client: Option<Client> = None;
    let mut attribution = crate::attribution::Resolver::new(handle.stop.clone());
    let initial = handle.config.lock().unwrap().clone();
    let mut connection_settings = (initial.endpoint, initial.secret);
    let mut interrupted = true;
    let sample_clock = Instant::now();
    let mut previous_sample_ms = 0u64;
    let mut tick = 0u64;
    let mut last_totals_refresh = Instant::now();
    let mut totals_day = crate::storage::local_day(chrono::Utc::now().timestamp_millis());
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    while !handle.stop.load(Ordering::Relaxed) {
        interval.tick().await;
        if handle.speeds.pause.load(Ordering::Acquire) {
            interrupted = true;
            handle.speeds.paused.store(true, Ordering::Release);
            let mut state = handle.shared.lock().unwrap();
            state.connected = false;
            state.up = 0.;
            state.down = 0.;
            state.app_rates.clear();
            state.application_rates.clear();
            state.live.clear();
            state.status = "文件空间回收中 · 采集暂停".into();
            continue;
        }
        handle.speeds.paused.store(false, Ordering::Release);
        let desired = handle.config.lock().unwrap().clone();
        if connection_settings != (desired.endpoint.clone(), desired.secret.clone()) {
            connection_settings = (desired.endpoint, desired.secret);
            client = None;
            interrupted = true;
            handle.reset_analysis();
            let mut state = handle.shared.lock().unwrap();
            state.app_rates.clear();
            state.application_rates.clear();
            state.connections = Default::default();
            state.live.clear();
            state.connected = false;
        }
        while let Ok(cmd) = rx.try_recv() {
            let result: Result<Option<String>> = match cmd {
                Command::Reconnect => {
                    client = None;
                    interrupted = true;
                    Ok(Some("正在重新连接…".into()))
                }
                Command::Backup => {
                    let _access = handle.speeds.access.read().unwrap();
                    let config = handle.config.lock().unwrap().clone();
                    crate::backup::destination(&config, "traffic").and_then(|p| {
                        db.backup(&p)
                            .map(|_| Some(format!("备份已保存：{}", p.display())))
                    })
                }
                Command::Restore(path) => {
                    let _access = handle.speeds.access.write().unwrap();
                    let _seconds_guard = handle.speeds.storage_gate.lock().unwrap();
                    let config = handle.config.lock().unwrap().clone();
                    let result = crate::backup::destination(&config, "before-restore")
                        .and_then(|rollback| db.restore(&path, &rollback));
                    result.map(|restored| {
                        ledger = restored.ledger;
                        let (today, month, cumulative) = restored.totals;
                        let mut state = handle.shared.lock().unwrap();
                        state.today = today;
                        state.month = month;
                        state.cumulative = cumulative;
                        state.storage_revision += 1;
                        state.storage_epoch += 1;
                        state.storage_management = Default::default();
                        state.identities = restored.identities;
                        handle.reset_analysis();
                        state.up = 0.;
                        state.down = 0.;
                        state.app_rates.clear();
                        state.application_rates.clear();
                        state.connections = Default::default();
                        state.live.clear();
                        interrupted = true;
                        handle.query_ready.notify_one();
                        Some("备份已恢复，恢复前数据已另行备份".into())
                    })
                }
                Command::Export(query) => {
                    let _access = handle.speeds.access.read().unwrap();
                    crate::csv_export::destination(&query.dimension).and_then(|p| {
                        db.export(&query, &p)
                            .map(|_| Some(format!("CSV 已导出：{}", p.display())))
                    })
                }
            };
            match result {
                Ok(Some(s)) => handle.notice(s),
                Ok(None) => {}
                Err(e) => handle.notice(format!("操作失败：{e}")),
            }
        }
        if handle.stop.load(Ordering::Relaxed) {
            break;
        }
        let now = chrono::Utc::now().timestamp_millis();
        if tick.is_multiple_of(10) {
            let _access = handle.speeds.access.read().unwrap();
            let bytes = db.database_bytes();
            handle.shared.lock().unwrap().database_bytes = bytes;
        }
        tick += 1;
        let day = crate::storage::local_day(now);
        if last_totals_refresh.elapsed() >= Duration::from_secs(5) || day != totals_day {
            let _access = handle.speeds.access.read().unwrap();
            if let (Ok(today), Ok(month)) = (
                db.total(day, now + 60_000),
                db.total(crate::storage::local_month(now), now + 60_000),
            ) {
                let mut state = handle.shared.lock().unwrap();
                state.today = today;
                state.month = month;
            }
            last_totals_refresh = Instant::now();
            totals_day = day;
        }
        if client.is_none() {
            {
                let mut s = handle.shared.lock().unwrap();
                s.connected = false;
                s.endpoint.clear();
                s.client_name.clear();
                s.core_version.clear();
                s.core_verified = false;
                s.process_mode.clear();
            }
            let config = handle.config.lock().unwrap().clone();
            match Client::new(&config) {
                Ok(mut c) => {
                    let info = c.request("/version").await;
                    let version = info
                        .ok()
                        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                        .and_then(|v| v["version"].as_str().map(str::to_string))
                        .unwrap_or_default();
                    {
                        let mut s = handle.shared.lock().unwrap();
                        s.endpoint = c.endpoint.clone();
                        s.client_name = c.client_name.clone();
                        s.core_version = version;
                    }
                    handle.retry_process_control();
                    client = Some(c);
                }
                Err(e) => {
                    let mut s = handle.shared.lock().unwrap();
                    s.connected = false;
                    s.status = e.to_string();
                    s.up = 0.;
                    s.down = 0.;
                    s.connections = Default::default();
                    continue;
                }
            }
        }
        let c = client.as_mut().unwrap();
        match c.snapshot().await {
            Ok(mut snap) => {
                let now = chrono::Utc::now().timestamp_millis();
                let sampled_ms =
                    sample_clock.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
                let duration_ms = sampled_ms.saturating_sub(previous_sample_ms).max(1);
                previous_sample_ms = sampled_ms;
                attribution.annotate(&mut snap, &c.identity, c.verified, now);
                let elapsed = (duration_ms as f64 / 1000.).max(0.01);
                let mut next = ledger.clone();
                let sample_from = ledger.time;
                let mut batch = next.ingest(&snap, &c.identity, now, interrupted);
                batch.sample_duration_ms = Some(duration_ms);
                let result = {
                    let _access = handle.speeds.access.read().unwrap();
                    let _write = handle.speeds.storage_gate.lock().unwrap();
                    db.commit(&next, &batch)
                };
                match result {
                    Ok(applied) => {
                        ledger = next;
                        interrupted = false;
                        if applied && handle.seconds_policy.lock().unwrap().enabled {
                            let revision = handle.shared.lock().unwrap().storage_epoch;
                            if let Some(sample) = crate::speed_stats::Sample::from_batch(
                                revision,
                                ledger.sequence,
                                now,
                                &batch,
                            ) && !handle.speeds.record(sample)
                            {
                                handle.shared.lock().unwrap().seconds_status.error =
                                    "秒级写入队列已满，此段明细未保存；分钟统计继续。".into();
                            }
                        }
                        let mut s = handle.shared.lock().unwrap();
                        if applied {
                            let day = crate::storage::local_day(now);
                            let month = crate::storage::local_month(now);
                            for (time, bytes) in &batch.totals {
                                s.cumulative.add(*bytes);
                                if *time >= day {
                                    s.today.add(*bytes);
                                }
                                if *time >= month {
                                    s.month.add(*bytes);
                                }
                            }
                        }
                        s.connected = true;
                        s.client_name = c.client_name.clone();
                        s.core_verified = c.verified;
                        s.core_process_id = c.process_id;
                        s.core_identity = c.identity.clone();
                        s.status = "采集正常 · 1 秒采样".into();
                        s.up = batch.rate.up as f64 / elapsed;
                        s.down = batch.rate.down as f64 / elapsed;
                        {
                            let mut samples = handle.analysis_cache.lock().unwrap();
                            if samples.advance(now) {
                                handle.analysis_generation.fetch_add(1, Ordering::AcqRel);
                            }
                            if applied && batch.gap.is_none() && batch.event.is_none() {
                                samples.record(
                                    sample_from,
                                    now,
                                    batch.rate,
                                    &batch.application_rates,
                                );
                            }
                        }
                        s.identities.extend(batch.identities);
                        s.application_rates = batch
                            .application_rates
                            .into_iter()
                            .map(|(key, bytes)| {
                                (
                                    key,
                                    Bytes {
                                        up: (bytes.up as f64 / elapsed) as u64,
                                        down: (bytes.down as f64 / elapsed) as u64,
                                    },
                                )
                            })
                            .collect();
                        s.app_rates = batch
                            .app_rates
                            .into_iter()
                            .map(|(k, v)| {
                                (
                                    k,
                                    Bytes {
                                        up: (v.up as f64 / elapsed) as u64,
                                        down: (v.down as f64 / elapsed) as u64,
                                    },
                                )
                            })
                            .collect();
                        s.connections = Arc::new(snap.connections.unwrap_or_default());
                        let point = LiveSample {
                            from: sample_from,
                            to: now,
                            up: s.up,
                            down: s.down,
                            valid: applied && batch.gap.is_none() && batch.event.is_none(),
                        };
                        crate::live_chart::record(&mut s.live, point, batch.event.is_some());
                    }
                    Err(e) => {
                        interrupted = true;
                        let mut s = handle.shared.lock().unwrap();
                        s.connected = false;
                        s.up = 0.;
                        s.down = 0.;
                        s.status = format!("数据库写入失败，已暂停记账：{e}");
                        log(&handle.dir, &s.status);
                    }
                }
            }
            Err(e) => {
                let mut s = handle.shared.lock().unwrap();
                if s.connected {
                    log(&handle.dir, &format!("采集断开：{e}"));
                }
                s.connected = false;
                s.up = 0.;
                s.down = 0.;
                s.app_rates.clear();
                s.application_rates.clear();
                s.connections = Default::default();
                s.status = format!("连接中断：{e}");
                interrupted = true;
                client = None;
            }
        }
    }
    process_task.abort();
    let _ = process_task.await;
    let _ = permanent_task.await;
    handle.speeds.finish();
    let _ = db.db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    handle.done.store(true, Ordering::Release);
}
