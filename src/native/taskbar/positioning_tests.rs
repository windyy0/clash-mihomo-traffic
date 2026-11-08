use super::*;
use crate::config_writer::ConfigWriter;
use std::sync::{Condvar, mpsc};

unsafe fn own_dock() -> HWND {
    let mut child = null_mut();
    loop {
        child = FindWindowExW(
            TASKBAR_WINDOW.load(Ordering::Acquire) as _,
            child,
            wide("MihomoTrafficShell").as_ptr(),
            null(),
        );
        if child.is_null() {
            return child;
        }
        let mut pid = 0;
        GetWindowThreadProcessId(child, &mut pid);
        if pid == std::process::id() {
            return child;
        }
    }
}
fn wait_for(mut condition: impl FnMut() -> bool, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while !condition() {
        assert!(Instant::now() < deadline, "taskbar condition timed out");
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn saw_event(event: u32, window: isize) -> bool {
    PROBE_ACTIVITY
        .lock()
        .unwrap()
        .events
        .contains(&(event, window))
}
unsafe extern "system" fn fixture_proc(
    window: HWND,
    message: u32,
    wp: WPARAM,
    lp: LPARAM,
) -> LRESULT {
    match message {
        // Perform operations in the foreign process, so SKIPOWNPROCESS
        // exercises the same event path as an actual third-party panel.
        m if m == WM_APP + 71 => {
            ShowWindow(window, wp as i32);
            0
        }
        m if m == WM_APP + 72 => {
            let parent = FindWindowW(wide("Shell_TrayWnd").as_ptr(), null());
            let mut rect = RECT::default();
            GetClientRect(parent, &mut rect);
            SetWindowPos(
                window,
                HWND_TOP,
                wp as i32,
                0,
                lp as i32,
                rect.bottom,
                SWP_NOACTIVATE,
            );
            0
        }
        m if m == WM_APP + 73 => {
            let parent = if wp == 0 {
                null_mut()
            } else {
                FindWindowW(wide("Shell_TrayWnd").as_ptr(), null())
            };
            SetParent(window, parent);
            0
        }
        _ => {
            let original = GetWindowLongPtrW(window, GWLP_USERDATA);
            CallWindowProcW(
                Some(std::mem::transmute::<
                    isize,
                    unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT,
                >(original)),
                window,
                message,
                wp,
                lp,
            )
        }
    }
}
// A separate process supplies a real UIA control. It is deliberately not
// excluded as this app's own child, so the live scan must reserve its bounds.
unsafe fn occupied_fixture(ready_path: &str) {
    let parent = FindWindowW(wide("Shell_TrayWnd").as_ptr(), null());
    let mut rect = RECT::default();
    assert_ne!(GetClientRect(parent, &mut rect), 0);
    let window = CreateWindowExW(
        WS_EX_NOACTIVATE | WS_EX_LAYERED | WS_EX_TRANSPARENT,
        wide("Button").as_ptr(),
        wide("Taskbar layout test fixture").as_ptr(),
        WS_CHILD | WS_VISIBLE | WS_DISABLED,
        0,
        0,
        rect.right,
        rect.bottom,
        parent,
        null_mut(),
        GetModuleHandleW(null()),
        null(),
    );
    assert!(!window.is_null());
    let original = SetWindowLongPtrW(window, GWLP_WNDPROC, fixture_proc as *const () as isize);
    SetWindowLongPtrW(window, GWLP_USERDATA, original);
    SetLayeredWindowAttributes(window, 0, 1, LWA_ALPHA);
    std::fs::write(ready_path, (window as isize).to_string()).unwrap();
    let started = Instant::now();
    while IsWindow(window) != 0 && started.elapsed() < Duration::from_secs(60) {
        let mut message = MSG::default();
        while PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) != 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    if IsWindow(window) != 0 {
        DestroyWindow(window);
    }
}
#[test]
#[ignore = "creates a temporary taskbar child on the current Windows desktop"]
fn real_window_moves_while_config_writer_is_blocked() {
    unsafe {
        SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    if let Ok(path) = std::env::var("MIHOMO_TASKBAR_FIXTURE_READY") {
        unsafe {
            occupied_fixture(&path);
        }
        return;
    }
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server_stop = Arc::new(AtomicBool::new(false));
    let server_done = server_stop.clone();
    let (network_tx, network_rx) = mpsc::channel();
    let network = std::thread::spawn(move || {
        let mut connections = Vec::new();
        while !server_done.load(Ordering::Acquire) {
            if let Ok((stream, _)) = listener.accept() {
                connections.push(stream); // Deliberately never answer HTTP requests.
                let _ = network_tx.send(());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    });
    let data_dir = tempfile::tempdir().unwrap();
    Config {
        endpoint,
        taskbar_position_mode: TaskbarPositionMode::Manual,
        taskbar: false,
        ..Config::default()
    }
    .save(data_dir.path())
    .unwrap();
    let backend = crate::backend::Backend::start(data_dir.path().to_path_buf()).unwrap();
    let shared = backend.shared.clone();
    let config = backend.config.clone();
    let _events = start_shell(shared.clone(), config.clone(), backend.stop.clone());
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    struct Cleanup(
        crate::backend::Backend,
        Arc<(Mutex<bool>, Condvar)>,
        Arc<AtomicBool>,
    );
    impl Drop for Cleanup {
        fn drop(&mut self) {
            *self.1.0.lock().unwrap() = true;
            self.1.1.notify_all();
            let _ = self.0.request_shutdown();
            self.2.store(true, Ordering::Release);
            unsafe {
                PostMessageW(SHELL_WINDOW.load(Ordering::Acquire) as _, WM_TIMER, 1, 0);
            }
        }
    }
    let cleanup = Cleanup(backend.clone(), gate.clone(), server_stop);
    wait_for(
        || SHELL_WINDOW.load(Ordering::Acquire) != 0,
        Duration::from_secs(3),
    );
    let (started_tx, started_rx) = mpsc::channel();
    let writer_gate = gate.clone();
    let writer = ConfigWriter::start(
        config.clone(),
        move |_, _| {
            let _ = started_tx.send(());
            let (lock, cv) = &*writer_gate;
            let mut ready = lock.lock().unwrap();
            while !*ready {
                ready = cv.wait(ready).unwrap();
            }
            Ok(())
        },
        |_| {},
    )
    .unwrap();
    let mut next = config.lock().unwrap().clone();
    next.taskbar = true;
    writer.submit(next.clone()).unwrap();
    wait_for(
        || !unsafe { own_dock() }.is_null() && shared.lock().unwrap().taskbar_geometry.ready,
        Duration::from_secs(3),
    );
    started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    network_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let geometry = shared.lock().unwrap().taskbar_geometry;
    let mut durations = vec![];
    for step in 1..=12 {
        let expected = (geometry.x + step).min(geometry.max_x);
        next.taskbar_offset = expected
            - taskbar_layout::anchor(
                next.taskbar_left,
                geometry.tray_start,
                geometry.widget_width,
                geometry.margin,
            );
        let started = Instant::now();
        writer.submit(next.clone()).unwrap();
        wait_for(
            || unsafe {
                let dock = own_dock();
                let mut r = RECT::default();
                if dock.is_null() || GetWindowRect(dock, &mut r) == 0 {
                    return false;
                }
                let mut origin = POINT {
                    x: r.left,
                    y: r.top,
                };
                ScreenToClient(TASKBAR_WINDOW.load(Ordering::Acquire) as _, &mut origin);
                origin.x == expected
            },
            Duration::from_millis(500),
        );
        durations.push(started.elapsed());
    }
    println!("12 physical window moves with blocked HTTP sampling and save thread: {durations:?}");
    assert!(
        durations
            .iter()
            .all(|duration| *duration < Duration::from_millis(100)),
        "manual movement exceeded the 100ms target"
    );
    *gate.0.lock().unwrap() = true;
    gate.1.notify_all();
    next.taskbar_position_mode = TaskbarPositionMode::AutoAvoid;
    writer.submit(next.clone()).unwrap();
    wait_for(
        || {
            shared
                .lock()
                .unwrap()
                .taskbar_status
                .starts_with("自动避让")
        },
        Duration::from_secs(5),
    );
    let automatic_x = shared.lock().unwrap().taskbar_geometry.x;
    request_taskbar_update(true);
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(
        shared.lock().unwrap().taskbar_geometry.x,
        automatic_x,
        "safe automatic position must remain stable"
    );
    next.taskbar = false;
    writer.submit(next.clone()).unwrap();
    wait_for(
        || shared.lock().unwrap().taskbar_status == "未启用",
        Duration::from_secs(2),
    );
    next.taskbar = true;
    writer.submit(next.clone()).unwrap();
    wait_for(
        || {
            shared
                .lock()
                .unwrap()
                .taskbar_status
                .starts_with("自动避让")
        },
        Duration::from_secs(5),
    );

    wait_for(
        || PROBE_ACTIVITY.lock().unwrap().poll_ms == STABLE_PROBE_MS,
        Duration::from_secs(2),
    );
    // Let the startup verification finish, then cross two old 2s periods.
    std::thread::sleep(Duration::from_secs(2));
    let idle_requests = PROBE_ACTIVITY.lock().unwrap().requests;
    let idle_events = PROBE_ACTIVITY.lock().unwrap().events.len();
    std::thread::sleep(Duration::from_millis(4200));
    if PROBE_ACTIVITY.lock().unwrap().requests != idle_requests {
        let events = PROBE_ACTIVITY.lock().unwrap().events[idle_events..].to_vec();
        for (event, handle) in events.into_iter().take(12) {
            let mut class = [0u16; 128];
            let mut process = 0;
            unsafe {
                GetClassNameW(handle as _, class.as_mut_ptr(), class.len() as i32);
                GetWindowThreadProcessId(handle as _, &mut process);
            }
            println!(
                "Idle scan source: event={event:#x} hwnd={handle:#x} pid={process} class={}",
                String::from_utf16_lossy(&class).trim_end_matches('\0')
            );
        }
    }
    assert_eq!(
        PROBE_ACTIVITY.lock().unwrap().requests,
        idle_requests,
        "stable layout still requested two-second full probes"
    );
    println!("Stable display uses a 60s fallback; no full scans during 4.2s of idle.");

    let fixture_dir = tempfile::tempdir().unwrap();
    let ready = fixture_dir.path().join("ready");
    let fixture = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["real_window_moves", "--ignored", "--nocapture"])
        .env("MIHOMO_TASKBAR_FIXTURE_READY", &ready)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    struct Fixture(std::process::Child, std::path::PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            if let Ok(value) = std::fs::read_to_string(&self.1)
                && let Ok(window) = value.parse::<isize>()
            {
                unsafe {
                    PostMessageW(window as _, WM_CLOSE, 0, 0);
                }
            }
            let _ = self.0.wait();
        }
    }
    let fixture = Fixture(fixture, ready.clone());
    wait_for(|| ready.exists(), Duration::from_secs(3));
    let taskbar_frame = taskbar_probe::frame(TASKBAR_WINDOW.load(Ordering::Acquire)).unwrap();
    let full_width = taskbar_layout::Interval {
        left: 0,
        right: taskbar_frame.right - taskbar_frame.left,
    };
    let native_bounds = taskbar_probe::native_occupied(taskbar_frame, false).unwrap();
    assert!(
        native_bounds.contains(&full_width),
        "native scan missed a foreign child"
    );
    let geometry = shared.lock().unwrap().taskbar_geometry;
    assert!(
        !native_bounds.contains(&taskbar_layout::Interval {
            left: geometry.x,
            right: geometry.x + geometry.widget_width,
        }),
        "native scan reserved this process's own dock"
    );
    let fixture_window = std::fs::read_to_string(&ready)
        .unwrap()
        .parse::<isize>()
        .unwrap();
    wait_for(
        || {
            shared
                .lock()
                .unwrap()
                .taskbar_status
                .starts_with("没有足够空位")
        },
        Duration::from_secs(5),
    );
    assert!(saw_event(EVENT_OBJECT_CREATE, fixture_window));
    assert_eq!(PROBE_ACTIVITY.lock().unwrap().poll_ms, RECOVERY_PROBE_MS);
    unsafe {
        PostMessageW(fixture_window as _, WM_APP + 71, SW_HIDE as usize, 0);
    }
    wait_for(
        || unsafe { IsWindowVisible(fixture_window as _) } == 0,
        Duration::from_secs(1),
    );
    assert!(
        !taskbar_probe::native_occupied(taskbar_frame, false)
            .unwrap()
            .contains(&full_width),
        "hidden native child still occupied space"
    );
    wait_for(
        || {
            shared
                .lock()
                .unwrap()
                .taskbar_status
                .starts_with("自动避让")
        },
        Duration::from_secs(6),
    );
    assert!(saw_event(EVENT_OBJECT_HIDE, fixture_window));
    assert_eq!(PROBE_ACTIVITY.lock().unwrap().poll_ms, STABLE_PROBE_MS);
    unsafe {
        PostMessageW(
            fixture_window as _,
            WM_APP + 71,
            SW_SHOWNOACTIVATE as usize,
            0,
        );
    }
    wait_for(
        || unsafe { IsWindowVisible(fixture_window as _) } != 0,
        Duration::from_secs(1),
    );
    assert!(
        taskbar_probe::native_occupied(taskbar_frame, false)
            .unwrap()
            .contains(&full_width)
    );
    println!(
        "Native child scan reserves visible foreign windows, skips hidden windows and its own dock."
    );
    wait_for(
        || {
            shared
                .lock()
                .unwrap()
                .taskbar_status
                .starts_with("没有足够空位")
        },
        Duration::from_secs(5),
    );
    assert_eq!(unsafe { IsWindowVisible(own_dock()) }, 0);
    assert!(saw_event(EVENT_OBJECT_SHOW, fixture_window));
    println!("A real foreign UIA control occupied the taskbar: automatic display hidden.");

    // Leave a narrow foreign child at the left edge, then move it over the
    // displayed widget while the fallback is 60s. Events must move us first.
    unsafe {
        PostMessageW(fixture_window as _, WM_APP + 72, 0, 1);
    }
    wait_for(
        || {
            shared
                .lock()
                .unwrap()
                .taskbar_status
                .starts_with("自动避让")
        },
        Duration::from_secs(6),
    );
    assert_eq!(PROBE_ACTIVITY.lock().unwrap().poll_ms, STABLE_PROBE_MS);
    let before_move = shared.lock().unwrap().taskbar_geometry;
    let moved = Instant::now();
    unsafe {
        PostMessageW(
            fixture_window as _,
            WM_APP + 72,
            before_move.x as usize,
            before_move.widget_width as isize,
        );
    }
    wait_for(
        || {
            let data = shared.lock().unwrap();
            data.taskbar_status.starts_with("自动避让") && data.taskbar_geometry.x != before_move.x
        },
        Duration::from_secs(6),
    );
    assert!(saw_event(EVENT_OBJECT_LOCATIONCHANGE, fixture_window));
    println!(
        "Foreign movement triggered avoidance in {:?}, with the fallback set to 60s.",
        moved.elapsed()
    );
    unsafe {
        PostMessageW(fixture_window as _, WM_APP + 73, 0, 0);
    }
    wait_for(|| unsafe { IsChild(TASKBAR_WINDOW.load(Ordering::Acquire) as _, fixture_window as _) } == 0,
        Duration::from_secs(1));
    wait_for(
        || saw_event(EVENT_OBJECT_PARENTCHANGE, fixture_window),
        Duration::from_secs(2),
    );
    PROBE_ACTIVITY
        .lock()
        .unwrap()
        .events
        .retain(|&(event, window)| event != EVENT_OBJECT_PARENTCHANGE || window != fixture_window);
    unsafe {
        PostMessageW(fixture_window as _, WM_APP + 73, 1, 0);
    }
    wait_for(
        || saw_event(EVENT_OBJECT_PARENTCHANGE, fixture_window),
        Duration::from_secs(2),
    );
    unsafe {
        PostMessageW(
            fixture_window as _,
            WM_APP + 72,
            0,
            full_width.right as isize,
        );
    }
    wait_for(
        || {
            shared
                .lock()
                .unwrap()
                .taskbar_status
                .starts_with("没有足够空位")
        },
        Duration::from_secs(6),
    );
    let restored = Instant::now();
    drop(fixture);
    // A destroyed HWND is no longer a child when its queued event arrives.
    // The identity cache must still recognize it and request a fresh scan.
    wait_for(
        || saw_event(EVENT_OBJECT_DESTROY, fixture_window),
        Duration::from_secs(2),
    );
    wait_for(
        || {
            shared
                .lock()
                .unwrap()
                .taskbar_status
                .starts_with("自动避让")
        },
        Duration::from_secs(5),
    );
    assert!(restored.elapsed() >= Duration::from_millis(300));
    assert_ne!(unsafe { IsWindowVisible(own_dock()) }, 0);
    println!(
        "Free space recovered and automatic display restored after {:?}.",
        restored.elapsed()
    );

    let old = unsafe { own_dock() };
    unsafe {
        PostMessageW(old, WM_CLOSE, 0, 0);
    }
    wait_for(|| unsafe { IsWindow(old) } == 0, Duration::from_millis(500));
    wait_for(
        || {
            shared
                .lock()
                .unwrap()
                .taskbar_status
                .starts_with("自动避让")
                && unsafe { IsWindowVisible(own_dock()) } != 0
        },
        Duration::from_secs(5),
    );
    println!("Destroyed taskbar child was recreated and automatic layout restored.");
    let requests = PROBE_ACTIVITY.lock().unwrap().requests;
    unsafe {
        PostMessageW(
            SHELL_WINDOW.load(Ordering::Acquire) as _,
            RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()),
            0,
            0,
        );
    }
    wait_for(
        || PROBE_ACTIVITY.lock().unwrap().requests > requests,
        Duration::from_secs(2),
    );
    next.taskbar_position_mode = TaskbarPositionMode::Manual;
    writer.submit(next.clone()).unwrap();
    wait_for(
        || PROBE_ACTIVITY.lock().unwrap().poll_ms == 0,
        Duration::from_secs(2),
    );
    assert!(!TASKBAR_WATCHING.load(Ordering::Acquire));
    let requests = PROBE_ACTIVITY.lock().unwrap().requests;
    next.taskbar_position_mode = TaskbarPositionMode::AutoAvoid;
    writer.submit(next.clone()).unwrap();
    wait_for(
        || PROBE_ACTIVITY.lock().unwrap().requests > requests,
        Duration::from_secs(2),
    );
    wait_for(
        || {
            shared
                .lock()
                .unwrap()
                .taskbar_status
                .starts_with("自动避让")
        },
        Duration::from_secs(6),
    );
    println!(
        "Detach, reattach and destroy events recognized; mode switch and Explorer notification force detection."
    );
    writer.close(Duration::from_secs(2)).unwrap();
    drop(cleanup);
    wait_for(
        || SHELL_WINDOW.load(Ordering::Acquire) == 0,
        Duration::from_secs(3),
    );
    network.join().unwrap();
    wait_for(
        || backend.done.load(Ordering::Acquire),
        Duration::from_secs(5),
    );
}
