use crate::model::Bytes;
use anyhow::{Context, Result};
use chrono::{Datelike, Local, Months, NaiveDate};
use rusqlite::{Connection, OptionalExtension, params};

pub const MINUTE: i64 = 60_000;
pub const DAY: i64 = 86_400_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coverage {
    Complete,
    Partial,
    Missing,
    Future,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grain {
    Minute,
    Hour,
    Day,
    Week,
    Months(u32),
}
#[derive(Clone, Debug)]
pub struct Bucket {
    pub from: i64,
    pub to: i64,
    pub bytes: Bytes,
    pub covered_ms: i64,
    pub expected_ms: i64,
    pub observed: bool,
}
impl Bucket {
    pub fn coverage(&self, now: i64) -> Coverage {
        if self.from >= now {
            Coverage::Future
        } else if !self.observed {
            Coverage::Missing
        } else if self.expected_ms > 0 && self.covered_ms >= self.expected_ms {
            Coverage::Complete
        } else {
            Coverage::Partial
        }
    }
}
#[derive(Clone, Debug)]
pub struct Trend {
    pub from: i64,
    pub to: i64,
    pub generated_at: i64,
    pub grain: Grain,
    pub buckets: Vec<Bucket>,
}
impl Trend {
    pub fn total(&self) -> Bytes {
        let mut total = Bytes::default();
        for b in &self.buckets {
            total.add(b.bytes);
        }
        total
    }
    pub fn observed(&self) -> bool {
        self.buckets.iter().any(|b| b.observed)
    }
    pub fn complete(&self) -> bool {
        self.buckets.iter().all(|b| {
            matches!(
                b.coverage(self.generated_at),
                Coverage::Complete | Coverage::Future
            )
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CalendarRequest {
    pub id: u64,
    pub generation: u64,
    pub year: i32,
    pub month: u32,
    pub as_of: i64,
}
#[derive(Clone, Debug)]
pub struct CalendarMonth {
    pub request: CalendarRequest,
    pub days: Vec<Bucket>,
}
impl CalendarMonth {
    pub fn level(&self, day: &Bucket) -> usize {
        let max = self
            .days
            .iter()
            .filter(|b| b.observed)
            .map(|b| b.bytes.total())
            .max()
            .unwrap_or(0);
        if max == 0 || day.bytes.total() == 0 {
            0
        } else {
            ((day.bytes.total() as u128 * 4).div_ceil(max as u128) as usize).clamp(1, 4)
        }
    }
}

pub fn midnight(date: NaiveDate) -> Result<i64> {
    (0..24)
        .find_map(|h| {
            date.and_hms_opt(h, 0, 0)?
                .and_local_timezone(Local)
                .earliest()
                .map(|t| t.timestamp_millis())
        })
        .context("无法确定本地日期边界")
}
pub fn local_date(time: i64) -> NaiveDate {
    chrono::DateTime::from_timestamp_millis(time)
        .expect("validated history timestamp")
        .with_timezone(&Local)
        .date_naive()
}
pub fn day_bounds(date: NaiveDate) -> Result<(i64, i64)> {
    Ok((
        midnight(date)?,
        midnight(date.succ_opt().context("日期超出范围")?)?,
    ))
}
pub fn month_bounds(year: i32, month: u32) -> Result<(i64, i64)> {
    let date = NaiveDate::from_ymd_opt(year, month, 1).context("月份无效")?;
    Ok((
        midnight(date)?,
        midnight(
            date.checked_add_months(Months::new(1))
                .context("月份超出范围")?,
        )?,
    ))
}
fn boundaries(from: i64, to: i64, grain: Grain, now: i64) -> Result<Vec<Bucket>> {
    let local = chrono::DateTime::from_timestamp_millis(from)
        .context("日期无效")?
        .with_timezone(&Local);
    let mut at = match grain {
        Grain::Minute => from.div_euclid(MINUTE) * MINUTE,
        Grain::Hour => {
            let offset = local.offset().local_minus_utc() as i64 * 1000;
            (from + offset).div_euclid(3_600_000) * 3_600_000 - offset
        }
        Grain::Day | Grain::Week => midnight(local.date_naive())?,
        Grain::Months(_) => midnight(local.date_naive().with_day(1).unwrap())?,
    };
    let mut buckets = Vec::new();
    while at < to {
        let next = match grain {
            Grain::Minute => at + MINUTE,
            Grain::Hour => at + 3_600_000,
            Grain::Day => midnight(local_date(at).succ_opt().context("日期无效")?)?,
            Grain::Week => midnight(
                local_date(at)
                    .checked_add_days(chrono::Days::new(7))
                    .context("日期无效")?,
            )?,
            Grain::Months(n) => midnight(
                local_date(at)
                    .checked_add_months(Months::new(n))
                    .context("月份无效")?,
            )?,
        };
        anyhow::ensure!(next > at && buckets.len() < 4000, "历史范围无效");
        let start = from.max(at);
        let end = to.min(next);
        buckets.push(Bucket {
            from: start,
            to: end,
            bytes: Bytes::default(),
            covered_ms: 0,
            expected_ms: (end.min(now.div_euclid(MINUTE) * MINUTE) - start).max(0),
            observed: false,
        });
        at = next;
    }
    Ok(buckets)
}
fn stream(db: &Connection, from: i64, to: i64, buckets: &mut [Bucket]) -> Result<()> {
    let earliest: Option<i64> = db.query_row("SELECT MIN(time) FROM totals", [], |r| r.get(0))?;
    let checkpoint: Option<String> = db
        .query_row("SELECT value FROM meta WHERE key='checkpoint'", [], |r| {
            r.get(0)
        })
        .optional()?;
    let latest = checkpoint
        .map(|s| serde_json::from_str::<crate::accounting::Ledger>(&s))
        .transpose()?
        .map(|l| l.time);
    let mut gaps: Vec<(i64, i64)> = Vec::new();
    let mut stmt = db
        .prepare("SELECT started,ended FROM gaps WHERE started<?2 AND ended>?1 ORDER BY started")?;
    for row in stmt.query_map(params![from, to], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
    })? {
        let (a, b) = row?;
        if let Some(last) = gaps.last_mut()
            && last.1 >= a
        {
            last.1 = last.1.max(b);
        } else {
            gaps.push((a, b));
        }
    }
    let mut stmt =
        db.prepare("SELECT time,up,down FROM totals WHERE time>=?1 AND time<?2 ORDER BY time")?;
    for row in stmt.query_map(params![from, to], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            Bytes {
                up: r.get::<_, i64>(1)?.max(0) as u64,
                down: r.get::<_, i64>(2)?.max(0) as u64,
            },
        ))
    })? {
        let (time, bytes) = row?;
        let index = buckets.partition_point(|b| b.to <= time);
        let Some(bucket) = buckets.get_mut(index).filter(|b| time >= b.from) else {
            continue;
        };
        bucket.observed = true;
        bucket.bytes.add(bytes);
        let gap = gaps.partition_point(|&(a, _)| a < time + MINUTE);
        if earliest.is_some_and(|a| time > a)
            && latest.is_some_and(|b| b >= time + MINUTE)
            && (gap == 0 || gaps[gap - 1].1 <= time)
        {
            bucket.covered_ms += (bucket.to.min(time + MINUTE) - bucket.from.max(time)).max(0);
        }
    }
    for b in buckets {
        b.covered_ms = b.covered_ms.min(b.expected_ms);
    }
    Ok(())
}
pub fn read(db: &Connection, from: i64, to: i64, now: i64) -> Result<Trend> {
    anyhow::ensure!(
        to > from && to.saturating_sub(from) <= 3660 * DAY,
        "时间范围无效或超过十年"
    );
    let span = to - from;
    let mut grain = if span <= 60 * MINUTE {
        Grain::Minute
    } else if span <= 2 * DAY {
        Grain::Hour
    } else if span <= 60 * DAY {
        Grain::Day
    } else if span <= 420 * DAY {
        Grain::Week
    } else {
        Grain::Months(1)
    };
    let mut buckets = boundaries(from, to, grain, now)?;
    while buckets.len() > 60 {
        grain = match grain {
            Grain::Minute => Grain::Hour,
            Grain::Hour => Grain::Day,
            Grain::Day => Grain::Week,
            Grain::Week => Grain::Months(1),
            Grain::Months(n) => Grain::Months(n + 1),
        };
        buckets = boundaries(from, to, grain, now)?;
    }
    stream(db, from, to.min(now.saturating_add(1)), &mut buckets)?;
    Ok(Trend {
        from,
        to,
        generated_at: now,
        grain,
        buckets,
    })
}
pub fn calendar(db: &Connection, request: CalendarRequest) -> Result<CalendarMonth> {
    let (from, to) = month_bounds(request.year, request.month)?;
    let mut days = boundaries(from, to, Grain::Day, request.as_of)?;
    stream(db, from, to.min(request.as_of.saturating_add(1)), &mut days)?;
    Ok(CalendarMonth { request, days })
}

#[cfg(test)]
pub fn fixture(from: i64, to: i64, now: i64) -> Trend {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE totals(time INTEGER PRIMARY KEY,up INTEGER,down INTEGER);CREATE TABLE gaps(started INTEGER,ended INTEGER);CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT);").unwrap();
    let mut result = read(&db, from, to, now).unwrap();
    for (i, b) in result.buckets.iter_mut().enumerate() {
        if b.from < now {
            b.observed = true;
            b.covered_ms = b.expected_ms;
            b.bytes = Bytes {
                down: (i as u64 % 7 + 1) * 32_000_000,
                up: 3_000_000,
            };
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn time(y: i32, m: u32, d: u32, h: u32, min: u32) -> i64 {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, min, 0)
            .unwrap()
            .and_local_timezone(Local)
            .earliest()
            .unwrap()
            .timestamp_millis()
    }
    fn schema(db: &Connection) {
        db.execute_batch("PRAGMA journal_mode=WAL;CREATE TABLE totals(time INTEGER PRIMARY KEY,up INTEGER,down INTEGER);CREATE TABLE gaps(started INTEGER,ended INTEGER);CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT);").unwrap();
    }
    fn checkpoint(db: &Connection, now: i64) {
        let ledger = crate::accounting::Ledger {
            time: now,
            ..Default::default()
        };
        db.execute(
            "INSERT OR REPLACE INTO meta VALUES('checkpoint',?1)",
            [serde_json::to_string(&ledger).unwrap()],
        )
        .unwrap();
    }
    #[test]
    fn calendar_distinguishes_zero_missing_partial_and_future_without_future_bytes() {
        let mut db = Connection::open_in_memory().unwrap();
        schema(&db);
        let a = time(2024, 2, 28, 0, 0);
        let b = time(2024, 2, 29, 0, 0);
        let now = time(2024, 2, 29, 12, 2) + 30_000;
        {
            let tx = db.transaction().unwrap();
            tx.execute("INSERT INTO totals VALUES(?1,0,0)", [a - MINUTE])
                .unwrap();
            for i in 0..1440 {
                tx.execute("INSERT INTO totals VALUES(?1,0,0)", [a + i * MINUTE])
                    .unwrap();
            }
            for i in 0..=722 {
                tx.execute("INSERT INTO totals VALUES(?1,3,7)", [b + i * MINUTE])
                    .unwrap();
            }
            tx.execute(
                "INSERT INTO totals VALUES(?1,999999,999999)",
                [time(2024, 2, 29, 13, 0)],
            )
            .unwrap();
            tx.execute(
                "INSERT INTO gaps VALUES(?1,?2)",
                params![b + 10 * MINUTE + 1, b + 10 * MINUTE + 2],
            )
            .unwrap();
            checkpoint(&tx, now);
            tx.commit().unwrap();
        }
        let month = calendar(
            &db,
            CalendarRequest {
                id: 1,
                generation: 4,
                year: 2024,
                month: 2,
                as_of: now,
            },
        )
        .unwrap();
        assert_eq!(month.days.len(), 29);
        assert_eq!(month.days[25].coverage(now), Coverage::Missing);
        assert_eq!(month.days[27].coverage(now), Coverage::Complete);
        assert_eq!(month.days[27].bytes.total(), 0);
        assert_eq!(month.days[28].coverage(now), Coverage::Partial);
        assert_eq!(month.days[28].covered_ms, 721 * MINUTE);
        assert_eq!(month.days[28].expected_ms, 722 * MINUTE);
        assert_eq!(month.days[28].bytes.total(), 7230);
        assert_eq!(month.level(&month.days[27]), 0);
        assert_eq!(month.level(&month.days[28]), 4);
        let future = calendar(
            &db,
            CalendarRequest {
                id: 2,
                generation: 4,
                year: 2024,
                month: 3,
                as_of: now,
            },
        )
        .unwrap();
        assert!(
            future
                .days
                .iter()
                .all(|d| d.coverage(now) == Coverage::Future)
        );
        let trend = read(&db, b, time(2024, 3, 1, 0, 0), now).unwrap();
        assert_eq!(trend.total().total(), 7230);
        assert!(!trend.complete());
    }
    #[test]
    fn minute_boundaries_tail_and_every_granularity_conserve_observed_bytes() {
        let mut db = Connection::open_in_memory().unwrap();
        schema(&db);
        let a = time(2024, 2, 28, 9, 17);
        let b = time(2024, 2, 28, 10, 39);
        {
            let tx = db.transaction().unwrap();
            tx.execute("INSERT INTO totals VALUES(?1,0,0)", [a - MINUTE])
                .unwrap();
            for i in 0..82 {
                tx.execute("INSERT INTO totals VALUES(?1,2,5)", [a + i * MINUTE])
                    .unwrap();
            }
            checkpoint(&tx, b);
            tx.commit().unwrap();
        }
        let clipped = read(&db, a, b, b).unwrap();
        assert_eq!(clipped.grain, Grain::Hour);
        assert_eq!(clipped.buckets.len(), 2);
        assert_eq!(clipped.buckets[0].from, a);
        assert_eq!(clipped.buckets[1].to, b);
        assert_eq!(clipped.total(), Bytes { up: 164, down: 410 });
        assert!(clipped.complete());
        for end in [
            b,
            a + 30 * DAY,
            a + 180 * DAY,
            a + 1000 * DAY,
            a + 3660 * DAY,
        ] {
            let report = read(&db, a, end, end).unwrap();
            assert!(report.buckets.len() <= 60);
            assert_eq!(report.total(), clipped.total());
            assert_eq!(report.buckets.last().unwrap().to, end);
        }
        assert!(read(&db, a, a + 3660 * DAY + 1, b).is_err());
        assert!(read(&db, b, a, b).is_err());
    }
    #[test]
    fn hour_drill_uses_minutes_and_preserves_missing_zero_future_and_totals() {
        let db = Connection::open_in_memory().unwrap();
        schema(&db);
        let from = time(2024, 2, 29, 14, 0);
        let to = from + 60 * MINUTE;
        db.execute("INSERT INTO totals VALUES(?1,0,0)", [from - MINUTE])
            .unwrap();
        for i in 0..60 {
            if i != 7 {
                db.execute(
                    "INSERT INTO totals VALUES(?1,?2,?3)",
                    params![from + i * MINUTE, i, 2 * i],
                )
                .unwrap();
            }
        }
        checkpoint(&db, to);
        let hour = read(&db, from, to, to).unwrap();
        assert_eq!(hour.grain, Grain::Minute);
        assert_eq!(hour.buckets.len(), 60);
        assert_eq!(hour.buckets[0].coverage(to), Coverage::Complete);
        assert_eq!(hour.buckets[0].bytes.total(), 0);
        assert_eq!(hour.buckets[7].coverage(to), Coverage::Missing);
        assert_eq!(hour.total().total(), (59 * 60 / 2 - 7) * 3);
        let day = read(&db, time(2024, 2, 29, 0, 0), time(2024, 3, 1, 0, 0), to).unwrap();
        assert_eq!(day.total(), hour.total());
        let partial = read(&db, from + 17 * MINUTE, from + 43 * MINUTE, to).unwrap();
        assert_eq!(partial.buckets.len(), 26);
        assert_eq!(partial.buckets.last().unwrap().to, from + 43 * MINUTE);
        let now = from + 30 * MINUTE + 30_000;
        let current = read(&db, from, to, now).unwrap();
        assert_eq!(current.buckets[31].coverage(now), Coverage::Future);
        assert_eq!(current.buckets[31].bytes.total(), 0);
        assert_eq!(current.total().total(), (30 * 31 / 2 - 7) * 3);
    }
    #[test]
    fn explicit_calendar_boundaries_use_actual_day_lengths() {
        let db = Connection::open_in_memory().unwrap();
        schema(&db);
        let a = time(2024, 3, 31, 0, 0);
        for duration in [23 * 3_600_000, 25 * 3_600_000] {
            let mut buckets = vec![Bucket {
                from: a,
                to: a + duration,
                bytes: Bytes::default(),
                covered_ms: 0,
                expected_ms: duration,
                observed: false,
            }];
            stream(&db, a, a + duration, &mut buckets).unwrap();
            assert_eq!(buckets[0].expected_ms, duration);
            assert_eq!(buckets[0].coverage(a + duration), Coverage::Missing);
        }
        let (start, end) = month_bounds(2024, 2).unwrap();
        let daily = boundaries(start, end, Grain::Day, end).unwrap();
        assert_eq!(daily.len(), 29);
        assert_eq!(daily.first().unwrap().from, start);
        assert_eq!(daily.last().unwrap().to, end);
    }
    #[test]
    fn read_only_wal_transaction_is_frozen_and_does_not_block_a_writer() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ledger.sqlite");
        let writer = Connection::open(&path).unwrap();
        schema(&writer);
        let a = time(2024, 1, 1, 0, 0);
        writer
            .execute("INSERT INTO totals VALUES(?1,0,0)", [a - MINUTE])
            .unwrap();
        writer
            .execute("INSERT INTO totals VALUES(?1,1,2)", [a])
            .unwrap();
        checkpoint(&writer, a + 3 * MINUTE);
        let mut reader =
            Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let tx = reader.transaction().unwrap();
        let original = read(&tx, a, a + 3 * MINUTE, a + 3 * MINUTE).unwrap();
        writer
            .execute("INSERT INTO totals VALUES(?1,10,20)", [a + MINUTE])
            .unwrap();
        assert_eq!(
            read(&tx, a, a + 3 * MINUTE, a + 3 * MINUTE)
                .unwrap()
                .total(),
            original.total()
        );
        tx.commit().unwrap();
        assert_eq!(
            read(&reader, a, a + 3 * MINUTE, a + 3 * MINUTE)
                .unwrap()
                .total()
                .total(),
            33
        );
        assert_eq!(original.total().total(), 3);
    }
}
