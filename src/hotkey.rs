//! Global shortcuts are registered and released on the shell window's thread.
use anyhow::{Result, bail, ensure};
use std::fmt;
use windows_sys::Win32::{
    Foundation::{ERROR_GEN_FAILURE, ERROR_HOTKEY_ALREADY_REGISTERED, GetLastError, HWND},
    UI::Input::KeyboardAndMouse::*,
};

pub const DEFAULT: &str = "Ctrl+Alt+M";
pub const APPLY_MESSAGE: u32 = windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 5;
const IDS: [i32; 2] = [0x4d01, 0x4d02];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hotkey {
    modifiers: u32,
    key: u32,
}
impl Hotkey {
    pub fn recorded(key: &str, control: bool, alt: bool, shift: bool, win: bool) -> Result<Self> {
        let mut shift = shift;
        let resolved = if key.chars().count() == 1 {
            let character = key.chars().next().unwrap();
            let virtual_key = if character.len_utf16() == 1 {
                unsafe { VkKeyScanW(character as u16) }
            } else {
                -1
            };
            let byte = (virtual_key & 0xff) as u8;
            if virtual_key != -1 && byte.is_ascii_alphanumeric() {
                // GPUI folds Shift into punctuation such as '(' for Shift+9.
                shift |= virtual_key & 0x100 != 0;
                char::from(byte).to_string()
            } else {
                key.to_string()
            }
        } else {
            key.to_string()
        };
        let mut parts = Vec::new();
        for (pressed, name) in [
            (control, "Ctrl"),
            (alt, "Alt"),
            (shift, "Shift"),
            (win, "Win"),
        ] {
            if pressed {
                parts.push(name.to_string());
            }
        }
        parts.push(resolved);
        Self::parse(&parts.join("+"))
    }
    pub fn parse(text: &str) -> Result<Self> {
        let mut modifiers = 0;
        let mut key = None;
        for token in text.split('+').map(str::trim) {
            let token = token.to_ascii_uppercase();
            let modifier = match token.as_str() {
                "CTRL" | "CONTROL" => MOD_CONTROL,
                "ALT" => MOD_ALT,
                "SHIFT" => MOD_SHIFT,
                "WIN" | "WINDOWS" => MOD_WIN,
                _ => 0,
            };
            if modifier != 0 {
                ensure!(modifiers & modifier == 0, "修饰键不能重复");
                modifiers |= modifier;
                continue;
            }
            let value = if token.len() == 1 && token.as_bytes()[0].is_ascii_alphanumeric() {
                u32::from(token.as_bytes()[0])
            } else if let Some(number) = token.strip_prefix('F').and_then(|v| v.parse::<u32>().ok())
            {
                ensure!((1..=24).contains(&number), "功能键范围为 F1–F24");
                ensure!(number != 12, "F12 由 Windows 调试器保留，请选择其他按键");
                u32::from(VK_F1) + number - 1
            } else {
                bail!("请选择字母、数字或 F1–F24（F12 除外）组成快捷键");
            };
            ensure!(
                key.replace(value).is_none(),
                "只能指定一个字母、数字或功能键"
            );
        }
        ensure!(
            modifiers & (MOD_CONTROL | MOD_ALT | MOD_WIN) != 0,
            "请至少包含 Ctrl、Alt 或 Win 中的一个修饰键"
        );
        Ok(Self {
            modifiers,
            key: key.ok_or_else(|| anyhow::anyhow!("请补充字母、数字或功能键"))?,
        })
    }
    pub fn packed(self) -> usize {
        (self.modifiers | (self.key << 16)) as usize
    }
    pub fn from_packed(value: usize) -> Option<Self> {
        let hotkey = Self {
            modifiers: value as u32 & 0xffff,
            key: (value as u32 >> 16) & 0xffff,
        };
        Self::parse(&hotkey.to_string())
            .ok()
            .filter(|parsed| *parsed == hotkey)
    }
}
impl fmt::Display for Hotkey {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (flag, label) in [
            (MOD_CONTROL, "Ctrl"),
            (MOD_ALT, "Alt"),
            (MOD_SHIFT, "Shift"),
            (MOD_WIN, "Win"),
        ] {
            if self.modifiers & flag != 0 {
                write!(out, "{label} + ")?;
            }
        }
        if (u32::from(VK_F1)..=u32::from(VK_F24)).contains(&self.key) {
            write!(out, "F{}", self.key - u32::from(VK_F1) + 1)
        } else {
            write!(out, "{}", char::from_u32(self.key).unwrap_or('?'))
        }
    }
}

pub fn failure(code: u32) -> String {
    if code == ERROR_HOTKEY_ALREADY_REGISTERED {
        "快捷键已被占用，请更换组合键".into()
    } else {
        format!(
            "快捷键注册失败：{}",
            std::io::Error::from_raw_os_error(code as i32)
        )
    }
}

pub struct Registration {
    owner: HWND,
    active: Option<(i32, Hotkey)>,
}
impl Registration {
    pub fn new(owner: HWND) -> Self {
        Self {
            owner,
            active: None,
        }
    }
    pub fn active(&self) -> Option<Hotkey> {
        self.active.map(|(_, key)| key)
    }
    pub fn replace(&mut self, hotkey: Hotkey) -> std::result::Result<(), u32> {
        if self.active() == Some(hotkey) {
            return Ok(());
        }
        let id = if self.active.is_some_and(|(id, _)| id == IDS[0]) {
            IDS[1]
        } else {
            IDS[0]
        };
        if unsafe { RegisterHotKey(self.owner, id, hotkey.modifiers | MOD_NOREPEAT, hotkey.key) }
            == 0
        {
            let code = unsafe { GetLastError() };
            return Err(if code == 0 { ERROR_GEN_FAILURE } else { code });
        }
        // Claim the new combination first, retaining the working shortcut on failure.
        if let Some((old_id, _)) = self.active.replace((id, hotkey)) {
            unsafe { UnregisterHotKey(self.owner, old_id) };
        }
        Ok(())
    }
    pub fn matches(&self, id: usize, message: isize) -> bool {
        self.active.is_some_and(|(active_id, key)| {
            active_id == id as i32 && key.packed() == message as usize
        })
    }
}
impl Drop for Registration {
    fn drop(&mut self) {
        if let Some((id, _)) = self.active {
            unsafe { UnregisterHotKey(self.owner, id) };
        }
    }
}

#[cfg(test)]
#[path = "hotkey_tests.rs"]
mod tests;
