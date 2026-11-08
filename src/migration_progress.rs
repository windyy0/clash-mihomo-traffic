//! A small native progress window survives the original GPUI process exiting.
use std::{sync::mpsc, thread::JoinHandle, time::Duration};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    Graphics::Gdi::{COLOR_WINDOW, DEFAULT_GUI_FONT, GetStockObject, GetSysColorBrush},
    System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::*,
};

const FINISH: u32 = WM_APP + 73;
unsafe extern "system" fn procedure(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        match message {
            WM_CLOSE => 0,
            FINISH => {
                DestroyWindow(hwnd);
                0
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, message, w, l),
        }
    }
}

pub struct Progress {
    hwnd: isize,
    thread: Option<JoinHandle<()>>,
}
impl Progress {
    pub fn show() -> Option<Self> {
        let (sender, receiver) = mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name("migration-progress".into())
            .spawn(move || unsafe {
                let instance = GetModuleHandleW(std::ptr::null());
                let class = crate::native::wide("MihomoTrafficMigration");
                RegisterClassW(&WNDCLASSW {
                    lpfnWndProc: Some(procedure),
                    hInstance: instance,
                    lpszClassName: class.as_ptr(),
                    hbrBackground: GetSysColorBrush(COLOR_WINDOW),
                    hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
                    ..std::mem::zeroed()
                });
                let hwnd = CreateWindowExW(
                    WS_EX_TOPMOST,
                    class.as_ptr(),
                    crate::native::wide("Mihomo Traffic · 数据迁移").as_ptr(),
                    WS_CAPTION | WS_BORDER | WS_VISIBLE,
                    (GetSystemMetrics(SM_CXSCREEN) - 520) / 2,
                    (GetSystemMetrics(SM_CYSCREEN) - 170) / 2,
                    520,
                    170,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    instance,
                    std::ptr::null(),
                );
                if hwnd.is_null() {
                    let _ = sender.send(0);
                    return;
                }
                let text = CreateWindowExW(
                    0,
                    crate::native::wide("STATIC").as_ptr(),
                    crate::native::wide(
                        "正在迁移和校验数据…\n完成后自动重新打开程序。\n原目录数据将保留，请稍候。",
                    )
                    .as_ptr(),
                    WS_CHILD | WS_VISIBLE,
                    24,
                    22,
                    470,
                    100,
                    hwnd,
                    std::ptr::null_mut(),
                    instance,
                    std::ptr::null(),
                );
                SendMessageW(
                    text,
                    WM_SETFONT,
                    GetStockObject(DEFAULT_GUI_FONT) as usize,
                    1,
                );
                let _ = sender.send(hwnd as isize);
                let mut message = std::mem::zeroed();
                while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            })
            .ok()?;
        let hwnd = receiver.recv_timeout(Duration::from_secs(3)).ok()?;
        (hwnd != 0).then_some(Self {
            hwnd,
            thread: Some(thread),
        })
    }
}
impl Drop for Progress {
    fn drop(&mut self) {
        unsafe {
            PostMessageW(self.hwnd as HWND, FINISH, 0, 0);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
