//! Windows integration. All HWND ownership stays on the creating UI thread.
//! Copy shared data under short locks before calling Win32.
#![allow(unsafe_op_in_unsafe_fn)]

mod process;
mod taskbar;
mod window;

pub(crate) use taskbar::request_matching_app_exit;
pub(crate) use window::{DESKTOP_MUTEX_NAME, try_instance};

pub use process::{
    autorun, continue_uninstall, local_api_process, nyanpasu_custom_config_dir, open_folder,
    pipe_owner, pipe_server_process, process_identity, proxy_applications, proxy_client_name,
    trim_working_set,
};
pub use taskbar::{
    UiEvent, adjustment_keys, apply_hotkey, request_app_exit, request_app_open,
    request_taskbar_update, set_hotkey_recording, start_shell, taskbar_dpi,
};
pub use window::{
    FloatPosition, client_area_animations_enabled, dark_system, drag, fit_float, float_corners,
    foreground, fullscreen_foreground, hwnd, position, set_app_icon, set_mica, setup_float, show,
    single_instance, titlebar_theme, visible,
};
#[cfg(test)]
pub use window::{
    FloatingComposition, floating_backdrop, floating_bounds, floating_client_size,
    floating_composited_colors, floating_corner_state, show_composition_fixture,
};

use anyhow::Result;
use std::{path::Path, ptr::null_mut};
use windows_sys::Win32::{Storage::FileSystem::*, System::Registry::*};

pub fn wide(s: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    s.as_ref().encode_wide().chain(Some(0)).collect()
}

pub fn replace_file(from: &Path, to: &Path) -> Result<()> {
    let replaced = unsafe {
        MoveFileExW(
            wide(from).as_ptr(),
            wide(to).as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if replaced == 0 {
        let error = std::io::Error::last_os_error();
        let message = format!("保存失败：{error}");
        return Err(anyhow::Error::new(error).context(message));
    }
    Ok(())
}

pub fn installed_app_dir() -> Option<std::path::PathBuf> {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt, path::PathBuf};

    let exe = std::env::current_exe().ok()?;
    let directory = exe.parent()?;
    // Matches AppId in scripts/installer.iss.
    const KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{ACF55E08-17D4-4F65-B135-FB96A4CEB535}_is1";
    let mut path = vec![0u16; 32768];
    let mut bytes = (path.len() * size_of::<u16>()) as u32;
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            wide(KEY).as_ptr(),
            wide("Inno Setup: App Path").as_ptr(),
            RRF_RT_REG_SZ,
            null_mut(),
            path.as_mut_ptr().cast(),
            &mut bytes,
        )
    };
    if result == 0 && bytes > 2 {
        let registered = PathBuf::from(OsString::from_wide(&path[..bytes as usize / 2 - 1]));
        if registered.is_absolute() {
            let registered_exe =
                std::fs::canonicalize(registered.join("mihomo-traffic.exe")).ok()?;
            return (std::fs::canonicalize(&exe).ok()? == registered_exe).then_some(registered);
        }
    }

    // An existing Inno installation may have lost its uninstall registry entry.
    (directory.join("unins000.exe").is_file() && directory.join("unins000.dat").is_file())
        .then(|| directory.to_path_buf())
}

fn dark_preference(name: &str) -> bool {
    unsafe {
        let mut v: u32 = 1;
        let mut n = 4;
        RegGetValueW(
            HKEY_CURRENT_USER,
            wide("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize").as_ptr(),
            wide(name).as_ptr(),
            RRF_RT_REG_DWORD,
            null_mut(),
            &mut v as *mut _ as _,
            &mut n,
        );
        v == 0
    }
}
