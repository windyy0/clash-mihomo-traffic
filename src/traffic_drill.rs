use crate::{
    history,
    model::{Bytes, Row},
    speed_stats::{Scope, SecondSample, SecondsRequest},
};
use anyhow::{Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct View {
    pub from: i64,
    pub to: i64,
    pub seconds: bool,
}
impl View {
    pub fn centered(at: i64, scope: &Scope) -> Self {
        let at = at.div_euclid(60_000) * 60_000;
        Self {
            from: (at - 300_000).max(scope.from),
            to: (at + 300_000).min(scope.to),
            seconds: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    Second(i64),
    Minute(i64, i64),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HoverRequest {
    pub id: u64,
    pub window: SecondsRequest,
    pub target: Target,
}
#[derive(Clone, Debug, Default)]
pub struct HoverData {
    pub observed: bool,
    pub from: i64,
    pub to: i64,
    pub down: f64,
    pub up: f64,
    pub duration_ms: u64,
    pub applications: Vec<Row>,
    pub unassigned: Bytes,
}
#[derive(Clone)]
pub struct HoverReply {
    pub request: HoverRequest,
    pub result: Result<HoverData, String>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct WindowStats {
    pub covered_ms: f64,
    pub down_peak: f64,
    pub up_peak: f64,
    pub down_average: f64,
    pub up_average: f64,
}
pub fn window_stats(samples: &[SecondSample], from: i64, to: i64) -> WindowStats {
    let mut result = WindowStats::default();
    let mut bytes = (0., 0.);
    for sample in samples.iter().filter(|s| s.valid) {
        let overlap = (sample.to.min(to) - sample.from.max(from)).max(0) as f64;
        let covered = overlap / (sample.to - sample.from).max(1) as f64 * sample.duration_ms as f64;
        if covered <= 0. {
            continue;
        }
        result.covered_ms += covered;
        result.down_peak = result.down_peak.max(sample.down);
        result.up_peak = result.up_peak.max(sample.up);
        bytes.0 += sample.down * covered / 1000.;
        bytes.1 += sample.up * covered / 1000.;
    }
    if result.covered_ms > 0. {
        result.down_average = bytes.0 * 1000. / result.covered_ms;
        result.up_average = bytes.1 * 1000. / result.covered_ms;
    }
    result
}

pub fn read_bins(db: &Connection, scope: &Scope, from: i64, to: i64) -> Result<history::Trend> {
    let now = chrono::Utc::now().timestamp_millis();
    let mut trend = history::read(db, from, to, now)?;
    if scope.app.is_empty() {
        return Ok(trend);
    }
    for bucket in &mut trend.buckets {
        bucket.bytes = Bytes::default();
    }
    let table = if scope.grouped {
        "application_minutes"
    } else {
        "app_minutes"
    };
    let mut stmt = db.prepare(&format!("SELECT a.time,a.up,a.down FROM {table} a JOIN dimensions d ON d.id=a.app WHERE d.kind='app' AND d.name=?1 AND a.time>=?2 AND a.time<?3 ORDER BY a.time"))?;
    let mut rows = stmt.query(params![scope.app, from, to.min(now + 1)])?;
    while let Some(row) = rows.next()? {
        let time: i64 = row.get(0)?;
        let index = trend.buckets.partition_point(|b| b.to <= time);
        if let Some(bucket) = trend.buckets.get_mut(index).filter(|b| time >= b.from) {
            bucket.observed = true;
            bucket.bytes.add(Bytes {
                up: row.get::<_, i64>(1)?.max(0) as u64,
                down: row.get::<_, i64>(2)?.max(0) as u64,
            });
        }
    }
    Ok(trend)
}

pub fn read_hover(db: &Connection, request: &HoverRequest) -> Result<HoverData> {
    let scope = &request.window.scope;
    let view = request
        .window
        .view
        .ok_or_else(|| anyhow::anyhow!("下钻窗口已失效"))?;
    let mut result = HoverData::default();
    match request.target {
        Target::Second(seq) => {
            let row = db.query_row("SELECT started,ended,duration,down,up FROM seconds_samples WHERE seq=?1 AND ended>?2 AND started<?3", params![seq,view.from,view.to], |r| Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?,r.get::<_,i64>(4)?))).optional()?;
            let (from, to, duration, down, up) =
                row.ok_or_else(|| anyhow::anyhow!("该采样已清理，请刷新"))?;
            ensure!(
                to > from && to - from <= 3000 && (1..=3000).contains(&duration),
                "该采样无效"
            );
            result.observed = true;
            result.from = from;
            result.to = to;
            result.duration_ms = duration as u64;
            let mut stmt = db.prepare(
                "SELECT d.name,SUM(a.down),SUM(a.up) FROM seconds_apps a
                 LEFT JOIN package_app_ids p ON ?2=1 AND p.app=a.app
                 JOIN dimensions d ON d.id=COALESCE(p.owner,a.app)
                 WHERE a.seq=?1 AND a.kind=?2 AND (?3='' OR d.name=?3)
                 GROUP BY d.id ORDER BY (SUM(a.down)+SUM(a.up)) DESC,d.name",
            )?;
            result.applications = stmt
                .query_map(params![seq, i64::from(scope.grouped), scope.app], |r| {
                    Ok(Row {
                        key: r.get(0)?,
                        bytes: Bytes {
                            down: r.get::<_, i64>(1)?.max(0) as u64,
                            up: r.get::<_, i64>(2)?.max(0) as u64,
                        },
                    })
                })?
                .collect::<rusqlite::Result<_>>()?;
            result.down = down.max(0) as f64 * 1000. / duration as f64;
            result.up = up.max(0) as f64 * 1000. / duration as f64;
        }
        Target::Minute(from, to) => {
            ensure!(
                from >= view.from && to <= view.to && to > from && to - from <= 60_000,
                "分钟范围已失效"
            );
            result.from = from;
            result.to = to;
            let table = if scope.grouped {
                "application_minutes"
            } else {
                "app_minutes"
            };
            let mut stmt=db.prepare(&format!("SELECT d.name,SUM(a.down),SUM(a.up) FROM {table} a JOIN dimensions d ON d.id=a.app WHERE a.time>=?1 AND a.time<?2 AND (?3='' OR d.name=?3) GROUP BY a.app ORDER BY (SUM(a.down)+SUM(a.up)) DESC,d.name"))?;
            result.applications = stmt
                .query_map(params![from, to, scope.app], |r| {
                    Ok(Row {
                        key: r.get(0)?,
                        bytes: Bytes {
                            down: r.get::<_, i64>(1)?.max(0) as u64,
                            up: r.get::<_, i64>(2)?.max(0) as u64,
                        },
                    })
                })?
                .collect::<rusqlite::Result<_>>()?;
            let total:Bytes=db.query_row("SELECT COALESCE(SUM(down),0),COALESCE(SUM(up),0) FROM totals WHERE time>=?1 AND time<?2",params![from,to],|r|Ok(Bytes{down:r.get::<_,i64>(0)?.max(0) as u64,up:r.get::<_,i64>(1)?.max(0) as u64}))?;
            let observed: bool = db.query_row(
                "SELECT EXISTS(SELECT 1 FROM totals WHERE time>=?1 AND time<?2)",
                params![from, to],
                |r| r.get(0),
            )?;
            result.observed = observed || !result.applications.is_empty();
            result.down = total.down as f64;
            result.up = total.up as f64;
        }
    }
    let mut assigned = Bytes::default();
    for row in &result.applications {
        assigned.add(row.bytes);
    }
    if scope.app.is_empty() {
        let factor = if result.duration_ms > 0 {
            result.duration_ms as f64 / 1000.
        } else {
            1.
        };
        result.unassigned = Bytes {
            down: (result.down * factor).round().max(0.) as u64,
            up: (result.up * factor).round().max(0.) as u64,
        };
        result.unassigned.down = result.unassigned.down.saturating_sub(assigned.down);
        result.unassigned.up = result.unassigned.up.saturating_sub(assigned.up);
    } else {
        let factor = if result.duration_ms > 0 {
            1000. / result.duration_ms as f64
        } else {
            1.
        };
        result.down = assigned.down as f64 * factor;
        result.up = assigned.up as f64 * factor;
    }
    Ok(result)
}

#[cfg(test)]
#[path = "traffic_drill_tests.rs"]
mod tests;
