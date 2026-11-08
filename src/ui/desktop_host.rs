use super::{
    UiBackend, UiEvent,
    bridge::Session,
    updates::ReleaseChecks,
    window::{InterfaceEvents, Launch, WindowState, run_interface, toggle_shell_display},
};
use crate::{backend::Backend, native};
use anyhow::{Context, Result};
use std::{
    ffi::OsString,
    os::windows::process::CommandExt,
    path::Path,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex, atomic::Ordering},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    System::Threading::CREATE_NO_WINDOW, UI::WindowsAndMessaging::AllowSetForegroundWindow,
};

struct Interface {
    child: Child,
    session: Arc<Session>,
}

impl Interface {
    fn start(
        executable: &Path,
        backend: &Backend,
        launch: Launch,
        updates: Arc<Mutex<ReleaseChecks>>,
        resume: Option<WindowState>,
    ) -> Result<Self> {
        let session = Arc::new(Session::new(resume));
        let mut interface = Self {
            child: Command::new(executable)
                .args(["--desktop-ui", launch.argument()])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .creation_flags(CREATE_NO_WINDOW)
                .spawn()
                .context("无法启动界面进程")?,
            session,
        };
        interface
            .session
            .serve(backend.clone(), updates, &mut interface.child)?;
        interface.allow_foreground();
        Ok(interface)
    }

    fn allow_foreground(&self) {
        unsafe { AllowSetForegroundWindow(self.child.id()) };
    }
}

impl Drop for Interface {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

impl Launch {
    fn argument(self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::Overview => "overview",
            Self::Floating => "floating",
        }
    }
}

pub fn run_interface_process(mode: Option<&str>) -> Result<()> {
    let launch = match mode {
        Some("main") => Launch::Main,
        Some("overview") => Launch::Overview,
        Some("floating") => Launch::Floating,
        _ => anyhow::bail!("界面启动参数无效"),
    };
    let (backend, resume, initial_checks) = UiBackend::connect()?;
    let mut checks = ReleaseChecks::mirror(backend.dir.clone());
    if let Some(snapshot) = initial_checks {
        checks.apply_snapshot(snapshot);
    }
    run_interface(backend, launch, resume, checks, InterfaceEvents::Remote)
}

pub fn run_desktop(backend: Backend) -> Result<()> {
    let args: Vec<_> = std::env::args_os().collect();
    let background = args.iter().any(|arg| arg == "--background");
    let migration_ready = args
        .iter()
        .position(|arg| arg == "--migration-ready")
        .and_then(|index| args.get(index + 1))
        .cloned();
    run_host(
        backend,
        &std::env::current_exe()?,
        background,
        migration_ready,
    )
}

fn run_host(
    backend: Backend,
    executable: &Path,
    background: bool,
    mut migration_ready: Option<OsString>,
) -> Result<()> {
    let mut shell = native::start_shell(
        backend.shared.clone(),
        backend.config.clone(),
        backend.stop.clone(),
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()?;
    let mut launch = if !background {
        Some(Launch::Main)
    } else if backend.config_snapshot().floating {
        Some(Launch::Floating)
    } else {
        if let Some(path) = migration_ready.take() {
            let _ = std::fs::write(path, b"ready");
        }
        None
    };
    let mut interface: Option<Interface> = None;
    let checks = Arc::new(Mutex::new(ReleaseChecks::new(backend.dir.clone())));
    let mut next_update = Instant::now();
    let mut resume = None;
    let mut quit_deadline = None;
    runtime.block_on(async {
        loop {
            if let Some(running) = &mut interface {
                if running.session.ready.load(Ordering::Acquire)
                    && let Some(path) = migration_ready.take()
                {
                    let _ = std::fs::write(path, b"ready");
                }
                if let Some(status) = running.child.try_wait()? {
                    let quit = running.session.quit.load(Ordering::Acquire);
                    let ready = running.session.ready.load(Ordering::Acquire);
                    resume = *running.session.resume.lock().unwrap();
                    let pending = running.session.take_events();
                    interface = None;
                    native::set_hotkey_recording(false);
                    backend.clear_interface_results();
                    if quit || quit_deadline.is_some() {
                        return Ok(());
                    }
                    if !ready || !status.success() {
                        backend.notice("界面进程意外退出，可从托盘重新打开。");
                        if migration_ready.is_some() {
                            anyhow::bail!("无法启动迁移后的界面");
                        }
                    }
                    for event in pending {
                        match event {
                            UiEvent::Open => launch = Some(Launch::Main),
                            UiEvent::OpenOverview => launch = Some(Launch::Overview),
                            UiEvent::ToggleFloat | UiEvent::ToggleTaskbar => {
                                toggle_shell_display(
                                    &backend,
                                    matches!(event, UiEvent::ToggleFloat),
                                );
                            }
                            _ => {}
                        }
                    }
                    if ready
                        && status.success()
                        && launch.is_none()
                        && backend.config_snapshot().floating
                    {
                        launch = Some(Launch::Floating);
                    }
                }
            }

            if let Some(request) = launch.take() {
                match Interface::start(executable, &backend, request, checks.clone(), resume) {
                    Ok(started) => interface = Some(started),
                    Err(error) => {
                        backend.notice(format!("无法启动界面：{error:#}"));
                        if migration_ready.is_some() {
                            return Err(error);
                        }
                    }
                }
            }

            if Instant::now() >= next_update {
                let frequency = backend.config.lock().unwrap().update_check_frequency;
                checks.lock().unwrap().tick(Some(frequency));
                next_update = Instant::now() + Duration::from_secs(5);
            }
            if quit_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                return Ok(());
            }

            let interval = if interface.is_some() {
                Duration::from_millis(100)
            } else {
                next_update.saturating_duration_since(Instant::now())
            };
            let event = tokio::select! {
                event = shell.recv() => event.context("Windows 托盘与任务栏集成已停止")?,
                _ = tokio::time::sleep(interval) => continue,
            };
            match event {
                UiEvent::Open | UiEvent::OpenOverview => {
                    if let Some(running) = &interface {
                        running.allow_foreground();
                        running.session.post(event);
                    } else {
                        launch = Some(if matches!(event, UiEvent::OpenOverview) {
                            Launch::Overview
                        } else {
                            Launch::Main
                        });
                    }
                }
                UiEvent::ToggleFloat | UiEvent::ToggleTaskbar => {
                    let floating = matches!(event, UiEvent::ToggleFloat);
                    if let Some(running) = &interface {
                        running.session.post(event);
                    } else {
                        toggle_shell_display(&backend, floating);
                        if floating && backend.config_snapshot().floating {
                            launch = Some(Launch::Floating);
                        }
                    }
                }
                UiEvent::Quit => {
                    let Some(running) = &interface else {
                        return Ok(());
                    };
                    running.session.post(event);
                    quit_deadline.get_or_insert_with(|| Instant::now() + Duration::from_secs(15));
                }
                UiEvent::HotkeyApplied { .. } | UiEvent::HotkeyRecorded(_) => {
                    if let Some(running) = &interface {
                        running.session.post(event);
                    }
                }
            }
        }
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::Config, native::wide, release_update::CheckFrequency};
    use std::{
        os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
        ptr::null_mut,
        thread,
    };
    use windows_sys::Win32::{
        Foundation::{HWND, WAIT_OBJECT_0, WAIT_TIMEOUT},
        System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
            WaitForSingleObject,
        },
        UI::WindowsAndMessaging::{
            FindWindowExW, GetWindowThreadProcessId, HWND_MESSAGE, IsWindow, PostMessageW,
            WM_CLOSE, WM_COMMAND,
        },
    };

    fn windows(parent: HWND, class: Option<&str>, title: Option<&str>) -> Vec<(HWND, u32)> {
        let class = class.map(wide);
        let title = title.map(wide);
        let mut window = null_mut();
        let mut found = Vec::new();
        loop {
            unsafe {
                window = FindWindowExW(
                    parent,
                    window,
                    class.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
                    title.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
                );
                if window.is_null() {
                    return found;
                }
                let mut pid = 0;
                GetWindowThreadProcessId(window, &mut pid);
                found.push((window, pid));
            }
        }
    }

    fn child_dashboard(parent_pid: u32) -> Option<(HWND, u32, OwnedHandle)> {
        use ntapi::ntpsapi::{
            NtQueryInformationProcess, PROCESS_BASIC_INFORMATION, ProcessBasicInformation,
        };
        windows(null_mut(), None, Some("Mihomo Traffic"))
            .into_iter()
            .filter(|(hwnd, _)| native::visible(*hwnd as isize))
            .find_map(|(hwnd, pid)| {
                let process = unsafe {
                    OpenProcess(
                        PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
                        0,
                        pid,
                    )
                };
                if process.is_null() {
                    return None;
                }
                let process = unsafe { OwnedHandle::from_raw_handle(process) };
                let mut info: PROCESS_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
                let status = unsafe {
                    NtQueryInformationProcess(
                        process.as_raw_handle().cast(),
                        ProcessBasicInformation,
                        (&mut info as *mut PROCESS_BASIC_INFORMATION).cast(),
                        std::mem::size_of_val(&info) as u32,
                        null_mut(),
                    )
                };
                (status == 0 && info.InheritedFromUniqueProcessId as usize == parent_pid as usize)
                    .then_some((hwnd, pid, process))
            })
    }

    fn wait_for<T>(mut check: impl FnMut() -> Option<T>, message: &str) -> Result<T> {
        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            if let Some(value) = check() {
                return Ok(value);
            }
            anyhow::ensure!(Instant::now() < deadline, "{message}");
            thread::sleep(Duration::from_millis(25));
        }
    }

    #[::core::prelude::v1::test]
    #[ignore = "requires an interactive Windows desktop and a built application"]
    fn gpui_child_exits_and_reopens_while_collector_runs() -> Result<()> {
        let executable = std::env::var_os("MIHOMO_DESKTOP_TEST_EXE")
            .context("run scripts/check-desktop.ps1 to build the interface executable")?;
        let dir = tempfile::tempdir()?;
        Config {
            endpoint: "http://127.0.0.1:1".into(),
            tray_visible: false,
            taskbar: false,
            floating: false,
            open_hotkey: String::new(),
            close_to_tray: true,
            update_check_frequency: CheckFrequency::Never,
            ..Config::default()
        }
        .save(dir.path())?;
        let backend = Backend::start(dir.path().into())?;
        let collector = backend.clone();
        let exercise = thread::spawn(move || -> Result<()> {
            let own_pid = std::process::id();
            let (shell, _) = wait_for(
                || {
                    windows(null_mut(), Some("MihomoTrafficShell"), None)
                        .into_iter()
                        .find(|(_, pid)| *pid == own_pid)
                },
                "the collector did not start its native shell",
            )?;
            let result = (|| {
                anyhow::ensure!(
                    windows(HWND_MESSAGE, Some("Zed::PlatformWindow"), None)
                        .iter()
                        .all(|(_, pid)| *pid != own_pid),
                    "the collector initialized GPUI"
                );
                let mut previous_pid = 0;
                let mut previous_query = 0;
                for cycle in 0..3 {
                    let floating = cycle == 2;
                    if floating {
                        anyhow::ensure!(unsafe { PostMessageW(shell, WM_COMMAND, 2, 0) } != 0);
                        wait_for(
                            || collector.config_snapshot().floating.then_some(()),
                            "the shell did not enable the floating window",
                        )?;
                    }
                    anyhow::ensure!(unsafe { PostMessageW(shell, WM_COMMAND, 1, 0) } != 0);
                    let (main, pid, process) = wait_for(
                        || child_dashboard(own_pid),
                        "the shell did not open the dashboard",
                    )?;
                    anyhow::ensure!(pid != previous_pid);
                    previous_pid = pid;
                    previous_query = wait_for(
                        || {
                            let state = collector.shared.lock().unwrap();
                            (state.report_id > previous_query && state.report.error.is_empty())
                                .then_some(state.report_id)
                        },
                        "the interface did not complete its query in the collector",
                    )?;
                    anyhow::ensure!(unsafe { PostMessageW(main, WM_CLOSE, 0, 0) } != 0);
                    if floating {
                        wait_for(
                            || (unsafe { IsWindow(main) } == 0).then_some(()),
                            "the dashboard did not close",
                        )?;
                        anyhow::ensure!(
                            unsafe { WaitForSingleObject(process.as_raw_handle(), 700) }
                                == WAIT_TIMEOUT,
                            "the interface exited with the floating window enabled"
                        );
                        anyhow::ensure!(unsafe { PostMessageW(shell, WM_COMMAND, 2, 0) } != 0);
                    }
                    anyhow::ensure!(
                        unsafe { WaitForSingleObject(process.as_raw_handle(), 8_000) }
                            == WAIT_OBJECT_0,
                        "closing to tray left the GPUI process resident"
                    );
                    anyhow::ensure!(
                        unsafe { IsWindow(shell) } != 0 && !collector.done.load(Ordering::Acquire),
                        "closing the GPUI process stopped the collector"
                    );
                }
                Ok(())
            })();
            unsafe { PostMessageW(shell, WM_COMMAND, 4, 0) };
            result
        });
        let host_result = run_host(backend.clone(), Path::new(&executable), true, None);
        let shutdown = backend.shutdown();
        host_result?;
        shutdown?;
        exercise.join().expect("desktop exercise panicked")
    }
}
