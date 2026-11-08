use super::*;
use crate::accounting::Fact;
use chrono::Datelike;
#[test]
fn history_chart_focus_rankings_detail_and_csv_share_the_actual_range() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ledger.sqlite");
    let mut writer = Storage::open(&path).unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    let from = now.div_euclid(3_600_000) * 3_600_000 - 4 * 3_600_000;
    let end = from + 2 * 3_600_000;
    let minute = now.div_euclid(60_000) * 60_000;
    let samples = [
        (from, "process.exe", Bytes { up: 1, down: 4 }),
        (from + 60_000, "process.exe", Bytes { up: 2, down: 7 }),
        (from + 3_600_000, "other.exe", Bytes { up: 3, down: 12 }),
        (minute, "process.exe", Bytes { up: 4, down: 16 }),
        (minute + 3_600_000, "process.exe", Bytes { up: 9, down: 90 }),
    ];
    let mut totals = vec![(from - 60_000, Bytes::default())];
    totals.extend(samples.iter().map(|(t, _, b)| (*t, *b)));
    let facts = samples
        .iter()
        .map(|(time, app, bytes)| Fact {
            time: *time,
            app: (*app).into(),
            domain: "example.org".into(),
            route: "DIRECT".into(),
            source: "local".into(),
            bytes: *bytes,
        })
        .collect();
    writer
        .commit(
            &Ledger {
                sequence: 1,
                time: now,
                ..Default::default()
            },
            &Batch {
                totals,
                facts,
                ..Default::default()
            },
        )
        .unwrap();
    let reader = Storage::read_only(&path).unwrap();
    let query = Query {
        from,
        to: end,
        history: true,
        page_size: 10,
        ..Default::default()
    };
    let full = reader.query_snapshot(&query).unwrap();
    let chart = full.history.unwrap();
    assert_eq!(full.total, chart.total());
    assert_eq!(full.total.total(), 29);
    let focus = Query {
        to: from + 3_600_000,
        history_context: Some((from, end)),
        ..query.clone()
    };
    for dimension in ["app", "domain", "route", "source"] {
        let q = Query {
            dimension: dimension.into(),
            ..focus.clone()
        };
        let report = reader
            .query_snapshot_cached(&q, Some(chart.clone()))
            .unwrap();
        assert_eq!(report.total, Bytes { up: 3, down: 11 });
        assert_eq!(report.rows.iter().map(|r| r.bytes.total()).sum::<u64>(), 14);
        assert!(std::sync::Arc::ptr_eq(
            report.history.as_ref().unwrap(),
            &chart
        ));
        assert_eq!(report.query, Some(q));
    }
    let detail = reader
        .query_snapshot(&Query {
            app: "process.exe".into(),
            dimension: "domain".into(),
            history_context: None,
            ..focus.clone()
        })
        .unwrap();
    assert_eq!(detail.total, Bytes { up: 3, down: 11 });
    assert!(detail.history.is_none());
    let filtered = Query {
        search: "process".into(),
        ..focus
    };
    let csv = dir.path().join("focus.csv");
    reader.export(&filtered, &csv).unwrap();
    let text = std::fs::read_to_string(csv).unwrap();
    let records: Vec<_> = csv::Reader::from_reader(text.trim_start_matches('\u{feff}').as_bytes())
        .records()
        .map(Result::unwrap)
        .collect();
    assert_eq!(records.len(), 1);
    assert_eq!(&records[0][4], "14");
    assert_eq!(records[0][5], from.to_string());
    assert_eq!(records[0][6], (from + 3_600_000).to_string());
    let current = Query {
        from: minute,
        to: minute + 2 * 3_600_000,
        ..query
    };
    let old = std::sync::Arc::new(crate::history::fixture(current.from, current.to, now));
    let fresh = reader
        .query_snapshot_cached(&current, Some(old.clone()))
        .unwrap();
    assert_eq!(fresh.total.total(), 20);
    assert_eq!(fresh.history.as_ref().unwrap().total(), fresh.total);
    assert!(!std::sync::Arc::ptr_eq(
        fresh.history.as_ref().unwrap(),
        &old
    ));
    let date = crate::history::local_date(now);
    let month = reader
        .calendar_snapshot(crate::history::CalendarRequest {
            id: 77,
            generation: 3,
            year: date.year(),
            month: date.month(),
            as_of: now,
        })
        .unwrap();
    let (a, b) = crate::history::month_bounds(date.year(), date.month()).unwrap();
    assert_eq!(
        month.days.iter().map(|d| d.bytes.total()).sum::<u64>(),
        reader.total(a, b.min(now + 1)).unwrap().total()
    );
    assert_eq!(month.request.id, 77);
}
