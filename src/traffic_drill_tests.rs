use super::*;
use crate::{
    accounting::Batch,
    speed_stats::{self, Sample, SecondsRequest},
    storage::Storage,
};

fn request(from: i64, to: i64, app: &str, grouped: bool) -> SecondsRequest {
    SecondsRequest {
        id: 1,
        generation: 0,
        revision: 0,
        scope: Scope {
            from: 0,
            to: 3_600_000,
            app: app.into(),
            grouped,
        },
        from: Some(from),
        view: Some(View {
            from,
            to,
            seconds: true,
        }),
    }
}
#[test]
fn ten_minute_queries_keep_all_samples_and_minute_ledger_without_raw() {
    let dir = tempfile::tempdir().unwrap();
    let mut storage = Storage::open(&dir.path().join("traffic.sqlite3")).unwrap();
    for minute in 0..10 {
        storage
            .db
            .execute("INSERT INTO totals VALUES(?1,2,10)", [minute * 60_000])
            .unwrap();
    }
    let req = request(0, 600_000, "", false);
    let empty = speed_stats::read_seconds(&storage.db, &req).unwrap();
    assert!(empty.samples.is_empty());
    assert_eq!(empty.trend.unwrap().total(), Bytes { down: 100, up: 20 });
    let tx = storage.db.transaction().unwrap();
    for seq in 0..1200 {
        tx.execute(
            "INSERT INTO seconds_samples VALUES(?1,?2,?3,500,10000,2000)",
            params![seq + 1, seq * 500, (seq + 1) * 500],
        )
        .unwrap();
    }
    tx.commit().unwrap();
    let data = speed_stats::read_seconds(&storage.db, &req).unwrap();
    assert_eq!(data.samples.len(), 1200);
    assert_eq!(data.samples[1199].to, 600_000);
    assert_eq!(data.trend.unwrap().total(), Bytes { down: 100, up: 20 });
    let stats = window_stats(&data.samples, 0, 600_000);
    assert_eq!(stats.covered_ms, 600_000.);
    assert_eq!(stats.down_average, 20_000.);
    let scope = Scope {
        from: 120_000,
        to: 900_000,
        app: String::new(),
        grouped: false,
    };
    assert_eq!(
        View::centered(130_999, &scope),
        View {
            from: 120_000,
            to: 420_000,
            seconds: true
        }
    );
    assert_eq!(
        View::centered(899_999, &scope),
        View {
            from: 540_000,
            to: 900_000,
            seconds: true
        }
    );
}
#[test]
fn window_stats_weight_actual_intervals_clip_edges_and_exclude_gaps() {
    let samples = vec![
        SecondSample {
            seq: 1,
            from: 0,
            to: 2000,
            duration_ms: 1500,
            down: 4000.,
            up: 0.,
            valid: true,
        },
        SecondSample {
            seq: 2,
            from: 2000,
            to: 2500,
            duration_ms: 500,
            down: 0.,
            up: 1000.,
            valid: true,
        },
        SecondSample {
            seq: 3,
            from: 4000,
            to: 5000,
            duration_ms: 1000,
            down: 500.,
            up: 500.,
            valid: true,
        },
        SecondSample {
            seq: 4,
            from: 3000,
            to: 4000,
            duration_ms: 1000,
            down: 1e9,
            up: 1e9,
            valid: false,
        },
    ];
    let stats = window_stats(&samples, 1000, 4500);
    assert_eq!(stats.covered_ms, 1750.);
    assert_eq!(stats.down_peak, 4000.);
    assert_eq!(stats.up_peak, 1000.);
    assert!((stats.down_average - 3250. * 1000. / 1750.).abs() < 0.0001);
    assert!((stats.up_average - 750. * 1000. / 1750.).abs() < 0.0001);
    assert_eq!(window_stats(&[], 0, 600_000).covered_ms, 0.);
}
#[test]
fn hover_uses_exact_batch_ranks_all_apps_and_keeps_grouping_and_unassigned_distinct() {
    let dir = tempfile::tempdir().unwrap();
    let mut storage = Storage::open(&dir.path().join("traffic.sqlite3")).unwrap();
    let mut data = Sample {
        generation: 0,
        seq: 1,
        from: 1000,
        to: 2000,
        duration_ms: 1000,
        bytes: Bytes {
            down: 10_000,
            up: 2000,
        },
        apps: vec![],
    };
    for index in 1..=7 {
        data.apps.push((
            false,
            format!("app-{index}.exe"),
            Bytes {
                down: index * 100,
                up: index * 10,
            },
        ));
    }
    data.apps
        .push((false, "未知进程".into(), Bytes { down: 50, up: 0 }));
    data.apps.push((
        true,
        "host.exe".into(),
        Bytes {
            down: 8000,
            up: 1000,
        },
    ));
    speed_stats::write_seconds(&mut storage.db, &data).unwrap();
    let mut hover = HoverRequest {
        id: 2,
        window: request(0, 600_000, "", false),
        target: Target::Second(1),
    };
    let raw = read_hover(&storage.db, &hover).unwrap();
    assert_eq!(raw.applications.len(), 8);
    assert_eq!(raw.applications[0].key, "app-7.exe");
    assert_eq!(raw.unassigned.down, 7150);
    assert_eq!(raw.down, 10_000.);
    storage
        .db
        .execute("INSERT INTO totals VALUES(0,2000,10000)", [])
        .unwrap();
    storage.db.execute("INSERT INTO app_minutes SELECT 0,id,10,100 FROM dimensions WHERE kind='app' AND name='app-7.exe'",[]).unwrap();
    let minute = read_hover(
        &storage.db,
        &HoverRequest {
            target: Target::Minute(0, 60_000),
            ..hover.clone()
        },
    )
    .unwrap();
    assert!(minute.observed);
    assert_eq!(minute.applications.len(), 1);
    assert_eq!(minute.down, 10_000.);
    assert_eq!(minute.unassigned.down, 9900);
    assert!(
        read_hover(
            &storage.db,
            &HoverRequest {
                target: Target::Minute(0, 120_000),
                ..hover.clone()
            }
        )
        .is_err()
    );
    let missing = read_hover(
        &storage.db,
        &HoverRequest {
            target: Target::Minute(60_000, 120_000),
            ..hover.clone()
        },
    )
    .unwrap();
    assert!(!missing.observed && missing.applications.is_empty());
    hover.window.scope.grouped = true;
    let host = read_hover(&storage.db, &hover).unwrap();
    assert_eq!(host.applications.len(), 1);
    assert_eq!(host.unassigned.down, 2000);
    hover.window.scope.app = "host.exe".into();
    let own = read_hover(&storage.db, &hover).unwrap();
    assert_eq!(own.down, 8000.);
    assert_eq!(own.unassigned.total(), 0);
    hover.target = Target::Second(999);
    assert!(read_hover(&storage.db, &hover).is_err());
}
#[test]
fn visible_row_batch_matches_individual_stats_including_idle_and_legacy() {
    let dir = tempfile::tempdir().unwrap();
    let storage = Storage::open(&dir.path().join("traffic.sqlite3")).unwrap();
    for minute in 0..3 {
        let mut batch = Batch {
            sample_from: Some(minute * 60_000),
            sample_duration_ms: Some(1000),
            rate: Bytes { down: 100, up: 200 },
            ..Default::default()
        };
        if minute != 1 {
            batch
                .app_rates
                .insert("a.exe".into(), Bytes { down: 100, up: 50 });
            batch
                .application_rates
                .insert("host.exe".into(), Bytes { down: 200, up: 100 });
        }
        speed_stats::write_summary(&storage.db, minute * 60_000 + 1000, &batch).unwrap();
    }
    for grouped in [false, true] {
        let query = crate::model::Query {
            from: 0,
            to: 180_000,
            group_apps: grouped,
            ..Default::default()
        };
        let global = speed_stats::read_stats(&storage.db, &query).unwrap();
        let rows = vec![
            Row {
                key: "a.exe".into(),
                ..Default::default()
            },
            Row {
                key: "host.exe".into(),
                ..Default::default()
            },
            Row {
                key: "old.exe".into(),
                ..Default::default()
            },
        ];
        let batch = speed_stats::read_row_stats(&storage.db, &query, &global, &rows).unwrap();
        for row in rows {
            let single = speed_stats::read_stats(
                &storage.db,
                &crate::model::Query {
                    app: row.key.clone(),
                    ..query.clone()
                },
            )
            .unwrap();
            let actual = &batch[&row.key];
            assert_eq!(actual.bytes, single.bytes);
            assert_eq!(actual.average(), single.average());
            assert_eq!(actual.covered_ms, 3000);
            assert_eq!(
                actual.down_peak.unwrap().rate,
                single.down_peak.unwrap().rate
            );
        }
    }
    let query = crate::model::Query {
        from: 1_000_000,
        to: 2_000_000,
        ..Default::default()
    };
    let global = speed_stats::read_stats(&storage.db, &query).unwrap();
    let batch = speed_stats::read_row_stats(
        &storage.db,
        &query,
        &global,
        &[Row {
            key: "a.exe".into(),
            ..Default::default()
        }],
    )
    .unwrap();
    assert!(batch["a.exe"].average().is_none());
}
