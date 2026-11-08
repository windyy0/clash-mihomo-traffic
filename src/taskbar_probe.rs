//! UI Automation lives only on this windowless MTA worker. No COM element is
//! shared with the GPUI or Shell message threads; results contain physical pixels.
use crate::taskbar_layout::Interval;
use anyhow::{Context, Result};
use std::{sync::mpsc, time::Instant};
use windows::Win32::{Foundation::HWND, System::Com::*, UI::Accessibility::*};
use windows_sys::Win32::{
    Foundation::RECT,
    UI::{HiDpi::*, WindowsAndMessaging::*},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub parent: isize,
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}
#[derive(Clone, Copy)]
pub struct Request {
    pub frame: Frame,
    pub revision: u64,
}
pub struct Snapshot {
    pub request: Request,
    pub occupied: Result<Vec<Interval>>,
    pub sampled_at: Instant,
}
pub struct Probe {
    pub requests: mpsc::SyncSender<Request>,
    pub results: mpsc::Receiver<Snapshot>,
}
impl Probe {
    pub fn start() -> Self {
        let (tx, rx) = mpsc::sync_channel::<Request>(1);
        let (result_tx, result_rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("taskbar-uia-mta".into())
            .spawn(move || {
                // Creation, use, and release of every UIA interface stay on this thread.
                let initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
                while let Ok(request) = rx.recv() {
                    let occupied = if initialized {
                        scan(request.frame, false)
                    } else {
                        Err(anyhow::anyhow!("无法初始化 UI Automation"))
                    };
                    if result_tx
                        .send(Snapshot {
                            request,
                            occupied,
                            sampled_at: Instant::now(),
                        })
                        .is_err()
                    {
                        break;
                    }
                    crate::native::request_taskbar_update(false);
                }
                if initialized {
                    unsafe {
                        CoUninitialize();
                    }
                }
            })
            .expect("创建任务栏识别线程失败");
        Self {
            requests: tx,
            results: result_rx,
        }
    }
}
pub fn frame(parent: isize) -> Option<Frame> {
    let mut r = RECT::default();
    if unsafe { GetWindowRect(parent as _, &mut r) } == 0 {
        return None;
    }
    Some(Frame {
        parent,
        left: r.left,
        top: r.top,
        right: r.right,
        bottom: r.bottom,
    })
}

struct ChildWindows {
    handles: Vec<isize>,
    truncated: bool,
}

unsafe extern "system" fn collect_child_window(
    window: windows_sys::Win32::Foundation::HWND,
    data: isize,
) -> i32 {
    let children = unsafe { &mut *(data as *mut ChildWindows) };
    if children.handles.len() >= 2000 {
        children.truncated = true;
        return 0;
    }
    children.handles.push(window as isize);
    1
}

pub(crate) fn child_windows(parent: isize) -> Result<Vec<isize>> {
    let mut children = ChildWindows {
        handles: vec![],
        truncated: false,
    };
    unsafe {
        EnumChildWindows(
            parent as _,
            Some(collect_child_window),
            &mut children as *mut ChildWindows as isize,
        );
    }
    anyhow::ensure!(!children.truncated, "任务栏原生子窗口数量异常");
    Ok(children.handles)
}

pub(crate) fn native_occupied(frame: Frame, verbose: bool) -> Result<Vec<Interval>> {
    let mut shell_process = 0;
    unsafe { GetWindowThreadProcessId(frame.parent as _, &mut shell_process) };
    anyhow::ensure!(shell_process != 0, "无法读取任务栏所属进程");
    let mut occupied = vec![];
    for handle in child_windows(frame.parent)? {
        let mut process = 0;
        unsafe { GetWindowThreadProcessId(handle as _, &mut process) };
        // UIA handles Explorer's own containers and controls. Foreign children
        // can draw into the taskbar without exposing any UIA element at all.
        if process == 0
            || process == shell_process
            || process == std::process::id()
            || unsafe { IsWindowVisible(handle as _) } == 0
        {
            continue;
        }
        let Some(bounds) = self::frame(handle) else {
            continue;
        };
        if bounds.bottom <= bounds.top || bounds.bottom <= frame.top || bounds.top >= frame.bottom {
            continue;
        }
        let interval = Interval {
            left: (bounds.left - frame.left).max(0),
            right: (bounds.right - frame.left).min(frame.right - frame.left),
        };
        if interval.right <= interval.left {
            continue;
        }
        if verbose {
            let mut class = [0u16; 256];
            let length =
                unsafe { GetClassNameW(handle as _, class.as_mut_ptr(), class.len() as i32) };
            let class = String::from_utf16_lossy(&class[..length.max(0) as usize]);
            println!(
                "native pid={process} class={class:?} rect=({},{})-({},{}) occupied={interval:?}",
                bounds.left, bounds.top, bounds.right, bounds.bottom
            );
        }
        occupied.push(interval);
    }
    Ok(occupied)
}

fn scan(frame: Frame, verbose: bool) -> Result<Vec<Interval>> {
    unsafe {
        let previous = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        struct Restore(windows_sys::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT);
        impl Drop for Restore {
            fn drop(&mut self) {
                unsafe {
                    SetThreadDpiAwarenessContext(self.0);
                }
            }
        }
        let _restore = Restore(previous);
        let automation: IUIAutomation2 =
            CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)?;
        automation.SetConnectionTimeout(500)?;
        automation.SetTransactionTimeout(500)?;
        let root = automation.ElementFromHandle(HWND(frame.parent as _))?;
        let cache = automation.CreateCacheRequest()?;
        for property in [
            UIA_ControlTypePropertyId,
            UIA_IsOffscreenPropertyId,
            UIA_BoundingRectanglePropertyId,
            UIA_ClassNamePropertyId,
            UIA_AutomationIdPropertyId,
            UIA_ProcessIdPropertyId,
        ] {
            cache.AddProperty(property)?;
        }
        let elements = root.FindAllBuildCache(
            TreeScope_Descendants,
            &automation.CreateTrueCondition()?,
            &cache,
        )?;
        let count = elements.Length()?;
        anyhow::ensure!((1..=2000).contains(&count), "任务栏控件树不可用");
        let mut occupied = vec![];
        let mut app_region = false;
        let mut system_region = false;
        for index in 0..count {
            let element = elements.GetElement(index)?;
            if element.CachedProcessId()? as u32 == std::process::id()
                || element.CachedIsOffscreen()?.as_bool()
            {
                continue;
            }
            let r = element.CachedBoundingRectangle()?;
            if r.right <= r.left
                || r.bottom <= r.top
                || r.bottom <= frame.top
                || r.top >= frame.bottom
                || r.right <= frame.left
                || r.left >= frame.right
            {
                continue;
            }
            let class = element.CachedClassName()?.to_string();
            let id = element.CachedAutomationId()?.to_string();
            let kind = element.CachedControlType()?;
            let identity = format!("{class} {id}").to_ascii_lowercase();
            if verbose {
                println!(
                    "type={} class={class:?} id={id:?} rect=({},{})-({},{})",
                    kind.0, r.left, r.top, r.right, r.bottom
                );
            }
            app_region |= identity.contains("tasklist") || identity.contains("mstask");
            let reserved = identity.contains("systemtray")
                || identity.contains("traynotify")
                || identity.contains("trayclock")
                || identity.contains("showdesktop")
                || identity.contains("clockbutton");
            system_region |= reserved;
            // Containers span empty background; reserve interactive controls and
            // known system areas, never the full XAML taskbar container.
            let interactive = [
                UIA_ButtonControlTypeId,
                UIA_SplitButtonControlTypeId,
                UIA_MenuItemControlTypeId,
                UIA_ListItemControlTypeId,
                UIA_TabItemControlTypeId,
                UIA_EditControlTypeId,
                UIA_CheckBoxControlTypeId,
                UIA_ComboBoxControlTypeId,
                UIA_HyperlinkControlTypeId,
            ]
            .contains(&kind);
            let background_container = matches!(
                class.as_str(),
                "Windows.UI.Input.InputSite.WindowClass"
                    | "Windows.UI.Composition.DesktopWindowContentBridge"
                    | "Microsoft.UI.Content.DesktopChildSiteBridge"
                    | "Taskbar.TaskbarFrameAutomationPeer"
                    | "MSTaskSwWClass"
                    | "MSTaskListWClass"
                    | "ReBarWindow32"
            );
            // Unrecognized content is occupied space, even when its provider
            // does not expose an interactive control type.
            if interactive || reserved || !background_container {
                occupied.push(Interval {
                    left: (r.left - frame.left).max(0),
                    right: (r.right - frame.left).min(frame.right - frame.left),
                });
            }
        }
        // Some third-party taskbar panels (including TrafficMonitor's dialog)
        // are native children missing from the UIA tree; reserve their HWND bounds.
        occupied.extend(native_occupied(frame, verbose)?);
        // Native notification area is a second authoritative reserved boundary.
        let tray = FindWindowExW(
            frame.parent as _,
            std::ptr::null_mut(),
            crate::native::wide("TrayNotifyWnd").as_ptr(),
            std::ptr::null(),
        );
        if !tray.is_null()
            && let Some(tray_frame) = self::frame(tray as isize)
        {
            system_region = true;
            occupied.push(Interval {
                left: tray_frame.left - frame.left,
                right: frame.right - frame.left,
            });
        }
        anyhow::ensure!(
            app_region && system_region && !occupied.is_empty(),
            "无法可靠识别应用按钮和通知区域"
        );
        anyhow::ensure!(
            self::frame(frame.parent) == Some(frame),
            "任务栏布局已变化，等待重新检测"
        );
        Ok(occupied)
    }
}
pub fn diagnostic() -> Result<()> {
    std::thread::spawn(|| unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let parent = FindWindowW(
            crate::native::wide("Shell_TrayWnd").as_ptr(),
            std::ptr::null(),
        );
        let taskbar_frame = frame(parent as isize).context("没有找到主屏任务栏")?;
        let result = scan(taskbar_frame, true);
        CoUninitialize();
        let occupied = result?;
        println!("recognized occupied={occupied:?}");
        Ok(())
    })
    .join()
    .map_err(|_| anyhow::anyhow!("识别线程异常"))?
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "requires an interactive Windows Explorer taskbar"]
    fn current_windows_controls_are_recognized() {
        super::diagnostic().unwrap();
    }
}
