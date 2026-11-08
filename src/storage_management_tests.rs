use super::*;
use crate::{
    accounting::{Batch, Fact, Ledger},
    model::{Bytes, Query},
    storage::Storage,
};
use chrono::NaiveDate;

const UNLIMITED: Policy = Policy {
    detail_days: None,
    seconds_days: None,
    max_bytes: None,
};
fn now() -> i64 {
    midnight(NaiveDate::from_ymd_opt(2026, 11, 2).unwrap()).unwrap() + 12 * 3_600_000
}
fn count(db: &Connection, table: &str) -> u64 {
    db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}
fn raw(db: &mut Connection, seq: i64, from: i64) {
    speed_stats::write_seconds(
        db,
        &speed_stats::Sample {
            generation: 0,
            seq: seq as u64,
            from,
            to: from + 1000,
            duration_ms: 1000,
            bytes: Bytes { up: 5, down: 10 },
            apps: vec![
                (false, "process.exe".into(), Bytes { up: 5, down: 10 }),
                (true, "host".into(), Bytes { up: 5, down: 10 }),
            ],
        },
    )
    .unwrap();
}
fn facts(store: &mut Storage, sequence: u64, at: i64, n: usize) {
    let bytes = Bytes { up: 5, down: 10 };
    store
        .commit(
            &Ledger {
                sequence,
                time: at + 1000,
                ..Default::default()
            },
            &Batch {
                sample_from: Some(at),
                sample_duration_ms: Some(1000),
                rate: bytes,
                totals: vec![(at, bytes)],
                facts: (0..n)
                    .map(|i| Fact {
                        time: at,
                        app: "process.exe".into(),
                        domain: format!("{i}.example.com"),
                        route: "DIRECT".into(),
                        source: "local".into(),
                        bytes,
                    })
                    .collect(),
                ..Default::default()
            },
        )
        .unwrap();
}
fn finish(w: &mut Worker, db: &mut Connection, policy: Policy) {
    for _ in 0..1000 {
        if !w.step(db, policy, now()).unwrap() {
            w.measure(db, policy, now()).unwrap();
            return;
        }
    }
    panic!("maintenance did not settle");
}
fn worker(path: &Path) -> Worker {
    let mut w = Worker::new(path.into(), Arc::new(Mutex::new(Shared::default())));
    w.reset(0);
    w
}
#[test]
fn unified_accounting_partitions_pages_indexes_and_physical_space() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("traffic.sqlite3");
    let mut store = Storage::open(&path).unwrap();
    facts(&mut store, 1, days_before(now(), 40).unwrap(), 12);
    raw(&mut store.db, 1, now() - 1000);
    store.db.execute_batch("CREATE INDEX extra_raw_index ON seconds_samples(started);CREATE INDEX extra_detail_index ON details(time,source);").unwrap();
    let s = snapshot(&store.db, &path, now()).unwrap();
    let all: u64 = store
        .db
        .query_row("SELECT SUM(pgsize) FROM dbstat", [], |r| r.get(0))
        .unwrap();
    assert_eq!(s.used(), all);
    let raw_bytes:u64=store.db.query_row("SELECT SUM(pgsize) FROM dbstat WHERE name IN (SELECT name FROM sqlite_schema WHERE tbl_name IN ('seconds_samples','seconds_apps'))",[],|r|r.get(0)).unwrap();
    assert_eq!(s.categories[0], raw_bytes);
    let details:u64=store.db.query_row("SELECT SUM(pgsize) FROM dbstat WHERE name IN (SELECT name FROM sqlite_schema WHERE tbl_name IN ('details','attribution_details','events'))",[],|r|r.get(0)).unwrap();
    assert_eq!(s.categories[1], details);
    assert!(s.categories.iter().all(|v| *v > 0));
    assert!(s.physical >= s.wal);
    store
        .db
        .execute(
            "INSERT INTO events VALUES(?1,zeroblob(2000000))",
            [days_before(now(), 40).unwrap()],
        )
        .unwrap();
    let before = snapshot(&store.db, &path, now()).unwrap();
    while clean_details(&mut store.db, days_before(now(), 0).unwrap())
        .unwrap()
        .rows
        > 0
    {}
    let after = snapshot(&store.db, &path, now()).unwrap();
    assert!(after.used() < before.used());
    assert!(after.reusable > before.reusable);
    assert!(after.physical >= after.used());
    reclaim(&store.db).unwrap();
    let packed = snapshot(&store.db, &path, now()).unwrap();
    assert!(packed.physical < after.physical);
    assert_eq!(packed.reusable, 0);
}
#[test]
fn unified_seconds_preview_preserves_new_batches_and_rejects_changed_or_replaced_data() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("traffic.sqlite3");
    let mut store = Storage::open(&path).unwrap();
    facts(&mut store, 1, days_before(now(), 40).unwrap(), 2);
    raw(&mut store.db, 1, now() - 4000);
    raw(&mut store.db, 2, now() - 3000);
    let protected: Vec<_> = ["totals", "app_minutes", "daily", "speed_minutes"]
        .map(|t| count(&store.db, t))
        .into();
    let p = preview(&store.db, 1, 0, Target::Seconds, None, now()).unwrap();
    assert_eq!((p.batches, p.rows), (2, 6));
    raw(&mut store.db, 3, now() - 2000);
    let mut w = worker(&path);
    w.request(&store.db, Request::Clean(p.clone()), now())
        .unwrap();
    finish(&mut w, &mut store.db, UNLIMITED);
    assert_eq!(count(&store.db, "seconds_samples"), 1);
    assert_eq!(count(&store.db, "seconds_apps"), 2);
    assert_eq!(
        store
            .db
            .query_row("SELECT MIN(seq) FROM seconds_samples", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        3
    );
    assert!(w.shared.lock().unwrap().storage_revision > 0);
    assert_eq!(w.shared.lock().unwrap().storage_epoch, 0);
    assert_eq!(
        protected,
        ["totals", "app_minutes", "daily", "speed_minutes"]
            .map(|t| count(&store.db, t))
            .to_vec()
    );
    assert!(w.request(&store.db, Request::Clean(p), now()).is_err());
    let p = preview(&store.db, 2, 0, Target::Seconds, None, now()).unwrap();
    w.reset(1);
    assert!(w.request(&store.db, Request::Clean(p), now()).is_err());
    assert_eq!(count(&store.db, "seconds_samples"), 1);
    let orphans:u64=store.db.query_row("SELECT COUNT(*) FROM seconds_apps a LEFT JOIN seconds_samples s ON a.seq=s.seq WHERE s.seq IS NULL",[],|r|r.get(0)).unwrap();
    assert_eq!(orphans, 0);
}
#[test]
fn unified_detail_boundary_atomic_queries_protect_today_and_retry_partial_failures() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("traffic.sqlite3");
    let mut store = Storage::open(&path).unwrap();
    let old = days_before(now(), 40).unwrap();
    let today = days_before(now(), 0).unwrap();
    facts(&mut store, 1, old, 6200);
    facts(&mut store, 2, today, 1);
    let query = Query {
        from: old,
        to: now(),
        dimension: "domain".into(),
        page_size: 50,
        ..Default::default()
    };
    let before = store.query_snapshot(&query).unwrap();
    let summaries = count(&store.db, "daily");
    let total = store.total(i64::MIN, i64::MAX).unwrap();
    let p = preview(&store.db, 1, 0, Target::Details, None, now()).unwrap();
    assert_eq!(p.cutoff, Some(today));
    assert_eq!(p.rows, 6200);
    let mut w = worker(&path);
    w.request(&store.db, Request::Clean(p), now()).unwrap();
    assert!(w.step(&mut store.db, UNLIMITED, now()).unwrap());
    assert_eq!(count(&store.db, "details"), 1201);
    let after = store.query_snapshot(&query).unwrap();
    assert_eq!(
        before.rows.iter().map(|r| r.bytes.total()).sum::<u64>(),
        after.rows.iter().map(|r| r.bytes.total()).sum::<u64>()
    );
    store.db.execute_batch("CREATE TRIGGER fail_clean BEFORE DELETE ON details BEGIN SELECT RAISE(ABORT,'forced partial failure'); END;").unwrap();
    let error = w.step(&mut store.db, UNLIMITED, now()).unwrap_err();
    w.failed(&error);
    assert!(
        w.shared
            .lock()
            .unwrap()
            .storage_management
            .job_error
            .contains("5000")
    );
    assert!(!w.shared.lock().unwrap().storage_management.busy);
    store.db.execute_batch("DROP TRIGGER fail_clean;").unwrap();
    // Restart resumes already archived chunks even when both expiry policies are off.
    let mut restarted = worker(&path);
    finish(&mut restarted, &mut store.db, UNLIMITED);
    assert_eq!(count(&store.db, "details"), 1);
    assert_eq!(count(&store.db, "daily"), summaries);
    assert_eq!(store.total(i64::MIN, i64::MAX).unwrap(), total);
    let cutoff: String = store
        .db
        .query_row(
            "SELECT value FROM meta WHERE key='detail_cutoff'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(cutoff, today.to_string());
    clean_details(&mut store.db, old).unwrap();
    let cutoff2: String = store
        .db
        .query_row(
            "SELECT value FROM meta WHERE key='detail_cutoff'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(cutoff, cutoff2);
}
#[test]
fn unified_limit_deletes_raw_first_then_old_details_and_stops_at_protected_data() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("traffic.sqlite3");
    let mut store = Storage::open(&path).unwrap();
    let old = days_before(now(), 40).unwrap();
    facts(&mut store, 1, old, 1);
    facts(&mut store, 2, days_before(now(), 0).unwrap(), 1);
    store
        .db
        .execute("INSERT INTO events VALUES(?1,zeroblob(2000000))", [old])
        .unwrap();
    for seq in 1..1500 {
        raw(&mut store.db, seq, old + seq * 1000);
    }
    let bytes = snapshot(&store.db, &path, now()).unwrap();
    let policy = Policy {
        max_bytes: Some(bytes.used() - bytes.categories[0] / 3),
        ..UNLIMITED
    };
    let mut w = worker(&path);
    finish(&mut w, &mut store.db, policy);
    assert_eq!(count(&store.db, "events"), 1);
    assert!(count(&store.db, "seconds_samples") < 1499);
    assert!(!w.shared.lock().unwrap().storage_management.over_limit);
    let protected = count(&store.db, "daily");
    finish(
        &mut w,
        &mut store.db,
        Policy {
            max_bytes: Some(1),
            ..UNLIMITED
        },
    );
    assert_eq!(count(&store.db, "seconds_samples"), 0);
    assert_eq!(count(&store.db, "seconds_apps"), 0);
    assert_eq!(count(&store.db, "events"), 0);
    assert_eq!(count(&store.db, "details"), 1);
    assert_eq!(count(&store.db, "daily"), protected);
    assert!(w.shared.lock().unwrap().storage_management.over_limit);
}
#[test]
fn unified_long_term_and_expiry_use_local_midnight_and_keep_summaries() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("traffic.sqlite3");
    let mut store = Storage::open(&path).unwrap();
    let cutoff = days_before(now(), 7).unwrap();
    raw(&mut store.db, 1, cutoff - 1000);
    raw(&mut store.db, 2, cutoff);
    facts(&mut store, 1, cutoff - 60_000, 1);
    facts(&mut store, 2, cutoff, 1);
    let mut w = worker(&path);
    finish(&mut w, &mut store.db, UNLIMITED);
    assert_eq!(count(&store.db, "seconds_samples"), 2);
    assert_eq!(count(&store.db, "details"), 2);
    let p = preview(&store.db, 1, 0, Target::Seconds, Some(cutoff), now()).unwrap();
    assert_eq!(p.batches, 1);
    assert_eq!(p.last, Some(cutoff));
    let p = preview(&store.db, 2, 0, Target::Details, Some(cutoff), now()).unwrap();
    assert_eq!(p.rows, 1);
    assert_eq!(p.last, Some(cutoff - 60_000));
    finish(
        &mut w,
        &mut store.db,
        Policy {
            detail_days: Some(7),
            seconds_days: Some(1),
            max_bytes: None,
        },
    );
    assert_eq!(count(&store.db, "seconds_samples"), 0);
    assert_eq!(count(&store.db, "details"), 1);
    assert_eq!(count(&store.db, "daily"), 6);
}
#[test]
fn unified_legacy_capacity_migration_stays_disabled_and_roundtrips() {
    let dir = tempfile::tempdir().unwrap();
    for enabled in [false, true] {
        let c = Config {
            retention_size_enabled: enabled,
            seconds_size_enabled: enabled,
            seconds_enabled: true,
            seconds_days_enabled: false,
            seconds_days: 123,
            ..Default::default()
        };
        let source = toml::to_string(&c)
            .unwrap()
            .lines()
            .filter(|l| !l.starts_with("storage_limit_"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(dir.path().join("config.toml"), source).unwrap();
        let loaded = Config::load(dir.path()).unwrap();
        assert!(!loaded.storage_limit_enabled);
        assert_eq!(loaded.storage_limit_mb, 2048);
        assert_eq!(loaded.storage_limit_migration_notice, enabled);
        assert!(loaded.seconds_enabled);
        assert!(!loaded.seconds_days_enabled);
        assert_eq!(loaded.seconds_days, 123);
        assert_eq!(Policy::from(&loaded).max_bytes, None);
        loaded.save(dir.path()).unwrap();
        assert_eq!(Config::load(dir.path()).unwrap(), loaded);
    }
}
