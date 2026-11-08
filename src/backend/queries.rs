use super::Backend;
use crate::{
    analysis::{AnalysisRequest, Snapshot as AnalysisSnapshot},
    history::{CalendarMonth, CalendarRequest, Trend},
    model::{Query, Report},
    storage::Storage,
};
use anyhow::Result;
use std::{
    collections::VecDeque,
    sync::{Arc, atomic::Ordering, mpsc},
    time::{Duration, Instant},
};

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct AnalysisReply {
    pub request: AnalysisRequest,
    /// The worker is finished; sampling coverage may still contain genuine gaps.
    pub complete: bool,
    pub result: std::result::Result<Arc<AnalysisSnapshot>, String>,
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct CalendarReply {
    pub request: CalendarRequest,
    pub result: std::result::Result<Arc<CalendarMonth>, String>,
}

pub(super) fn start(
    handle: &Backend,
    history: mpsc::Receiver<(u64, Query)>,
    analysis: mpsc::Receiver<AnalysisRequest>,
    calendar: mpsc::Receiver<CalendarRequest>,
) -> Result<()> {
    start_calendar(handle.clone(), calendar)?;
    start_analysis(handle.clone(), analysis)?;
    start_history(handle, history)
}

fn start_calendar(calendar_handle: Backend, crx: mpsc::Receiver<CalendarRequest>) -> Result<()> {
    std::thread::Builder::new()
        .name("calendar-query".into())
        .spawn(move || {
            let mut reader = None;
            while !calendar_handle.stop.load(Ordering::Relaxed) {
                let Ok(mut request) = crx.recv_timeout(Duration::from_millis(500)) else {
                    continue;
                };
                while let Ok(newer) = crx.try_recv() {
                    request = newer;
                }
                let result = (|| -> Result<Arc<CalendarMonth>> {
                    let _access = calendar_handle.speeds.access.read().unwrap();
                    anyhow::ensure!(
                        calendar_handle.shared.lock().unwrap().storage_revision
                            == request.generation,
                        "账本已变化"
                    );
                    if reader.is_none() {
                        reader = Some(Storage::read_only(
                            &calendar_handle.dir.join("traffic.sqlite3"),
                        )?);
                    }
                    let month = reader
                        .as_ref()
                        .unwrap()
                        .calendar_snapshot(request.clone())?;
                    anyhow::ensure!(
                        calendar_handle.shared.lock().unwrap().storage_revision
                            == request.generation,
                        "账本已变化"
                    );
                    Ok(Arc::new(month))
                })()
                .map_err(|e| e.to_string());
                *calendar_handle.calendar_result.lock().unwrap() =
                    Some(CalendarReply { request, result });
                calendar_handle.query_ready.notify_one();
            }
        })?;
    Ok(())
}

fn start_analysis(analysis: Backend, arx: mpsc::Receiver<AnalysisRequest>) -> Result<()> {
    std::thread::Builder::new()
        .name("analysis-query".into())
        .spawn(move || {
            let mut queued = None;
            while !analysis.stop.load(Ordering::Relaxed) {
                let mut request = if let Some(request) = queued.take() {
                    request
                } else {
                    let Ok(request) = arx.recv_timeout(Duration::from_millis(500)) else {
                        continue;
                    };
                    request
                };
                while let Ok(newer) = arx.try_recv() {
                    request = newer;
                }
                let result = (|| -> Result<Arc<AnalysisSnapshot>> {
                    anyhow::ensure!(
                        request.generation == analysis.analysis_generation.load(Ordering::Acquire),
                        "数据源已变化，请刷新快照"
                    );
                    let snapshot = if request.range == crate::analysis::AnalysisRange::FiveMinutes {
                        // Allow the next one-second sample to close the clicked
                        // boundary, then apportion only up to that fixed instant.
                        let deadline = Instant::now() + Duration::from_millis(1500);
                        let cached = if analysis.shared.lock().unwrap().connected {
                            let cache = analysis.analysis_cache.lock().unwrap();
                            cache
                                .latest_sample()
                                .filter(|time| *time < request.anchor)
                                .map(|_| cache.clone())
                        } else {
                            None
                        };
                        if let Some(cached) = cached {
                            let preview = Arc::new(cached.snapshot(request.clone()));
                            if request.generation
                                == analysis.analysis_generation.load(Ordering::Acquire)
                            {
                                analysis.publish_analysis(AnalysisReply {
                                    request: request.clone(),
                                    complete: false,
                                    result: Ok(preview),
                                });
                            }
                        }
                        loop {
                            anyhow::ensure!(
                                request.generation
                                    == analysis.analysis_generation.load(Ordering::Acquire),
                                "数据源已变化，请刷新快照"
                            );
                            if let Ok(newer) = arx.try_recv() {
                                queued = Some(newer);
                                anyhow::bail!("快照请求已更新");
                            }
                            let latest = analysis.analysis_cache.lock().unwrap().latest_sample();
                            if latest.is_some_and(|time| time >= request.anchor)
                                || analysis.stop.load(Ordering::Relaxed)
                                || Instant::now() >= deadline
                            {
                                break;
                            }
                            let connected = analysis.shared.lock().unwrap().connected;
                            if !connected {
                                break;
                            }
                            std::thread::sleep(Duration::from_millis(25));
                        }
                        let samples = analysis.analysis_cache.lock().unwrap().clone();
                        samples.snapshot(request.clone())
                    } else {
                        let _access = analysis.speeds.access.read().unwrap();
                        crate::analysis::historical(
                            &analysis.dir.join("traffic.sqlite3"),
                            request.clone(),
                        )?
                    };
                    anyhow::ensure!(
                        request.generation == analysis.analysis_generation.load(Ordering::Acquire),
                        "数据源已变化，请刷新快照"
                    );
                    Ok(Arc::new(snapshot))
                })()
                .map_err(|error| error.to_string());
                if queued.is_none() {
                    analysis.publish_analysis(AnalysisReply {
                        request,
                        complete: true,
                        result,
                    });
                }
            }
        })?;
    Ok(())
}

fn start_history(handle: &Backend, qrx: mpsc::Receiver<(u64, Query)>) -> Result<()> {
    let qpath = handle.dir.join("traffic.sqlite3");
    let qshared = handle.shared.clone();
    let qstop = handle.stop.clone();
    let qready = handle.query_ready.clone();
    let qaccess = handle.speeds.access.clone();
    std::thread::Builder::new()
        .name("history-query".into())
        .spawn(move || {
            let mut reader = None;
            let mut report_summary: Option<(u64, Instant)> = None;
            let mut history_cache: VecDeque<(u64, Arc<Trend>)> = VecDeque::new();
            while !qstop.load(Ordering::Relaxed) {
                let Ok(job) = qrx.recv_timeout(Duration::from_millis(500)) else {
                    continue;
                };
                let mut jobs = vec![job];
                while let Ok(newer) = qrx.try_recv() {
                    if let Some(index) = jobs
                        .iter()
                        .position(|old| old.1.app.is_empty() == newer.1.app.is_empty())
                    {
                        jobs[index] = newer;
                    } else {
                        jobs.push(newer);
                    }
                }
                jobs.sort_by_key(|job| std::cmp::Reverse(job.0));
                for job in jobs {
                    let (revision, cached_page) = {
                        let state = qshared.lock().unwrap();
                        let revision = state.storage_revision;
                        let cached_page = (job.1.app.is_empty()
                            && report_summary.is_some_and(|(version, time)| {
                                version == revision && time.elapsed() < Duration::from_secs(5)
                            })
                            && state.report.error.is_empty()
                            && state
                                .report
                                .query
                                .as_ref()
                                .is_some_and(|query| query.differs_only_by_page(&job.1)))
                        .then(|| state.report.as_ref().clone());
                        (revision, cached_page)
                    };
                    let page_only = cached_page.is_some();
                    let result = (|| -> Result<Report> {
                        let _access = qaccess.read().unwrap();
                        if reader.is_none() {
                            reader = Some(Storage::read_only(&qpath)?);
                        }
                        if let Some(previous) = cached_page {
                            reader.as_ref().unwrap().query_page(&job.1, previous)
                        } else if job.1.history {
                            let (from, to) =
                                job.1.history_context.unwrap_or((job.1.from, job.1.to));
                            let now = chrono::Utc::now().timestamp_millis();
                            let cached = history_cache
                                .iter()
                                .find(|(version, data)| {
                                    *version == revision
                                        && data.from == from
                                        && data.to == to
                                        && to < now - 120_000
                                })
                                .map(|(_, data)| data.clone());
                            let report = reader
                                .as_ref()
                                .unwrap()
                                .query_snapshot_cached(&job.1, cached)?;
                            if let Some(data) = &report.history {
                                history_cache.retain(|(version, old)| {
                                    *version == revision && (old.from != from || old.to != to)
                                });
                                history_cache.push_front((revision, data.clone()));
                                history_cache.truncate(12);
                            }
                            Ok(report)
                        } else {
                            reader.as_ref().unwrap().query_snapshot(&job.1)
                        }
                    })();
                    let mut state = qshared.lock().unwrap();
                    if state.storage_revision != revision {
                        drop(state);
                        qready.notify_one();
                        continue;
                    }
                    let report = match result {
                        Ok(r) => r,
                        Err(e) => Report {
                            query: Some(job.1.clone()),
                            error: e.to_string(),
                            ..Default::default()
                        },
                    };
                    if job.1.app.is_empty() {
                        if !page_only {
                            report_summary =
                                report.error.is_empty().then(|| (revision, Instant::now()));
                        }
                        state.report_id = job.0;
                        state.report = Arc::new(report);
                    } else {
                        state.detail_id = job.0;
                        state.detail = Arc::new(report);
                    }
                    drop(state);
                    qready.notify_one();
                }
            }
        })?;
    Ok(())
}

impl Backend {
    pub fn query(&self, id: u64, q: Query) -> bool {
        self.query_tx.try_send((id, q)).is_ok()
    }
    pub fn query_results(
        &self,
        report_id: Option<u64>,
        detail_id: Option<u64>,
    ) -> (u64, Option<Arc<Report>>, Option<Arc<Report>>) {
        let state = self.shared.lock().unwrap();
        (
            state.storage_revision,
            (report_id == Some(state.report_id)).then(|| state.report.clone()),
            (detail_id == Some(state.detail_id)).then(|| state.detail.clone()),
        )
    }
    fn publish_analysis(&self, reply: AnalysisReply) {
        *self.analysis_result.lock().unwrap() = Some(reply);
        self.query_ready.notify_one();
    }
    pub fn query_analysis(&self, request: AnalysisRequest) -> bool {
        self.analysis_tx.try_send(request).is_ok()
    }
    pub fn analysis_result(&self) -> Option<AnalysisReply> {
        self.analysis_result.lock().unwrap().clone()
    }
    pub fn analysis_generation(&self) -> u64 {
        self.analysis_generation.load(Ordering::Acquire)
    }
    #[cfg(test)]
    pub fn analysis_fixture_queue(&mut self) -> mpsc::Receiver<AnalysisRequest> {
        let (sender, receiver) = mpsc::sync_channel(64);
        self.analysis_tx = sender;
        receiver
    }
    pub fn query_calendar(&self, request: CalendarRequest) -> bool {
        self.calendar_tx.try_send(request).is_ok()
    }
    pub fn calendar_result(&self) -> Option<CalendarReply> {
        self.calendar_result.lock().unwrap().clone()
    }
    pub(super) fn reset_analysis(&self) {
        self.analysis_cache.lock().unwrap().clear();
        self.analysis_generation.fetch_add(1, Ordering::AcqRel);
    }
}
