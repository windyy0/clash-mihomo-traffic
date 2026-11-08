use super::*;
use crate::{accounting::Ledger, storage::Storage};

fn database() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("PRAGMA foreign_keys=ON; CREATE TABLE dimensions(id INTEGER PRIMARY KEY,kind TEXT,name TEXT,UNIQUE(kind,name));").unwrap();
    db.execute_batch(SCHEMA).unwrap();
    db.execute_batch(include_str!("attribution.sql")).unwrap();
    db
}
fn batch(from: i64, to: i64, down: u64, up: u64) -> Batch {
    let bytes = Bytes { down, up };
    Batch {
        sample_from: Some(from),
        rate: bytes,
        totals: crate::accounting::split(from, to, bytes),
        ..Default::default()
    }
}
fn query(from: i64, to: i64, app: &str, grouped: bool) -> Query {
    Query {
        from,
        to,
        app: app.into(),
        group_apps: grouped,
        ..Default::default()
    }
}
fn request(from: Option<i64>, start: i64, end: i64, app: &str, grouped: bool) -> SecondsRequest {
    SecondsRequest {
        id: 1,
        generation: 0,
        revision: 0,
        scope: Scope {
            from: start,
            to: end,
            app: app.into(),
            grouped,
        },
        from,
        view: None,
    }
}
#[test]
fn actual_intervals_idle_zero_independent_peaks_and_partial_legacy_coverage() {
    let db = database();
    write_summary(&db, 12_000, &batch(10_000, 12_000, 4000, 100)).unwrap();
    write_summary(&db, 13_000, &batch(12_000, 13_000, 0, 4000)).unwrap();
    write_summary(&db, 14_000, &batch(13_000, 14_000, 2000, 0)).unwrap();
    write_summary(&db, 15_000, &batch(14_000, 15_000, 0, 0)).unwrap();
    let s = read_stats(&db, &query(0, 60_000, "", true)).unwrap();
    assert_eq!(
        s.bytes,
        Bytes {
            down: 6000,
            up: 4100
        }
    );
    assert_eq!(s.covered_ms, 5000);
    assert_eq!(s.average(), Some((1200., 820.)));
    assert!(s.partial());
    assert_eq!(s.down_peak.unwrap().at, 10_000);
    assert_eq!(s.down_peak.unwrap().rate, 2000.);
    assert_eq!(s.up_peak.unwrap().at, 12_000);
    assert_eq!(s.up_peak.unwrap().rate, 4000.);
    let old = read_stats(&db, &query(0, 5000, "", true)).unwrap();
    assert!(old.average().is_none());
    assert!(old.down_peak.is_none());
    let complete = read_stats(&db, &query(10_000, 60_000, "", true)).unwrap();
    assert!(!complete.partial());
}
#[test]
fn cross_minute_bytes_and_duration_are_conserved_with_earliest_ties() {
    let db = database();
    let mut b = batch(59_333, 61_001, 17, 43);
    b.app_rates
        .insert("child".into(), Bytes { down: 7, up: 19 });
    b.application_rates
        .insert("owner".into(), Bytes { down: 7, up: 19 });
    write_summary(&db, 61_001, &b).unwrap();
    let all = read_stats(&db, &query(0, 120_000, "", false)).unwrap();
    let a = read_stats(&db, &query(0, 60_000, "", false)).unwrap();
    let z = read_stats(&db, &query(60_000, 120_000, "", false)).unwrap();
    assert_eq!(a.bytes.total() + z.bytes.total(), all.bytes.total());
    assert_eq!(all.bytes, b.rate);
    assert_eq!(a.covered_ms + z.covered_ms, 1668);
    assert_eq!(z.down_peak.unwrap().at, 60_000);
    assert_eq!(
        read_stats(&db, &query(0, 120_000, "owner", true))
            .unwrap()
            .bytes,
        Bytes { down: 7, up: 19 }
    );
}
#[test]
fn grouped_peaks_are_per_sample_and_apps_share_idle_denominator() {
    let db = database();
    for (from, a, b) in [(1000, 900, 100), (2000, 100, 900), (3000, 0, 0)] {
        let mut sample = batch(from, from + 1000, a + b, 0);
        if a > 0 {
            sample
                .app_rates
                .insert("a".into(), Bytes { down: a, up: 0 });
        }
        if b > 0 {
            sample
                .app_rates
                .insert("b".into(), Bytes { down: b, up: 0 });
        }
        if a + b > 0 {
            sample
                .application_rates
                .insert("host".into(), Bytes { down: a + b, up: 0 });
        }
        write_summary(&db, from + 1000, &sample).unwrap();
    }
    let host = read_stats(&db, &query(1000, 60_000, "host", true)).unwrap();
    assert_eq!(host.down_peak.unwrap().rate, 1000.);
    assert_eq!(host.down_peak.unwrap().at, 1000);
    let child = read_stats(&db, &query(1000, 60_000, "a", false)).unwrap();
    assert_eq!(child.average().unwrap().0, 1000. / 3.);
    let idle = read_stats(&db, &query(1000, 60_000, "never-active", true)).unwrap();
    assert_eq!(idle.average(), Some((0., 0.)));
    assert_eq!(idle.down_peak.unwrap().rate, 0.);
}
#[test]
fn baseline_disconnect_counter_reset_and_recovery_never_create_speed_data() {
    let db = database();
    let mut ledger = Ledger::default();
    let snapshot = |down| crate::model::Snapshot {
        download_total: down,
        upload_total: 0,
        connections: Some(vec![]),
    };
    for (time, down, interrupted) in [
        (1000, 100, false),
        (2000, 110, false),
        (10_000, 10_000, true),
        (11_000, 2, false),
        (12_000, 12, false),
    ] {
        let b = ledger.ingest(&snapshot(down), "same-core", time, interrupted);
        write_summary(&db, time, &b).unwrap();
        assert_eq!(
            Sample::from_batch(0, ledger.sequence, time, &b).is_some(),
            matches!(time, 2000 | 12_000)
        );
    }
    let s = read_stats(&db, &query(0, 60_000, "", false)).unwrap();
    assert_eq!(s.covered_ms, 2000);
    assert_eq!(s.bytes.down, 20);
    assert_eq!(s.down_peak.unwrap().rate, 10.);
    assert!(s.partial());
}
#[test]
fn main_transaction_retry_is_atomic_and_duplicate_commit_does_not_repeat_summaries() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("traffic.sqlite3");
    let mut store = Storage::open(&path).unwrap();
    let external = Connection::open(&path).unwrap();
    external.execute_batch("CREATE TRIGGER reject_speed BEFORE INSERT ON speed_minutes BEGIN SELECT RAISE(FAIL,'forced summary failure'); END;").unwrap();
    let ledger = Ledger {
        time: 2000,
        sequence: 1,
        ..Default::default()
    };
    let b = batch(1000, 2000, 99, 33);
    assert!(store.commit(&ledger, &b).is_err());
    assert_eq!(store.checkpoint().unwrap().sequence, 0);
    external
        .execute_batch("DROP TRIGGER reject_speed;")
        .unwrap();
    assert!(store.commit(&ledger, &b).unwrap());
    assert!(!store.commit(&ledger, &b).unwrap());
    let report = store.query_snapshot(&query(0, 60_000, "", false)).unwrap();
    assert_eq!(report.total, b.rate);
    assert_eq!(report.speeds.bytes, b.rate);
    assert_eq!(report.speeds.covered_ms, 1000);
}
#[test]
fn seconds_sparse_zero_gaps_exact_intervals_and_ten_minute_bounds() {
    let mut db = database();
    for (seq, from, to, down) in [
        (1, 1000, 2500, 3000),
        (2, 2500, 4000, 0),
        (3, 10_000, 11_000, 1000),
        (4, 400_000, 401_000, 1),
    ] {
        let mut b = batch(from, to, down, 0);
        if down > 0 {
            b.app_rates.insert(
                "a".into(),
                Bytes {
                    down: down / 2,
                    up: 0,
                },
            );
            b.application_rates
                .insert("host".into(), Bytes { down, up: 0 });
        }
        let sample = Sample::from_batch(0, seq, to, &b).unwrap();
        write_seconds(&mut db, &sample).unwrap();
        write_seconds(&mut db, &sample).unwrap();
    }
    let data = read_seconds(&db, &request(Some(0), 0, 500_000, "a", false)).unwrap();
    assert_eq!((data.from, data.to), (0, 500_000));
    assert_eq!(data.samples.len(), 4);
    assert_eq!(data.samples[0].down, 1000.);
    assert_eq!(
        read_seconds(&db, &request(Some(0), 0, 500_000, "host", true))
            .unwrap()
            .samples[0]
            .down,
        2000.
    );
    assert_eq!(data.samples[1].down, 0.);
    assert!(data.samples[2].from > data.samples[1].to);
    let idle = read_seconds(&db, &request(Some(0), 0, 500_000, "absent", true)).unwrap();
    assert!(idle.samples.iter().all(|s| s.down == 0.));
    let latest = read_seconds(&db, &request(None, 0, 500_000, "", false)).unwrap();
    assert_eq!(latest.to, 401_000);
    assert!(read_seconds(&db, &request(Some(500_000), 0, 500_000, "", false)).is_err());
    assert!(
        read_seconds(&db, &request(Some(450_000), 0, 500_000, "", false))
            .unwrap()
            .samples
            .is_empty()
    );
}
#[test]
fn raw_failure_rolls_back_entire_batch_without_affecting_next_summary() {
    let mut db = database();
    let mut b = batch(1000, 2000, 100, 50);
    b.app_rates.insert("a".into(), b.rate);
    write_summary(&db, 2000, &b).unwrap();
    db.execute_batch("CREATE TRIGGER reject_raw BEFORE INSERT ON seconds_apps BEGIN SELECT RAISE(FAIL,'forced raw failure'); END;").unwrap();
    assert!(write_seconds(&mut db, &Sample::from_batch(0, 1, 2000, &b).unwrap()).is_err());
    assert_eq!(status(&db).unwrap().first, None);
    write_summary(&db, 3000, &batch(2000, 3000, 10, 10)).unwrap();
    assert_eq!(
        read_stats(&db, &query(0, 60_000, "", false))
            .unwrap()
            .covered_ms,
        2000
    );
}
#[test]
fn cleanup_when_disabled_removes_whole_batches_and_isolates_summaries() {
    let mut db = database();
    for (seq, from) in [(1, 1000), (2, 86_401_000)] {
        let mut b = batch(from, from + 1000, 40, 20);
        b.app_rates.insert("a".into(), b.rate);
        write_summary(&db, from + 1000, &b).unwrap();
        write_seconds(
            &mut db,
            &Sample::from_batch(0, seq, from + 1000, &b).unwrap(),
        )
        .unwrap();
    }
    let policy = SecondsPolicy {
        enabled: false,
        days: Some(1),
        max_bytes: None,
    };
    assert!(prune_chunk(&mut db, policy, 86_403_000).unwrap());
    assert!(!prune_chunk(&mut db, policy, 86_403_000).unwrap());
    assert_eq!(status(&db).unwrap().first, Some(86_401_000));
    let orphan:i64=db.query_row("SELECT COUNT(*) FROM seconds_apps a LEFT JOIN seconds_samples s ON s.seq=a.seq WHERE s.seq IS NULL",[],|r|r.get(0)).unwrap();
    assert_eq!(orphan, 0);
    assert_eq!(
        read_stats(&db, &query(0, 2 * 86_400_000, "", false))
            .unwrap()
            .bytes
            .down,
        80
    );
    let cap = SecondsPolicy {
        max_bytes: Some(0),
        ..policy
    };
    while prune_chunk(&mut db, cap, 86_403_000).unwrap() {}
    assert!(status(&db).unwrap().first.is_none());
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM seconds_apps", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
#[test]
fn raw_capacity_never_spends_legacy_retention_quota_and_indexes_are_counted() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("traffic.sqlite3");
    let store = Storage::open(&path).unwrap();
    let db = Connection::open(&path).unwrap();
    db.execute_batch("INSERT INTO events VALUES(1,'keep this legacy event'); WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<75000) INSERT INTO seconds_samples SELECT x,x*1000,(x+1)*1000,1000,100,200 FROM n;").unwrap();
    assert!(raw_bytes(&db).unwrap() > 1024 * 1024);
    assert!(ledger_bytes(&db).unwrap() < 1024 * 1024);
    let data_only: i64 = db
        .query_row(
            "SELECT SUM(pgsize) FROM dbstat WHERE name IN ('seconds_samples','seconds_apps')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(raw_bytes(&db).unwrap() > data_only as u64);
    let maintained = store
        .maintain_policy(
            crate::config::RetentionPolicy {
                days: None,
                max_bytes: Some(1024 * 1024),
            },
            100 * 86_400_000,
        )
        .unwrap();
    assert!(!maintained.over_limit);
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM events", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}
#[test]
fn single_file_backup_includes_seconds_and_old_restore_initializes_tables() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.sqlite3");
    let mut store = Storage::open(&source).unwrap();
    let b = batch(1000, 2000, 7, 3);
    store
        .commit(
            &Ledger {
                time: 2000,
                sequence: 1,
                ..Default::default()
            },
            &b,
        )
        .unwrap();
    let mut conn = Connection::open(&source).unwrap();
    write_seconds(&mut conn, &Sample::from_batch(0, 1, 2000, &b).unwrap()).unwrap();
    let backup = dir.path().join("backup.sqlite3");
    store.backup(&backup).unwrap();
    let mut target = Storage::open(&dir.path().join("target.sqlite3")).unwrap();
    target
        .restore(&backup, &dir.path().join("rollback.sqlite3"))
        .unwrap();
    assert_eq!(
        target
            .query_snapshot(&query(0, 60_000, "", false))
            .unwrap()
            .speeds
            .bytes,
        b.rate
    );
    assert_eq!(
        status(&Connection::open(dir.path().join("target.sqlite3")).unwrap())
            .unwrap()
            .first,
        Some(1000)
    );
    let legacy = Connection::open(&backup).unwrap();
    legacy.execute_batch("DROP TABLE seconds_apps;DROP TABLE seconds_samples;DROP TABLE speed_app_minutes;DROP TABLE speed_minutes;").unwrap();
    drop(legacy);
    target
        .restore(&backup, &dir.path().join("rollback2.sqlite3"))
        .unwrap();
    let report = target.query_snapshot(&query(0, 60_000, "", false)).unwrap();
    assert_eq!(report.total, b.rate);
    assert!(report.speeds.average().is_none());
    assert!(
        status(&Connection::open(dir.path().join("target.sqlite3")).unwrap())
            .unwrap()
            .first
            .is_none()
    );
}
#[test]
fn old_config_defaults_and_seconds_preferences_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.toml"), "retention_days = 90\n").unwrap();
    let mut c = Config::load(dir.path()).unwrap();
    assert!(!c.seconds_enabled);
    assert_eq!(c.seconds_days, 3);
    assert!(c.seconds_days_enabled);
    assert_eq!(SecondsPolicy::from(&c).days, Some(3));
    assert_eq!(c.seconds_time_unit, crate::config::RetentionTimeUnit::Days);
    assert_eq!(c.seconds_size_mb, 2048);
    assert!(!c.seconds_size_enabled);
    c.seconds_enabled = true;
    c.seconds_days = 60;
    c.seconds_size_enabled = true;
    c.seconds_days_enabled = false;
    c.save(dir.path()).unwrap();
    assert_eq!(Config::load(dir.path()).unwrap(), c);
    assert_eq!(
        SecondsPolicy::from(&Config::load(dir.path()).unwrap()).days,
        None
    );
    c.seconds_days = 0;
    assert!(c.validate().is_err());
    c.seconds_days = 30;
    c.seconds_size_mb = 0;
    assert!(c.validate().is_err());
}

#[test]
fn capacity_cleans_recent_data_oldest_first_in_bounded_chunks() {
    let mut db = database();
    db.execute_batch("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<75000) INSERT INTO seconds_samples SELECT x,x*1000,(x+1)*1000,1000,100,200 FROM n;").unwrap();
    let policy = SecondsPolicy {
        enabled: false,
        days: Some(30),
        max_bytes: Some(1024 * 1024),
    };
    let mut budget = capacity_budget(&db, policy).unwrap();
    assert!(budget > 0);
    assert!(prune_budget_chunk(&mut db, policy, 80_000_000, &mut budget).unwrap());
    let remaining: i64 = db
        .query_row("SELECT COUNT(*) FROM seconds_samples", [], |r| r.get(0))
        .unwrap();
    assert!(remaining >= 75000 - 512);
    loop {
        if !prune_budget_chunk(&mut db, policy, 80_000_000, &mut budget).unwrap() {
            budget = capacity_budget(&db, policy).unwrap();
            if budget == 0 {
                break;
            }
        }
    }
    assert!(raw_bytes(&db).unwrap() <= 1024 * 1024);
    let first: i64 = db
        .query_row("SELECT MIN(seq) FROM seconds_samples", [], |r| r.get(0))
        .unwrap();
    assert!(first > 1);
    assert_eq!(
        db.query_row("SELECT MAX(seq) FROM seconds_samples", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        75000
    );
}

#[test]
#[ignore = "prints isolated SQLite capacity estimates"]
fn raw_capacity_estimate() {
    for active in [0, 10, 20] {
        let mut db = database();
        let tx = db.transaction().unwrap();
        for i in 0..6000 {
            tx.execute(
                "INSERT INTO seconds_samples VALUES(?1,?2,?3,1000,1000000,500000)",
                params![
                    1_000_000 + i,
                    1_793_491_200_000i64 + i * 1000,
                    1_793_491_201_000i64 + i * 1000
                ],
            )
            .unwrap();
            for j in 0..active {
                tx.execute(
                    "INSERT INTO seconds_apps VALUES(?1,?2,?3,100000,50000)",
                    params![1_000_000 + i, j % 2, 10000 + j],
                )
                .unwrap();
            }
        }
        tx.commit().unwrap();
        let per = raw_bytes(&db).unwrap() as f64 / 6000.;
        println!(
            "{active} active statistics items: {per:.1} bytes/sample including indexes, {:.2} GiB/30 days continuously active",
            per * 86400. * 30. / 1024f64.powi(3)
        );
    }
}

#[test]
fn monotonic_duration_drives_speeds_and_is_conserved_across_minutes() {
    let mut db = database();
    let mut b = batch(59_000, 61_000, 3000, 1500);
    b.sample_duration_ms = Some(1500);
    write_summary(&db, 61_000, &b).unwrap();
    let stats = read_stats(&db, &query(0, 120_000, "", false)).unwrap();
    assert_eq!(stats.covered_ms, 1500);
    assert_eq!(stats.average(), Some((2000., 1000.)));
    assert_eq!(stats.down_peak.unwrap().rate, 2000.);
    write_seconds(&mut db, &Sample::from_batch(0, 1, 61_000, &b).unwrap()).unwrap();
    let point = read_seconds(&db, &request(Some(0), 0, 120_000, "", false))
        .unwrap()
        .samples
        .remove(0);
    assert_eq!(point.duration_ms, 1500);
    assert_eq!(point.down, 2000.);
    b.sample_duration_ms = Some(4000);
    assert!(Sample::from_batch(0, 2, 61_000, &b).is_none());
}

#[test]
fn long_term_keeps_old_seconds_but_capacity_still_deletes_complete_batches() {
    let mut db = database();
    let mut b = batch(1000, 2000, 100, 50);
    b.app_rates.insert("child".into(), b.rate);
    write_summary(&db, 2000, &b).unwrap();
    write_seconds(&mut db, &Sample::from_batch(0, 1, 2000, &b).unwrap()).unwrap();
    let long = SecondsPolicy {
        enabled: false,
        days: None,
        max_bytes: None,
    };
    assert!(!prune_chunk(&mut db, long, 365 * 86_400_000).unwrap());
    assert_eq!(status(&db).unwrap().first, Some(1000));
    let capped = SecondsPolicy {
        max_bytes: Some(0),
        ..long
    };
    assert!(prune_chunk(&mut db, capped, 365 * 86_400_000).unwrap());
    assert!(!prune_chunk(&mut db, capped, 365 * 86_400_000).unwrap());
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM seconds_apps", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        read_stats(&db, &query(0, 60_000, "", false)).unwrap().bytes,
        b.rate
    );
}
