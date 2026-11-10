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
        let session = Arc::new(Session::new(resume)?);
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
            Self::Prewarm => "prewarm",
        }
    }
}

pub fn run_interface_process(mode: Option<&str>) -> Result<()> {
    let launch = match mode {
        Some("main") => Launch::Main,
        Some("overview") => Launch::Overview,
        Some("floating") => Launch::Floating,
        Some("prewarm") => Launch::Prewarm,
        _ => anyhow::bail!("界面启动参数无效"),
    };
    let (backend, resume, initial_checks, wake) = UiBackend::connect()?;
    let mut checks = ReleaseChecks::mirror(backend.dir.clone());
    if let Some(snapshot) = initial_checks {
        checks.apply_snapshot(snapshot);
    }
    run_interface(
        backend,
        launch,
        resume,
        checks,
        InterfaceEvents::Remote {
            wake: Arc::new(wake),
            first_poll: true,
            trimmed: false,
        },
    )
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
    let config = backend.config_snapshot();
    if background
        && !config.floating
        && let Some(path) = migration_ready.take()
    {
        let _ = std::fs::write(path, b"ready");
    }
    let mut launch = if !background {
        Some(Launch::Main)
    } else if config.close_to_tray {
        Some(Launch::Prewarm)
    } else if config.floating {
        Some(Launch::Floating)
    } else {
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
        Foundation::{HWND, WAIT_OBJECT_0},
        System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
            WaitForSingleObject,
        },
        UI::WindowsAndMessaging::{
            FindWindowExW, GetWindowThreadProcessId, HWND_MESSAGE, IsIconic, IsWindow,
            IsWindowVisible, PostMessageW, SW_MINIMIZE, ShowWindowAsync, WM_CLOSE, WM_COMMAND,
            WM_LBUTTONUP,
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

    fn child_dashboard(parent_pid: u32, visible: bool) -> Option<(HWND, u32, OwnedHandle)> {
        use ntapi::ntpsapi::{
            NtQueryInformationProcess, PROCESS_BASIC_INFORMATION, ProcessBasicInformation,
        };
        windows(null_mut(), None, Some("Mihomo Traffic"))
            .into_iter()
            .filter(|(hwnd, _)| (unsafe { IsWindowVisible(*hwnd) } != 0) == visible)
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
    fn gpui_dashboard_closes_app_when_background_collection_is_disabled() -> Result<()> {
        let executable = std::env::var_os("MIHOMO_DESKTOP_TEST_EXE")
            .context("run scripts/check-desktop.ps1 to build the interface executable")?;
        let dir = tempfile::tempdir()?;
        Config {
            endpoint: "http://127.0.0.1:1".into(),
            tray_visible: false,
            taskbar: false,
            floating: false,
            open_hotkey: String::new(),
            close_to_tray: false,
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
                anyhow::ensure!(unsafe { PostMessageW(shell, WM_COMMAND, 1, 0) } != 0);
                let (main, _, process) = wait_for(
                    || child_dashboard(own_pid, true),
                    "the shell did not open the dashboard",
                )?;
                wait_for(
                    || {
                        let state = collector.shared.lock().unwrap();
                        (state.report_id > 0 && state.report.error.is_empty()).then_some(())
                    },
                    "the interface did not complete its query in the collector",
                )?;
                anyhow::ensure!(unsafe { ShowWindowAsync(main, SW_MINIMIZE) } != 0);
                wait_for(
                    || (unsafe { IsIconic(main) } != 0).then_some(()),
                    "the dashboard did not minimize",
                )?;
                anyhow::ensure!(
                    unsafe { PostMessageW(shell, crate::tray::CALLBACK, 0, WM_LBUTTONUP as isize) }
                        != 0
                );
                wait_for(
                    || native::visible(main as isize).then_some(()),
                    "the tray did not restore the minimized dashboard",
                )?;
                anyhow::ensure!(unsafe { PostMessageW(main, WM_CLOSE, 0, 0) } != 0);
                anyhow::ensure!(
                    unsafe { WaitForSingleObject(process.as_raw_handle(), 8_000) } == WAIT_OBJECT_0,
                    "closing the dashboard did not exit the interface"
                );
                Ok(())
            })();
            if result.is_err() {
                unsafe { PostMessageW(shell, WM_COMMAND, 4, 0) };
            }
            result
        });
        let host_result = run_host(backend.clone(), Path::new(&executable), true, None);
        let shutdown = backend.shutdown();
        host_result?;
        shutdown?;
        exercise.join().expect("desktop exercise panicked")
    }

    #[::core::prelude::v1::test]
    #[ignore = "requires an interactive Windows desktop and a built application"]
    fn gpui_tray_prewarms_and_reuses_hidden_dashboard_by_default() -> Result<()> {
        let executable = std::env::var_os("MIHOMO_DESKTOP_TEST_EXE")
            .context("run scripts/check-desktop.ps1 to build the interface executable")?;
        let dir = tempfile::tempdir()?;
        Config {
            endpoint: "http://127.0.0.1:1".into(),
            tray_visible: false,
            taskbar: false,
            floating: false,
            open_hotkey: String::new(),
            update_check_frequency: CheckFrequency::Never,
            ..Config::default()
        }
        .save(dir.path())?;
        let path = dir.path().join("config.toml");
        let config = std::fs::read_to_string(&path)?;
        std::fs::write(path, format!("fast_tray_open = false\n{config}"))?;
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
                let (main, pid, process) = wait_for(
                    || child_dashboard(own_pid, false),
                    "the dashboard was not prewarmed in the background",
                )?;
                for cycle in 0..3 {
                    thread::sleep(if cycle == 2 {
                        Duration::from_secs(6)
                    } else {
                        Duration::from_millis(750)
                    });
                    let opened = Instant::now();
                    anyhow::ensure!(
                        unsafe {
                            PostMessageW(shell, crate::tray::CALLBACK, 0, WM_LBUTTONUP as isize)
                        } != 0
                    );
                    let (reopened, reopened_pid, _) = wait_for(
                        || child_dashboard(own_pid, true),
                        "the tray did not show the prewarmed dashboard",
                    )?;
                    anyhow::ensure!(reopened == main && reopened_pid == pid);
                    println!("fast tray open {cycle}: visible in {:?}", opened.elapsed());
                    anyhow::ensure!(unsafe { PostMessageW(main, WM_CLOSE, 0, 0) } != 0);
                    wait_for(
                        || (unsafe { IsWindowVisible(main) } == 0).then_some(()),
                        "closing did not hide the dashboard",
                    )?;
                    anyhow::ensure!(unsafe { IsWindow(main) } != 0);
                }
                for enabled in [true, false, true, false] {
                    anyhow::ensure!(unsafe { PostMessageW(shell, WM_COMMAND, 2, 0) } != 0);
                    wait_for(
                        || (collector.config_snapshot().floating == enabled).then_some(()),
                        "the hidden interface did not process the floating window toggle",
                    )?;
                    wait_for(
                        || {
                            let visible = windows(null_mut(), Some("Zed::Window"), None)
                                .into_iter()
                                .any(|(hwnd, owner)| {
                                    owner == pid && hwnd != main && native::visible(hwnd as isize)
                                });
                            (visible == enabled).then_some(())
                        },
                        "the floating window did not follow the toggle",
                    )?;
                    anyhow::ensure!(unsafe { IsWindowVisible(main) } == 0);
                }
                thread::sleep(Duration::from_millis(750));
                Ok(process)
            })();
            unsafe { PostMessageW(shell, WM_COMMAND, 4, 0) };
            let process = result?;
            anyhow::ensure!(
                unsafe { WaitForSingleObject(process.as_raw_handle(), 8_000) } == WAIT_OBJECT_0,
                "quitting did not stop the idle interface process"
            );
            Ok(())
        });
        let host_result = run_host(backend.clone(), Path::new(&executable), true, None);
        let shutdown = backend.shutdown();
        host_result?;
        shutdown?;
        exercise.join().expect("desktop exercise panicked")
    }
}
