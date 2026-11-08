use super::*;
#[test]
#[ignore = "measures read-only queries against this computer's existing traffic database"]
fn inspect_local_detail_query_latency() {
    let default =
        std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap()).join("MihomoTraffic");
    let root = crate::data_location::resolve(&default).unwrap();
    let reader = Storage::read_only(&root.join("traffic.sqlite3")).unwrap();
    let owner = reader
        .identities()
        .unwrap()
        .into_values()
        .find(|owner| owner.label == "Codex")
        .unwrap();
    let query = Query {
        app: owner.key,
        group_apps: true,
        dimension: "domain".into(),
        ..Default::default()
    };
    let snapshot = reader.db.unchecked_transaction().unwrap();
    let start = std::time::Instant::now();
    let full = reader.query(&query).unwrap();
    let full_ms = start.elapsed().as_millis();
    let start = std::time::Instant::now();
    let detail = reader.application_detail(&query).unwrap();
    let detail_ms = start.elapsed().as_millis();
    assert_eq!(detail.total, full.total);
    snapshot.commit().unwrap();
    println!(
        "detail_read_ms={detail_ms} full_report_ms={full_ms} domains={} routes={} sources={}",
        detail.domains.len(),
        detail.routes.len(),
        detail.sources.len()
    );
}
use crate::{
    attribution::{Attribution, Owner},
    model::{Connection, Metadata, Snapshot},
};
#[test]
fn ownership_conserves_process_facts_and_separates_shared_runtime_in_all_views() {
    let dir = tempfile::tempdir().unwrap();
    let mut storage = Storage::open(&dir.path().join("traffic.sqlite3")).unwrap();
    let runtime = r"C:\Runtime\msedgewebview2.exe";
    let host_a = r"C:\Apps\a.exe";
    let host_b = r"C:\Apps\b.exe";
    let mut connections: Vec<_> = [
        ("a", runtime, Some(host_a)),
        ("b", runtime, Some(host_b)),
        ("unknown", runtime, None),
        ("root", host_a, None),
    ]
    .into_iter()
    .map(|(id, path, owner)| Connection {
        id: id.into(),
        start: chrono::DateTime::from_timestamp_millis(0)
            .unwrap()
            .to_rfc3339(),
        metadata: Metadata {
            process_path: path.into(),
            host: "example.org".into(),
            ..Default::default()
        },
        attribution: owner.map(|owner| Attribution {
            owner: Owner {
                key: owner.into(),
                label: format!("Host {id}"),
                icon: owner.into(),
            },
            pid: 10,
        }),
        ..Default::default()
    })
    .collect();
    let mut ledger = Ledger::default();
    let baseline = Snapshot {
        connections: Some(connections.clone()),
        ..Default::default()
    };
    let first = ledger.ingest(&baseline, "core", 119_000, false);
    storage.commit(&ledger, &first).unwrap();
    for (connection, (up, down)) in
        connections
            .iter_mut()
            .zip([(20, 100), (30, 200), (5, 40), (1, 5)])
    {
        connection.upload = up;
        connection.download = down;
    }
    let snapshot = Snapshot {
        upload_total: 56,
        download_total: 345,
        connections: Some(connections),
    };
    let batch = ledger.ingest(&snapshot, "core", 120_000, false);
    assert_eq!(batch.owners.len(), 2);
    assert_eq!(batch.application_rates[host_a], Bytes { up: 21, down: 105 });
    assert_eq!(batch.application_rates[runtime], Bytes { up: 5, down: 40 });
    assert!(storage.commit(&ledger, &batch).unwrap());
    assert!(!storage.commit(&ledger, &batch).unwrap());
    let mut query = Query {
        from: 0,
        to: 180_000,
        ..Default::default()
    };
    let raw = storage.query(&query).unwrap();
    assert_eq!(raw.rows.len(), 2);
    assert_eq!(
        raw.rows
            .iter()
            .find(|row| row.key == runtime)
            .unwrap()
            .bytes,
        Bytes { up: 55, down: 340 }
    );
    query.group_apps = true;
    let grouped = storage.query(&query).unwrap();
    assert_eq!(grouped.observed, raw.observed);
    assert_eq!(grouped.app_count, 3);
    assert_eq!(grouped.rows.len(), 3);
    assert_eq!(
        grouped
            .rows
            .iter()
            .find(|row| row.key == host_b)
            .unwrap()
            .bytes,
        Bytes { up: 30, down: 200 }
    );
    query.search = "Host b".into();
    assert_eq!(storage.query(&query).unwrap().rows[0].key, host_b);
    query.search.clear();
    query.app = host_a.into();
    let detail = storage.query(&query).unwrap();
    assert_eq!(detail.members.len(), 2);
    assert_eq!(detail.total, Bytes { up: 21, down: 105 });
    assert_eq!(detail.domains[0].bytes, detail.total);
    let mut histogram_total = Bytes::default();
    for bucket in &detail.histogram.buckets {
        histogram_total.add(bucket.bytes);
    }
    assert_eq!(histogram_total, detail.total);
    let reader = Storage::read_only(&storage.path).unwrap();
    let fast = reader.query_snapshot(&query).unwrap();
    assert_eq!(fast.total, detail.total);
    assert_eq!(fast.observed, detail.observed);
    assert_eq!(
        fast.domains
            .iter()
            .map(|row| (&row.key, row.bytes))
            .collect::<Vec<_>>(),
        detail
            .domains
            .iter()
            .map(|row| (&row.key, row.bytes))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        fast.routes
            .iter()
            .map(|row| (&row.key, row.bytes))
            .collect::<Vec<_>>(),
        detail
            .routes
            .iter()
            .map(|row| (&row.key, row.bytes))
            .collect::<Vec<_>>()
    );
    assert!(!fast.sources.is_empty());
    assert_eq!(fast.members.len(), detail.members.len());
    assert_eq!(
        fast.histogram
            .buckets
            .iter()
            .map(|bucket| bucket.bytes)
            .collect::<Vec<_>>(),
        detail
            .histogram
            .buckets
            .iter()
            .map(|bucket| bucket.bytes)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        fast.series
            .iter()
            .map(|point| (point.time, point.up, point.down))
            .collect::<Vec<_>>(),
        detail
            .series
            .iter()
            .map(|point| (point.time, point.up, point.down))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        detail
            .series
            .iter()
            .map(|point| point.up as u64)
            .sum::<u64>(),
        21
    );
    let backup = dir.path().join("backup.sqlite3");
    storage.backup(&backup).unwrap();
    let mut restored = Storage::open(&dir.path().join("restored.sqlite3")).unwrap();
    restored
        .restore(&backup, &dir.path().join("rollback.sqlite3"))
        .unwrap();
    assert_eq!(restored.query(&query).unwrap().total, detail.total);
    assert_eq!(restored.identities().unwrap()[host_a].label, "Host a");
    query.dimension = "domain".into();
    storage.maintain(1).unwrap();
    let archived = storage.query(&query).unwrap();
    assert!(archived.archive_daily);
    assert_eq!(archived.domains[0].bytes, detail.total);
    assert_eq!(archived.members.len(), 2);
    let fast_archive = reader.query_snapshot(&query).unwrap();
    assert!(fast_archive.archive_daily);
    assert_eq!(
        fast_archive
            .domains
            .iter()
            .map(|row| (&row.key, row.bytes))
            .collect::<Vec<_>>(),
        archived
            .domains
            .iter()
            .map(|row| (&row.key, row.bytes))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        storage
            .db
            .query_row("SELECT COUNT(*) FROM attribution_details", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    query.group_apps = false;
    query.app = runtime.into();
    assert_eq!(
        storage.query(&query).unwrap().total,
        Bytes { up: 55, down: 340 }
    );
}
#[test]
fn legacy_restore_adds_attribution_schema_without_reassigning_old_history() {
    let dir = tempfile::tempdir().unwrap();
    let legacy = dir.path().join("legacy.sqlite3");
    let db = Storage::open(&legacy).unwrap();
    db.db.execute_batch("DROP VIEW application_minutes; DROP VIEW application_process_minutes; DROP VIEW application_details; DROP VIEW application_daily; DROP TABLE owner_metadata; DROP TABLE app_attribution_minutes; DROP TABLE attribution_details; DROP TABLE attribution_daily; INSERT INTO dimensions VALUES(1,'app','msedgewebview2.exe'); INSERT INTO app_minutes VALUES(60000,1,3,7);").unwrap();
    drop(db);
    let mut storage = Storage::open(&dir.path().join("current.sqlite3")).unwrap();
    storage
        .restore(&legacy, &dir.path().join("before.sqlite3"))
        .unwrap();
    drop(storage);
    let storage = Storage::open(&dir.path().join("current.sqlite3")).unwrap();
    let query = Query {
        from: 0,
        to: 180_000,
        group_apps: true,
        ..Default::default()
    };
    let report = storage.query(&query).unwrap();
    assert_eq!(report.rows[0].key, "msedgewebview2.exe");
    assert_eq!(report.observed, Bytes { up: 3, down: 7 });
}
#[test]
fn read_only_queries_use_committed_wal_data_without_waiting_for_the_writer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("traffic.sqlite3");
    let mut writer = Storage::open(&path).unwrap();
    writer.db.execute_batch("INSERT INTO dimensions VALUES(1,'app','app.exe');INSERT INTO app_minutes VALUES(60000,1,7,9);").unwrap();
    let reader = Storage::read_only(&path).unwrap();
    let transaction = writer
        .db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    transaction
        .execute("UPDATE app_minutes SET up=999", [])
        .unwrap();
    let query = Query {
        from: 0,
        to: 180_000,
        app: "app.exe".into(),
        ..Default::default()
    };
    let report = reader.query_snapshot(&query).unwrap();
    assert_eq!(report.total, Bytes { up: 7, down: 9 });
    assert!(
        reader
            .db
            .execute("UPDATE app_minutes SET up=123", [])
            .is_err()
    );
    transaction.commit().unwrap();
    assert_eq!(
        reader.query_snapshot(&query).unwrap().total,
        Bytes { up: 999, down: 9 }
    );
}
#[test]
fn packaged_apps_group_existing_versions_and_redirects_without_changing_raw_rows() {
    use crate::accounting::{Fact, OwnerFact};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("traffic.sqlite3");
    let mut storage = Storage::open(&path).unwrap();
    let old = r"C:\Program Files\WindowsApps\Example.Writer_1.2.3.4_x64__8wekyb3d8bbwe\Writer.exe";
    let helper = r"D:\WindowsApps\Example.Writer_2.0.0.0_x64__8wekyb3d8bbwe\tools\helper.exe";
    let standalone = r"C:\Tools\helper.exe";
    let from = local_day(chrono::Utc::now().timestamp_millis() - 3 * 86_400_000);
    let time = from + 60_000;
    let fact = |app: &str, down| Fact {
        time,
        app: app.into(),
        domain: "example.com".into(),
        route: "DIRECT".into(),
        source: "local".into(),
        bytes: Bytes { up: 0, down },
    };
    let legacy_owner = crate::attribution::Owner {
        key: old.into(),
        label: "Writer".into(),
        icon: old.into(),
    };
    let mut batch = Batch {
        facts: vec![fact(old, 10), fact(helper, 20), fact(standalone, 5)],
        owners: vec![OwnerFact {
            fact: fact(helper, 8),
            owner: legacy_owner,
        }],
        sample_from: Some(time),
        ..Default::default()
    };
    batch
        .application_rates
        .insert(old.into(), Bytes { up: 0, down: 18 });
    batch
        .application_rates
        .insert(helper.into(), Bytes { up: 0, down: 12 });
    storage
        .commit(
            &Ledger {
                sequence: 1,
                time: time + 1000,
                ..Default::default()
            },
            &batch,
        )
        .unwrap();
    storage
        .db
        .execute_batch(
            "DELETE FROM package_app_ids; DELETE FROM meta WHERE key='package_application_groups';",
        )
        .unwrap();
    drop(storage);

    let storage = Storage::open(&path).unwrap();
    let reader = Storage::read_only(&path).unwrap();
    let key = crate::attribution::package_owner(old).unwrap().key;
    let mut query = Query {
        from,
        to: from + 86_400_000,
        group_apps: true,
        ..Default::default()
    };
    let grouped = reader.query_snapshot(&query).unwrap();
    assert_eq!(grouped.rows.len(), 2);
    assert_eq!(grouped.rows[0].key, key);
    assert_eq!(grouped.rows[0].bytes.down, 30);
    assert_eq!(grouped.rows[1].key, standalone);
    query.app = key;
    let detail = reader.query_snapshot(&query).unwrap();
    assert_eq!(detail.total.down, 30);
    assert_eq!(detail.speeds.bytes.down, 30);
    assert_eq!(detail.domains[0].bytes.down, 30);
    assert_eq!(detail.members[0].key, helper);
    assert_eq!(detail.members[0].bytes.down, 20);
    storage.maintain(1).unwrap();
    assert_eq!(
        reader.query_snapshot(&query).unwrap().domains[0].bytes.down,
        30
    );
    query.app.clear();
    query.group_apps = false;
    let raw = reader.query_snapshot(&query).unwrap();
    assert_eq!(raw.rows.len(), 3);
    assert_eq!(raw.rows[0].key, helper);
    assert_eq!(raw.rows[0].bytes.down, 20);
}

#[test]
fn history_domain_grouping_applies_before_pagination_and_matches_export() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("traffic.sqlite3");
    let mut storage = Storage::open(&path).unwrap();
    let from = local_day(chrono::Utc::now().timestamp_millis() - 3 * 86_400_000);
    let facts = [
        ("api.example.co.uk", 4),
        ("IMG.Example.CO.UK.", 7),
        ("192.0.2.1", 8),
        ("a.github.io", 2),
        ("b.github.io", 1),
    ]
    .into_iter()
    .map(|(domain, down)| crate::accounting::Fact {
        time: from + 60_000,
        app: "demo.exe".into(),
        domain: domain.into(),
        route: "DIRECT".into(),
        source: "local".into(),
        bytes: Bytes { up: 1, down },
    })
    .collect();
    storage
        .commit(
            &Ledger {
                sequence: 1,
                time: from + 60_000,
                ..Default::default()
            },
            &Batch {
                facts,
                ..Default::default()
            },
        )
        .unwrap();
    let reader = Storage::read_only(&path).unwrap();
    let mut query = Query {
        history: true,
        from,
        to: from + 86_400_000,
        dimension: "domain".into(),
        page_size: 1,
        ..Default::default()
    };
    for archived in [false, true] {
        if archived {
            storage.maintain(1).unwrap();
        }
        query.group_domains = false;
        let raw = reader.query_snapshot(&query).unwrap();
        assert_eq!(raw.archive_daily, archived);
        assert_eq!(raw.row_count, 5);
        assert_eq!(raw.rows[0].key, "192.0.2.1");

        query.group_domains = true;
        let grouped = reader.query_snapshot(&query).unwrap();
        assert_eq!(grouped.row_count, 4);
        assert_eq!(grouped.rows[0].key, "example.co.uk");
        assert_eq!(grouped.rows[0].bytes, Bytes { up: 2, down: 11 });
        assert_eq!(grouped.domains[0].key, "192.0.2.1");

        query.page = 1;
        let paged = reader.query_page(&query, grouped).unwrap();
        assert_eq!(paged.query.as_ref(), Some(&query));
        assert_eq!(paged.row_count, 4);
        assert_eq!(paged.rows[0].key, "192.0.2.1");
        assert_eq!(paged.archive_daily, archived);
        assert_eq!(
            reader.query_snapshot(&query).unwrap().rows[0].key,
            "192.0.2.1"
        );
        let csv_path = dir.path().join("export.csv");
        storage.export(&query, &csv_path).unwrap();
        let names: Vec<_> = csv::Reader::from_path(&csv_path)
            .unwrap()
            .records()
            .map(|row| row.unwrap()[1].to_string())
            .collect();
        assert_eq!(
            names,
            ["example.co.uk", "192.0.2.1", "a.github.io", "b.github.io"]
        );
        query.page = 0;
    }
}

#[test]
fn combined_destination_rankings_match_existing_queries_before_and_after_archive() {
    use crate::{
        accounting::{Fact, OwnerFact},
        attribution::Owner,
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("traffic.sqlite3");
    let mut storage = Storage::open(&path).unwrap();
    let runtime = r"C:\runtime.exe";
    let owner = Owner {
        key: r"C:\host.exe".into(),
        label: "host".into(),
        icon: r"C:\host.exe".into(),
    };
    let mut batch = Batch::default();
    for index in 0..20 {
        let fact = Fact {
            time: 60_000 + (index % 3) * 60_000,
            app: runtime.into(),
            domain: if index == 0 {
                "A.example".into()
            } else if index == 3 {
                "a.example".into()
            } else {
                format!("domain-{index}.example")
            },
            route: format!("route-{}", index % 3),
            source: format!("192.0.2.{}", index % 2),
            bytes: Bytes {
                up: (index % 3) as u64,
                down: 100,
            },
        };
        if index % 2 == 0 {
            batch.owners.push(OwnerFact {
                fact: fact.clone(),
                owner: owner.clone(),
            });
        }
        batch.facts.push(fact);
    }
    storage
        .commit(
            &Ledger {
                sequence: 1,
                time: 240_000,
                ..Default::default()
            },
            &batch,
        )
        .unwrap();
    let reader = Storage::read_only(&path).unwrap();
    for archived in [false, true] {
        if archived {
            storage.maintain(1).unwrap();
        }
        for (app, group_apps) in [
            (runtime, false),
            (runtime, true),
            (owner.key.as_str(), true),
        ] {
            let query = Query {
                from: 0,
                to: 300_000,
                app: app.into(),
                group_apps,
                dimension: "domain".into(),
                ..Default::default()
            };
            let report = reader.query_snapshot(&query).unwrap();
            assert_eq!(report.archive_daily, archived);
            for (dimension, actual) in [
                ("domain", &report.domains),
                ("route", &report.routes),
                ("source", &report.sources),
            ] {
                let expected = storage.rows(&query, dimension, 8, 0).unwrap().0;
                assert_eq!(
                    actual
                        .iter()
                        .map(|row| (&row.key, row.bytes))
                        .collect::<Vec<_>>(),
                    expected
                        .iter()
                        .map(|row| (&row.key, row.bytes))
                        .collect::<Vec<_>>(),
                    "{dimension} group={group_apps} archive={archived}"
                );
            }
        }
    }
}
