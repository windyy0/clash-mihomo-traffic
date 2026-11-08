use super::*;
use windows_sys::Win32::{Foundation::RECT, Graphics::Gdi::*};

#[test]
fn embedded_icons_load_at_notification_area_dpi_sizes() {
    for style in [TrayIconStyle::Monochrome, TrayIconStyle::Color] {
        for dark in [false, true] {
            for size in [16, 20, 24, 32, 40, 48, 64] {
                let icon =
                    Image::load(IconKey::new(style, dark, size)).expect("missing icon resource");
                unsafe {
                    let mut info: ICONINFO = std::mem::zeroed();
                    assert_ne!(GetIconInfo(icon.handle, &mut info), 0);
                    let mut bitmap: BITMAP = std::mem::zeroed();
                    assert_ne!(
                        GetObjectW(
                            info.hbmColor,
                            std::mem::size_of::<BITMAP>() as i32,
                            &mut bitmap as *mut _ as _
                        ),
                        0
                    );
                    assert_eq!(
                        (bitmap.bmWidth, bitmap.bmHeight, bitmap.bmBitsPixel),
                        (size, size, 32)
                    );
                    DeleteObject(info.hbmColor);
                    DeleteObject(info.hbmMask);
                }
            }
        }
    }
}

#[test]
fn single_click_keyboard_and_context_menu_decode_both_callback_versions() {
    for icon_id in [0, ICON_ID] {
        for event in [WM_LBUTTONUP, WM_LBUTTONDBLCLK, NIN_SELECT, NIN_KEYSELECT] {
            assert_eq!(
                activation(((icon_id << 16) | event) as isize),
                Some(Activation::Open)
            );
        }
        for event in [WM_RBUTTONUP, WM_CONTEXTMENU] {
            assert_eq!(
                activation(((icon_id << 16) | event) as isize),
                Some(Activation::Menu)
            );
        }
        assert_eq!(activation(((icon_id << 16) | WM_MOUSEMOVE) as isize), None);
    }
}

struct Window(HWND);
impl Window {
    fn new() -> Self {
        let class: Vec<_> = "STATIC\0".encode_utf16().collect();
        let owner = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                null(),
                WS_POPUP,
                0,
                0,
                1,
                1,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                GetModuleHandleW(null()),
                null(),
            )
        };
        assert!(!owner.is_null());
        Self(owner)
    }
    fn icon_exists(&self) -> bool {
        let identifier = NOTIFYICONIDENTIFIER {
            cbSize: std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32,
            hWnd: self.0,
            uID: ICON_ID,
            ..unsafe { std::mem::zeroed() }
        };
        unsafe { Shell_NotifyIconGetRect(&identifier, &mut RECT::default()) >= 0 }
    }
}
impl Drop for Window {
    fn drop(&mut self) {
        unsafe {
            Shell_NotifyIconW(NIM_DELETE, &data(self.0));
            DestroyWindow(self.0);
        }
    }
}

#[test]
#[ignore = "briefly adds a real notification icon; requires Explorer"]
fn shell_icon_changes_theme_size_visibility_and_recovers_registration() {
    let window = Window::new();
    let mut tray = Tray::default();
    for (style, dark, size, resource) in [
        (TrayIconStyle::Monochrome, false, 16, 102),
        (TrayIconStyle::Monochrome, true, 24, 103),
        (TrayIconStyle::Color, true, 32, 101),
        (TrayIconStyle::Color, false, 20, 101),
    ] {
        tray.sync(window.0, true, style, dark, size);
        assert!(tray.added && window.icon_exists());
        let image = tray.image.as_ref().unwrap();
        assert_eq!(image.key, IconKey { resource, size });
        let handle = image.handle;
        tray.sync(window.0, true, style, dark, size);
        assert_eq!(
            tray.image.as_ref().unwrap().handle,
            handle,
            "unchanged icon was reloaded"
        );
    }
    unsafe { assert_ne!(Shell_NotifyIconW(NIM_DELETE, &data(window.0)), 0) };
    tray.explorer_restarted();
    tray.sync(window.0, true, TrayIconStyle::Monochrome, true, 24);
    assert!(
        tray.added && window.icon_exists(),
        "registration was not recovered"
    );
    tray.sync(window.0, false, TrayIconStyle::Monochrome, true, 24);
    assert!(!tray.added && tray.image.is_none() && !window.icon_exists());
    tray.explorer_restarted();
    tray.sync(window.0, false, TrayIconStyle::Color, false, 16);
    assert!(
        !window.icon_exists(),
        "Explorer restart resurrected a hidden icon"
    );
    tray.sync(window.0, true, TrayIconStyle::Color, false, 16);
    assert!(tray.added && window.icon_exists());
    tray.remove(window.0);
    assert!(!window.icon_exists());
}
