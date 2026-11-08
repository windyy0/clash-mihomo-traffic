//! Windows integration. All HWND ownership stays on the creating UI thread.
//! Copy shared data under short locks before calling Win32.
#![allow(unsafe_op_in_unsafe_fn)]

mod process;
mod taskbar;
mod window;

pub use process::{
    autorun, continue_uninstall, local_api_process, nyanpasu_custom_config_dir, open_folder,
    pipe_owner, pipe_server_process, process_identity, proxy_applications, proxy_client_name,
};
pub use taskbar::{
    UiEvent, adjustment_keys, apply_hotkey, request_app_exit, request_taskbar_update,
    set_hotkey_recording, start_shell, taskbar_dpi,
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
