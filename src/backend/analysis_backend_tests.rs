use super::*;
#[test]
fn worker_freezes_raw_bytes_and_rejects_a_stale_source_generation() {
    let dir = tempfile::tempdir().unwrap();
    Config {
        endpoint: "http://127.0.0.1:9".into(),
        floating: false,
        taskbar: false,
        ..Default::default()
    }
    .save(dir.path())
    .unwrap();
    let backend = Backend::start(dir.path().into()).unwrap();
    let anchor = chrono::Utc::now().timestamp_millis() / 1000 * 1000;
    {
        let mut cache = backend.analysis_cache.lock().unwrap();
        for index in 0..900 {
            let from = anchor - 900_000 + index * 1000;
            cache.record(
                from,
                from + 1000,
                Bytes { down: 3, up: 2 },
                &std::collections::HashMap::from([("app".into(), Bytes { down: 3, up: 2 })]),
            );
        }
    }
    let wait = |id| {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(reply) = backend.analysis_result.lock().unwrap().clone()
                && reply.request.id == id
            {
                break reply;
            }
            assert!(Instant::now() < deadline, "analysis worker timed out");
            std::thread::sleep(Duration::from_millis(10));
        }
    };
    let request = AnalysisRequest::new(1, 0, crate::analysis::AnalysisRange::FiveMinutes, anchor);
    let notified = backend.query_ready.notified();
    assert!(backend.query_analysis(request.clone()));
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            tokio::time::timeout(Duration::from_secs(3), notified)
                .await
                .expect("snapshot completion did not wake the UI");
        });
    let reply = wait(1);
    assert!(reply.complete);
    let snapshot = reply.result.unwrap();
    assert_eq!(
        snapshot.metrics(Some("app")).current,
        Bytes { down: 900, up: 600 }
    );
    assert!(snapshot.comparable());
    backend.reset_analysis();
    assert!(backend.query_analysis(AnalysisRequest {
        id: 2,
        ..request.clone()
    }));
    assert!(wait(2).result.unwrap_err().contains("数据源"));
    assert!(backend.query_analysis(AnalysisRequest {
        id: 3,
        generation: 1,
        ..request
    }));
    assert!(!wait(3).result.unwrap().comparable());
    assert_eq!(snapshot.metrics(None).current.total(), 1500);
    backend.shutdown().unwrap();
}
