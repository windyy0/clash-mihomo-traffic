use crate::{
    accounting::Batch,
    config::Config,
    model::{Bytes, Query},
};
use anyhow::{Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};

pub const MINUTE: i64 = 60_000;
pub const WINDOW: i64 = 600_000;
pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS speed_minutes(
 time INTEGER PRIMARY KEY,down INTEGER NOT NULL,up INTEGER NOT NULL,covered INTEGER NOT NULL,
 first INTEGER NOT NULL,last INTEGER NOT NULL,dp REAL NOT NULL,da INTEGER NOT NULL,upk REAL NOT NULL,ua INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS speed_app_minutes(
 time INTEGER NOT NULL,kind INTEGER NOT NULL,app INTEGER NOT NULL,down INTEGER NOT NULL,up INTEGER NOT NULL,
 dp REAL NOT NULL,da INTEGER NOT NULL,upk REAL NOT NULL,ua INTEGER NOT NULL,PRIMARY KEY(time,kind,app));
CREATE INDEX IF NOT EXISTS speed_app_range ON speed_app_minutes(kind,app,time);
CREATE TABLE IF NOT EXISTS seconds_samples(
 seq INTEGER PRIMARY KEY,started INTEGER NOT NULL,ended INTEGER NOT NULL,duration INTEGER NOT NULL,down INTEGER NOT NULL,up INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS seconds_window ON seconds_samples(ended,started);
CREATE TABLE IF NOT EXISTS seconds_apps(
 seq INTEGER NOT NULL REFERENCES seconds_samples(seq) ON DELETE CASCADE,kind INTEGER NOT NULL,app INTEGER NOT NULL,
 down INTEGER NOT NULL,up INTEGER NOT NULL,PRIMARY KEY(seq,kind,app));
CREATE INDEX IF NOT EXISTS seconds_app_range ON seconds_apps(kind,app,seq);
"#;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SecondsPolicy {
    pub enabled: bool,
    pub days: Option<u32>,
    pub max_bytes: Option<u64>,
}
impl From<&Config> for SecondsPolicy {
    fn from(c: &Config) -> Self {
        Self {
            enabled: c.seconds_enabled,
            days: c.seconds_days_enabled.then_some(c.seconds_days),
            max_bytes: None,
        }
    }
}
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct Peak {
    pub rate: f64,
    pub at: i64,
}
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct SpeedStats {
    pub bytes: Bytes,
    pub covered_ms: u64,
    pub expected_ms: u64,
    pub first: Option<i64>,
    pub as_of: Option<i64>,
    pub down_peak: Option<Peak>,
    pub up_peak: Option<Peak>,
}
impl SpeedStats {
    pub fn average(&self) -> Option<(f64, f64)> {
        (self.covered_ms > 0).then(|| {
            (
                self.bytes.down as f64 * 1000. / self.covered_ms as f64,
                self.bytes.up as f64 * 1000. / self.covered_ms as f64,
            )
        })
    }
    pub fn partial(&self) -> bool {
        self.covered_ms.saturating_add(2) < self.expected_ms
    }
}
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct SecondsStatus {
    pub bytes: u64,
    pub first: Option<i64>,
    pub last: Option<i64>,
    pub error: String,
    pub revision: u64,
}
#[derive(Clone)]
pub struct Sample {
    pub generation: u64,
    pub seq: u64,
    pub from: i64,
    pub to: i64,
    pub duration_ms: u64,
    pub bytes: Bytes,
    pub apps: Vec<(bool, String, Bytes)>,
}
impl Sample {
    pub fn from_batch(generation: u64, seq: u64, to: i64, b: &Batch) -> Option<Self> {
        let from = b.sample_from?;
        let duration_ms = b
            .sample_duration_ms
            .unwrap_or_else(|| to.saturating_sub(from).max(0) as u64);
        if !(1..=3000).contains(&(to - from))
            || !(1..=3000).contains(&duration_ms)
            || b.gap.is_some()
            || b.event.is_some()
        {
            return None;
        }
        Some(Self {
            generation,
            seq,
            from,
            to,
            duration_ms,
            bytes: b.rate,
            apps: [(false, &b.app_rates), (true, &b.application_rates)]
                .into_iter()
                .flat_map(|(group, rates)| {
                    rates
                        .iter()
                        .filter(|(_, bytes)| bytes.total() > 0)
                        .map(move |(key, bytes)| (group, key.clone(), *bytes))
                })
                .collect(),
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Scope {
    pub from: i64,
    pub to: i64,
    pub app: String,
    pub grouped: bool,
}
impl From<&Query> for Scope {
    fn from(q: &Query) -> Self {
        Self {
            from: q.from,
            to: q.to,
            app: q.app.clone(),
            grouped: q.group_apps,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SecondsRequest {
    pub id: u64,
    pub generation: u64,
    pub revision: u64,
    pub scope: Scope,
    pub from: Option<i64>,
    pub view: Option<crate::traffic_drill::View>,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SecondsData {
    pub from: i64,
    pub to: i64,
    pub first: Option<i64>,
    pub last: Option<i64>,
    pub samples: Vec<SecondSample>,
    pub trend: Option<crate::history::Trend>,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SecondSample {
    pub seq: i64,
    pub from: i64,
    pub to: i64,
    pub duration_ms: u64,
    pub down: f64,
    pub up: f64,
    pub valid: bool,
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct SecondsReply {
    pub request: SecondsRequest,
    pub result: Result<SecondsData, String>,
}

fn integer(v: u64) -> i64 {
    v.min(i64::MAX as u64) as i64
}
fn app_id(db: &Connection, key: &str) -> Result<i64> {
    db.execute(
        "INSERT OR IGNORE INTO dimensions(kind,name) VALUES('app',?1)",
        [key],
    )?;
    Ok(db.query_row(
        "SELECT id FROM dimensions WHERE kind='app' AND name=?1",
        [key],
        |r| r.get(0),
    )?)
}
fn choose(peak: &mut Option<Peak>, rate: f64, at: i64) {
    if peak.is_none_or(|old| rate > old.rate || (rate == old.rate && at < old.at)) {
        *peak = Some(Peak { rate, at });
    }
}

pub fn write_summary(db: &Connection, to: i64, b: &Batch) -> Result<()> {
    let Some(sample) = Sample::from_batch(0, 0, to, b) else {
        return Ok(());
    };
    let dt = sample.duration_ms as f64;
    let dp = sample.bytes.down as f64 * 1000. / dt;
    let up = sample.bytes.up as f64 * 1000. / dt;
    for (time, bytes) in crate::accounting::split(sample.from, to, sample.bytes) {
        let first = sample.from.max(time);
        let last = to.min(time + MINUTE);
        let at = first - time;
        let coverage = (((last - sample.from) as u128 * u128::from(sample.duration_ms)
            / (to - sample.from) as u128)
            - ((first - sample.from) as u128 * u128::from(sample.duration_ms)
                / (to - sample.from) as u128)) as u64;
        db.execute(r#"INSERT INTO speed_minutes VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?8)
        ON CONFLICT(time) DO UPDATE SET down=down+excluded.down,up=up+excluded.up,covered=covered+excluded.covered,
        first=MIN(first,excluded.first),last=MAX(last,excluded.last),
        da=CASE WHEN excluded.dp>dp OR (excluded.dp=dp AND excluded.da<da) THEN excluded.da ELSE da END,
        ua=CASE WHEN excluded.upk>upk OR (excluded.upk=upk AND excluded.ua<ua) THEN excluded.ua ELSE ua END,
        dp=MAX(dp,excluded.dp),upk=MAX(upk,excluded.upk)"#,
            params![time,integer(bytes.down),integer(bytes.up),integer(coverage),first,last,dp,at,up])?;
    }
    for (group, key, bytes) in &sample.apps {
        let id = app_id(db, key)?;
        let dp = bytes.down as f64 * 1000. / dt;
        let up = bytes.up as f64 * 1000. / dt;
        for (time, bytes) in crate::accounting::split(sample.from, to, *bytes) {
            let at = sample.from.max(time) - time;
            db.execute(r#"INSERT INTO speed_app_minutes VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?7)
            ON CONFLICT(time,kind,app) DO UPDATE SET down=down+excluded.down,up=up+excluded.up,
            da=CASE WHEN excluded.dp>dp OR (excluded.dp=dp AND excluded.da<da) THEN excluded.da ELSE da END,
            ua=CASE WHEN excluded.upk>upk OR (excluded.upk=upk AND excluded.ua<ua) THEN excluded.ua ELSE ua END,
            dp=MAX(dp,excluded.dp),upk=MAX(upk,excluded.upk)"#,
                params![time,i64::from(*group),id,integer(bytes.down),integer(bytes.up),dp,at,up])?;
        }
    }
    Ok(())
}

pub fn read_stats(db: &Connection, q: &Query) -> Result<SpeedStats> {
    let latest: Option<i64> = db
        .query_row(
            "SELECT last FROM speed_minutes ORDER BY time DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    let mut stats = SpeedStats {
        expected_ms: latest.map_or(0, |last| {
            q.to.min(last).saturating_sub(q.from).max(0) as u64
        }),
        ..Default::default()
    };
    let mut stmt = db.prepare("SELECT time,down,up,covered,first,last,dp,da,upk,ua FROM speed_minutes WHERE time>=?1 AND time<?2 AND first>=?3 AND last<=?2 ORDER BY time")?;
    let mut rows = stmt.query(params![q.from.div_euclid(MINUTE) * MINUTE, q.to, q.from])?;
    while let Some(row) = rows.next()? {
        let time: i64 = row.get(0)?;
        stats.bytes.add(Bytes {
            down: row.get::<_, i64>(1)?.max(0) as u64,
            up: row.get::<_, i64>(2)?.max(0) as u64,
        });
        stats.covered_ms = stats
            .covered_ms
            .saturating_add(row.get::<_, i64>(3)?.max(0) as u64);
        let first = row.get::<_, i64>(4)?;
        let last = row.get::<_, i64>(5)?;
        stats.first = Some(stats.first.map_or(first, |old| old.min(first)));
        stats.as_of = Some(stats.as_of.map_or(last, |old| old.max(last)));
        choose(
            &mut stats.down_peak,
            row.get(6)?,
            time + row.get::<_, i64>(7)?,
        );
        choose(
            &mut stats.up_peak,
            row.get(8)?,
            time + row.get::<_, i64>(9)?,
        );
    }
    if !q.app.is_empty() && stats.covered_ms > 0 {
        stats.bytes = Bytes::default();
        stats.down_peak = Some(Peak {
            rate: 0.,
            at: stats.first.unwrap(),
        });
        stats.up_peak = stats.down_peak;
        let mut stmt = db.prepare(
            r#"SELECT a.time,a.down,a.up,a.dp,a.da,a.upk,a.ua FROM speed_app_minutes a
        JOIN speed_minutes m ON m.time=a.time
        WHERE a.kind=?1 AND a.app IN (
            SELECT id FROM dimensions WHERE kind='app' AND name=?2
            UNION ALL SELECT p.app FROM package_app_ids p JOIN dimensions d ON d.id=p.owner
            WHERE ?1=1 AND d.kind='app' AND d.name=?2
        ) AND a.time>=?3 AND a.time<?4 AND m.first>=?5 AND m.last<=?4 ORDER BY a.time"#,
        )?;
        let mut rows = stmt.query(params![
            i64::from(q.group_apps),
            q.app,
            q.from.div_euclid(MINUTE) * MINUTE,
            q.to,
            q.from
        ])?;
        while let Some(row) = rows.next()? {
            let time: i64 = row.get(0)?;
            stats.bytes.add(Bytes {
                down: row.get::<_, i64>(1)?.max(0) as u64,
                up: row.get::<_, i64>(2)?.max(0) as u64,
            });
            choose(
                &mut stats.down_peak,
                row.get(3)?,
                time + row.get::<_, i64>(4)?,
            );
            choose(
                &mut stats.up_peak,
                row.get(5)?,
                time + row.get::<_, i64>(6)?,
            );
        }
    }
    Ok(stats)
}

pub fn read_row_stats(
    db: &Connection,
    q: &Query,
    global: &SpeedStats,
    rows: &[crate::model::Row],
) -> Result<std::collections::HashMap<String, SpeedStats>> {
    let mut result = std::collections::HashMap::new();
    for row in rows {
        let mut stats = global.clone();
        stats.bytes = Bytes::default();
        stats.down_peak = (stats.covered_ms > 0).then(|| Peak {
            rate: 0.,
            at: stats.first.unwrap_or(q.from),
        });
        stats.up_peak = stats.down_peak;
        result.insert(row.key.clone(), stats);
    }
    if rows.is_empty() || global.covered_ms == 0 {
        return Ok(result);
    }
    let names = (6..6 + rows.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "WITH requested AS (SELECT id,name FROM dimensions WHERE kind='app' AND name IN ({names})),
         matching AS (SELECT id,name FROM requested UNION ALL
             SELECT p.app,r.name FROM requested r JOIN package_app_ids p ON p.owner=r.id WHERE ?1=1)
         SELECT d.name,a.time,a.down,a.up,a.dp,a.da,a.upk,a.ua
         FROM matching d JOIN speed_app_minutes a ON a.app=d.id JOIN speed_minutes m ON m.time=a.time
         WHERE a.kind=?1 AND a.time>=?2 AND a.time<?3 AND m.first>=?4 AND m.last<=?5 ORDER BY a.time"
    );
    let mut values = vec![
        rusqlite::types::Value::Integer(i64::from(q.group_apps)),
        (q.from.div_euclid(MINUTE) * MINUTE).into(),
        q.to.into(),
        q.from.into(),
        q.to.into(),
    ];
    values.extend(
        rows.iter()
            .map(|r| rusqlite::types::Value::Text(r.key.clone())),
    );
    let mut stmt = db.prepare(&sql)?;
    let mut records = stmt.query(rusqlite::params_from_iter(values))?;
    while let Some(record) = records.next()? {
        let key: String = record.get(0)?;
        let stats = result.get_mut(&key).expect("queried row");
        let time: i64 = record.get(1)?;
        stats.bytes.add(Bytes {
            down: record.get::<_, i64>(2)?.max(0) as u64,
            up: record.get::<_, i64>(3)?.max(0) as u64,
        });
        choose(
            &mut stats.down_peak,
            record.get(4)?,
            time + record.get::<_, i64>(5)?,
        );
        choose(
            &mut stats.up_peak,
            record.get(6)?,
            time + record.get::<_, i64>(7)?,
        );
    }
    Ok(result)
}

pub fn write_seconds(db: &mut Connection, sample: &Sample) -> Result<()> {
    ensure!(
        (1..=3000).contains(&(sample.to - sample.from)) && (1..=3000).contains(&sample.duration_ms),
        "无效秒级采样区间"
    );
    let tx = db.transaction()?;
    let inserted = tx.execute(
        "INSERT OR IGNORE INTO seconds_samples VALUES(?1,?2,?3,?4,?5,?6)",
        params![
            integer(sample.seq),
            sample.from,
            sample.to,
            integer(sample.duration_ms),
            integer(sample.bytes.down),
            integer(sample.bytes.up)
        ],
    )?;
    if inserted > 0 {
        for (group, key, bytes) in &sample.apps {
            let id = app_id(&tx, key)?;
            tx.execute(
                "INSERT INTO seconds_apps VALUES(?1,?2,?3,?4,?5)",
                params![
                    integer(sample.seq),
                    i64::from(*group),
                    id,
                    integer(bytes.down),
                    integer(bytes.up)
                ],
            )?;
        }
    }
    tx.commit()?;
    Ok(())
}

pub fn raw_bytes(db: &Connection) -> Result<u64> {
    Ok(db.query_row(r#"SELECT COALESCE(SUM(pgsize),0) FROM dbstat WHERE name IN
    ('seconds_samples','seconds_window','seconds_apps','sqlite_autoindex_seconds_apps_1','seconds_app_range')"#,[],|r|r.get::<_,i64>(0))?.max(0) as u64)
}
#[cfg(test)]
pub fn ledger_bytes(db: &Connection) -> Result<u64> {
    // Name equality lets dbstat avoid walking potentially gigabytes of raw pages.
    let mut names = vec!["sqlite_schema".to_string()];
    let mut stmt=db.prepare("SELECT name FROM sqlite_schema WHERE type IN ('table','index') AND name NOT IN ('seconds_samples','seconds_window','seconds_apps','sqlite_autoindex_seconds_apps_1','seconds_app_range')")?;
    names.extend(
        stmt.query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?,
    );
    let mut total = 0u64;
    for name in names {
        total = total.saturating_add(
            db.query_row(
                "SELECT COALESCE(SUM(pgsize),0) FROM dbstat WHERE name=?1",
                [name],
                |r| r.get::<_, i64>(0),
            )?
            .max(0) as u64,
        );
    }
    Ok(total)
}
pub fn status(db: &Connection) -> Result<SecondsStatus> {
    let (first, last) = availability(db, i64::MIN, i64::MAX)?;
    Ok(SecondsStatus {
        bytes: raw_bytes(db)?,
        first,
        last,
        ..Default::default()
    })
}
fn availability(db: &Connection, from: i64, to: i64) -> Result<(Option<i64>, Option<i64>)> {
    let first_end:Option<i64>=db.query_row("SELECT ended FROM seconds_samples WHERE ended>?1 AND ended<=?3 AND started<?2 ORDER BY ended LIMIT 1",params![from,to,to.saturating_add(3000)],|r|r.get(0)).optional()?;
    let first = if let Some(end) = first_end {
        db.query_row(
            "SELECT MIN(started) FROM seconds_samples WHERE ended>=?1 AND ended<=?2 AND started<?3",
            params![end, end.saturating_add(3000), to],
            |r| r.get(0),
        )?
    } else {
        None
    };
    let last=db.query_row("SELECT ended FROM seconds_samples WHERE ended>?1 AND ended<=?3 AND started<?2 ORDER BY ended DESC LIMIT 1",params![from,to,to.saturating_add(3000)],|r|r.get(0)).optional()?;
    Ok((first, last))
}
/// Estimate a row budget from measured pages, then remeasure after that budget.
/// This avoids a complete dbstat walk after every tiny deletion transaction.
pub fn capacity_budget(db: &Connection, policy: SecondsPolicy) -> Result<u64> {
    let Some(limit) = policy.max_bytes else {
        return Ok(0);
    };
    let (bytes,rows):(u64,u64)=db.query_row(r#"SELECT COALESCE(SUM(pgsize),0),
    COALESCE(SUM(CASE WHEN name IN ('seconds_samples','seconds_apps') THEN ncell ELSE 0 END),0)
    FROM dbstat WHERE name IN ('seconds_samples','seconds_window','seconds_apps','sqlite_autoindex_seconds_apps_1','seconds_app_range')"#,[],|r|Ok((r.get(0)?,r.get(1)?)))?;
    if bytes <= limit || rows == 0 {
        return Ok(0);
    }
    Ok(
        ((u128::from(bytes - limit) * u128::from(rows)).div_ceil(u128::from(bytes)))
            .min(u128::from(u64::MAX)) as u64,
    )
}
/// Up to 5,000 rows or 512 complete batches. A larger single batch stays atomic.
#[cfg(test)]
pub fn prune_budget_chunk(
    db: &mut Connection,
    policy: SecondsPolicy,
    now: i64,
    capacity: &mut u64,
) -> Result<bool> {
    let cutoff = policy
        .days
        .map(|days| now.saturating_sub(i64::from(days) * 86_400_000));
    let force = *capacity > 0;
    if cutoff.is_none() && !force {
        return Ok(false);
    }
    let tx = db.transaction()?;
    let mut ids = Vec::new();
    let mut removed = 0u64;
    {
        let mut stmt=tx.prepare("SELECT s.seq,1+(SELECT COUNT(*) FROM seconds_apps a WHERE a.seq=s.seq) FROM seconds_samples s WHERE (?1=1 OR s.ended<=?2) ORDER BY s.ended LIMIT 512")?;
        let mut rows = stmt.query(params![i64::from(force), cutoff])?;
        let target = if force { (*capacity).min(5000) } else { 5000 };
        while let Some(row) = rows.next()? {
            let count = row.get::<_, i64>(1)?.max(1) as u64;
            if !ids.is_empty() && removed + count > 5000 {
                break;
            }
            ids.push(row.get::<_, i64>(0)?);
            removed += count;
            if removed >= target {
                break;
            }
        }
    }
    for id in &ids {
        tx.execute("DELETE FROM seconds_apps WHERE seq=?1", [id])?;
        tx.execute("DELETE FROM seconds_samples WHERE seq=?1", [id])?;
    }
    tx.commit()?;
    *capacity = capacity.saturating_sub(removed);
    Ok(!ids.is_empty())
}
#[cfg(test)]
pub fn prune_chunk(db: &mut Connection, policy: SecondsPolicy, now: i64) -> Result<bool> {
    let mut capacity = capacity_budget(db, policy)?;
    prune_budget_chunk(db, policy, now, &mut capacity)
}

pub fn read_seconds(db: &Connection, req: &SecondsRequest) -> Result<SecondsData> {
    let scope = &req.scope;
    ensure!(scope.to > scope.from, "无效时间范围");
    let (first, last) = availability(db, scope.from, scope.to)?;
    let from = req
        .from
        .unwrap_or_else(|| {
            last.unwrap_or(scope.to.min(chrono::Utc::now().timestamp_millis()))
                .saturating_sub(WINDOW)
        })
        .max(scope.from);
    ensure!(from < scope.to, "定位时间不在所选时段内");
    let (from, to, seconds) = if let Some(view) = req.view {
        (
            view.from.max(scope.from),
            view.to.min(scope.to),
            view.seconds,
        )
    } else {
        (
            from,
            from.saturating_add(WINDOW)
                .min(scope.to)
                .min(if req.from.is_none() {
                    last.unwrap_or(scope.to)
                } else {
                    scope.to
                }),
            true,
        )
    };
    ensure!(
        to > from && (!seconds || to - from <= WINDOW),
        "无效下钻窗口"
    );
    let trend = req
        .view
        .map(|_| crate::traffic_drill::read_bins(db, scope, from, to))
        .transpose()?;
    if !seconds {
        return Ok(SecondsData {
            from,
            to,
            first,
            last,
            samples: vec![],
            trend,
        });
    }
    let id: Option<i64> = if scope.app.is_empty() {
        None
    } else {
        db.query_row(
            "SELECT id FROM dimensions WHERE kind='app' AND name=?1",
            [&scope.app],
            |r| r.get(0),
        )
        .optional()?
    };
    let mut stmt = db.prepare(
        r#"SELECT s.started,s.ended,
    CASE WHEN ?3=1 THEN s.down ELSE COALESCE(SUM(a.down),0) END,
    CASE WHEN ?3=1 THEN s.up ELSE COALESCE(SUM(a.up),0) END,s.duration,s.seq
    FROM seconds_samples s LEFT JOIN seconds_apps a ON a.seq=s.seq AND a.kind=?4 AND a.app IN (
        SELECT ?5 UNION ALL SELECT app FROM package_app_ids WHERE ?4=1 AND owner=?5
    )
    WHERE s.started<?2 AND s.ended>?1 AND s.ended<=?2+3000 GROUP BY s.seq ORDER BY s.ended,s.seq"#,
    )?;
    let samples = stmt
        .query_map(
            params![
                from,
                to,
                i64::from(scope.app.is_empty()),
                i64::from(scope.grouped),
                id
            ],
            |r| {
                let a: i64 = r.get(0)?;
                let b: i64 = r.get(1)?;
                let duration_ms = r.get::<_, i64>(4)?.max(0) as u64;
                let dt = duration_ms.max(1) as f64;
                Ok(SecondSample {
                    seq: r.get(5)?,
                    from: a,
                    to: b,
                    down: r.get::<_, i64>(2)?.max(0) as f64 * 1000. / dt,
                    up: r.get::<_, i64>(3)?.max(0) as f64 * 1000. / dt,
                    duration_ms,
                    valid: b > a && b - a <= 3000 && (1..=3000).contains(&duration_ms),
                })
            },
        )?
        .collect::<rusqlite::Result<_>>()?;
    Ok(SecondsData {
        from,
        to,
        first,
        last,
        samples,
        trend,
    })
}

#[cfg(test)]
#[path = "speed_stats_tests.rs"]
mod tests;
