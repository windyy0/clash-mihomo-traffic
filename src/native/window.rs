//! Native window handles, appearance, floating geometry, and single-instance activation.
use super::{dark_preference, wide};
use anyhow::Result;
use std::ptr::{null, null_mut};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::{LibraryLoader::*, Threading::CreateMutexW},
    UI::{HiDpi::*, Shell::*, WindowsAndMessaging::*},
};

pub fn client_area_animations_enabled() -> bool {
    let mut enabled = 1i32;
    unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            (&mut enabled as *mut i32).cast(),
            0,
        );
    }
    enabled != 0
}

pub struct Instance(HANDLE);
impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
pub fn single_instance() -> Result<Option<Instance>> {
    unsafe {
        let h = CreateMutexW(null(), 0, wide("Local\\MihomoTrafficDesktop").as_ptr());
        anyhow::ensure!(!h.is_null(), "无法创建单实例锁");
        if GetLastError() == ERROR_ALREADY_EXISTS {
            CloseHandle(h);
            let main = FindWindowW(null(), wide("Mihomo Traffic").as_ptr());
            if !main.is_null() {
                show(main as isize, true);
                SetForegroundWindow(main);
            }
            return Ok(None);
        }
        Ok(Some(Instance(h)))
    }
}

#[cfg(test)]
pub fn show_composition_fixture(underlay: isize, floating: isize) {
    unsafe {
        SetWindowPos(
            underlay as HWND,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
        SetWindowPos(
            floating as HWND,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}
pub fn hwnd(window: &gpui::Window) -> isize {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match HasWindowHandle::window_handle(window).map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Win32(h)) => h.hwnd.get(),
        _ => 0,
    }
}
pub fn show(h: isize, visible: bool) {
    unsafe {
        let h = h as HWND;
        if !h.is_null() {
            if visible && IsIconic(h) != 0 {
                ShowWindow(h, SW_SHOWNOACTIVATE);
            } else if (IsWindowVisible(h) != 0) != visible {
                // SW_SHOWNOACTIVATE restores normal placement. A floating popup
                // must keep its current screen position when shown again.
                ShowWindow(h, if visible { SW_SHOWNA } else { SW_HIDE });
            }
        }
    }
}
pub fn foreground(h: isize) {
    unsafe {
        ShowWindow(h as HWND, SW_RESTORE);
        SetForegroundWindow(h as HWND);
    }
}
pub fn set_app_icon(h: isize) {
    unsafe {
        for (size, kind) in [(32, ICON_BIG), (16, ICON_SMALL)] {
            let icon = LoadImageW(
                GetModuleHandleW(null()),
                101usize as *const u16,
                IMAGE_ICON,
                size,
                size,
                LR_SHARED,
            );
            if !icon.is_null() {
                SendMessageW(h as HWND, WM_SETICON, kind as usize, icon as isize);
            }
        }
    }
}
pub fn visible(h: isize) -> bool {
    unsafe { h != 0 && IsWindowVisible(h as HWND) != 0 && IsIconic(h as HWND) == 0 }
}

unsafe extern "system" fn floating_proc(
    h: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    id: usize,
    _: usize,
) -> LRESULT {
    match message {
        // GPUI reserves resize-frame insets even for a non-resizable popup.
        // The floating panel owns the whole window, including its border.
        WM_NCCALCSIZE => return 0,
        WM_NCHITTEST => return HTCLIENT as LRESULT,
        // The panel paints its own outline. Do not add a classic rectangle.
        WM_NCPAINT => return 0,
        WM_NCDESTROY => {
            RemoveWindowSubclass(h, Some(floating_proc), id);
        }
        _ => {}
    }
    DefSubclassProc(h, message, wparam, lparam)
}

pub fn setup_float(h: isize, x: i32, y: i32) {
    unsafe {
        let h = h as HWND;
        let margin = floating_margin(h);
        if SetWindowSubclass(h, Some(floating_proc), 1, 0) == 0 {
            tracing::error!("Unable to remove floating window frame: {}", GetLastError());
        }
        let style = GetWindowLongPtrW(h, GWL_STYLE) as u32;
        SetWindowLongPtrW(
            h,
            GWL_STYLE,
            ((style & !(WS_CAPTION | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX | WS_SYSMENU))
                | WS_POPUP) as isize,
        );
        SetWindowLongPtrW(
            h,
            GWL_EXSTYLE,
            (GetWindowLongPtrW(h, GWL_EXSTYLE)
                & !((WS_EX_APPWINDOW
                    | WS_EX_WINDOWEDGE
                    | WS_EX_CLIENTEDGE
                    | WS_EX_STATICEDGE
                    | WS_EX_DLGMODALFRAME) as isize))
                | WS_EX_TOOLWINDOW as isize,
        );
        SetWindowPos(
            h,
            HWND_TOPMOST,
            x - margin,
            y - margin,
            0,
            0,
            SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
        let mut rect = RECT::default();
        GetWindowRect(h, &mut rect);
        let scale = GetDpiForWindow(h).max(96) as f32 / 96.;
        fit_float(
            h as isize,
            (rect.right - rect.left) as f32 / scale,
            (rect.bottom - rect.top) as f32 / scale,
            FloatPosition::Keep,
        );
        SetWindowTextW(h, wide("Mihomo Traffic · 悬浮窗").as_ptr());
        // The panel paints its own border; an opaque DWM border would remain
        // visible even when the user makes the panel background transparent.
        let border: u32 = 0xfffffffe;
        windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute(
            h,
            34,
            &border as *const _ as _,
            4,
        );
    }
}
pub fn drag(h: isize) {
    unsafe {
        windows_sys::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture();
        PostMessageW(h as HWND, WM_NCLBUTTONDOWN, HTCAPTION as usize, 0);
    }
}
#[derive(Clone, Copy, Debug)]
pub enum FloatPosition {
    Keep,
    Default,
    Saved(i32, i32),
}

pub fn fit_float(h: isize, width: f32, height: f32, placement: FloatPosition) {
    unsafe {
        let h = h as HWND;
        if IsWindow(h) == 0 {
            return;
        }
        let scale = GetDpiForWindow(h).max(96) as f32 / 96.;
        let mut r = RECT::default();
        GetWindowRect(h, &mut r);
        let monitor = MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return;
        }
        let work = info.rcWork;
        let (mut x, mut y, w, height) = fit_float_rect(
            (r.left, r.top),
            (width, height),
            scale,
            (work.left, work.top, work.right, work.bottom),
        );
        match placement {
            FloatPosition::Keep => {}
            FloatPosition::Default => {
                (x, y) = default_float_position(
                    (w, height),
                    scale,
                    (work.left, work.top, work.right, work.bottom),
                );
            }
            FloatPosition::Saved(saved_x, saved_y) => {
                let margin = floating_margin(h);
                x = (saved_x - margin).clamp(work.left, work.right - w);
                y = (saved_y - margin).clamp(work.top, work.bottom - height);
            }
        }
        let resized = r.right - r.left != w || r.bottom - r.top != height;
        if resized || r.left != x || r.top != y {
            SetWindowPos(h, HWND_TOPMOST, x, y, w, height, SWP_NOACTIVATE);
        }
    }
}

fn default_float_position(size: (i32, i32), scale: f32, work: (i32, i32, i32, i32)) -> (i32, i32) {
    let margin = (24. * scale).round() as i32;
    (
        (work.2 - size.0 - margin).max(work.0),
        (work.1 + ((work.3 - work.1) as f32 * 0.65 - size.1 as f32 / 2.).round() as i32)
            .clamp(work.1, work.3 - size.1),
    )
}

pub fn float_corners(h: isize) {
    unsafe {
        use windows_sys::Win32::{
            Graphics::Dwm::DwmExtendFrameIntoClientArea, UI::Controls::MARGINS,
        };
        // The transparent composition visual owns the panel's 14 DIP mask.
        SetWindowRgn(h as HWND, null_mut(), 1);
        let preference: i32 = 1;
        windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute(
            h as HWND,
            33,
            &preference as *const _ as _,
            4,
        );
        // Suppress the directional DWM shadow; GPUI paints a centered halo.
        let rendering_policy: i32 = 1;
        windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute(
            h as HWND,
            2,
            &rendering_policy as *const _ as _,
            4,
        );
        // The backdrop covers the client without an extended glass frame.
        DwmExtendFrameIntoClientArea(h as HWND, &MARGINS::default());
    }
}

unsafe fn floating_margin(h: HWND) -> i32 {
    (crate::floating_backdrop::SHADOW_MARGIN * GetDpiForWindow(h).max(96) as f32 / 96.).round()
        as i32
}

fn fit_float_rect(
    position: (i32, i32),
    size: (f32, f32),
    scale: f32,
    work: (i32, i32, i32, i32),
) -> (i32, i32, i32, i32) {
    let width = (size.0 * scale)
        .ceil()
        .max(1.)
        .min((work.2 - work.0).max(1) as f32) as i32;
    let height = (size.1 * scale)
        .ceil()
        .max(1.)
        .min((work.3 - work.1).max(1) as f32) as i32;
    (
        position.0.clamp(work.0, work.2 - width),
        position.1.clamp(work.1, work.3 - height),
        width,
        height,
    )
}

#[cfg(test)]
pub fn floating_bounds(h: isize) -> (i32, i32, i32, i32, u32) {
    unsafe {
        let mut rect = RECT::default();
        GetWindowRect(h as HWND, &mut rect);
        (
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            GetDpiForWindow(h as HWND),
        )
    }
}

#[cfg(test)]
pub fn floating_client_size(h: isize) -> (i32, i32) {
    unsafe {
        let mut rect = RECT::default();
        GetClientRect(h as HWND, &mut rect);
        (rect.right - rect.left, rect.bottom - rect.top)
    }
}

#[cfg(test)]
pub fn floating_backdrop(h: isize) -> Option<i32> {
    unsafe {
        let mut backdrop = 0i32;
        (windows_sys::Win32::Graphics::Dwm::DwmGetWindowAttribute(
            h as HWND,
            38,
            &mut backdrop as *mut _ as _,
            std::mem::size_of_val(&backdrop) as u32,
        ) >= 0)
            .then_some(backdrop)
    }
}

#[cfg(test)]
pub fn floating_corner_state(h: isize) -> (i32, Option<(i32, i32)>) {
    unsafe {
        let mut preference = 0i32;
        windows_sys::Win32::Graphics::Dwm::DwmGetWindowAttribute(
            h as HWND,
            33,
            &mut preference as *mut _ as _,
            4,
        );
        let region = CreateRectRgn(0, 0, 0, 0);
        let region_size = if GetWindowRgn(h as HWND, region) != 0 {
            let mut bounds = RECT::default();
            GetRgnBox(region, &mut bounds);
            Some((bounds.right - bounds.left, bounds.bottom - bounds.top))
        } else {
            None
        };
        DeleteObject(region);
        (preference, region_size)
    }
}

#[cfg(test)]
pub struct FloatingComposition {
    pub outside: u32,
    pub corners: [u32; 4],
    pub inside: u32,
    pub halo: [u32; 4],
    pub far_shadow: u32,
}

#[cfg(test)]
pub fn floating_composited_colors(h: isize) -> Result<FloatingComposition> {
    unsafe {
        windows_sys::Win32::Graphics::Dwm::DwmFlush();
        let mut rect = RECT::default();
        anyhow::ensure!(
            GetWindowRect(h as HWND, &mut rect) != 0,
            "floating window is unavailable"
        );
        let dc = GetDC(null_mut());
        anyhow::ensure!(!dc.is_null(), "cannot sample the composition fixture");
        let pixel = |x, y| {
            let color = GetPixel(dc, x, y);
            if color == CLR_INVALID {
                None
            } else {
                Some(((color & 255) << 16) | (color & 0xff00) | ((color >> 16) & 255))
            }
        };
        let outside = pixel(rect.left - 4, rect.top - 4);
        let scale = GetDpiForWindow(h as HWND).max(96) as f32 / 96.;
        let margin = (crate::floating_backdrop::SHADOW_MARGIN * scale).round() as i32;
        let corner_inset = margin + (3. * scale).round() as i32;
        let corners = [
            (rect.left + corner_inset, rect.top + corner_inset),
            (rect.right - 1 - corner_inset, rect.top + corner_inset),
            (rect.left + corner_inset, rect.bottom - 1 - corner_inset),
            (
                rect.right - 1 - corner_inset,
                rect.bottom - 1 - corner_inset,
            ),
        ]
        .map(|(x, y)| pixel(x, y));
        let inset = (5. * scale).round() as i32;
        let inside = pixel((rect.left + rect.right) / 2, rect.top + margin + inset);
        let edge = margin - (2. * scale).round() as i32;
        let halo = [
            (rect.left + edge, (rect.top + rect.bottom) / 2),
            (rect.right - 1 - edge, (rect.top + rect.bottom) / 2),
            ((rect.left + rect.right) / 2, rect.top + edge),
            ((rect.left + rect.right) / 2, rect.bottom - 1 - edge),
        ]
        .map(|(x, y)| pixel(x, y));
        let shadow = pixel((rect.left + rect.right) / 2, rect.bottom + inset * 4);
        ReleaseDC(null_mut(), dc);
        anyhow::ensure!(
            outside.is_some()
                && corners.iter().all(Option::is_some)
                && inside.is_some()
                && halo.iter().all(Option::is_some)
                && shadow.is_some(),
            "composition fixture pixels are unavailable"
        );
        Ok(FloatingComposition {
            outside: outside.unwrap(),
            corners: corners.map(Option::unwrap),
            inside: inside.unwrap(),
            halo: halo.map(Option::unwrap),
            far_shadow: shadow.unwrap(),
        })
    }
}

#[cfg(test)]
mod floating_geometry_tests {
    use super::{default_float_position, fit_float_rect};
    #[test]
    fn default_float_position_is_lower_right_and_respects_dpi_and_negative_monitors() {
        assert_eq!(
            default_float_position((200, 100), 1., (0, 0, 1920, 1040)),
            (1696, 626)
        );
        assert_eq!(
            default_float_position((300, 150), 1.5, (0, 0, 1920, 1040)),
            (1584, 601)
        );
        assert_eq!(
            default_float_position((200, 100), 1., (-1920, -200, 0, 840)),
            (-224, 426)
        );
        assert_eq!(
            default_float_position((1920, 1040), 1., (0, 0, 1920, 1040)),
            (0, 0)
        );
    }
    #[test]
    fn floating_geometry_handles_scaling_negative_monitors_and_oversized_contents() {
        assert_eq!(
            fit_float_rect((80, 100), (200., 100.), 1., (0, 0, 1920, 1040)),
            (80, 100, 200, 100)
        );
        assert_eq!(
            fit_float_rect((1800, 1000), (200., 100.), 1.5, (0, 0, 1920, 1040)),
            (1620, 890, 300, 150)
        );
        assert_eq!(
            fit_float_rect((-1900, 50), (200., 100.), 1., (-1920, 0, 0, 1040)),
            (-1900, 50, 200, 100)
        );
        assert_eq!(
            fit_float_rect((-1900, 2000), (3000., 2000.), 1.5, (0, 0, 1920, 1040)),
            (0, 0, 1920, 1040)
        );
    }
}
unsafe extern "system" fn mica_proc(
    h: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    id: usize,
    _: usize,
) -> LRESULT {
    match message {
        // Keep the material's active appearance, without intercepting actual
        // activation or keyboard focus (WM_ACTIVATE / WM_SETFOCUS / WM_KILLFOCUS).
        // Returning FALSE here would prevent normal window deactivation.
        // -1 updates DWM's active material without asking it to repaint a
        // native caption over GPUI's custom window controls.
        WM_NCACTIVATE => return DefSubclassProc(h, message, 1, -1),
        WM_NCDESTROY => {
            RemoveWindowSubclass(h, Some(mica_proc), id);
        }
        _ => {}
    }
    DefSubclassProc(h, message, wparam, lparam)
}

pub fn set_mica(h: isize, enabled: bool) -> bool {
    let mica = set_system_mica(h, enabled);
    unsafe {
        let h = h as HWND;
        if mica {
            if SetWindowSubclass(h, Some(mica_proc), 1, 0) == 0 {
                tracing::error!("Unable to retain Mica appearance: {}", GetLastError());
            }
        } else {
            RemoveWindowSubclass(h, Some(mica_proc), 1);
        }
        // Also apply the correct appearance when changing themes while inactive.
        SendMessageW(
            h,
            WM_NCACTIVATE,
            (mica || GetForegroundWindow() == h) as usize,
            0,
        );
    }
    mica
}

#[cfg(test)]
mod mica_tests {
    use super::*;
    use std::cell::Cell;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetActiveWindow, GetFocus, SetActiveWindow, SetFocus,
    };

    unsafe extern "system" fn observe_paint(
        h: HWND,
        message: u32,
        wp: WPARAM,
        lp: LPARAM,
        id: usize,
        data: usize,
    ) -> LRESULT {
        if message == WM_NCACTIVATE {
            (*(data as *const Cell<bool>)).set(wp != 0);
        } else if message == WM_NCDESTROY {
            RemoveWindowSubclass(h, Some(observe_paint), id);
        }
        DefSubclassProc(h, message, wp, lp)
    }

    struct TestWindow(HWND, Box<Cell<bool>>);
    impl TestWindow {
        unsafe fn new(title: &str, x: i32) -> Self {
            let window = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                wide("STATIC").as_ptr(),
                wide(title).as_ptr(),
                WS_OVERLAPPEDWINDOW,
                x,
                20,
                240,
                120,
                null_mut(),
                null_mut(),
                GetModuleHandleW(null()),
                null(),
            );
            assert!(!window.is_null());
            let painting = Box::new(Cell::new(false));
            assert_ne!(
                SetWindowSubclass(
                    window,
                    Some(observe_paint),
                    1,
                    (&*painting as *const Cell<bool>) as usize
                ),
                0
            );
            Self(window, painting)
        }
        fn active_paint(&self) -> bool {
            self.1.get()
        }
    }
    impl Drop for TestWindow {
        fn drop(&mut self) {
            unsafe { DestroyWindow(self.0) };
        }
    }

    #[test]
    #[ignore = "creates two native windows to verify Mica appearance and real focus transfer"]
    fn main_mica_keeps_material_while_focus_transfers() {
        unsafe {
            let main = TestWindow::new("Mihomo Traffic · Mica test", 20);
            let other = TestWindow::new("Mihomo Traffic · Focus test", 280);
            assert!(set_mica(main.0 as isize, true));
            ShowWindow(main.0, SW_SHOWNOACTIVATE);
            ShowWindow(other.0, SW_SHOWNOACTIVATE);
            SetActiveWindow(main.0);
            SetFocus(main.0);
            assert!(main.active_paint());
            SetActiveWindow(other.0);
            SetFocus(other.0);
            assert_eq!(
                GetActiveWindow(),
                other.0,
                "Mica window prevented deactivation"
            );
            assert_eq!(GetFocus(), other.0, "Mica window retained keyboard focus");
            SendMessageW(main.0, WM_NCACTIVATE, 0, 0);
            assert!(
                main.active_paint(),
                "inactive Mica lost its active appearance"
            );
            assert!(!set_mica(main.0 as isize, false));
            assert!(
                !main.active_paint(),
                "changing themes left active painting enabled"
            );
            assert_eq!(GetActiveWindow(), other.0);
            assert_eq!(GetFocus(), other.0);
            SetActiveWindow(main.0);
            SetFocus(main.0);
            SendMessageW(main.0, WM_NCACTIVATE, 1, 0);
            assert!(main.active_paint());
            assert!(set_mica(main.0 as isize, true));
            SetActiveWindow(other.0);
            SetFocus(other.0);
            assert!(main.active_paint(), "reenabling Mica lost the focus policy");
            assert_eq!(GetActiveWindow(), other.0);
            assert_eq!(GetFocus(), other.0);
        }
    }
}

fn set_system_mica(h: isize, enabled: bool) -> bool {
    unsafe {
        use windows_sys::Win32::Graphics::Dwm::{
            DWMSBT_MAINWINDOW, DWMSBT_NONE, DWMWA_SYSTEMBACKDROP_TYPE,
            DwmExtendFrameIntoClientArea, DwmSetWindowAttribute,
        };
        use windows_sys::Win32::UI::Controls::MARGINS;
        let h = h as HWND;
        let backdrop = if enabled {
            DWMSBT_MAINWINDOW
        } else {
            DWMSBT_NONE
        };
        let result = DwmSetWindowAttribute(
            h,
            DWMWA_SYSTEMBACKDROP_TYPE as u32,
            &backdrop as *const _ as _,
            std::mem::size_of_val(&backdrop) as u32,
        );
        let mica = enabled && result >= 0;
        // The system backdrop already covers the transparent client. Extending
        // glass across the whole window (-1) also exposes DWM caption buttons
        // when maximized. A one-pixel top extension preserves the backdrop and
        // resize frame while leaving caption rendering to GPUI.
        let margins = MARGINS {
            cyTopHeight: i32::from(mica),
            ..Default::default()
        };
        if DwmExtendFrameIntoClientArea(h, &margins) >= 0 {
            mica
        } else {
            // Keep a solid client background if the system cannot provide the material.
            let backdrop = DWMSBT_NONE;
            DwmSetWindowAttribute(
                h,
                DWMWA_SYSTEMBACKDROP_TYPE as u32,
                &backdrop as *const _ as _,
                std::mem::size_of_val(&backdrop) as u32,
            );
            false
        }
    }
}

pub fn titlebar_theme(h: isize, dark: bool) {
    unsafe {
        let flag: i32 = if dark { 1 } else { 0 };
        windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute(
            h as HWND,
            20,
            &flag as *const _ as _,
            4,
        );
    }
}
pub fn position(h: isize) -> (i32, i32) {
    unsafe {
        let mut r = RECT::default();
        GetWindowRect(h as HWND, &mut r);
        let margin = floating_margin(h as HWND);
        (r.left + margin, r.top + margin)
    }
}
pub fn fullscreen_foreground() -> bool {
    unsafe {
        let h = GetForegroundWindow();
        if h.is_null() {
            return false;
        }
        let class = class_name(h);
        if ["Progman", "WorkerW", "Shell_TrayWnd"].contains(&class.as_str()) {
            return false;
        }
        let monitor = MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST);
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        let mut r = RECT::default();
        GetWindowRect(h, &mut r);
        GetMonitorInfoW(monitor, &mut info);
        r.left <= info.rcMonitor.left
            && r.top <= info.rcMonitor.top
            && r.right >= info.rcMonitor.right
            && r.bottom >= info.rcMonitor.bottom
    }
}
pub fn dark_system() -> bool {
    dark_preference("AppsUseLightTheme")
}

unsafe fn class_name(h: HWND) -> String {
    let mut buf = [0u16; 128];
    let n = GetClassNameW(h, buf.as_mut_ptr(), 128);
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}
