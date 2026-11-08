use super::*;
use crate::backend::Command;
use crate::config::ProcessMode;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, AtomicUsize},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, windows::named_pipe::ServerOptions},
};

struct Request {
    method: String,
    path: String,
    headers: String,
    body: Vec<u8>,
}
async fn receive(stream: &mut (impl AsyncRead + Unpin)) -> Option<Request> {
    let mut bytes = Vec::new();
    let (headers, offset) = loop {
        let mut buffer = [0; 2048];
        let n = stream.read(&mut buffer).await.unwrap();
        if n == 0 && bytes.is_empty() {
            return None; // A paused or obsolete operation may cancel before writing headers.
        }
        assert_ne!(n, 0, "request ended before headers");
        bytes.extend_from_slice(&buffer[..n]);
        if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
            break (String::from_utf8(bytes[..end].to_vec()).unwrap(), end + 4);
        }
        assert!(bytes.len() < 8192);
    };
    let length: usize = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().unwrap())
        })
        .unwrap_or(0);
    while bytes.len() < offset + length {
        let mut buffer = [0; 2048];
        let n = stream.read(&mut buffer).await.unwrap();
        assert_ne!(n, 0);
        bytes.extend_from_slice(&buffer[..n]);
    }
    let mut first = headers.lines().next().unwrap().split_whitespace();
    Some(Request {
        method: first.next().unwrap().into(),
        path: first.next().unwrap().into(),
        headers: headers.clone(),
        body: bytes[offset..offset + length].to_vec(),
    })
}
async fn reply(stream: &mut (impl AsyncWrite + Unpin), code: u16, body: &str) {
    let response = format!(
        "HTTP/1.1 {code} Test\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    // Obsolete/cancelled requests intentionally close before receiving a response.
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}
struct Step {
    method: &'static str,
    target: Option<&'static str>,
    code: u16,
    body: String,
    delay: Duration,
}
fn get(mode: &str) -> Step {
    Step {
        method: "GET",
        target: None,
        code: 200,
        body: serde_json::json!({"find-process-mode":mode}).to_string(),
        delay: Duration::ZERO,
    }
}
fn patch(target: &'static str) -> Step {
    Step {
        method: "PATCH",
        target: Some(target),
        code: 204,
        body: String::new(),
        delay: Duration::ZERO,
    }
}

#[tokio::test]
async fn all_modes_apply_and_confirm_over_both_transports() {
    for pipe in [false, true] {
        for mode in ProcessMode::ALL {
            let initial = if mode == ProcessMode::Off {
                "always"
            } else {
                "off"
            };
            for policy in [
                ProcessControl::ApplyOnce(mode),
                ProcessControl::Maintain(mode),
            ] {
                let outcome = scripted(
                    pipe,
                    vec![get(initial), patch(mode.as_str()), get(mode.as_str())],
                    policy,
                    true,
                )
                .await;
                assert!(outcome.result.is_ok(), "{:?}", outcome.result);
                assert_eq!(outcome.mode, mode.as_str());
            }
        }
    }
}

#[tokio::test]
async fn stopping_maintenance_leaves_the_current_mode_and_once_does_not_reapply() {
    let core = Core::new().await;
    let dir = tempfile::tempdir().unwrap();
    Config {
        endpoint: core.endpoint.clone(),
        ..Default::default()
    }
    .save(dir.path())
    .unwrap();
    let handle = Backend::start(dir.path().to_owned()).unwrap();
    until(Duration::from_secs(5), || {
        handle.shared.lock().unwrap().connected
    })
    .await;
    handle
        .apply_process_mode(ProcessControl::Maintain(ProcessMode::Off))
        .unwrap();
    until(Duration::from_secs(5), || {
        handle.shared.lock().unwrap().process_mode == "off"
    })
    .await;
    handle
        .apply_process_mode(ProcessControl::Unmanaged)
        .unwrap();
    until(Duration::from_secs(4), || {
        handle.shared.lock().unwrap().process_status == ProcessControlStatus::Idle
    })
    .await;
    assert_eq!(*core.mode.lock().unwrap(), "off");
    let patches = core.patches.load(Ordering::Relaxed);
    *core.mode.lock().unwrap() = "strict".into();
    handle.retry_process_control();
    until(Duration::from_secs(4), || {
        handle.shared.lock().unwrap().process_mode == "strict"
    })
    .await;
    assert_eq!(core.patches.load(Ordering::Relaxed), patches);
    handle
        .apply_process_mode(ProcessControl::ApplyOnce(ProcessMode::Always))
        .unwrap();
    until(Duration::from_secs(4), || {
        handle.config.lock().unwrap().process_control == ProcessControl::Unmanaged
            && handle.shared.lock().unwrap().process_mode == "always"
    })
    .await;
    let patches = core.patches.load(Ordering::Relaxed);
    *core.mode.lock().unwrap() = "off".into();
    handle.retry_process_control();
    until(Duration::from_secs(4), || {
        handle.shared.lock().unwrap().process_mode == "off"
    })
    .await;
    assert_eq!(core.patches.load(Ordering::Relaxed), patches);
    stop(&handle).await;
}
async fn scripted_connection(mut stream: impl AsyncRead + AsyncWrite + Unpin, step: Step) {
    let request = receive(&mut stream)
        .await
        .expect("scripted request was cancelled before headers");
    assert_eq!(request.path, "/configs");
    assert_eq!(request.method, step.method);
    assert!(
        request
            .headers
            .to_lowercase()
            .contains("authorization: bearer unit-secret")
    );
    if let Some(target) = step.target {
        assert!(
            request
                .headers
                .to_lowercase()
                .contains("content-type: application/json")
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&request.body).unwrap(),
            serde_json::json!({"find-process-mode":target})
        );
    } else {
        assert!(request.body.is_empty());
    }
    tokio::time::sleep(step.delay).await;
    reply(&mut stream, step.code, &step.body).await;
}
async fn scripted(pipe: bool, steps: Vec<Step>, policy: ProcessControl, valid: bool) -> Outcome {
    let endpoint;
    let server;
    if pipe {
        endpoint = format!(r"\\.\pipe\mihomo-traffic-test-{}", uuid::Uuid::new_v4());
        let mut instance = ServerOptions::new()
            .first_pipe_instance(true)
            .create(&endpoint)
            .unwrap();
        let name = endpoint.clone();
        server = tokio::spawn(async move {
            let count = steps.len();
            for (i, step) in steps.into_iter().enumerate() {
                instance.connect().await.unwrap();
                // Keep an instance listening while the previous one is served.
                let next = (i + 1 < count).then(|| ServerOptions::new().create(&name).unwrap());
                scripted_connection(instance, step).await;
                if let Some(next) = next {
                    instance = next;
                } else {
                    break;
                }
            }
        });
    } else {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        endpoint = format!("http://{}", listener.local_addr().unwrap());
        server = tokio::spawn(async move {
            for step in steps {
                scripted_connection(listener.accept().await.unwrap().0, step).await;
            }
        });
    }
    let mut client = Client::new(&Config {
        endpoint,
        secret: "unit-secret".into(),
        ..Default::default()
    })
    .unwrap();
    let result = check_and_apply(&mut client, policy, || valid).await;
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
    result
}

#[tokio::test]
async fn both_transports_patch_authenticate_and_verify_empty_204() {
    for pipe in [false, true] {
        let outcome = scripted(
            pipe,
            vec![get("strict"), patch("always"), get("always")],
            ProcessControl::KeepAlways,
            true,
        )
        .await;
        outcome.result.unwrap();
        assert_eq!(outcome.mode, "always");
        scripted(
            pipe,
            vec![get("always"), patch("strict"), get("strict")],
            ProcessControl::RestoreStrict,
            true,
        )
        .await
        .result
        .unwrap();
        // Already at target, and unmanaged mode, must never PATCH.
        scripted(pipe, vec![get("always")], ProcessControl::KeepAlways, true)
            .await
            .result
            .unwrap();
        scripted(pipe, vec![get("off")], ProcessControl::Unmanaged, true)
            .await
            .result
            .unwrap();
    }
}

#[tokio::test]
async fn both_transports_reject_denied_unsupported_unknown_and_unconfirmed_changes() {
    for pipe in [false, true] {
        let mut denied = get("strict");
        denied.code = 401;
        assert!(
            scripted(pipe, vec![denied], ProcessControl::KeepAlways, true)
                .await
                .result
                .is_err()
        );
        let mut unsupported = patch("always");
        unsupported.code = 405;
        let failed = scripted(
            pipe,
            vec![get("strict"), unsupported],
            ProcessControl::KeepAlways,
            true,
        )
        .await;
        assert_eq!(failed.mode, "strict");
        assert!(failed.result.is_err());
        let failed = scripted(
            pipe,
            vec![get("strict"), patch("always"), get("strict")],
            ProcessControl::KeepAlways,
            true,
        )
        .await;
        assert!(
            failed.result.is_err(),
            "204 without matching readback was accepted"
        );
        let unknown = scripted(
            pipe,
            vec![get("future-mode")],
            ProcessControl::KeepAlways,
            true,
        )
        .await;
        assert_eq!(unknown.mode, "future-mode");
        assert!(unknown.result.is_err());
        let mut missing = get("strict");
        missing.body = "{}".into();
        assert!(
            scripted(pipe, vec![missing], ProcessControl::KeepAlways, true)
                .await
                .result
                .is_err()
        );
        assert!(
            scripted(pipe, vec![get("strict")], ProcessControl::KeepAlways, false)
                .await
                .result
                .is_err()
        );
    }
}

#[tokio::test]
async fn both_transports_timeout_without_leaking_the_request() {
    for pipe in [false, true] {
        let mut slow = get("strict");
        slow.delay = Duration::from_millis(3200);
        let start = Instant::now();
        let failed = scripted(pipe, vec![slow], ProcessControl::KeepAlways, true).await;
        assert!(failed.result.is_err());
        assert!(start.elapsed() < Duration::from_secs(4));
    }
}

struct Core {
    endpoint: String,
    mode: Arc<Mutex<String>>,
    patches: Arc<AtomicUsize>,
    samples: Arc<AtomicUsize>,
    config_reads: Arc<AtomicUsize>,
    delay: Arc<AtomicU64>,
    reject_patch: Arc<AtomicBool>,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for Core {
    fn drop(&mut self) {
        self.server.abort();
    }
}
impl Core {
    async fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let mode = Arc::new(Mutex::new("strict".to_string()));
        let patches = Arc::new(AtomicUsize::new(0));
        let samples = Arc::new(AtomicUsize::new(0));
        let config_reads = Arc::new(AtomicUsize::new(0));
        let delay = Arc::new(AtomicU64::new(0));
        let reject_patch = Arc::new(AtomicBool::new(false));
        let (m, p, s, r, d, deny) = (
            mode.clone(),
            patches.clone(),
            samples.clone(),
            config_reads.clone(),
            delay.clone(),
            reject_patch.clone(),
        );
        let server = tokio::spawn(async move {
            let mut tasks = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (mut stream,_) = accepted.unwrap();
                        let (m,p,s,r,d,deny) = (m.clone(),p.clone(),s.clone(),r.clone(),d.clone(),deny.clone());
                        tasks.spawn(async move {
                            let Some(request) = receive(&mut stream).await else { return; };
                            let mut code = 200;
                            let body = match (request.method.as_str(),request.path.as_str()) {
                                ("GET","/version") => r#"{"version":"test-core"}"#.into(),
                                ("GET","/providers/proxies") => r#"{"providers":{}}"#.into(),
                                ("GET","/connections") => {
                                    let n = s.fetch_add(1,Ordering::Relaxed) + 1;
                                    serde_json::json!({"uploadTotal":n*100,"downloadTotal":n*500,"connections":[]}).to_string()
                                },
                                ("GET","/configs") => {
                                    r.fetch_add(1,Ordering::Relaxed);
                                    tokio::time::sleep(Duration::from_millis(d.load(Ordering::Relaxed))).await;
                                    serde_json::json!({"find-process-mode":*m.lock().unwrap()}).to_string()
                                },
                                ("PATCH","/configs") => {
                                    if deny.load(Ordering::Relaxed) { code = 403; String::new() }
                                    else {
                                        let value:serde_json::Value = serde_json::from_slice(&request.body).unwrap();
                                        *m.lock().unwrap() = value["find-process-mode"].as_str().unwrap().into();
                                        p.fetch_add(1,Ordering::Relaxed); code=204; String::new()
                                    }
                                },
                                _ => panic!("unexpected route: {} {}", request.method, request.path),
                            };
                            reply(&mut stream,code,&body).await;
                        });
                    },
                    _ = tasks.join_next(), if !tasks.is_empty() => {},
                }
            }
        });
        Self {
            endpoint,
            mode,
            patches,
            samples,
            config_reads,
            delay,
            reject_patch,
            server,
        }
    }
}
async fn until(timeout: Duration, predicate: impl Fn() -> bool) {
    let start = Instant::now();
    while !predicate() {
        assert!(start.elapsed() < timeout, "condition timed out");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
async fn stop(handle: &Backend) {
    let _ = handle.request_shutdown();
    until(Duration::from_secs(5), || {
        handle.done.load(Ordering::Acquire)
    })
    .await;
}

#[tokio::test]
async fn permanent_save_stops_maintenance_retains_file_on_api_failure_and_retries_once() {
    use crate::{
        client_config::{Kind, SavedMode, Target},
        permanent_control::{Request as PermanentRequest, Status},
    };
    let core = Core::new().await;
    core.reject_patch.store(true, Ordering::Relaxed);
    let data = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    std::fs::create_dir(files.path().join("profiles")).unwrap();
    std::fs::write(
        files.path().join("config.yaml"),
        "mixed-port: 7890\nexternal-controller-pipe: test\n",
    )
    .unwrap();
    std::fs::write(
        files.path().join("profiles.yaml"),
        "items:\n- uid: Merge\n  type: merge\n  file: Merge.yaml\n",
    )
    .unwrap();
    std::fs::write(
        files.path().join("profiles/Merge.yaml"),
        "# preserve\nunknown: keep\n",
    )
    .unwrap();
    Config {
        endpoint: core.endpoint.clone(),
        process_control: ProcessControl::Maintain(ProcessMode::Strict),
        ..Default::default()
    }
    .save(data.path())
    .unwrap();
    let handle = Backend::start(data.path().to_owned()).unwrap();
    *handle.permanent_control.fixture.lock().unwrap() = Some((
        Target {
            kind: Kind::Verge,
            root: files.path().to_owned(),
            executable: None,
        },
        true,
    ));
    handle
        .permanent_control
        .post(&handle, PermanentRequest::Refresh);
    until(Duration::from_secs(5), || {
        handle.shared.lock().unwrap().permanent.available
    })
    .await;
    handle
        .permanent_control
        .post(&handle, PermanentRequest::Save(ProcessMode::Always));
    until(Duration::from_secs(5), || {
        let shared = handle.shared.lock().unwrap();
        shared.permanent.mode == SavedMode::Explicit(ProcessMode::Always)
            && shared.permanent.error.contains("内核应用失败")
    })
    .await;
    assert_eq!(
        handle.config.lock().unwrap().process_control,
        ProcessControl::ApplyOnce(ProcessMode::Always)
    );
    assert_eq!(*core.mode.lock().unwrap(), "strict");
    assert!(
        std::fs::read_to_string(files.path().join("profiles/Merge.yaml"))
            .unwrap()
            .contains("unknown: keep")
    );
    assert!(!handle.shared.lock().unwrap().permanent.backup.is_empty());
    core.reject_patch.store(false, Ordering::Relaxed);
    handle.retry_process_control();
    until(Duration::from_secs(5), || {
        handle.shared.lock().unwrap().permanent.status == Status::Applied
    })
    .await;
    assert_eq!(
        handle.config.lock().unwrap().process_control,
        ProcessControl::Unmanaged
    );
    let patches = core.patches.load(Ordering::Relaxed);
    *core.mode.lock().unwrap() = "off".into();
    handle.retry_process_control();
    until(Duration::from_secs(5), || {
        handle.shared.lock().unwrap().process_mode == "off"
    })
    .await;
    assert_eq!(core.patches.load(Ordering::Relaxed), patches);
    stop(&handle).await;
}

#[tokio::test]
async fn permanent_waits_for_exit_can_cancel_and_saves_after_exit() {
    use crate::{
        client_config::{Kind, Target},
        permanent_control::{Request as PermanentRequest, Status},
    };
    let core = Core::new().await;
    let data = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let config = serde_json::json!({"patchClashConfig":{"find-process-mode":"strict"},"keep":true})
        .to_string();
    let original =
        serde_json::json!({"flutter.config":config,"flutter.other":"untouched"}).to_string();
    let path = files.path().join("shared_preferences.json");
    std::fs::write(&path, &original).unwrap();
    Config {
        endpoint: core.endpoint.clone(),
        ..Default::default()
    }
    .save(data.path())
    .unwrap();
    let handle = Backend::start(data.path().to_owned()).unwrap();
    let target = Target {
        kind: Kind::FlClash,
        root: files.path().to_owned(),
        executable: None,
    };
    *handle.permanent_control.fixture.lock().unwrap() = Some((target.clone(), true));
    handle
        .permanent_control
        .post(&handle, PermanentRequest::Refresh);
    until(Duration::from_secs(5), || {
        handle.shared.lock().unwrap().permanent.available
    })
    .await;
    handle
        .permanent_control
        .post(&handle, PermanentRequest::Save(ProcessMode::Off));
    until(Duration::from_secs(4), || {
        handle.shared.lock().unwrap().permanent.status == Status::WaitingExit
    })
    .await;
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    handle
        .permanent_control
        .post(&handle, PermanentRequest::Cancel);
    until(Duration::from_secs(3), || {
        handle.shared.lock().unwrap().permanent.status == Status::Idle
    })
    .await;
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    handle
        .permanent_control
        .post(&handle, PermanentRequest::Save(ProcessMode::Off));
    until(Duration::from_secs(4), || {
        handle.shared.lock().unwrap().permanent.status == Status::WaitingExit
    })
    .await;
    // Changing the saved connection cancels the pending client file operation.
    let other = Core::new().await;
    handle
        .writer
        .update(|config| {
            config.endpoint = other.endpoint.clone();
            true
        })
        .unwrap();
    handle.send(Command::Reconnect);
    until(Duration::from_secs(5), || {
        let shared = handle.shared.lock().unwrap();
        shared.connected
            && shared.endpoint == other.endpoint
            && shared.permanent.status == Status::Idle
            && shared.permanent.available
    })
    .await;
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    handle
        .permanent_control
        .post(&handle, PermanentRequest::Save(ProcessMode::Off));
    until(Duration::from_secs(4), || {
        handle.shared.lock().unwrap().permanent.status == Status::WaitingExit
    })
    .await;
    *handle.permanent_control.fixture.lock().unwrap() = Some((target, false));
    until(Duration::from_secs(5), || {
        let shared = handle.shared.lock().unwrap();
        assert!(
            shared.permanent.status != Status::Failed && shared.permanent.error.is_empty(),
            "{}",
            shared.permanent.error
        );
        shared.permanent.status == Status::Applied
    })
    .await;
    assert_eq!(*core.mode.lock().unwrap(), "strict");
    assert_eq!(*other.mode.lock().unwrap(), "off");
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("untouched")
    );
    stop(&handle).await;
}

fn restore_fixture(
    kind: crate::client_config::Kind,
    root: &std::path::Path,
) -> crate::client_config::Target {
    use crate::client_config::{Kind, Target};
    std::fs::create_dir(root.join("profiles")).unwrap();
    match kind {
        Kind::Verge => {
            std::fs::write(
                root.join("config.yaml"),
                "mixed-port: 7890\nexternal-controller-pipe: test\n",
            )
            .unwrap();
            std::fs::write(
                root.join("profiles.yaml"),
                "items:\n- uid: Merge\n  type: merge\n  file: Merge.yaml\n",
            )
            .unwrap();
        }
        Kind::Nyanpasu => {
            std::fs::write(root.join("profiles.yaml"), "_nyanpasu:\n  document: profiles\n  schema_revision: 4\nrevision: 1\nitems: []\nglobal_transforms: []\nvalid: []\n").unwrap();
        }
        Kind::FlClash => {
            let inner = serde_json::json!({"patchClashConfig":{},"unknown":"keep"}).to_string();
            std::fs::write(
                root.join("shared_preferences.json"),
                serde_json::json!({"flutter.config":inner,"other":"keep"}).to_string(),
            )
            .unwrap();
        }
    }
    let target = Target {
        kind,
        root: root.to_owned(),
        executable: None,
    };
    target
        .prepare(ProcessMode::Always)
        .unwrap()
        .commit(&root.join("backups"), || true)
        .unwrap();
    target
}

#[tokio::test]
async fn permanent_restore_all_clients_stops_maintenance_and_waits_for_client_reload() {
    use crate::{
        client_config::{Kind, SavedMode},
        permanent_control::{Request as PermanentRequest, Status},
    };
    for kind in [Kind::Verge, Kind::Nyanpasu, Kind::FlClash] {
        let core = Core::new().await;
        let data = tempfile::tempdir().unwrap();
        let files = tempfile::tempdir().unwrap();
        let target = restore_fixture(kind, files.path());
        Config {
            endpoint: core.endpoint.clone(),
            process_control: ProcessControl::Maintain(ProcessMode::Always),
            ..Default::default()
        }
        .save(data.path())
        .unwrap();
        let handle = Backend::start(data.path().to_owned()).unwrap();
        *handle.permanent_control.fixture.lock().unwrap() = Some((target.clone(), true));
        handle
            .permanent_control
            .post(&handle, PermanentRequest::Refresh);
        until(Duration::from_secs(5), || {
            let shared = handle.shared.lock().unwrap();
            shared.permanent.available
                && shared.process_mode == "always"
                && shared.process_status == ProcessControlStatus::Applied
        })
        .await;
        let path = target.read().unwrap().source;
        let before = std::fs::read(&path).unwrap();
        let patches = core.patches.load(Ordering::Relaxed);
        handle
            .permanent_control
            .post(&handle, PermanentRequest::Restore);
        if kind.needs_exit() {
            until(Duration::from_secs(4), || {
                handle.shared.lock().unwrap().permanent.status == Status::WaitingExit
            })
            .await;
            assert_eq!(std::fs::read(&path).unwrap(), before);
            handle
                .permanent_control
                .post(&handle, PermanentRequest::Cancel);
            until(Duration::from_secs(4), || {
                handle.shared.lock().unwrap().permanent.status == Status::Idle
            })
            .await;
            assert_eq!(std::fs::read(&path).unwrap(), before);
            handle
                .permanent_control
                .post(&handle, PermanentRequest::Restore);
            until(Duration::from_secs(4), || {
                handle.shared.lock().unwrap().permanent.status == Status::WaitingExit
            })
            .await;
            *handle.permanent_control.fixture.lock().unwrap() = Some((target.clone(), false));
        }
        until(Duration::from_secs(5), || {
            let shared = handle.shared.lock().unwrap();
            shared.permanent.status == Status::Restored
                && shared.permanent.mode == SavedMode::Inherited
                && shared.permanent.error.is_empty()
        })
        .await;
        assert_eq!(target.read().unwrap().mode, SavedMode::Inherited);
        let config = Config::load(data.path()).unwrap();
        assert_eq!(config.process_control, ProcessControl::Unmanaged);
        assert!(config.process_endpoint.is_none() && config.process_client.is_none());
        assert_eq!(
            *core.mode.lock().unwrap(),
            "always",
            "restoring inheritance must not guess an effective core mode"
        );
        assert_eq!(core.patches.load(Ordering::Relaxed), patches);
        assert!(!handle.shared.lock().unwrap().permanent.backup.is_empty());
        *core.mode.lock().unwrap() = "off".into();
        handle.retry_process_control();
        until(Duration::from_secs(5), || {
            handle.shared.lock().unwrap().process_mode == "off"
        })
        .await;
        assert_eq!(
            core.patches.load(Ordering::Relaxed),
            patches,
            "restoration did not stop ongoing maintenance"
        );
        stop(&handle).await;
    }
}

#[tokio::test]
async fn permanent_restore_pending_is_cancelled_by_connection_switch() {
    use crate::{
        client_config::Kind,
        permanent_control::{Request as PermanentRequest, Status},
    };
    let old = Core::new().await;
    let new = Core::new().await;
    let data = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let target = restore_fixture(Kind::FlClash, files.path());
    let path = target.read().unwrap().source;
    let before = std::fs::read(&path).unwrap();
    Config {
        endpoint: old.endpoint.clone(),
        ..Default::default()
    }
    .save(data.path())
    .unwrap();
    let handle = Backend::start(data.path().to_owned()).unwrap();
    *handle.permanent_control.fixture.lock().unwrap() = Some((target.clone(), true));
    handle
        .permanent_control
        .post(&handle, PermanentRequest::Refresh);
    until(Duration::from_secs(5), || {
        handle.shared.lock().unwrap().permanent.available
    })
    .await;
    handle
        .permanent_control
        .post(&handle, PermanentRequest::Restore);
    until(Duration::from_secs(4), || {
        handle.shared.lock().unwrap().permanent.status == Status::WaitingExit
    })
    .await;
    handle
        .writer
        .update(|config| {
            config.endpoint = new.endpoint.clone();
            true
        })
        .unwrap();
    handle.send(Command::Reconnect);
    *handle.permanent_control.fixture.lock().unwrap() = Some((target, false));
    until(Duration::from_secs(5), || {
        let shared = handle.shared.lock().unwrap();
        shared.endpoint == new.endpoint
            && shared.connected
            && shared.permanent.status == Status::Idle
    })
    .await;
    assert_eq!(std::fs::read(&path).unwrap(), before);
    stop(&handle).await;
}

#[tokio::test]
async fn lifecycle_persists_restores_after_reload_and_stops_managing_after_disable() {
    let core = Core::new().await;
    let dir = tempfile::tempdir().unwrap();
    Config {
        endpoint: core.endpoint.clone(),
        ..Default::default()
    }
    .save(dir.path())
    .unwrap();
    let handle = Backend::start(dir.path().to_path_buf()).unwrap();
    until(Duration::from_secs(5), || {
        handle.shared.lock().unwrap().process_mode == "strict"
    })
    .await;
    assert_eq!(core.patches.load(Ordering::Relaxed), 0);
    handle.set_process_control(true).unwrap();
    until(Duration::from_secs(4), || {
        handle.shared.lock().unwrap().process_status == ProcessControlStatus::Applied
    })
    .await;
    assert_eq!(*core.mode.lock().unwrap(), "always");
    let samples = core.samples.load(Ordering::Relaxed);
    *core.mode.lock().unwrap() = "strict".into();
    tokio::time::sleep(Duration::from_secs(12)).await;
    assert_eq!(*core.mode.lock().unwrap(), "strict");
    assert!(core.samples.load(Ordering::Relaxed) >= samples + 8);
    until(Duration::from_secs(52), || {
        *core.mode.lock().unwrap() == "always"
    })
    .await;
    assert!(core.samples.load(Ordering::Relaxed) >= samples + 50);
    stop(&handle).await;
    assert_eq!(
        Config::load(dir.path()).unwrap().process_control,
        ProcessControl::KeepAlways
    );
    drop(handle);
    *core.mode.lock().unwrap() = "strict".into();
    let handle = Backend::start(dir.path().to_path_buf()).unwrap();
    until(Duration::from_secs(5), || {
        handle.shared.lock().unwrap().process_mode == "always"
    })
    .await;
    handle
        .writer
        .update(|config| {
            config.theme = "dark".into();
            true
        })
        .unwrap();
    handle.set_process_control(false).unwrap();
    until(Duration::from_secs(4), || {
        handle.config.lock().unwrap().process_control == ProcessControl::Unmanaged
    })
    .await;
    assert_eq!(*core.mode.lock().unwrap(), "strict");
    assert_eq!(handle.config.lock().unwrap().theme, "dark");
    let patches = core.patches.load(Ordering::Relaxed);
    *core.mode.lock().unwrap() = "off".into();
    handle.retry_process_control();
    until(Duration::from_secs(4), || {
        handle.shared.lock().unwrap().process_mode == "off"
    })
    .await;
    assert_eq!(core.patches.load(Ordering::Relaxed), patches);
    stop(&handle).await;
    assert_eq!(
        Config::load(dir.path()).unwrap().process_control,
        ProcessControl::Unmanaged
    );
}

#[tokio::test]
async fn slow_control_does_not_block_samples_and_old_target_cannot_patch_after_switch() {
    let old = Core::new().await;
    let new = Core::new().await;
    let dir = tempfile::tempdir().unwrap();
    Config {
        endpoint: old.endpoint.clone(),
        process_control: ProcessControl::KeepAlways,
        ..Default::default()
    }
    .save(dir.path())
    .unwrap();
    old.delay.store(2700, Ordering::Relaxed);
    let handle = Backend::start(dir.path().to_path_buf()).unwrap();
    until(Duration::from_secs(5), || {
        old.config_reads.load(Ordering::Relaxed) > 0
    })
    .await;
    until(Duration::from_secs(4), || {
        old.samples.load(Ordering::Relaxed) >= 3
    })
    .await;
    handle
        .writer
        .update(|config| {
            config.endpoint = new.endpoint.clone();
            config.process_control = ProcessControl::Unmanaged;
            true
        })
        .unwrap();
    handle.send(Command::Reconnect);
    until(Duration::from_secs(5), || {
        let shared = handle.shared.lock().unwrap();
        shared.endpoint == new.endpoint && shared.process_mode == "strict"
    })
    .await;
    assert_eq!(old.patches.load(Ordering::Relaxed), 0);
    assert_eq!(new.patches.load(Ordering::Relaxed), 0);
    stop(&handle).await;
}
