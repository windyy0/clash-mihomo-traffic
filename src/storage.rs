#[cfg(test)]
use crate::config::RetentionPolicy;
use crate::{
    accounting::{Batch, Ledger},
    model::*,
};
use anyhow::{Context, Result};
use rusqlite::functions::FunctionFlags;
use rusqlite::{Connection as Db, OptionalExtension, params};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

#[cfg(test)]
#[path = "storage_history_tests.rs"]
mod history_snapshot_tests;

#[cfg(test)]
#[path = "storage_application_tests.rs"]
mod application_tests;

pub struct Storage {
    pub db: Db,
    path: PathBuf,
}
pub struct Restored {
    pub ledger: Ledger,
    pub totals: (Bytes, Bytes, Bytes),
    pub identities: std::collections::HashMap<String, crate::attribution::Owner>,
}
#[cfg(test)]
pub struct Maintenance {
    pub database_bytes: u64,
    pub over_limit: bool,
}
fn i(v: u64) -> i64 {
    v.min(i64::MAX as u64) as i64
}
fn read_bytes(r: &rusqlite::Row<'_>, n: usize) -> rusqlite::Result<Bytes> {
    Ok(Bytes {
        up: r.get::<_, i64>(n)?.max(0) as u64,
        down: r.get::<_, i64>(n + 1)?.max(0) as u64,
    })
}
pub fn local_day(ms: i64) -> i64 {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap()
        .with_timezone(&chrono::Local)
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_local_timezone(chrono::Local)
        .earliest()
        .unwrap()
        .timestamp_millis()
}
pub fn local_month(ms: i64) -> i64 {
    use chrono::Datelike;
    let now = chrono::DateTime::from_timestamp_millis(ms)
        .unwrap()
        .with_timezone(&chrono::Local);
    now.date_naive()
        .with_day(1)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_local_timezone(chrono::Local)
        .earliest()
        .unwrap()
        .timestamp_millis()
}
impl Storage {
    pub fn read_only(path: &Path) -> Result<Self> {
        let db = Db::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        db.busy_timeout(Duration::from_millis(500))?;
        Self::register_query_functions(&db)?;
        Ok(Self {
            db,
            path: path.to_path_buf(),
        })
    }
    pub fn query_snapshot(&self, query: &Query) -> Result<Report> {
        self.query_snapshot_cached(query, None)
    }
    pub fn query_page(&self, query: &Query, mut report: Report) -> Result<Report> {
        let transaction = self.db.unchecked_transaction()?;
        let mut effective = query.clone();
        if query.history {
            effective.to = effective.to.min(chrono::Utc::now().timestamp_millis() + 1);
        }
        self.fill_report_page(&effective, &mut report)?;
        if query.history && query.dimension == "app" {
            report.row_speeds = crate::speed_stats::read_row_stats(
                &self.db,
                &effective,
                &report.speeds,
                &report.rows,
            )?;
        }
        report.query = Some(query.clone());
        transaction.commit()?;
        Ok(report)
    }
    pub fn query_snapshot_cached(
        &self,
        query: &Query,
        cached: Option<std::sync::Arc<crate::history::Trend>>,
    ) -> Result<Report> {
        anyhow::ensure!(
            query.to > query.from && query.to.saturating_sub(query.from) <= 3660 * 86_400_000_i64,
            "时间范围无效或超过十年"
        );
        let transaction = self.db.unchecked_transaction()?;
        let now = chrono::Utc::now().timestamp_millis();
        let mut effective = query.clone();
        if query.history {
            effective.to = effective.to.min(now.saturating_add(1));
        }
        let mut report = if effective.to <= effective.from {
            Report::default()
        } else if effective.app.is_empty() {
            self.query(&effective)?
        } else {
            self.application_detail(&effective)?
        };
        report.query = Some(query.clone());
        report.speeds = crate::speed_stats::read_stats(&self.db, &effective)?;
        if query.history && query.dimension == "app" && query.app.is_empty() {
            report.row_speeds = crate::speed_stats::read_row_stats(
                &self.db,
                &effective,
                &report.speeds,
                &report.rows,
            )?;
        }
        if query.history && query.app.is_empty() {
            let (from, to) = query.history_context.unwrap_or((query.from, query.to));
            report.history = Some(
                match cached.filter(|h| h.from == from && h.to == to && to < now - 120_000) {
                    Some(value) => value,
                    None => std::sync::Arc::new(crate::history::read(&self.db, from, to, now)?),
                },
            );
        }
        transaction.commit()?;
        Ok(report)
    }
    pub fn calendar_snapshot(
        &self,
        request: crate::history::CalendarRequest,
    ) -> Result<crate::history::CalendarMonth> {
        let transaction = self.db.unchecked_transaction()?;
        let result = crate::history::calendar(&self.db, request)?;
        transaction.commit()?;
        Ok(result)
    }
    fn application_detail(&self, query: &Query) -> Result<Report> {
        anyhow::ensure!(
            query.to > query.from && query.to.saturating_sub(query.from) <= 3660 * 86_400_000_i64,
            "时间范围无效或超过十年"
        );
        let mut report = Report {
            query: Some(query.clone()),
            ..Default::default()
        };
        let table = if query.group_apps {
            "application_minutes"
        } else {
            "app_minutes"
        };
        report.total = self.fill_series(query, table, &mut report)?;
        report.observed = report.total;
        report.app_count = usize::from(report.total.total() > 0);
        let (ranks, archive) = self.application_flow_ranks(query)?;
        [report.domains, report.routes, report.sources] = ranks;
        report.archive_daily = archive;
        report.rows = match query.dimension.as_str() {
            "route" => report.routes.clone(),
            "source" => report.sources.clone(),
            _ => report.domains.clone(),
        };
        report.row_count = report.rows.len();
        if query.group_apps {
            report.members = self.application_members(query)?;
        }
        Ok(report)
    }
    fn application_flow_ranks(&self, query: &Query) -> Result<([Vec<Row>; 3], bool)> {
        type Rank = std::collections::HashMap<i64, (String, Bytes)>;
        let cutoff: i64 = self
            .db
            .query_row(
                "SELECT COALESCE((SELECT value FROM meta WHERE key='detail_cutoff'),'0')",
                [],
                |row| row.get::<_, String>(0),
            )?
            .parse()?;
        let archive = query.from < cutoff;
        let mut ranks: [Rank; 3] = Default::default();
        if archive {
            let table = if query.group_apps {
                "application_daily"
            } else {
                "daily"
            };
            let sql = format!(
                "SELECT f.kind,f.dim,d.name,SUM(f.up),SUM(f.down) FROM {table} f JOIN dimensions a ON a.id=f.app JOIN dimensions d ON d.id=f.dim WHERE f.time>=?1 AND f.time<?2 AND a.name=?3 AND f.kind IN ('domain','route','source') GROUP BY f.kind,f.dim"
            );
            let mut statement = self.db.prepare(&sql)?;
            let rows = statement.query_map(
                params![local_day(query.from), query.to, query.app],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        read_bytes(row, 3)?,
                    ))
                },
            )?;
            for row in rows {
                let (kind, id, name, bytes) = row?;
                let index = match kind.as_str() {
                    "domain" => 0,
                    "route" => 1,
                    _ => 2,
                };
                ranks[index]
                    .entry(id)
                    .or_insert_with(|| (name, Bytes::default()))
                    .1
                    .add(bytes);
            }
        } else {
            let table = if query.group_apps {
                "application_details"
            } else {
                "details"
            };
            let sql = format!(
                "SELECT f.domain,d.name,f.route,r.name,f.source,s.name,SUM(f.up),SUM(f.down) FROM {table} f JOIN dimensions a ON a.id=f.app LEFT JOIN dimensions d ON d.id=f.domain LEFT JOIN dimensions r ON r.id=f.route LEFT JOIN dimensions s ON s.id=f.source WHERE f.time>=?1 AND f.time<?2 AND a.name=?3 GROUP BY f.domain,f.route,f.source"
            );
            let mut statement = self.db.prepare(&sql)?;
            let mut rows = statement.query(params![query.from, query.to, query.app])?;
            while let Some(row) = rows.next()? {
                let bytes = read_bytes(row, 6)?;
                for (index, rank) in ranks.iter_mut().enumerate() {
                    if let (Some(id), Some(name)) = (
                        row.get::<_, Option<i64>>(index * 2)?,
                        row.get::<_, Option<String>>(index * 2 + 1)?,
                    ) {
                        rank.entry(id)
                            .or_insert_with(|| (name, Bytes::default()))
                            .1
                            .add(bytes);
                    }
                }
            }
        }
        Ok((
            ranks.map(|rank| {
                let mut rows: Vec<_> = rank.into_iter().collect();
                rows.sort_by(|(id_a, (name_a, bytes_a)), (id_b, (name_b, bytes_b))| {
                    bytes_b
                        .total()
                        .cmp(&bytes_a.total())
                        .then_with(|| {
                            name_a
                                .to_ascii_lowercase()
                                .cmp(&name_b.to_ascii_lowercase())
                        })
                        .then_with(|| id_a.cmp(id_b))
                });
                rows.into_iter()
                    .take(8)
                    .map(|(_, (key, bytes))| Row { key, bytes })
                    .collect()
            }),
            archive,
        ))
    }
    fn application_members(&self, query: &Query) -> Result<Vec<Row>> {
        Ok(self.db.prepare("SELECT p.name,SUM(f.up),SUM(f.down) FROM application_process_minutes f JOIN dimensions a ON a.id=f.app JOIN dimensions p ON p.id=f.process WHERE f.time>=?1 AND f.time<?2 AND a.name=?3 GROUP BY p.id ORDER BY SUM(f.up)+SUM(f.down) DESC,p.name")?.query_map(params![query.from,query.to,query.app], |row| Ok(Row { key: row.get(0)?, bytes: read_bytes(row, 1)? }))?.collect::<rusqlite::Result<_>>()?)
    }
    fn register_query_functions(db: &Db) -> rusqlite::Result<()> {
        db.create_scalar_function(
            "registrable_domain",
            1,
            FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
            |ctx| {
                let name = ctx.get::<String>(0)?;
                let Ok(url::Host::Domain(host)) = url::Host::parse(name.trim_end_matches('.'))
                else {
                    return Ok(name);
                };
                Ok(psl::domain(host.as_bytes())
                    .filter(|domain| domain.suffix().is_known())
                    .and_then(|domain| std::str::from_utf8(domain.as_bytes()).ok())
                    .map_or(name, str::to_owned))
            },
        )
    }
    pub fn open(path: &Path) -> Result<Self> {
        let db = Db::open(path)?;
        Self::register_query_functions(&db)?;
        db.busy_timeout(Duration::from_secs(5))?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;
        CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY,value TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS dimensions(id INTEGER PRIMARY KEY,kind TEXT NOT NULL,name TEXT NOT NULL,UNIQUE(kind,name));
        CREATE TABLE IF NOT EXISTS totals(time INTEGER PRIMARY KEY,up INTEGER NOT NULL,down INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS app_minutes(time INTEGER,app INTEGER REFERENCES dimensions(id),up INTEGER NOT NULL,down INTEGER NOT NULL,PRIMARY KEY(time,app));
        CREATE INDEX IF NOT EXISTS app_range ON app_minutes(app,time);
        CREATE TABLE IF NOT EXISTS details(time INTEGER,app INTEGER,domain INTEGER,route INTEGER,source INTEGER,up INTEGER NOT NULL,down INTEGER NOT NULL,PRIMARY KEY(time,app,domain,route,source));
        CREATE INDEX IF NOT EXISTS detail_app ON details(app,time);
        CREATE TABLE IF NOT EXISTS daily(time INTEGER,kind TEXT,dim INTEGER,app INTEGER,up INTEGER NOT NULL,down INTEGER NOT NULL,PRIMARY KEY(time,kind,dim,app));
        CREATE TABLE IF NOT EXISTS gaps(id INTEGER PRIMARY KEY,started INTEGER,ended INTEGER,up INTEGER,down INTEGER,reason TEXT);
        CREATE TABLE IF NOT EXISTS events(time INTEGER,message TEXT);
        INSERT OR IGNORE INTO meta VALUES('schema','1');")?;
        let version: String =
            db.query_row("SELECT value FROM meta WHERE key='schema'", [], |r| {
                r.get(0)
            })?;
        anyhow::ensure!(version == "1", "数据库版本不兼容");
        Self::initialize_attribution(&db)?;
        db.execute_batch(crate::speed_stats::SCHEMA)?;
        Ok(Self {
            db,
            path: path.to_path_buf(),
        })
    }
    fn initialize_attribution(db: &Db) -> Result<()> {
        let tx = db.unchecked_transaction()?;
        let migrated: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM meta WHERE key='package_application_groups')",
            [],
            |row| row.get(0),
        )?;
        if !migrated {
            tx.execute_batch(
                "DROP VIEW IF EXISTS application_minutes;
                 DROP VIEW IF EXISTS application_process_minutes;
                 DROP VIEW IF EXISTS application_details;
                 DROP VIEW IF EXISTS application_daily;",
            )?;
        }
        tx.execute_batch(include_str!("attribution.sql"))?;
        let applications = {
            let mut stmt = tx.prepare(
                "SELECT d.id,d.name FROM dimensions d
                 LEFT JOIN owner_metadata o ON o.key=d.name
                 WHERE d.kind='app' AND NOT EXISTS(SELECT 1 FROM package_app_ids p WHERE p.app=d.id)
                 ORDER BY o.key IS NULL,d.id DESC",
            )?;
            stmt.query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (id, path) in applications {
            Self::remember_package(&tx, id, &path)?;
        }
        tx.execute(
            "INSERT OR IGNORE INTO meta VALUES('package_application_groups','1')",
            [],
        )?;
        tx.commit()?;
        Ok(())
    }
    fn remember_package(db: &Db, app: i64, path: &str) -> Result<()> {
        if let Some(owner) = crate::attribution::package_owner(path) {
            db.execute(
                "INSERT OR IGNORE INTO dimensions(kind,name) VALUES('app',?1)",
                [&owner.key],
            )?;
            db.execute(
                "INSERT OR IGNORE INTO package_app_ids
                 SELECT ?1,id FROM dimensions WHERE kind='app' AND name=?2",
                params![app, owner.key],
            )?;
            db.execute(
                "INSERT OR IGNORE INTO owner_metadata VALUES(?1,?2,?3)",
                params![owner.key, owner.label, owner.icon],
            )?;
        }
        Ok(())
    }
    pub fn checkpoint(&self) -> Result<Ledger> {
        let raw: Option<String> = self
            .db
            .query_row("SELECT value FROM meta WHERE key='checkpoint'", [], |r| {
                r.get(0)
            })
            .optional()?;
        raw.map(|s| serde_json::from_str(&s).map_err(Into::into))
            .unwrap_or_else(|| Ok(Ledger::default()))
    }
    pub fn commit(&mut self, ledger: &Ledger, b: &Batch) -> Result<bool> {
        let tx = self.db.transaction()?;
        let seq: u64 = tx
            .query_row(
                "SELECT COALESCE((SELECT value FROM meta WHERE key='sequence'),'0')",
                [],
                |r| r.get::<_, String>(0),
            )?
            .parse()?;
        if ledger.sequence <= seq {
            return Ok(false);
        }
        for (t, v) in &b.totals {
            tx.execute("INSERT INTO totals VALUES(?1,?2,?3) ON CONFLICT(time) DO UPDATE SET up=up+excluded.up,down=down+excluded.down",params![t,i(v.up),i(v.down)])?;
        }
        let mut ids = std::collections::HashMap::<(String, String), i64>::new();
        let mut get_id = |kind: &str, name: &str| -> Result<i64> {
            let key = (kind.to_string(), name.to_string());
            if let Some(id) = ids.get(&key) {
                return Ok(*id);
            }
            let inserted = tx.execute(
                "INSERT OR IGNORE INTO dimensions(kind,name) VALUES(?1,?2)",
                params![kind, name],
            )?;
            let id = tx.query_row(
                "SELECT id FROM dimensions WHERE kind=?1 AND name=?2",
                params![kind, name],
                |r| r.get(0),
            )?;
            if inserted > 0 && kind == "app" {
                Self::remember_package(&tx, id, name)?;
            }
            ids.insert(key, id);
            Ok(id)
        };
        for f in &b.facts {
            let app = get_id("app", &f.app)?;
            let domain = get_id("domain", &f.domain)?;
            let route = get_id("route", &f.route)?;
            let source = get_id("source", &f.source)?;
            tx.execute("INSERT INTO app_minutes VALUES(?1,?2,?3,?4) ON CONFLICT(time,app) DO UPDATE SET up=up+excluded.up,down=down+excluded.down",params![f.time,app,i(f.bytes.up),i(f.bytes.down)])?;
            tx.execute("INSERT INTO details VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(time,app,domain,route,source) DO UPDATE SET up=up+excluded.up,down=down+excluded.down",params![f.time,app,domain,route,source,i(f.bytes.up),i(f.bytes.down)])?;
            for (kind, dim) in [("domain", domain), ("route", route), ("source", source)] {
                tx.execute("INSERT INTO daily VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(time,kind,dim,app) DO UPDATE SET up=up+excluded.up,down=down+excluded.down",params![local_day(f.time),kind,dim,app,i(f.bytes.up),i(f.bytes.down)])?;
            }
        }
        for redirected in &b.owners {
            let f = &redirected.fact;
            let app = get_id("app", &f.app)?;
            let owner = get_id("app", &redirected.owner.key)?;
            let domain = get_id("domain", &f.domain)?;
            let route = get_id("route", &f.route)?;
            let source = get_id("source", &f.source)?;
            tx.execute("INSERT INTO app_attribution_minutes VALUES(?1,?2,?3,?4,?5) ON CONFLICT(time,app,owner) DO UPDATE SET up=up+excluded.up,down=down+excluded.down", params![f.time,app,owner,i(f.bytes.up),i(f.bytes.down)])?;
            tx.execute("INSERT INTO attribution_details VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(time,app,owner,domain,route,source) DO UPDATE SET up=up+excluded.up,down=down+excluded.down", params![f.time,app,owner,domain,route,source,i(f.bytes.up),i(f.bytes.down)])?;
            for (kind, dim) in [("domain", domain), ("route", route), ("source", source)] {
                tx.execute("INSERT INTO attribution_daily VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(time,kind,dim,app,owner) DO UPDATE SET up=up+excluded.up,down=down+excluded.down",params![local_day(f.time),kind,dim,app,owner,i(f.bytes.up),i(f.bytes.down)])?;
            }
        }
        for owner in b
            .identities
            .values()
            .chain(b.owners.iter().map(|fact| &fact.owner))
        {
            tx.execute("INSERT INTO owner_metadata VALUES(?1,?2,?3) ON CONFLICT(key) DO UPDATE SET label=excluded.label,icon=excluded.icon WHERE label!=excluded.label OR icon!=excluded.icon", params![owner.key,owner.label,owner.icon])?;
        }
        if let Some((start, end, v, reason)) = &b.gap {
            tx.execute(
                "INSERT INTO gaps(started,ended,up,down,reason) VALUES(?1,?2,?3,?4,?5)",
                params![start, end, i(v.up), i(v.down), reason],
            )?;
        }
        if let Some(event) = &b.event {
            tx.execute(
                "INSERT INTO events VALUES(?1,?2)",
                params![ledger.time, event],
            )?;
        }
        crate::speed_stats::write_summary(&tx, ledger.time, b)?;
        tx.execute(
            "INSERT OR REPLACE INTO meta VALUES('checkpoint',?1)",
            [serde_json::to_string(ledger)?],
        )?;
        tx.execute(
            "INSERT OR REPLACE INTO meta VALUES('sequence',?1)",
            [ledger.sequence.to_string()],
        )?;
        tx.commit()?;
        Ok(true)
    }
    pub fn total(&self, from: i64, to: i64) -> Result<Bytes> {
        Ok(self.db.query_row("SELECT COALESCE(SUM(up),0),COALESCE(SUM(down),0) FROM totals WHERE time>=?1 AND time<?2",params![from,to],|r|read_bytes(r,0))?)
    }
    pub fn identities(
        &self,
    ) -> Result<std::collections::HashMap<String, crate::attribution::Owner>> {
        Ok(self
            .db
            .prepare("SELECT key,label,icon FROM owner_metadata")?
            .query_map([], |row| {
                let owner = crate::attribution::Owner {
                    key: row.get(0)?,
                    label: row.get(1)?,
                    icon: row.get(2)?,
                };
                Ok((owner.key.clone(), owner))
            })?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn dashboard_totals(&self, now: i64) -> Result<(Bytes, Bytes, Bytes)> {
        Ok((
            self.total(local_day(now), now + 60_000)?,
            self.total(local_month(now), now + 60_000)?,
            self.total(i64::MIN, i64::MAX)?,
        ))
    }
    fn row_sql(&self, q: &Query, dimension: &str) -> Result<(String, bool)> {
        anyhow::ensure!(
            ["app", "domain", "route", "source"].contains(&dimension),
            "未知统计维度"
        );
        let cutoff: i64 = self
            .db
            .query_row(
                "SELECT COALESCE((SELECT value FROM meta WHERE key='detail_cutoff'),'0')",
                [],
                |r| r.get::<_, String>(0),
            )?
            .parse()?;
        let archive = dimension != "app" && q.from < cutoff;
        let table = if dimension == "app" {
            if q.group_apps {
                "application_minutes"
            } else {
                "app_minutes"
            }
        } else if archive {
            if q.group_apps {
                "application_daily"
            } else {
                "daily"
            }
        } else {
            if q.group_apps {
                "application_details"
            } else {
                "details"
            }
        };
        let col = if archive { "dim" } else { dimension };
        let extra = if archive {
            format!(" AND f.kind='{dimension}'")
        } else {
            String::new()
        };
        let sort = match q.sort {
            crate::model::TrafficSort::Total => "SUM(f.up)+SUM(f.down)",
            crate::model::TrafficSort::Download => "SUM(f.down)",
            crate::model::TrafficSort::Upload => "SUM(f.up)",
        };
        let (owner_join, owner_search) = if q.group_apps && dimension == "app" {
            (
                " LEFT JOIN owner_metadata o ON o.key=d.name",
                " OR instr(lower(COALESCE(o.label,'')),lower(?4))>0
                  OR EXISTS(
                      SELECT 1 FROM package_app_ids p JOIN dimensions member ON member.id=p.app
                      WHERE p.owner=d.id AND instr(lower(member.name),lower(?4))>0
                  )",
            )
        } else {
            ("", "")
        };
        let direction = if q.ascending { "ASC" } else { "DESC" };
        let from = format!(
            "FROM {table} f JOIN dimensions d ON d.id=f.{col} JOIN dimensions a ON a.id=f.app{owner_join} WHERE f.time>=?1 AND f.time<?2 AND (?3='' OR a.name=?3) AND (?4='' OR instr(lower(d.name),lower(?4))>0{owner_search}){extra} GROUP BY d.id"
        );
        let sql = if q.history && q.group_domains && dimension == "domain" {
            format!(
                "WITH hosts AS (SELECT d.name,SUM(f.up) AS up,SUM(f.down) AS down {from})
                 SELECT registrable_domain(f.name) AS domain_key,SUM(f.up),SUM(f.down)
                 FROM hosts f GROUP BY domain_key
                 ORDER BY {sort} {direction},domain_key COLLATE NOCASE ASC,domain_key ASC"
            )
        } else {
            format!(
                "SELECT d.name,SUM(f.up),SUM(f.down) {from} ORDER BY {sort} {direction},d.name COLLATE NOCASE ASC,d.id ASC"
            )
        };
        Ok((sql, archive))
    }
    fn rows(
        &self,
        q: &Query,
        dimension: &str,
        limit: usize,
        offset: usize,
    ) -> Result<(Vec<Row>, bool)> {
        let (sql, archive) = self.row_sql(q, dimension)?;
        let from = if archive { local_day(q.from) } else { q.from };
        let mut stmt = self
            .db
            .prepare(&format!("{sql} LIMIT {limit} OFFSET {offset}"))?;
        let rows = stmt
            .query_map(params![from, q.to, q.app, q.search], |r| {
                Ok(Row {
                    key: r.get(0)?,
                    bytes: read_bytes(r, 1)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok((rows, archive))
    }
    pub fn query(&self, q: &Query) -> Result<Report> {
        anyhow::ensure!(
            q.to > q.from && q.to - q.from <= 3660 * 86_400_000_i64,
            "时间范围无效或超过十年"
        );
        let mut r = Report {
            query: Some(q.clone()),
            total: self.total(q.from, q.to)?,
            ..Default::default()
        };
        let app_table = if q.group_apps {
            "application_minutes"
        } else {
            "app_minutes"
        };
        r.app_count = self.db.query_row(
            &format!("SELECT COUNT(DISTINCT f.app) FROM {app_table} f JOIN dimensions d ON d.id=f.app WHERE time>=?1 AND time<?2 AND d.name!='未知进程' AND (?3='' OR d.name=?3)"),
            params![q.from, q.to, q.app], |r| r.get::<_, i64>(0),
        )? as usize;
        r.observed=self.db.query_row(&format!("SELECT COALESCE(SUM(f.up),0),COALESCE(SUM(f.down),0) FROM {app_table} f JOIN dimensions d ON d.id=f.app WHERE time>=?1 AND time<?2 AND (?3='' OR d.name=?3)"),params![q.from,q.to,q.app],|r|read_bytes(r,0))?;
        r.unknown=self.db.query_row("SELECT COALESCE(SUM(f.up),0),COALESCE(SUM(f.down),0) FROM app_minutes f JOIN dimensions d ON d.id=f.app WHERE time>=?1 AND time<?2 AND d.name='未知进程'",params![q.from,q.to],|r|read_bytes(r,0))?;
        if !q.app.is_empty() {
            r.total = r.observed;
            if q.group_apps {
                r.members = self.db.prepare("SELECT p.name,SUM(f.up),SUM(f.down) FROM application_process_minutes f JOIN dimensions a ON a.id=f.app JOIN dimensions p ON p.id=f.process WHERE f.time>=?1 AND f.time<?2 AND a.name=?3 GROUP BY p.id ORDER BY SUM(f.up)+SUM(f.down) DESC,p.name")?.query_map(params![q.from,q.to,q.app], |row| Ok(Row { key: row.get(0)?, bytes: read_bytes(row, 1)? }))?.collect::<rusqlite::Result<_>>()?;
            }
        }
        let (recovered,count)=self.db.query_row("SELECT COALESCE(SUM(up),0),COALESCE(SUM(down),0),COUNT(*) FROM gaps WHERE started>=?1 AND ended<=?2",params![q.from,q.to],|r|Ok((read_bytes(r,0)?,r.get::<_,i64>(2)?)))?;
        r.recovered = recovered;
        r.gaps = count as usize;
        r.crossing_gaps=self.db.query_row("SELECT COUNT(*) FROM gaps WHERE started<?2 AND ended>?1 AND NOT(started>=?1 AND ended<=?2)",params![q.from,q.to],|r|r.get::<_,i64>(0))? as usize;
        r.gap_rows=self.db.prepare("SELECT started,ended,up,down,reason FROM gaps WHERE started<?2 AND ended>?1 ORDER BY started DESC LIMIT 30")?.query_map(params![q.from,q.to],|r|Ok(Gap{from:r.get(0)?,to:r.get(1)?,bytes:read_bytes(r,2)?,reason:r.get(4)?}))?.collect::<rusqlite::Result<Vec<_>>>()?;
        self.fill_report_page(q, &mut r)?;
        let mut rank = q.clone();
        rank.group_domains = false;
        rank.search.clear();
        rank.ascending = false;
        rank.sort = crate::model::TrafficSort::Total;
        r.apps = self.rows(&rank, "app", 8, 0)?.0;
        r.domains = self.rows(&rank, "domain", 8, 0)?.0;
        r.routes = self.rows(&rank, "route", 8, 0)?.0;
        self.fill_series(q, app_table, &mut r)?;
        Ok(r)
    }
    fn fill_report_page(&self, q: &Query, report: &mut Report) -> Result<()> {
        anyhow::ensure!((1..=50).contains(&q.page_size), "每页数量须为 1–50");
        let offset = q.page.checked_mul(q.page_size).context("分页位置无效")?;
        (report.rows, report.archive_daily) = self.rows(q, &q.dimension, q.page_size, offset)?;
        let (sql, archive) = self.row_sql(q, &q.dimension)?;
        report.row_count = self.db.query_row(
            &format!("SELECT COUNT(*) FROM ({sql})"),
            params![
                if archive { local_day(q.from) } else { q.from },
                q.to,
                q.app,
                q.search
            ],
            |r| r.get::<_, i64>(0),
        )? as usize;
        Ok(())
    }
    fn fill_series(&self, q: &Query, app_table: &str, report: &mut Report) -> Result<Bytes> {
        let chart_to = q.to.min(chrono::Utc::now().timestamp_millis() + 60_000);
        let step = ((chart_to - q.from) / 120 / 60_000).max(1) * 60_000;
        let sql = if q.app.is_empty() {
            "SELECT (time/?3)*?3, SUM(up),SUM(down) FROM totals WHERE time>=?1 AND time<?2 AND ?4='' GROUP BY 1".to_string()
        } else {
            format!(
                "SELECT (f.time/?3)*?3,SUM(f.up),SUM(f.down) FROM {app_table} f JOIN dimensions d ON d.id=f.app WHERE time>=?1 AND time<?2 AND d.name=?4 GROUP BY 1"
            )
        };
        let mut histogram = crate::app_trend::Builder::new(q.from, chart_to, q.to);
        let mut raw = std::collections::HashMap::<i64, Bytes>::new();
        let mut statement = self.db.prepare(&sql)?;
        let rows = statement.query_map(
            params![
                q.from,
                q.to,
                if q.app.is_empty() { step } else { 60_000 },
                q.app
            ],
            |r| Ok((r.get::<_, i64>(0)?, read_bytes(r, 1)?)),
        )?;
        let mut observed = Bytes::default();
        for row in rows {
            let (time, bytes) = row?;
            observed.add(bytes);
            raw.entry((time / step) * step).or_default().add(bytes);
            if !q.app.is_empty() {
                histogram.add(time, bytes)?;
            }
        }
        if !q.app.is_empty() {
            report.histogram = histogram.finish()?;
        }
        let mut t = q.from.div_euclid(step) * step;
        while t < chart_to {
            let b = raw.get(&t).copied().unwrap_or_default();
            report.series.push(SeriesPoint {
                time: t.max(q.from),
                up: b.up as f64,
                down: b.down as f64,
            });
            t += step;
        }
        Ok(observed)
    }
    pub fn export(&self, q: &Query, path: &Path) -> Result<()> {
        let mut effective = q.clone();
        if effective.history {
            effective.to = effective.to.min(chrono::Utc::now().timestamp_millis() + 1);
        }
        let q = &effective;
        let (sql, archive) = self.row_sql(q, &q.dimension)?;
        let mut stmt = self.db.prepare(&sql)?;
        let mut rows = stmt.query(params![
            if archive { local_day(q.from) } else { q.from },
            q.to,
            q.app,
            q.search
        ])?;
        let mut file = std::fs::File::create(path)?;
        use std::io::Write;
        file.write_all(&[0xef, 0xbb, 0xbf])?;
        let mut writer = csv::Writer::from_writer(file);
        writer.write_record([
            "维度",
            "项目",
            "上传字节",
            "下载字节",
            "合计字节",
            "开始时间 UTC ms",
            "结束时间 UTC ms",
            "精度",
        ])?;
        while let Some(row) = rows.next()? {
            let name: String = row.get(0)?;
            let b = read_bytes(row, 1)?;
            // Excel formula injection protection for untrusted domain/process names.
            let formula = name.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
            let safe = if name.starts_with(['\t', '\r', '\n'])
                || formula.starts_with(['=', '+', '-', '@', '＝', '＋', '－', '＠'])
            {
                format!("'{name}")
            } else {
                name
            };
            writer.write_record([
                q.dimension.clone(),
                safe,
                b.up.to_string(),
                b.down.to_string(),
                b.total().to_string(),
                q.from.to_string(),
                q.to.to_string(),
                if archive {
                    "按日归档"
                } else {
                    "分钟采样"
                }
                .into(),
            ])?;
        }
        writer.flush()?;
        Ok(())
    }
    pub fn database_bytes(&self) -> u64 {
        let mut wal = self.path.as_os_str().to_os_string();
        wal.push("-wal");
        [&self.path, &PathBuf::from(wal)]
            .into_iter()
            .filter_map(|path| std::fs::metadata(path).ok())
            .map(|meta| meta.len())
            .sum()
    }

    #[cfg(test)]
    fn prune_before(&self, proposed: i64) -> Result<()> {
        let tx = self.db.unchecked_transaction()?;
        let previous: i64 = self
            .db
            .query_row(
                "SELECT COALESCE((SELECT value FROM meta WHERE key='detail_cutoff'),'0')",
                [],
                |r| r.get::<_, String>(0),
            )?
            .parse()?;
        let cutoff = proposed.max(previous);
        tx.execute("DELETE FROM details WHERE time<?1", [cutoff])?;
        tx.execute("DELETE FROM attribution_details WHERE time<?1", [cutoff])?;
        tx.execute("DELETE FROM events WHERE time<?1", [cutoff])?;
        tx.execute(
            "INSERT OR REPLACE INTO meta VALUES('detail_cutoff',?1)",
            [cutoff.to_string()],
        )?;
        tx.commit()?;
        Ok(())
    }

    #[cfg(test)]
    pub fn maintain_policy(&self, policy: RetentionPolicy, now: i64) -> Result<Maintenance> {
        let seconds_saved: bool =
            self.db
                .query_row("SELECT EXISTS(SELECT 1 FROM seconds_samples)", [], |r| {
                    r.get(0)
                })?;
        if let Some(days) = policy.days {
            self.prune_before(local_day(now - i64::from(days) * 86_400_000))?;
        }
        self.db
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA optimize;")?;
        if !seconds_saved
            && policy
                .max_bytes
                .is_some_and(|limit| self.database_bytes() > limit)
        {
            self.compact()?;
        }
        if let Some(limit) = policy.max_bytes
            && crate::speed_stats::ledger_bytes(&self.db)? > limit
        {
            // DELETE leaves reusable pages in the file. Compact before deciding
            // that more historical detail must be discarded.
            if !seconds_saved {
                self.compact()?;
            }
            let today = local_day(now);
            while crate::speed_stats::ledger_bytes(&self.db)? > limit {
                let oldest: Option<i64> = self.db.query_row(
                    "SELECT MIN(time) FROM (SELECT time FROM details UNION ALL SELECT time FROM events) WHERE time<?1", [today], |r| r.get(0),
                )?;
                let Some(oldest) = oldest else { break };
                // Advance the calendar date rather than adding 24 hours;
                // a local day can be 23 or 25 hours long.
                let date = chrono::DateTime::from_timestamp_millis(oldest)
                    .unwrap()
                    .with_timezone(&chrono::Local)
                    .date_naive()
                    .succ_opt()
                    .unwrap();
                let cutoff = date
                    .and_hms_opt(0, 0, 0)
                    .unwrap()
                    .and_local_timezone(chrono::Local)
                    .earliest()
                    .context("无法确定归档日期")?
                    .timestamp_millis();
                self.prune_before(cutoff.min(today))?;
                if !seconds_saved {
                    self.compact()?;
                }
            }
        }
        let database_bytes = self.database_bytes();
        Ok(Maintenance {
            database_bytes,
            over_limit: match policy.max_bytes {
                Some(limit) => crate::speed_stats::ledger_bytes(&self.db)? > limit,
                None => false,
            },
        })
    }

    #[cfg(test)]
    fn compact(&self) -> Result<()> {
        self.db
            .execute_batch("VACUUM; PRAGMA wal_checkpoint(TRUNCATE);")?;
        Ok(())
    }

    #[cfg(test)]
    pub fn maintain(&self, days: u32) -> Result<()> {
        self.maintain_policy(
            RetentionPolicy {
                days: Some(days),
                max_bytes: None,
            },
            chrono::Utc::now().timestamp_millis(),
        )?;
        Ok(())
    }
    pub fn backup(&self, path: &Path) -> Result<()> {
        let mut target = Db::open(path)?;
        rusqlite::backup::Backup::new(&self.db, &mut target)?.run_to_completion(
            128,
            Duration::from_millis(5),
            None,
        )?;
        Ok(())
    }
    pub fn restore(&mut self, path: &Path, rollback: &Path) -> Result<Restored> {
        let source = Db::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        source.busy_timeout(Duration::from_secs(5))?;
        let parent = self.path.parent().context("无法定位账本目录")?;
        let stage = tempfile::Builder::new()
            .prefix(".restore-")
            .tempdir_in(parent)?;
        let stage_path = stage.path().join("traffic.sqlite3");
        let mut staged_db = Db::open(&stage_path)?;
        staged_db.busy_timeout(Duration::from_secs(5))?;
        copy_restore_db(&source, &mut staged_db)?;
        staged_db.execute_batch("PRAGMA trusted_schema=OFF;")?;
        let check: String = staged_db.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
        anyhow::ensure!(check == "ok", "备份校验失败");
        let schema: String = staged_db
            .query_row("SELECT value FROM meta WHERE key='schema'", [], |r| {
                r.get(0)
            })
            .context("不是有效的流量数据库")?;
        anyhow::ensure!(schema == "1", "备份版本不兼容");
        // Older backups may lack optional attribution/speed tables. A missing
        // ledger table is corruption and must not be initialized as empty.
        for (table, columns) in [
            ("meta", "key,value"),
            ("dimensions", "id,kind,name"),
            ("totals", "time,up,down"),
            ("app_minutes", "time,app,up,down"),
            ("details", "time,app,domain,route,source,up,down"),
            ("daily", "time,kind,dim,app,up,down"),
            ("gaps", "id,started,ended,up,down,reason"),
            ("events", "time,message"),
        ] {
            let is_table: bool = staged_db.query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_list(?1) WHERE schema='main' AND type='table')",
                [table],
                |row| row.get(0),
            )?;
            anyhow::ensure!(is_table, "备份缺少账本表 {table}");
            staged_db.prepare(&format!("SELECT {columns} FROM main.{table} LIMIT 0"))?;
        }
        let has_triggers: bool = staged_db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='trigger')",
            [],
            |row| row.get(0),
        )?;
        anyhow::ensure!(!has_triggers, "备份包含不支持的触发器");
        drop(staged_db);
        drop(source);

        let prepared = Self::open(&stage_path)?;
        let now = chrono::Utc::now().timestamp_millis();
        let restored = Restored {
            ledger: prepared.checkpoint().context("备份采集基线无效")?,
            totals: prepared.dashboard_totals(now)?,
            identities: prepared.identities()?,
        };
        self.backup(rollback)?;
        copy_restore_db(&prepared.db, &mut self.db)?;
        Ok(restored)
    }
}

fn copy_restore_db(source: &Db, target: &mut Db) -> Result<()> {
    let backup = rusqlite::backup::Backup::new(source, target)?;
    anyhow::ensure!(
        matches!(backup.step(-1)?, rusqlite::backup::StepResult::Done),
        "数据库正在被占用，请稍后重试"
    );
    Ok(())
}
#[cfg(test)]
#[path = "storage_tests.rs"]
mod tests;
