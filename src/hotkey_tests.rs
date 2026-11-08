use super::*;
use std::ptr::{null, null_mut};
use windows_sys::Win32::{System::LibraryLoader::GetModuleHandleW, UI::WindowsAndMessaging::*};

#[test]
fn recorded_keys_retain_modifiers_and_resolve_shifted_digits() {
    assert_eq!(
        Hotkey::recorded("k", true, false, true, false)
            .unwrap()
            .to_string(),
        "Ctrl + Shift + K"
    );
    assert_eq!(
        Hotkey::recorded("f24", false, true, false, false)
            .unwrap()
            .to_string(),
        "Alt + F24"
    );
    let scan = unsafe { VkKeyScanW('(' as u16) };
    if scan != -1 && (scan & 0xff) == i16::from(b'9') {
        assert_eq!(
            Hotkey::recorded("(", true, false, false, false)
                .unwrap()
                .to_string(),
            "Ctrl + Shift + 9"
        );
    }
    assert!(Hotkey::recorded("m", false, false, false, false).is_err());
    assert!(Hotkey::recorded("space", true, false, false, false).is_err());
}

#[test]
fn hotkey_input_normalizes_and_rejects_unsafe_or_ambiguous_combinations() {
    for (input, expected) in [
        (" alt + control + m ", "Ctrl + Alt + M"),
        ("ctrl+shift+9", "Ctrl + Shift + 9"),
        ("Win+F24", "Win + F24"),
    ] {
        let key = Hotkey::parse(input).unwrap();
        assert_eq!(key.to_string(), expected);
        assert_eq!(Hotkey::from_packed(key.packed()), Some(key));
    }
    for invalid in [
        "",
        "M",
        "Shift+M",
        "Ctrl",
        "Ctrl++M",
        "Ctrl+M+N",
        "Ctrl+Ctrl+M",
        "Alt+F12",
        "Ctrl+F0",
        "Alt+F25",
        "Ctrl+中文",
    ] {
        assert!(Hotkey::parse(invalid).is_err(), "accepted {invalid}");
    }
    assert_eq!(Hotkey::from_packed(0), None);
    assert_eq!(Hotkey::from_packed(0x41_ffff), None);
}

#[test]
fn legacy_config_gets_default_and_custom_hotkey_survives_restart() {
    use crate::config::Config;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "tray_visible = false\n").unwrap();
    let mut config = Config::load(dir.path()).unwrap();
    assert_eq!(config.open_hotkey, DEFAULT);
    assert!(!config.tray_visible);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "tray_visible = false\n"
    );
    config.open_hotkey = Hotkey::parse("Ctrl+Shift+K").unwrap().to_string();
    config.save(dir.path()).unwrap();
    assert_eq!(Config::load(dir.path()).unwrap(), config);
}

struct Window(HWND);
impl Window {
    fn new() -> Self {
        let class: Vec<_> = "STATIC\0".encode_utf16().collect();
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW,
                class.as_ptr(),
                null(),
                WS_POPUP,
                0,
                0,
                1,
                1,
                null_mut(),
                null_mut(),
                GetModuleHandleW(null()),
                null(),
            )
        };
        assert!(!hwnd.is_null());
        Self(hwnd)
    }
}
impl Drop for Window {
    fn drop(&mut self) {
        unsafe { DestroyWindow(self.0) };
    }
}

#[test]
#[ignore = "temporarily registers real global shortcuts on hidden fixture windows"]
fn registration_preserves_old_key_on_conflict_and_releases_replaced_keys() {
    let first = Window::new();
    let second = Window::new();
    let mut active = Registration::new(first.0);
    let mut occupied = Registration::new(second.0);
    let old = Hotkey::parse("Ctrl+Alt+Shift+F23").unwrap();
    let next = Hotkey::parse("Ctrl+Alt+Shift+F24").unwrap();
    active.replace(old).unwrap();
    occupied.replace(next).unwrap();
    assert_eq!(active.replace(next), Err(ERROR_HOTKEY_ALREADY_REGISTERED));
    assert_eq!(active.active(), Some(old));
    assert!(active.matches(IDS[0] as usize, old.packed() as isize));
    drop(occupied);
    active.replace(next).unwrap();
    assert!(!active.matches(IDS[0] as usize, old.packed() as isize));
    assert!(active.matches(IDS[1] as usize, next.packed() as isize));
    let mut probe = Registration::new(second.0);
    probe
        .replace(old)
        .expect("replaced shortcut was not released");
    active.replace(next).unwrap();
    drop(active);
    probe
        .replace(next)
        .expect("shortcut was not released at shutdown");
}
