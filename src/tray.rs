//! Notification icon resources belong to the Windows shell thread.
use crate::config::TrayIconStyle;
use std::ptr::null;
use windows_sys::Win32::{
    Foundation::HWND,
    System::LibraryLoader::GetModuleHandleW,
    UI::{Shell::*, WindowsAndMessaging::*},
};

pub const CALLBACK: u32 = WM_APP + 1;
const ICON_ID: u32 = 1;
const NIN_KEYSELECT: u32 = NIN_SELECT | NINF_KEY;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct IconKey {
    resource: u16,
    size: i32,
}
impl IconKey {
    fn new(style: TrayIconStyle, dark_taskbar: bool, size: i32) -> Self {
        Self {
            resource: match style {
                TrayIconStyle::Color => 101,
                TrayIconStyle::Monochrome if dark_taskbar => 103,
                TrayIconStyle::Monochrome => 102,
            },
            size,
        }
    }
}

struct Image {
    key: IconKey,
    handle: HICON,
}
impl Image {
    fn load(key: IconKey) -> Option<Self> {
        let handle = unsafe {
            LoadImageW(
                GetModuleHandleW(null()),
                key.resource as usize as *const u16,
                IMAGE_ICON,
                key.size,
                key.size,
                LR_DEFAULTCOLOR,
            )
        };
        (!handle.is_null()).then_some(Self { key, handle })
    }
}
impl Drop for Image {
    fn drop(&mut self) {
        unsafe { DestroyIcon(self.handle) };
    }
}

#[derive(Default)]
pub struct Tray {
    image: Option<Image>,
    added: bool,
}
impl Tray {
    pub fn explorer_restarted(&mut self) {
        self.added = false;
    }

    pub fn sync(
        &mut self,
        owner: HWND,
        visible: bool,
        style: TrayIconStyle,
        dark_taskbar: bool,
        size: i32,
    ) {
        if !visible {
            self.remove(owner);
            return;
        }
        let key = IconKey::new(style, dark_taskbar, size);
        if self.added && self.image.as_ref().is_some_and(|image| image.key == key) {
            return;
        }
        let replacement = if self.image.as_ref().is_some_and(|image| image.key == key) {
            None
        } else {
            let Some(image) = Image::load(key) else {
                return;
            };
            Some(image)
        };
        let image = replacement.as_ref().or(self.image.as_ref()).unwrap();
        let mut data = data(owner);
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
        data.uCallbackMessage = CALLBACK;
        data.hIcon = image.handle;
        let tip: Vec<_> = "Mihomo Traffic · 单击打开，右键菜单\0"
            .encode_utf16()
            .collect();
        data.szTip[..tip.len()].copy_from_slice(&tip);
        let mut action = if self.added { NIM_MODIFY } else { NIM_ADD };
        if unsafe { Shell_NotifyIconW(action, &data) } == 0 {
            // Recover both a lost registration and an icon that already exists.
            // Keep ownership on failure so hiding can still remove the old icon.
            action = if action == NIM_ADD {
                NIM_MODIFY
            } else {
                NIM_ADD
            };
            if unsafe { Shell_NotifyIconW(action, &data) } == 0 {
                return;
            }
        }
        self.added = true;
        if replacement.is_some() {
            self.image = replacement;
        }
        if action == NIM_ADD {
            data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            unsafe { Shell_NotifyIconW(NIM_SETVERSION, &data) };
        }
    }

    pub fn remove(&mut self, owner: HWND) {
        if self.added && unsafe { Shell_NotifyIconW(NIM_DELETE, &data(owner)) } == 0 {
            return;
        }
        self.added = false;
        self.image = None;
    }
}

fn data(owner: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: owner,
        uID: ICON_ID,
        ..unsafe { std::mem::zeroed() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activation {
    Open,
    Menu,
}
pub fn activation(message: isize) -> Option<Activation> {
    // Version 4 places the notification in LOWORD and the icon ID in HIWORD.
    // The same decoding also supports the legacy callback if negotiation fails.
    match message as u32 & 0xffff {
        WM_LBUTTONUP | WM_LBUTTONDBLCLK | NIN_SELECT | NIN_KEYSELECT => Some(Activation::Open),
        WM_RBUTTONUP | WM_CONTEXTMENU => Some(Activation::Menu),
        _ => None,
    }
}

#[cfg(test)]
#[path = "tray_tests.rs"]
mod tests;
