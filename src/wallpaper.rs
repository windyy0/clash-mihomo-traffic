//! Read wallpaper colors on a dedicated COM thread; never decode images on the UI thread.
use std::{
    ffi::OsString, os::windows::ffi::OsStringExt, path::PathBuf, sync::mpsc, time::SystemTime,
};
use windows::{
    Win32::{
        System::Com::{
            CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
            CoUninitialize,
        },
        UI::Shell::{DesktopWallpaper, IDesktopWallpaper},
    },
    core::{PCWSTR, PWSTR},
};
use windows_sys::Win32::{
    Foundation::HWND,
    Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow},
    UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext},
};

pub struct WallpaperWatcher {
    requests: mpsc::Sender<isize>,
    results: mpsc::Receiver<Option<u32>>,
    pending: bool,
    color: Option<u32>,
}

impl WallpaperWatcher {
    pub fn new() -> Self {
        let (requests, receiver) = mpsc::channel();
        let (sender, results) = mpsc::channel();
        let _ = std::thread::Builder::new()
            .name("wallpaper-color".into())
            .spawn(move || {
                // Shell monitor rectangles use physical pixels, including at fractional scaling.
                unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
                if unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_err() {
                    return;
                }
                let _apartment = Apartment;
                let mut cache: Option<(Source, u32)> = None;
                while let Ok(hwnd) = receiver.recv() {
                    let color = read_color(hwnd, &mut cache);
                    if sender.send(color).is_err() {
                        break;
                    }
                }
            });
        Self {
            requests,
            results,
            pending: false,
            color: None,
        }
    }

    pub fn request(&mut self, hwnd: isize) {
        if !self.pending {
            self.pending = self.requests.send(hwnd).is_ok();
        }
    }

    pub fn poll(&mut self) -> bool {
        if !self.pending {
            return false;
        }
        match self.results.try_recv() {
            Ok(color) => {
                self.pending = false;
                if let Some(color) = color {
                    let changed = self.color != Some(color);
                    self.color = Some(color);
                    changed
                } else {
                    false
                }
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.pending = false;
                false
            }
            Err(mpsc::TryRecvError::Empty) => false,
        }
    }

    pub fn color(&self) -> Option<u32> {
        self.color
    }
}

struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

struct ShellString(PWSTR);
impl Drop for ShellString {
    fn drop(&mut self) {
        unsafe { CoTaskMemFree(Some(self.0.0.cast())) };
    }
}

#[derive(PartialEq, Eq)]
struct Source {
    path: PathBuf,
    modified: Option<SystemTime>,
    length: u64,
    background: u32,
}

fn source(hwnd: isize) -> Option<Source> {
    unsafe {
        let monitor = MonitorFromWindow(hwnd as HWND, MONITOR_DEFAULTTONEAREST);
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return None;
        }
        let desktop: IDesktopWallpaper =
            CoCreateInstance(&DesktopWallpaper, None, CLSCTX_ALL).ok()?;
        for index in 0..desktop.GetMonitorDevicePathCount().ok()? {
            let id = ShellString(desktop.GetMonitorDevicePathAt(index).ok()?);
            if id.0.is_null() {
                continue;
            }
            let rect = desktop.GetMonitorRECT(PCWSTR(id.0.0)).ok()?;
            if (rect.left, rect.top, rect.right, rect.bottom)
                != (
                    info.rcMonitor.left,
                    info.rcMonitor.top,
                    info.rcMonitor.right,
                    info.rcMonitor.bottom,
                )
            {
                continue;
            }
            let wallpaper = ShellString(desktop.GetWallpaper(PCWSTR(id.0.0)).ok()?);
            let path = if wallpaper.0.is_null() {
                PathBuf::new()
            } else {
                PathBuf::from(OsString::from_wide(wallpaper.0.as_wide()))
            };
            let background = desktop.GetBackgroundColor().ok()?.0;
            let background =
                ((background & 255) << 16) | (background & 0xff00) | ((background >> 16) & 255);
            let metadata = if path.as_os_str().is_empty() {
                None
            } else {
                Some(std::fs::metadata(&path).ok()?)
            };
            return Some(Source {
                path,
                modified: metadata.as_ref().and_then(|m| m.modified().ok()),
                length: metadata.as_ref().map_or(0, |m| m.len()),
                background,
            });
        }
        None
    }
}

fn read_color(hwnd: isize, cache: &mut Option<(Source, u32)>) -> Option<u32> {
    let source = source(hwnd)?;
    if let Some((previous, color)) = cache
        && *previous == source
    {
        return Some(*color);
    }
    let color = if source.path.as_os_str().is_empty() {
        source.background
    } else {
        // Windows may use an extensionless wallpaper cache, so inspect the file signature.
        let mut reader = image::ImageReader::open(&source.path)
            .ok()?
            .with_guessed_format()
            .ok()?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(16384);
        limits.max_image_height = Some(16384);
        limits.max_alloc = Some(256 * 1024 * 1024);
        reader.limits(limits);
        let sample = reader.decode().ok()?.thumbnail(96, 96).to_rgba8();
        let background = [
            ((source.background >> 16) & 255) as f64,
            ((source.background >> 8) & 255) as f64,
            (source.background & 255) as f64,
        ];
        let mut sum = [0.; 3];
        let mut total = 0.;
        for pixel in sample.pixels() {
            let alpha = f64::from(pixel[3]) / 255.;
            let rgb: [f64; 3] =
                std::array::from_fn(|i| f64::from(pixel[i]) * alpha + background[i] * (1. - alpha));
            let max = rgb[0].max(rgb[1]).max(rgb[2]);
            let min = rgb[0].min(rgb[1]).min(rgb[2]);
            // Favor colorful areas without allowing a tiny vivid patch to dominate the image.
            let weight = 0.25 + (max - min) / 255.;
            for i in 0..3 {
                sum[i] += rgb[i] * weight;
            }
            total += weight;
        }
        if total == 0. {
            return None;
        }
        let channel = |i: usize| (sum[i] / total).round().clamp(0., 255.) as u32;
        (channel(0) << 16) | (channel(1) << 8) | channel(2)
    };
    *cache = Some((source, color));
    Some(color)
}
