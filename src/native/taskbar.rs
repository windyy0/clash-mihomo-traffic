//! Shell message thread for the taskbar child, tray icon, and global hotkey.
use super::{dark_preference, wide};
use crate::{
    config::{Config, TaskbarPositionMode},
    model::{Shared, TaskbarGeometry},
    taskbar_layout,
    taskbar_probe::{self, Frame, Probe, Request, Snapshot},
    taskbar_text::{ContentFrame, TaskbarText},
};
use anyhow::Result;
use std::{
    path::Path,
    ptr::{null, null_mut},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicIsize, Ordering},
    },
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{Accessibility::*, HiDpi::*, WindowsAndMessaging::*},
};

static SHELL_WINDOW: AtomicIsize = AtomicIsize::new(0);
static HOTKEY_RECORDING: AtomicBool = AtomicBool::new(false);
static TASKBAR_WINDOW: AtomicIsize = AtomicIsize::new(0);
static LAYOUT_QUEUED: AtomicBool = AtomicBool::new(false);
static RESCAN: AtomicBool = AtomicBool::new(false);
static EVENT_QUEUED: AtomicBool = AtomicBool::new(false);
static TASKBAR_WATCHING: AtomicBool = AtomicBool::new(false);
// HWNDs survive here until their destroy/detach event is delivered: IsChild
// alone cannot identify a child after its native window has already gone away.
static TRACKED_WINDOWS: Mutex<Vec<isize>> = Mutex::new(Vec::new());
const STABLE_PROBE_MS: u32 = 60_000;
const RECOVERY_PROBE_MS: u32 = 2_000;
#[cfg(test)]
#[derive(Default)]
struct ProbeActivity {
    poll_ms: u32,
    requests: usize,
    events: Vec<(u32, isize)>,
}
#[cfg(test)]
static PROBE_ACTIVITY: Mutex<ProbeActivity> = Mutex::new(ProbeActivity {
    poll_ms: 0,
    requests: 0,
    events: Vec::new(),
});
/// Wake the owning message thread directly, independently of sampling or disk I/O.
pub fn request_taskbar_update(rescan: bool) {
    if rescan {
        RESCAN.store(true, Ordering::Release);
    }
    let window = SHELL_WINDOW.load(Ordering::Acquire);
    if window != 0
        && !LAYOUT_QUEUED.swap(true, Ordering::AcqRel)
        && unsafe { PostMessageW(window as _, WM_APP + 2, 0, 0) } == 0
    {
        LAYOUT_QUEUED.store(false, Ordering::Release);
    }
}
pub fn adjustment_keys() -> (bool, bool) {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
    unsafe {
        (
            GetAsyncKeyState(VK_LBUTTON as i32) < 0,
            GetAsyncKeyState(VK_SHIFT as i32) < 0,
        )
    }
}
unsafe extern "system" fn taskbar_event(
    _: HWINEVENTHOOK,
    event: u32,
    window: HWND,
    object: i32,
    _: i32,
    _: u32,
    _: u32,
) {
    if !TASKBAR_WATCHING.load(Ordering::Acquire)
        || !matches!(
            event,
            EVENT_OBJECT_CREATE
                | EVENT_OBJECT_DESTROY
                | EVENT_OBJECT_SHOW
                | EVENT_OBJECT_HIDE
                | EVENT_OBJECT_REORDER
                | EVENT_OBJECT_LOCATIONCHANGE
                | EVENT_OBJECT_PARENTCHANGE
        )
        || (object != OBJID_WINDOW && object != OBJID_CLIENT)
    {
        return;
    }
    let parent = TASKBAR_WINDOW.load(Ordering::Acquire) as HWND;
    if window.is_null() || parent.is_null() {
        return;
    }
    let related = window == parent || IsChild(parent, window) != 0;
    {
        let mut tracked = TRACKED_WINDOWS.lock().unwrap();
        let known = tracked.iter().position(|&handle| handle == window as isize);
        if !related && known.is_none() {
            return;
        }
        if (event == EVENT_OBJECT_DESTROY && object == OBJID_WINDOW)
            || (event == EVENT_OBJECT_PARENTCHANGE && !related)
        {
            if let Some(index) = known {
                tracked.swap_remove(index);
            }
        } else if related && known.is_none() && tracked.len() < 2000 {
            tracked.push(window as isize);
        }
    }
    #[cfg(test)]
    {
        let mut activity = PROBE_ACTIVITY.lock().unwrap();
        if activity.events.len() < 2000 {
            activity.events.push((event, window as isize));
        }
    }
    if !EVENT_QUEUED.swap(true, Ordering::AcqRel)
        && PostMessageW(SHELL_WINDOW.load(Ordering::Acquire) as _, WM_APP + 3, 0, 0) == 0
    {
        EVENT_QUEUED.store(false, Ordering::Release);
    }
}

/// Use the tray's normal exit path, including pending configuration and data saves.
pub fn request_app_exit() -> Result<()> {
    request_matching_app_exit(None)
}

pub(crate) fn request_matching_app_exit(executable: Option<&Path>) -> Result<()> {
    unsafe {
        let shell = FindWindowW(
            wide("MihomoTrafficShell").as_ptr(),
            wide("Mihomo Traffic Shell").as_ptr(),
        );
        if !shell.is_null() {
            if let Some(executable) = executable {
                let mut pid = 0;
                GetWindowThreadProcessId(shell, &mut pid);
                let running = super::process::process_executable(pid)
                    .and_then(|path| std::fs::canonicalize(path).ok());
                anyhow::ensure!(
                    running.as_deref() == Some(executable),
                    "当前运行的是另一份 Mihomo Traffic，无法更新此便携版。"
                );
            }
            anyhow::ensure!(
                PostMessageW(shell, WM_COMMAND, 4, 0) != 0,
                "无法请求程序正常退出"
            );
        }
    }
    Ok(())
}

pub fn request_app_open() {
    unsafe {
        let mut shell = SHELL_WINDOW.load(Ordering::Acquire) as HWND;
        if shell.is_null() {
            shell = FindWindowW(
                wide("MihomoTrafficShell").as_ptr(),
                wide("Mihomo Traffic Shell").as_ptr(),
            );
        }
        if !shell.is_null() {
            let mut pid = 0;
            GetWindowThreadProcessId(shell, &mut pid);
            if pid != 0 {
                AllowSetForegroundWindow(pid);
            }
            PostMessageW(shell, WM_COMMAND, 1, 0);
        }
    }
}

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub enum UiEvent {
    Open,
    OpenOverview,
    ToggleFloat,
    ToggleTaskbar,
    Quit,
    HotkeyApplied { request: u32, error: u32 },
    HotkeyRecorded(crate::hotkey::Hotkey),
}
struct ShellState {
    shared: Arc<Mutex<Shared>>,
    config: Arc<Mutex<Config>>,
    stop: Arc<AtomicBool>,
    events: tokio::sync::mpsc::UnboundedSender<UiEvent>,
    dock: HWND,
    parent: HWND,
    owner: HWND,
    taskbar_created: u32,
    text: Option<TaskbarText>,
    dpi: u32,
    probe: Probe,
    scan_revision: u64,
    scan_pending: Option<Instant>,
    snapshot: Option<Snapshot>,
    auto_x: Option<i32>,
    auto_key: Option<(bool, bool, Frame)>,
    reveal: Option<(i32, Instant)>,
    hook: HWINEVENTHOOK,
    probe_poll_ms: u32,
    tray: crate::tray::Tray,
    hotkey: Option<crate::hotkey::Registration>,
    hotkey_config: Option<String>,
}
unsafe fn set_probe_poll(s: &mut ShellState, interval: u32) {
    // Resetting SetTimer on every one-second paint would postpone it forever.
    if s.probe_poll_ms != interval {
        if interval == 0 {
            KillTimer(s.owner, 4);
        } else {
            SetTimer(s.owner, 4, interval, None);
        }
        s.probe_poll_ms = interval;
        #[cfg(test)]
        {
            PROBE_ACTIVITY.lock().unwrap().poll_ms = interval;
        }
    }
}
unsafe fn stop_taskbar_watch(s: &mut ShellState) {
    TASKBAR_WATCHING.store(false, Ordering::Release);
    if !s.hook.is_null() {
        UnhookWinEvent(s.hook);
        s.hook = null_mut();
    }
    TRACKED_WINDOWS.lock().unwrap().clear();
    EVENT_QUEUED.store(false, Ordering::Release);
    KillTimer(s.owner, 2);
}
unsafe fn hide_dock(s: &mut ShellState) {
    ShowWindow(s.dock, SW_HIDE);
    if s.probe_poll_ms != 0 {
        set_probe_poll(s, RECOVERY_PROBE_MS);
    }
}
pub fn taskbar_dpi() -> u32 {
    unsafe {
        let taskbar = FindWindowW(wide("Shell_TrayWnd").as_ptr(), null());
        GetDpiForWindow(taskbar).max(96)
    }
}

pub fn start_shell(
    shared: Arc<Mutex<Shared>>,
    config: Arc<Mutex<Config>>,
    stop: Arc<AtomicBool>,
) -> tokio::sync::mpsc::UnboundedReceiver<UiEvent> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    std::thread::Builder::new()
        .name("windows-shell".into())
        .spawn(move || unsafe {
            SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let instance = GetModuleHandleW(null());
            let class = wide("MihomoTrafficShell");
            let wc = WNDCLASSW {
                lpfnWndProc: Some(shell_proc),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                hCursor: LoadCursorW(null_mut(), IDC_ARROW),
                ..std::mem::zeroed()
            };
            RegisterClassW(&wc);
            let mut state = Box::new(ShellState {
                shared,
                config,
                stop,
                events: tx,
                dock: null_mut(),
                parent: null_mut(),
                owner: null_mut(),
                taskbar_created: RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()),
                text: None,
                dpi: 96,
                probe: Probe::start(),
                scan_revision: 0,
                scan_pending: None,
                snapshot: None,
                auto_x: None,
                auto_key: None,
                reveal: None,
                hook: null_mut(),
                probe_poll_ms: 0,
                tray: crate::tray::Tray::default(),
                hotkey: None,
                hotkey_config: None,
            });
            let raw = &mut *state as *mut ShellState;
            let h = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                class.as_ptr(),
                wide("Mihomo Traffic Shell").as_ptr(),
                WS_POPUP,
                0,
                0,
                1,
                1,
                null_mut(),
                null_mut(),
                instance,
                raw as _,
            );
            if h.is_null() {
                state.shared.lock().unwrap().taskbar_status = "任务栏窗口创建失败".into();
                return;
            }
            state.owner = h;
            state.hotkey = Some(crate::hotkey::Registration::new(h));
            SHELL_WINDOW.store(h as isize, Ordering::Release);
            sync_tray(&mut state);
            sync_hotkey(&mut state);
            SetTimer(h, 1, 1000, None);
            dock_update(&mut state);
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            state.tray.remove(h);
            state.hotkey = None;
            HOTKEY_RECORDING.store(false, Ordering::Release);
            SHELL_WINDOW.store(0, Ordering::Release);
            TASKBAR_WINDOW.store(0, Ordering::Release);
            stop_taskbar_watch(&mut state);
            if !state.dock.is_null() {
                DestroyWindow(state.dock);
            }
            DestroyWindow(h);
        })
        .expect("创建 Windows 集成线程失败");
    rx
}
unsafe fn sync_tray(s: &mut ShellState) {
    let config = s.config.lock().unwrap().clone();
    s.tray.sync(
        s.owner,
        config.tray_visible,
        config.tray_icon_style,
        dark_preference("SystemUsesLightTheme"),
        GetSystemMetricsForDpi(SM_CXSMICON, taskbar_dpi()).max(16),
    );
}

pub fn apply_hotkey(request: u32, hotkey: crate::hotkey::Hotkey) -> Result<()> {
    let owner = SHELL_WINDOW.load(Ordering::Acquire);
    anyhow::ensure!(owner != 0, "快捷键服务尚未就绪，请稍后重试");
    anyhow::ensure!(
        unsafe {
            PostMessageW(
                owner as _,
                crate::hotkey::APPLY_MESSAGE,
                hotkey.packed(),
                request as isize,
            )
        } != 0,
        "无法应用快捷键，请稍后重试"
    );
    Ok(())
}

pub fn set_hotkey_recording(recording: bool) {
    HOTKEY_RECORDING.store(recording, Ordering::Release);
}

fn publish_hotkey(s: &ShellState, error: String) {
    let mut shared = s.shared.lock().unwrap();
    shared.open_hotkey_active = s
        .hotkey
        .as_ref()
        .and_then(|key| key.active())
        .map(|key| key.to_string())
        .unwrap_or_default();
    shared.open_hotkey_error = error;
}

fn sync_hotkey(s: &mut ShellState) {
    let configured = s.config.lock().unwrap().open_hotkey.clone();
    if s.hotkey_config.as_ref() == Some(&configured) {
        return;
    }
    s.hotkey_config = Some(configured.clone());
    let error = match crate::hotkey::Hotkey::parse(&configured) {
        Ok(key) => s
            .hotkey
            .as_mut()
            .unwrap()
            .replace(key)
            .err()
            .map(crate::hotkey::failure)
            .unwrap_or_default(),
        Err(error) => error.to_string(),
    };
    publish_hotkey(s, error);
}
unsafe fn dock_update(s: &mut ShellState) {
    let config = s.config.lock().unwrap().clone();
    let automatic =
        config.taskbar && config.taskbar_position_mode == TaskbarPositionMode::AutoAvoid;
    if !automatic {
        set_probe_poll(s, 0);
        stop_taskbar_watch(s);
        KillTimer(s.owner, 3);
    } else if s.probe_poll_ms == 0 {
        set_probe_poll(s, RECOVERY_PROBE_MS);
    }
    if !config.taskbar {
        if !s.dock.is_null() {
            hide_dock(s);
        }
        s.shared.lock().unwrap().taskbar_status = "未启用".into();
        s.snapshot = None;
        s.auto_x = None;
        s.auto_key = None;
        s.reveal = None;
        KillTimer(s.owner, 3);
        return;
    }
    let parent = FindWindowW(wide("Shell_TrayWnd").as_ptr(), null());
    if parent.is_null() {
        s.shared.lock().unwrap().taskbar_geometry.ready = false;
        if !s.dock.is_null() {
            hide_dock(s);
        }
        s.shared.lock().unwrap().taskbar_status = "等待 Explorer 任务栏".into();
        return;
    }
    if s.parent != parent || s.dock.is_null() || IsWindow(s.dock) == 0 {
        if !s.dock.is_null() && IsWindow(s.dock) != 0 {
            DestroyWindow(s.dock);
        }
        stop_taskbar_watch(s);
        s.parent = parent;
        TASKBAR_WINDOW.store(parent as isize, Ordering::Release);
        s.snapshot = None;
        s.scan_revision += 1;
        s.scan_pending = None;
        s.auto_x = None;
        s.auto_key = None;
        s.dock = CreateWindowExW(
            // Give text its own redirected surface above Explorer's XAML layer.
            // An ordinary visible child can be present but never composited.
            WS_EX_NOACTIVATE | WS_EX_LAYERED,
            wide("MihomoTrafficShell").as_ptr(),
            wide("Mihomo Traffic Taskbar").as_ptr(),
            WS_CHILD | WS_CLIPSIBLINGS,
            0,
            0,
            130,
            44,
            parent,
            null_mut(),
            GetModuleHandleW(null()),
            s as *mut _ as _,
        );
        if s.dock.is_null() {
            s.shared.lock().unwrap().taskbar_status =
                format!("挂接失败：{}", std::io::Error::last_os_error());
            return;
        }
    }
    if automatic && s.hook.is_null() {
        // Only seed native identity here. UIA and all occupied-boundary reads
        // remain on the MTA worker. Cache hidden children for later show events.
        *TRACKED_WINDOWS.lock().unwrap() =
            taskbar_probe::child_windows(parent as isize).unwrap_or_default();
        s.hook = SetWinEventHook(
            EVENT_OBJECT_CREATE,
            EVENT_OBJECT_PARENTCHANGE,
            null_mut(),
            Some(taskbar_event),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        );
        TASKBAR_WATCHING.store(!s.hook.is_null(), Ordering::Release);
        if s.hook.is_null() {
            hide_dock(s);
            s.shared.lock().unwrap().taskbar_status =
                "任务栏事件监听失败，已隐藏；请重新检测或使用手动模式".into();
            return;
        }
    }
    let dpi = GetDpiForWindow(parent).max(96);
    let scale = dpi as f32 / 96.;
    s.dpi = dpi;
    if s.text.is_none() {
        match TaskbarText::new() {
            Ok(text) => s.text = Some(text),
            Err(error) => {
                hide_dock(s);
                s.shared.lock().unwrap().taskbar_status = format!("任务栏文字初始化失败：{error}");
                return;
            }
        }
    }
    let mut r = RECT::default();
    if GetClientRect(parent, &mut r) == 0 {
        hide_dock(s);
        let mut shared = s.shared.lock().unwrap();
        shared.taskbar_geometry.ready = false;
        shared.taskbar_status = "无法读取任务栏尺寸，已隐藏；可稍后重新检测".into();
        return;
    }
    let data = {
        let data = s.shared.lock().unwrap();
        Shared {
            connected: data.connected,
            down: data.down,
            up: data.up,
            today: data.today,
            month: data.month,
            cumulative: data.cumulative,
            system: data.system.clone(),
            ..Shared::default()
        }
    };
    let frame = s
        .text
        .as_mut()
        .unwrap()
        .frame(&config.taskbar_content, &data);
    let frame = match frame {
        Ok(frame) => frame,
        Err(error) => {
            s.text = None;
            hide_dock(s);
            s.shared.lock().unwrap().taskbar_geometry.ready = false;
            s.shared.lock().unwrap().taskbar_status = format!("任务栏文字测量失败：{error}");
            return;
        }
    };
    let w = (frame.width * scale).ceil() as i32;
    if w > r.right {
        hide_dock(s);
        let mut shared = s.shared.lock().unwrap();
        shared.taskbar_geometry.ready = false;
        shared.taskbar_status = "无法容纳当前布局，请改为双行、减少指标或缩短文案".into();
        return;
    }
    let tray_area = FindWindowExW(parent, null_mut(), wide("TrayNotifyWnd").as_ptr(), null());
    let mut tr = RECT::default();
    let right = if !tray_area.is_null() {
        GetWindowRect(tray_area, &mut tr);
        let mut p = POINT {
            x: tr.left,
            y: tr.top,
        };
        ScreenToClient(parent, &mut p);
        p.x
    } else {
        r.right - (200. * scale) as i32
    };
    let margin = (8. * scale).round() as i32;
    let max_x = (r.right - w).max(0);
    let mut geometry = TaskbarGeometry {
        ready: true,
        x: 0,
        max_x,
        tray_start: right,
        widget_width: w,
        margin,
    };
    if r.bottom > r.right {
        s.shared.lock().unwrap().taskbar_geometry.ready = false;
        s.shared.lock().unwrap().taskbar_status = "暂不支持竖向任务栏，请使用悬浮窗".into();
        hide_dock(s);
        return;
    }
    let mut status = "手动调整";
    let x = if config.taskbar_position_mode == TaskbarPositionMode::Manual {
        s.auto_x = None;
        s.auto_key = None;
        s.reveal = None;
        taskbar_layout::manual(
            taskbar_layout::anchor(config.taskbar_left, right, w, margin),
            config.taskbar_offset,
            max_x,
        )
    } else {
        let Some(frame) = taskbar_probe::frame(parent as isize) else {
            hide_dock(s);
            let mut shared = s.shared.lock().unwrap();
            shared.taskbar_geometry.ready = false;
            shared.taskbar_status = "无法读取任务栏边界，已隐藏；请重新检测或使用手动模式".into();
            return;
        };
        let key = (
            config.taskbar_auto_prefer_left,
            config.taskbar_horizontal,
            frame,
        );
        if s.auto_key != Some(key) {
            s.auto_key = Some(key);
            s.auto_x = None;
            s.reveal = None;
            s.snapshot = None;
            s.scan_pending = None;
            s.scan_revision += 1;
        }
        while let Ok(snapshot) = s.probe.results.try_recv() {
            if snapshot.request.revision == s.scan_revision {
                s.scan_pending = None;
                if snapshot.request.frame == frame {
                    s.snapshot = Some(snapshot);
                }
            }
        }
        let rescan = RESCAN.load(Ordering::Acquire);
        if s.scan_pending.is_none() && (rescan || s.snapshot.is_none()) {
            let revision = s.scan_revision + 1;
            if s.probe
                .requests
                .try_send(Request { frame, revision })
                .is_ok()
            {
                RESCAN.store(false, Ordering::Release);
                s.scan_revision = revision;
                s.scan_pending = Some(Instant::now());
                #[cfg(test)]
                {
                    PROBE_ACTIVITY.lock().unwrap().requests += 1;
                }
            }
        }
        let occupied = s
            .snapshot
            .as_ref()
            .filter(|snapshot| snapshot.request.frame == frame)
            .map(|snapshot| &snapshot.occupied);
        let candidate = match occupied {
            Some(Ok(occupied))
                if !s
                    .scan_pending
                    .is_some_and(|at| at.elapsed() > Duration::from_secs(3)) =>
            {
                let preferred =
                    taskbar_layout::anchor(config.taskbar_auto_prefer_left, right, w, margin);
                taskbar_layout::choose(r.right, w, margin, occupied, preferred)
            }
            _ => None,
        };
        if let Some(x) = candidate {
            if s.auto_x.is_none() {
                let reveal = s.reveal.get_or_insert((x, Instant::now()));
                if reveal.0 != x {
                    *reveal = (x, Instant::now());
                }
                let ready_at = reveal.1 + Duration::from_millis(300);
                let verified = s
                    .snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.sampled_at >= ready_at);
                if !verified {
                    if Instant::now() >= ready_at && s.scan_pending.is_none() {
                        RESCAN.store(true, Ordering::Release);
                    }
                    SetTimer(s.owner, 3, 50, None);
                    hide_dock(s);
                    s.shared.lock().unwrap().taskbar_status = "空位已找到，等待布局稳定…".into();
                    s.shared.lock().unwrap().taskbar_geometry = geometry;
                    return;
                }
            }
            s.auto_x = Some(x);
            s.reveal = None;
            KillTimer(s.owner, 3);
            status = "自动避让";
            x
        } else {
            s.auto_x = None;
            s.reveal = None;
            KillTimer(s.owner, 3);
            let message = match occupied {
                Some(Err(error)) => format!("控件识别失败，已隐藏。请切换手动调整：{error}"),
                Some(Ok(_))
                    if s.scan_pending
                        .is_some_and(|at| at.elapsed() > Duration::from_secs(3)) =>
                {
                    "控件识别超时，已隐藏。请切换手动调整".into()
                }
                Some(Ok(_)) => "没有足够空位，已临时隐藏；空位恢复后自动显示".into(),
                None if s
                    .scan_pending
                    .is_some_and(|at| at.elapsed() > Duration::from_secs(3)) =>
                {
                    "控件识别超时，已隐藏。请切换手动调整".into()
                }
                None => "正在检测任务栏控件…".into(),
            };
            hide_dock(s);
            let mut shared = s.shared.lock().unwrap();
            shared.taskbar_status = message;
            shared.taskbar_geometry = geometry;
            return;
        }
    };
    geometry.x = x;
    s.shared.lock().unwrap().taskbar_geometry = geometry;
    let mut previous = RECT::default();
    let positioned = GetWindowRect(s.dock, &mut previous) != 0;
    let mut origin = POINT {
        x: previous.left,
        y: previous.top,
    };
    let positioned = positioned && ScreenToClient(parent, &mut origin) != 0;
    let visible = IsWindowVisible(s.dock) != 0;
    let needs_position = !positioned
        || !visible
        || origin.x != x
        || origin.y != 0
        || previous.right - previous.left != w
        || previous.bottom - previous.top != r.bottom;
    // Reasserting HWND_TOP / ShowWindow for an unchanged visible child creates
    // location/reorder events. Two taskbar widgets can otherwise make each
    // other rescan continuously even though their rectangles are stable.
    if needs_position && SetWindowPos(s.dock, HWND_TOP, x, 0, w, r.bottom, SWP_NOACTIVATE) == 0 {
        let error = std::io::Error::last_os_error();
        hide_dock(s);
        s.shared.lock().unwrap().taskbar_status = format!("任务栏定位失败：{error}");
        return;
    }
    if let Err(error) = paint_dock(s.dock, s, &frame) {
        s.text = None;
        hide_dock(s);
        s.shared.lock().unwrap().taskbar_status = format!("任务栏绘制失败：{error}");
        return;
    }
    if !visible {
        ShowWindow(s.dock, SW_SHOWNOACTIVATE);
    }
    if automatic {
        set_probe_poll(s, STABLE_PROBE_MS);
    }
    s.shared.lock().unwrap().taskbar_status = format!("{status} · 距左侧 {x} px");
}
unsafe fn paint_dock(h: HWND, s: &ShellState, frame: &ContentFrame) -> std::io::Result<()> {
    let mut r = RECT::default();
    if GetClientRect(h, &mut r) == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let width = r.right - r.left;
    let height = r.bottom - r.top;
    if width <= 0 || height <= 0 {
        return Err(std::io::Error::other("任务栏尺寸无效"));
    }
    let dark = dark_preference("SystemUsesLightTheme");
    let config = s.config.lock().unwrap().clone();
    let palette = crate::theme::Palette::new(config.color_theme, dark);
    let dc = CreateCompatibleDC(null_mut());
    if dc.is_null() {
        return Err(std::io::Error::last_os_error());
    }
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits = null_mut();
    let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
    if bitmap.is_null() {
        let error = std::io::Error::last_os_error();
        DeleteDC(dc);
        return Err(error);
    }
    let old_bitmap = SelectObject(dc, bitmap);
    let result = (|| {
        if old_bitmap.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let byte_len = width as usize * height as usize * 4;
        std::ptr::write_bytes(bits.cast::<u8>(), 0, byte_len);
        let renderer = s
            .text
            .as_ref()
            .ok_or_else(|| std::io::Error::other("任务栏文字尚未初始化"))?;
        renderer
            .draw_frame(dc, r, s.dpi, frame, palette.text)
            .map_err(std::io::Error::other)?;
        if GdiFlush() == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let size = SIZE {
            cx: width,
            cy: height,
        };
        let origin = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        if UpdateLayeredWindow(
            h,
            null_mut(),
            null(),
            &size,
            dc,
            &origin,
            0,
            &blend,
            ULW_ALPHA,
        ) == 0
        {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    })();
    if !old_bitmap.is_null() {
        SelectObject(dc, old_bitmap);
    }
    DeleteObject(bitmap);
    DeleteDC(dc);
    result
}
unsafe extern "system" fn shell_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        let cs = &*(lp as *const CREATESTRUCTW);
        SetWindowLongPtrW(h, GWLP_USERDATA, cs.lpCreateParams as isize);
        return DefWindowProcW(h, msg, wp, lp);
    }
    let ptr = GetWindowLongPtrW(h, GWLP_USERDATA) as *mut ShellState;
    if ptr.is_null() {
        return DefWindowProcW(h, msg, wp, lp);
    }
    let s = &mut *ptr;
    if msg == s.taskbar_created {
        s.tray.explorer_restarted();
        sync_tray(s);
        s.snapshot = None;
        s.scan_pending = None;
        s.scan_revision += 1;
        RESCAN.store(true, Ordering::Release);
        dock_update(s);
        return 0;
    }
    match msg {
        m if m == WM_APP + 2 => {
            LAYOUT_QUEUED.store(false, Ordering::Release);
            sync_tray(s);
            sync_hotkey(s);
            dock_update(s);
            0
        }
        m if m == WM_APP + 3 => {
            EVENT_QUEUED.store(false, Ordering::Release);
            if TASKBAR_WATCHING.load(Ordering::Acquire) {
                SetTimer(s.owner, 2, 100, None);
            }
            0
        }
        WM_TIMER => {
            if wp == 2 {
                KillTimer(s.owner, 2);
                RESCAN.store(true, Ordering::Release);
            }
            if wp == 4 {
                RESCAN.store(true, Ordering::Release);
            }
            if s.stop.load(Ordering::Relaxed) {
                PostQuitMessage(0);
            } else {
                if wp == 1 {
                    sync_tray(s);
                    sync_hotkey(s);
                }
                dock_update(s);
            }
            0
        }
        WM_PAINT if h == s.dock => {
            // UpdateLayeredWindow owns the pixels; never fill an opaque GDI
            // background when Explorer asks this child to repaint.
            ValidateRect(h, null());
            0
        }
        WM_ERASEBKGND => 1,
        WM_LBUTTONDBLCLK | WM_LBUTTONUP => {
            let _ = s.events.send(UiEvent::Open);
            0
        }
        WM_RBUTTONUP => {
            popup(h, s);
            0
        }
        WM_SETTINGCHANGE | WM_THEMECHANGED => {
            sync_tray(s);
            DefWindowProcW(h, msg, wp, lp)
        }
        crate::tray::CALLBACK => {
            match crate::tray::activation(lp) {
                Some(crate::tray::Activation::Open) => {
                    let _ = s.events.send(UiEvent::OpenOverview);
                }
                Some(crate::tray::Activation::Menu) => popup(h, s),
                None => {}
            }
            0
        }
        crate::hotkey::APPLY_MESSAGE => {
            let error = crate::hotkey::Hotkey::from_packed(wp)
                .ok_or(ERROR_INVALID_PARAMETER)
                .and_then(|key| s.hotkey.as_mut().unwrap().replace(key))
                .err()
                .unwrap_or(0);
            publish_hotkey(
                s,
                if error == 0 {
                    String::new()
                } else {
                    crate::hotkey::failure(error)
                },
            );
            let _ = s.events.send(UiEvent::HotkeyApplied {
                request: lp as u32,
                error,
            });
            0
        }
        WM_HOTKEY => {
            if s.hotkey.as_ref().is_some_and(|key| key.matches(wp, lp)) {
                let event = if HOTKEY_RECORDING.load(Ordering::Acquire) {
                    UiEvent::HotkeyRecorded(s.hotkey.as_ref().unwrap().active().unwrap())
                } else {
                    UiEvent::Open
                };
                let _ = s.events.send(event);
            }
            0
        }
        WM_COMMAND => {
            let event = match wp as u32 & 0xffff {
                1 => UiEvent::Open,
                2 => UiEvent::ToggleFloat,
                3 => UiEvent::ToggleTaskbar,
                4 => UiEvent::Quit,
                _ => return 0,
            };
            let _ = s.events.send(event);
            0
        }
        _ => DefWindowProcW(h, msg, wp, lp),
    }
}
unsafe fn popup(h: HWND, s: &ShellState) {
    let menu = CreatePopupMenu();
    let config = s.config.lock().unwrap().clone();
    for (id, label, on) in [
        (1, "打开主界面", false),
        (2, "悬浮窗", config.floating),
        (3, "任务栏信息", config.taskbar),
        (4, "退出", false),
    ] {
        AppendMenuW(
            menu,
            MF_STRING | if on { MF_CHECKED } else { 0 },
            id,
            wide(label).as_ptr(),
        );
    }
    let mut p = POINT::default();
    GetCursorPos(&mut p);
    SetForegroundWindow(h);
    TrackPopupMenu(menu, TPM_RIGHTBUTTON, p.x, p.y, 0, h, null());
    PostMessageW(h, WM_NULL, 0, 0);
    DestroyMenu(menu);
}

#[cfg(test)]
mod positioning_tests;
