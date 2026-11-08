//! On-demand, read-only minute-ledger details for overview usage buckets.
use crate::{
    backend::Backend,
    model::{Bytes, Row, SeriesPoint},
};
use anyhow::{Result, ensure};
use rusqlite::{Connection, OpenFlags, params};
use std::{
    sync::{Condvar, Mutex, atomic::Ordering},
    time::Duration,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bin {
    pub from: i64,
    pub to: i64,
    pub bytes: Bytes,
}

pub fn bins(series: &[SeriesPoint], from: i64, to: i64, now: i64) -> Vec<Bin> {
    let end = to.min(
        now.div_euclid(60_000)
            .saturating_add(1)
            .saturating_mul(60_000),
    );
    let points: Vec<_> = series
        .iter()
        .filter(|p| p.time >= from && p.time < end)
        .collect();
    let count = points.len().min(24);
    let mut result = Vec::with_capacity(count);
    for index in 0..count {
        let first = (index * points.len()).div_ceil(count);
        let last = ((index + 1) * points.len()).div_ceil(count);
        let mut bytes = Bytes::default();
        for point in &points[first..last] {
            let valid = |v: f64| if v.is_finite() && v > 0. { v as u64 } else { 0 };
            bytes.add(Bytes {
                down: valid(point.down),
                up: valid(point.up),
            });
        }
        let start = points[first].time;
        let stop = points.get(last).map_or(end, |p| p.time);
        if stop > start {
            result.push(Bin {
                from: start,
                to: stop,
                bytes,
            });
        }
    }
    result
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub id: u64,
    pub source: (u64, u64),
    pub from: i64,
    pub to: i64,
}
#[derive(Clone, Debug, Default)]
pub struct Detail {
    pub observed: bool,
    pub total: Bytes,
    pub applications: Vec<Row>,
    pub count: usize,
    pub unassigned: Bytes,
}
#[derive(Clone)]
pub struct Reply {
    pub request: Request,
    pub result: Result<Detail, String>,
}
#[derive(Default)]
pub struct Control {
    queued: Mutex<Option<Request>>,
    wake: Condvar,
    pub reply: Mutex<Option<Reply>>,
}
impl Control {
    pub fn query(&self, request: Request) {
        *self.queued.lock().unwrap() = Some(request);
        self.wake.notify_one();
    }
}

pub fn read(db: &Connection, from: i64, to: i64) -> Result<Detail> {
    ensure!(
        to > from && to.saturating_sub(from) <= 3660 * 86_400_000_i64,
        "时段范围无效"
    );
    let tx = db.unchecked_transaction()?;
    let (total,observed):(Bytes,bool)=tx.query_row("SELECT COALESCE(SUM(down),0),COALESCE(SUM(up),0),COUNT(*)>0 FROM totals WHERE time>=?1 AND time<?2",params![from,to],|r|Ok((Bytes{down:r.get::<_,i64>(0)?.max(0) as u64,up:r.get::<_,i64>(1)?.max(0) as u64},r.get(2)?)))?;
    let (assigned,count):(Bytes,usize)=tx.query_row("SELECT COALESCE(SUM(down),0),COALESCE(SUM(up),0),COUNT(DISTINCT app) FROM app_minutes WHERE time>=?1 AND time<?2",params![from,to],|r|Ok((Bytes{down:r.get::<_,i64>(0)?.max(0) as u64,up:r.get::<_,i64>(1)?.max(0) as u64},r.get(2)?)))?;
    let applications = {
        let mut statement=tx.prepare("SELECT d.name,SUM(a.down),SUM(a.up) FROM app_minutes a JOIN dimensions d ON d.id=a.app WHERE a.time>=?1 AND a.time<?2 GROUP BY a.app ORDER BY SUM(a.down)+SUM(a.up) DESC,d.name LIMIT 5")?;
        statement
            .query_map(params![from, to], |r| {
                Ok(Row {
                    key: r.get(0)?,
                    bytes: Bytes {
                        down: r.get::<_, i64>(1)?.max(0) as u64,
                        up: r.get::<_, i64>(2)?.max(0) as u64,
                    },
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    tx.commit()?;
    Ok(Detail {
        observed: observed || count > 0,
        total,
        applications,
        count,
        unassigned: Bytes {
            down: total.down.saturating_sub(assigned.down),
            up: total.up.saturating_sub(assigned.up),
        },
    })
}

pub fn start(handle: Backend) -> Result<()> {
    std::thread::Builder::new()
        .name("overview-usage-query".into())
        .spawn(move || {
            let mut connection = None;
            let mut epoch = u64::MAX;
            while !handle.stop.load(Ordering::Acquire) {
                let mut queued = handle.overview_usage.queued.lock().unwrap();
                if queued.is_none() {
                    queued = handle
                        .overview_usage
                        .wake
                        .wait_timeout(queued, Duration::from_millis(100))
                        .unwrap()
                        .0;
                }
                let request = queued.take();
                drop(queued);
                let Some(request) = request else { continue };
                let result = (|| -> Result<Detail> {
                    let _access = handle.speeds.access.read().unwrap();
                    let state = handle.shared.lock().unwrap();
                    ensure!(
                        request.source == (state.storage_revision, state.storage_epoch),
                        "数据来源已变化"
                    );
                    let source = state.storage_epoch;
                    drop(state);
                    if connection.is_none() || source != epoch {
                        let db = Connection::open_with_flags(
                            handle.dir.join("traffic.sqlite3"),
                            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
                        )?;
                        db.busy_timeout(Duration::from_millis(500))?;
                        connection = Some(db);
                        epoch = source;
                    }
                    let data = read(connection.as_ref().unwrap(), request.from, request.to)?;
                    let state = handle.shared.lock().unwrap();
                    ensure!(
                        request.source == (state.storage_revision, state.storage_epoch),
                        "数据来源已变化"
                    );
                    Ok(data)
                })()
                .map_err(|_| "时段明细读取失败，请重新悬浮重试".to_string());
                *handle.overview_usage.reply.lock().unwrap() = Some(Reply { request, result });
                handle.query_ready.notify_one();
            }
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn buckets_cover_exact_contiguous_minute_ranges_and_preserve_bytes() {
        for length in [0, 1, 5, 24, 25, 120] {
            let series = (0..length)
                .map(|index| SeriesPoint {
                    time: index * 60_000,
                    down: 10.,
                    up: 2.,
                })
                .collect::<Vec<_>>();
            let result = bins(&series, 0, length * 60_000, length * 60_000);
            assert_eq!(result.len(), (length as usize).min(24));
            assert_eq!(
                result.iter().map(|b| b.bytes.total()).sum::<u64>(),
                length as u64 * 12
            );
            for pair in result.windows(2) {
                assert_eq!(pair[0].to, pair[1].from);
            }
            if let Some(last) = result.last() {
                assert_eq!(last.to, length * 60_000);
            }
        }
    }
    #[test]
    fn detail_uses_raw_application_history_and_distinguishes_missing_and_idle() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE totals(time INTEGER,down INTEGER,up INTEGER); CREATE TABLE dimensions(id INTEGER,name TEXT); CREATE TABLE app_minutes(time INTEGER,app INTEGER,down INTEGER,up INTEGER); INSERT INTO totals VALUES(0,100,20),(60000,0,0),(120000,999,999); INSERT INTO dimensions VALUES(1,'browser.exe'),(2,'未知进程'); INSERT INTO app_minutes VALUES(0,1,60,10),(0,2,10,5),(120000,1,999,999);") .unwrap();
        let detail = read(&db, 0, 60_000).unwrap();
        assert!(detail.observed);
        assert_eq!(detail.count, 2);
        assert_eq!(detail.total, Bytes { down: 100, up: 20 });
        assert_eq!(detail.applications[0].key, "browser.exe");
        assert_eq!(detail.unassigned, Bytes { down: 30, up: 5 });
        assert!(read(&db, 60_000, 120_000).unwrap().observed);
        assert!(!read(&db, 180_000, 240_000).unwrap().observed);
        assert!(read(&db, 1, 0).is_err());
    }
}
