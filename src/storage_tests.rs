use super::*;
#[test]
fn application_page_sizes_cover_empty_partial_and_multiple_pages_without_changing_totals() {
    for count in [0usize, 1, 10, 11, 20, 21] {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(&root.path().join("traffic.sqlite3")).unwrap();
        let time = local_day(chrono::Utc::now().timestamp_millis());
        for index in 0..count {
            storage
                .db
                .execute(
                    "INSERT INTO dimensions(kind,name) VALUES('app',?1)",
                    [format!("app-{index:02}.exe")],
                )
                .unwrap();
            let id = storage.db.last_insert_rowid();
            storage
                .db
                .execute(
                    "INSERT INTO app_minutes VALUES(?1,?2,?3,?4)",
                    params![time, id, index as i64 + 1, (index as i64 + 1) * 3],
                )
                .unwrap();
        }
        storage
            .db
            .execute(
                "INSERT INTO totals VALUES(?1,?2,?3)",
                params![time, 10000, 30000],
            )
            .unwrap();
        let base = Query {
            from: time,
            to: time + 60_000,
            ..Default::default()
        };
        let all = storage.query(&base).unwrap();
        assert_eq!(all.rows.len(), count);
        assert_eq!(all.query.as_ref().unwrap().page_size, 50);
        let mut keys = Vec::new();
        for page in 0..=count / 10 {
            let query = Query {
                page,
                page_size: 10,
                ..base.clone()
            };
            let report = storage.query(&query).unwrap();
            assert_eq!(report.query, Some(query));
            assert_eq!(report.rows.len(), count.saturating_sub(page * 10).min(10));
            assert_eq!(report.total, all.total);
            assert_eq!(report.observed, all.observed);
            assert_eq!(report.app_count, count);
            assert_eq!(report.row_count, count);
            keys.extend(report.rows.into_iter().map(|row| row.key));
        }
        assert_eq!(
            keys,
            all.rows
                .iter()
                .map(|row| row.key.clone())
                .collect::<Vec<_>>()
        );
        let sorted = storage
            .query(&Query {
                page_size: 10,
                sort: TrafficSort::Upload,
                ascending: true,
                ..base.clone()
            })
            .unwrap();
        if count > 0 {
            assert_eq!(sorted.rows[0].key, "app-00.exe");
        }
        let filtered = storage
            .query(&Query {
                page_size: 10,
                search: "app-00".into(),
                ..base.clone()
            })
            .unwrap();
        assert_eq!(filtered.row_count, usize::from(count > 0));
        assert_eq!(filtered.observed, all.observed);
        assert_eq!(filtered.app_count, count);
        for page_size in [0, 51] {
            assert!(
                storage
                    .query(&Query {
                        page_size,
                        ..base.clone()
                    })
                    .is_err()
            );
        }
        assert!(
            storage
                .query(&Query {
                    page: usize::MAX,
                    page_size: 10,
                    ..base
                })
                .is_err()
        );
    }
}
#[test]
fn application_sorting_counts_and_search_use_the_whole_period() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(&root.path().join("traffic.sqlite3")).unwrap();
    let time = local_day(chrono::Utc::now().timestamp_millis());
    for (key, up, down) in [
        ("a.exe", 900, 10),
        ("b.exe", 1, 500),
        ("c.exe", 50, 50),
        ("d.exe", 50, 50),
        ("未知进程", 2, 20),
    ] {
        storage
            .db
            .execute("INSERT INTO dimensions(kind,name) VALUES('app',?1)", [key])
            .unwrap();
        let id = storage.db.last_insert_rowid();
        storage
            .db
            .execute(
                "INSERT INTO app_minutes VALUES(?1,?2,?3,?4)",
                params![time, id, up, down],
            )
            .unwrap();
    }
    storage
        .db
        .execute(
            "INSERT INTO totals VALUES(?1,?2,?3)",
            params![time, 1100, 700],
        )
        .unwrap();
    let base = Query {
        from: time,
        to: time + 60_000,
        ..Default::default()
    };
    let total = storage.query(&base).unwrap();
    assert_eq!(total.app_count, 4);
    assert_eq!(total.row_count, 5);
    assert_eq!(total.total.total(), 1800);
    assert_eq!(
        total
            .rows
            .iter()
            .map(|row| row.key.as_str())
            .collect::<Vec<_>>(),
        ["a.exe", "b.exe", "c.exe", "d.exe", "未知进程"]
    );
    let download = storage
        .query(&Query {
            sort: TrafficSort::Download,
            ..base.clone()
        })
        .unwrap();
    assert_eq!(download.rows[0].key, "b.exe");
    assert_eq!(
        download.apps[0].key, "a.exe",
        "rankings remain ordered by total traffic"
    );
    let upload = storage
        .query(&Query {
            sort: TrafficSort::Upload,
            ..base.clone()
        })
        .unwrap();
    assert_eq!(upload.rows[0].key, "a.exe");
    let ascending = storage
        .query(&Query {
            ascending: true,
            ..base.clone()
        })
        .unwrap();
    assert_eq!(ascending.rows[0].key, "未知进程");
    let filtered = storage
        .query(&Query {
            search: "B.EXE".into(),
            ..base.clone()
        })
        .unwrap();
    assert_eq!(filtered.row_count, 1);
    assert_eq!(filtered.rows[0].key, "b.exe");
    assert_eq!(filtered.app_count, 4);
    assert_eq!(filtered.observed, total.observed);
    let detail = storage
        .query(&Query {
            app: "a.exe".into(),
            ..base
        })
        .unwrap();
    assert_eq!(detail.app_count, 1);
    assert_eq!(detail.total.total(), 910);
}
use crate::accounting::*;
fn detail_fixture() -> (tempfile::TempDir, Storage, i64) {
    let dir = tempfile::tempdir().unwrap();
    let mut storage = Storage::open(&dir.path().join("traffic.sqlite3")).unwrap();
    let today = local_day(chrono::Utc::now().timestamp_millis());
    let facts = [today - 5 * 86_400_000, today - 3 * 86_400_000, today]
        .into_iter()
        .map(|time| Fact {
            time,
            app: "test.exe".into(),
            domain: "example.test".into(),
            route: "DIRECT".into(),
            source: "local".into(),
            bytes: Bytes { up: 10, down: 20 },
        })
        .collect();
    storage
        .commit(
            &Ledger {
                sequence: 1,
                ..Default::default()
            },
            &Batch {
                facts,
                totals: vec![(today, Bytes { up: 30, down: 60 })],
                ..Default::default()
            },
        )
        .unwrap();
    (dir, storage, today + 60_000)
}

#[test]
fn unlimited_keeps_details_and_events_and_never_rewinds_archive_boundary() {
    let (_dir, storage, now) = detail_fixture();
    storage
        .db
        .execute(
            "INSERT INTO events VALUES(?1,'old event')",
            [now - 10 * 86_400_000],
        )
        .unwrap();
    storage
        .maintain_policy(
            RetentionPolicy {
                days: None,
                max_bytes: None,
            },
            now,
        )
        .unwrap();
    assert_eq!(
        storage
            .db
            .query_row("SELECT COUNT(*) FROM details", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        3
    );
    assert_eq!(
        storage
            .db
            .query_row("SELECT COUNT(*) FROM events", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    storage
        .maintain_policy(
            RetentionPolicy {
                days: Some(2),
                max_bytes: None,
            },
            now,
        )
        .unwrap();
    let before: String = storage
        .db
        .query_row(
            "SELECT value FROM meta WHERE key='detail_cutoff'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    storage
        .maintain_policy(
            RetentionPolicy {
                days: None,
                max_bytes: None,
            },
            now,
        )
        .unwrap();
    let after: String = storage
        .db
        .query_row(
            "SELECT value FROM meta WHERE key='detail_cutoff'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(before, after);
}

#[test]
fn capacity_prunes_old_days_preserves_today_and_all_long_term_statistics() {
    let (_dir, storage, now) = detail_fixture();
    let before = storage.dashboard_totals(now).unwrap();
    let counts = |table: &str| {
        storage
            .db
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap()
    };
    let long_term = [counts("app_minutes"), counts("daily"), counts("gaps")];
    let result = storage
        .maintain_policy(
            RetentionPolicy {
                days: Some(90),
                max_bytes: Some(1),
            },
            now,
        )
        .unwrap();
    assert!(result.over_limit);
    assert_eq!(counts("details"), 1);
    assert_eq!(
        long_term,
        [counts("app_minutes"), counts("daily"), counts("gaps")]
    );
    let after = storage.dashboard_totals(now).unwrap();
    assert_eq!(
        (before.0.total(), before.1.total(), before.2.total()),
        (after.0.total(), after.1.total(), after.2.total())
    );
    let cutoff: String = storage
        .db
        .query_row(
            "SELECT value FROM meta WHERE key='detail_cutoff'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    storage
        .maintain_policy(
            RetentionPolicy {
                days: Some(3650),
                max_bytes: Some(u64::MAX),
            },
            now,
        )
        .unwrap();
    assert_eq!(
        cutoff,
        storage
            .db
            .query_row::<String, _, _>(
                "SELECT value FROM meta WHERE key='detail_cutoff'",
                [],
                |r| r.get(0)
            )
            .unwrap()
    );
    assert_eq!(result.database_bytes, storage.database_bytes());
}

#[test]
fn capacity_reclaims_unused_pages_before_deleting_detail() {
    let (_dir, storage, now) = detail_fixture();
    storage.db.execute_batch("CREATE TABLE scratch(value BLOB); INSERT INTO scratch VALUES(zeroblob(2097152)); DROP TABLE scratch; PRAGMA wal_checkpoint(TRUNCATE);").unwrap();
    assert!(storage.database_bytes() > 1024 * 1024);
    let result = storage
        .maintain_policy(
            RetentionPolicy {
                days: None,
                max_bytes: Some(1024 * 1024),
            },
            now,
        )
        .unwrap();
    assert!(!result.over_limit);
    assert!(result.database_bytes < 1024 * 1024);
    assert_eq!(
        storage
            .db
            .query_row("SELECT COUNT(*) FROM details", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        3
    );
}

#[test]
fn capacity_stops_after_oldest_day_once_below_limit() {
    let (_dir, storage, now) = detail_fixture();
    let today = local_day(now);
    for day in [today - 5 * 86_400_000, today - 3 * 86_400_000] {
        storage
            .db
            .execute("INSERT INTO events VALUES(?1,zeroblob(2097152))", [day])
            .unwrap();
    }
    let result = storage
        .maintain_policy(
            RetentionPolicy {
                days: None,
                max_bytes: Some(3 * 1024 * 1024),
            },
            now,
        )
        .unwrap();
    assert!(!result.over_limit);
    assert_eq!(
        storage
            .db
            .query_row("SELECT COUNT(*) FROM details", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        storage
            .db
            .query_row("SELECT MIN(time) FROM details", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        today - 3 * 86_400_000
    );
}

#[test]
fn failed_prune_rolls_back_deletions_and_cutoff_together() {
    let (_dir, storage, now) = detail_fixture();
    storage.db.execute_batch("CREATE TRIGGER fail_cutoff BEFORE INSERT ON meta WHEN NEW.key='detail_cutoff' BEGIN SELECT RAISE(ABORT,'test failure'); END;").unwrap();
    assert!(
        storage
            .maintain_policy(
                RetentionPolicy {
                    days: Some(1),
                    max_bytes: None
                },
                now
            )
            .is_err()
    );
    assert_eq!(
        storage
            .db
            .query_row("SELECT COUNT(*) FROM details", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        3
    );
}
#[test]
fn transaction_failure_does_not_advance_checkpoint() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Storage::open(&d.path().join("atomic.db")).unwrap();
    s.db.execute_batch("CREATE TRIGGER fail_write BEFORE INSERT ON app_minutes BEGIN SELECT RAISE(ABORT,'test failure'); END;").unwrap();
    let b = Batch {
        totals: vec![(60000, Bytes { up: 10, down: 20 })],
        facts: vec![Fact {
            time: 60000,
            app: "x".into(),
            domain: "x".into(),
            route: "DIRECT".into(),
            source: "local".into(),
            bytes: Bytes { up: 1, down: 2 },
        }],
        ..Default::default()
    };
    assert!(
        s.commit(
            &Ledger {
                sequence: 1,
                ..Default::default()
            },
            &b
        )
        .is_err()
    );
    assert_eq!(s.total(0, 120000).unwrap().total(), 0);
    assert_eq!(s.checkpoint().unwrap().sequence, 0);
}
#[test]
fn dashboard_totals_use_local_day_and_month_and_retain_lifetime() {
    use chrono::TimeZone;
    let dir = tempfile::tempdir().unwrap();
    let storage = Storage::open(&dir.path().join("summary.sqlite3")).unwrap();
    let now = chrono::Local
        .with_ymd_and_hms(2026, 10, 21, 12, 0, 0)
        .single()
        .unwrap()
        .timestamp_millis();
    let day = local_day(now);
    let month = local_month(now);
    for (time, up, down) in [(month - 60_000, 1, 2), (day - 60_000, 3, 4), (day, 5, 6)] {
        storage
            .db
            .execute(
                "INSERT INTO totals VALUES(?1,?2,?3)",
                params![time, up, down],
            )
            .unwrap();
    }
    let (today, this_month, lifetime) = storage.dashboard_totals(now).unwrap();
    assert_eq!(
        (today.total(), this_month.total(), lifetime.total()),
        (11, 18, 21)
    );
    storage.maintain(1).unwrap();
    let next_month = chrono::Local
        .with_ymd_and_hms(2026, 11, 1, 0, 0, 0)
        .single()
        .unwrap()
        .timestamp_millis();
    let (today, this_month, lifetime) = storage.dashboard_totals(next_month).unwrap();
    assert_eq!(
        (today.total(), this_month.total(), lifetime.total()),
        (0, 0, 21)
    );
}
#[test]
fn longer_retention_does_not_claim_deleted_minutes_exist() {
    let d = tempfile::tempdir().unwrap();
    let s = Storage::open(&d.path().join("retention.db")).unwrap();
    s.maintain(1).unwrap();
    let before: String =
        s.db.query_row(
            "SELECT value FROM meta WHERE key='detail_cutoff'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    s.maintain(100).unwrap();
    let after: String =
        s.db.query_row(
            "SELECT value FROM meta WHERE key='detail_cutoff'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(before, after);
}
#[test]
fn csv_protects_formulas_and_obeys_filter() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Storage::open(&d.path().join("csv.db")).unwrap();
    let t = local_day(chrono::Utc::now().timestamp_millis());
    let b = Batch {
        facts: vec![Fact {
            time: t,
            app: " \n＝untrusted.exe".into(),
            domain: "host".into(),
            route: "DIRECT".into(),
            source: "local".into(),
            bytes: Bytes { up: 1, down: 2 },
        }],
        ..Default::default()
    };
    s.commit(
        &Ledger {
            sequence: 1,
            ..Default::default()
        },
        &b,
    )
    .unwrap();
    let p = d.path().join("export.csv");
    let q = Query {
        from: t,
        to: t + 60000,
        ..Default::default()
    };
    s.export(&q, &p).unwrap();
    let text = std::fs::read_to_string(p).unwrap();
    assert!(text.contains("' \n＝untrusted.exe"));
    assert_eq!(
        s.query(&Query {
            search: "missing".into(),
            ..q
        })
        .unwrap()
        .row_count,
        0
    );
}
#[test]
fn crossing_gap_is_never_assigned_to_partial_range() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Storage::open(&d.path().join("gap.db")).unwrap();
    s.commit(
        &Ledger {
            sequence: 1,
            ..Default::default()
        },
        &Batch {
            gap: Some((30000, 180000, Bytes { up: 10, down: 90 }, "sleep".into())),
            ..Default::default()
        },
    )
    .unwrap();
    let r = s
        .query(&Query {
            from: 60000,
            to: 120000,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(r.crossing_gaps, 1);
    assert_eq!(r.recovered.total(), 0);
    assert_eq!(r.total.total(), 0);
}
#[test]
fn atomic_checkpoint_and_idempotence() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Storage::open(&dir.path().join("test.db")).unwrap();
    let l = Ledger {
        sequence: 1,
        time: 60_000,
        ..Default::default()
    };
    let b = Batch {
        totals: vec![(60_000, Bytes { up: 12, down: 24 })],
        ..Default::default()
    };
    assert!(s.commit(&l, &b).unwrap());
    assert!(!s.commit(&l, &b).unwrap());
    assert_eq!(s.total(0, 120_000).unwrap().total(), 36);
    assert_eq!(s.checkpoint().unwrap().sequence, 1);
}
#[test]
fn range_and_retention_and_backup() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Storage::open(&dir.path().join("test.db")).unwrap();
    let t = local_day(chrono::Utc::now().timestamp_millis());
    let b = Batch {
        totals: vec![(t, Bytes { up: 1, down: 4 })],
        facts: vec![Fact {
            time: t,
            app: "test.exe".into(),
            domain: "test.example".into(),
            route: "DIRECT".into(),
            source: "local".into(),
            bytes: Bytes { up: 1, down: 2 },
        }],
        gap: Some((t + 100, t + 1000, Bytes { up: 2, down: 3 }, "gap".into())),
        ..Default::default()
    };
    s.commit(
        &Ledger {
            sequence: 1,
            ..Default::default()
        },
        &b,
    )
    .unwrap();
    let q = Query {
        from: t,
        to: t + 60_000,
        ..Default::default()
    };
    let r = s.query(&q).unwrap();
    assert_eq!(r.total.total(), 5);
    assert_eq!(r.observed.total(), 3);
    assert_eq!(r.recovered.total(), 5);
    assert_eq!(r.rows.len(), 1);
    assert_eq!(
        s.query(&Query {
            from: t + 60_000,
            to: t + 120_000,
            ..q.clone()
        })
        .unwrap()
        .total
        .total(),
        0
    );
    s.maintain(30).unwrap();
    assert_eq!(s.query(&q).unwrap().rows.len(), 1);
    let p = dir.path().join("backup.db");
    s.backup(&p).unwrap();
    s.restore(&p, &dir.path().join("rollback.db")).unwrap();
    assert_eq!(s.total(t, t + 60_000).unwrap().total(), 5);
}

#[test]
fn restore_preserves_active_ledger_on_validation_failure_then_resumes_from_backup() {
    let dir = tempfile::tempdir().unwrap();
    let mut active = Storage::open(&dir.path().join("traffic.sqlite3")).unwrap();
    let record = |storage: &mut Storage, sequence, up| {
        storage
            .commit(
                &Ledger {
                    sequence,
                    time: 60_000,
                    ..Default::default()
                },
                &Batch {
                    totals: vec![(60_000, Bytes { up, down: 0 })],
                    ..Default::default()
                },
            )
            .unwrap()
    };
    assert!(record(&mut active, 8, 100));
    let backup_path = dir.path().join("backup.sqlite3");
    let mut backup = Storage::open(&backup_path).unwrap();
    assert!(record(&mut backup, 2, 20));
    backup.db.execute_batch("DROP TABLE totals;").unwrap();
    drop(backup);

    let rollback = dir.path().join("rollback.sqlite3");
    assert!(active.restore(&backup_path, &rollback).is_err());
    assert!(!rollback.exists());
    assert_eq!(active.checkpoint().unwrap().sequence, 8);
    assert_eq!(active.total(0, 120_000).unwrap().up, 100);

    let backup = Storage::open(&backup_path).unwrap();
    backup
        .db
        .execute_batch(
            "INSERT INTO totals VALUES(60000,20,0);
         DROP TABLE seconds_apps;
         DROP TABLE seconds_samples;
         DROP TABLE speed_app_minutes;
         DROP TABLE speed_minutes;",
        )
        .unwrap();
    drop(backup);
    let restored = active.restore(&backup_path, &rollback).unwrap();
    assert_eq!(restored.ledger.sequence, 2);
    assert_eq!(restored.totals.2.up, 20);
    assert!(record(&mut active, restored.ledger.sequence + 1, 5));
    assert_eq!(active.total(0, 120_000).unwrap().up, 25);
    assert_eq!(
        Storage::read_only(&rollback)
            .unwrap()
            .total(0, 120_000)
            .unwrap()
            .up,
        100
    );
    assert!(!std::fs::read_dir(dir.path()).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".restore-")
    }));
}
