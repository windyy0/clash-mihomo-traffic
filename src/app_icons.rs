//! Application resources are read off the UI/collector threads; only BGRA pixels cross threads.
use gpui::RenderImage;
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex,
        mpsc::{self, SyncSender},
    },
    time::{Duration, Instant, SystemTime},
};

const CACHE_LIMIT: usize = 256;
const CACHE_BYTES: usize = 8 * 1024 * 1024;
const RETRY: Duration = Duration::from_secs(60);
const REVALIDATE: Duration = Duration::from_secs(60);
const BUSY_RETRY: Duration = Duration::from_secs(1);
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Key {
    path: String,
    pixels: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileStamp {
    length: u64,
    modified: SystemTime,
}
struct Request {
    key: Key,
    id: u64,
    stamp: Option<FileStamp>,
    ready: bool,
}
enum Loaded {
    Unchanged,
    Image(Arc<RenderImage>, FileStamp),
    Unavailable,
    Changed,
}
struct Entry {
    image: Option<Arc<RenderImage>>,
    pending: bool,
    retry: Instant,
    used: u64,
    request: u64,
    stamp: Option<FileStamp>,
}
#[derive(Default)]
struct State {
    entries: HashMap<Key, Entry>,
    aliases: HashMap<String, Option<String>>,
    known_paths: HashSet<String>,
    generation: u64,
    clock: u64,
    bytes: usize,
}
fn image_bytes(image: &Option<Arc<RenderImage>>) -> usize {
    image
        .as_ref()
        .and_then(|image| image.as_bytes(0))
        .map_or(0, <[u8]>::len)
}
impl State {
    fn trim(&mut self, keep: Option<&Key>, incoming: usize) {
        while self.entries.len() + incoming > CACHE_LIMIT || self.bytes > CACHE_BYTES {
            let oldest = self
                .entries
                .iter()
                .filter(|(key, _)| Some(*key) != keep)
                .min_by_key(|(_, entry)| (entry.pending, entry.used))
                .map(|(key, _)| key.clone());
            let Some(oldest) = oldest else { break };
            if let Some(entry) = self.entries.remove(&oldest) {
                self.bytes -= image_bytes(&entry.image);
            }
        }
    }
}
pub struct AppIcons {
    state: Arc<Mutex<State>>,
    sender: SyncSender<Request>,
}

fn local_path(key: &str) -> Option<String> {
    let path = key.strip_prefix(r"\\?\").unwrap_or(key).replace('/', "\\");
    let bytes = path.as_bytes();
    (bytes.len() > 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && bytes[2] == b'\\'
        && !path.contains('\0'))
    .then_some(path)
}
fn basename(key: &str) -> String {
    key.rsplit(['\\', '/']).next().unwrap_or(key).to_lowercase()
}
impl AppIcons {
    pub fn new() -> Self {
        Self::with_source(extract, platform::file_stamp)
    }
    #[cfg(test)]
    fn with_loader(
        loader: impl Fn(&str, u32) -> Option<Arc<RenderImage>> + Send + 'static,
    ) -> Self {
        Self::with_source(loader, |_| {
            Some(FileStamp {
                length: 0,
                modified: SystemTime::UNIX_EPOCH,
            })
        })
    }
    fn with_source(
        loader: impl Fn(&str, u32) -> Option<Arc<RenderImage>> + Send + 'static,
        inspect: impl Fn(&str) -> Option<FileStamp> + Send + 'static,
    ) -> Self {
        let state = Arc::new(Mutex::new(State::default()));
        let worker = state.clone();
        let (sender, receiver) = mpsc::sync_channel::<Request>(64);
        std::thread::Builder::new()
            .name("application-icons".into())
            .spawn(move || {
                // No joins or collector/storage locks: a slow file cannot block shutdown.
                let _com = platform::Com::new();
                while let Ok(request) = receiver.recv() {
                    let key = &request.key;
                    // Metadata and shell calls stay on this worker, never the UI thread.
                    let before = inspect(&key.path);
                    let loaded = if request.ready && before.is_some() && before == request.stamp {
                        Loaded::Unchanged
                    } else if let Some(stamp) = before {
                        let image = loader(&key.path, key.pixels);
                        if inspect(&key.path) != before {
                            Loaded::Changed
                        } else if let Some(image) = image {
                            Loaded::Image(image, stamp)
                        } else {
                            Loaded::Unavailable
                        }
                    } else {
                        Loaded::Unavailable
                    };
                    if let Ok(mut state) = worker.lock()
                        && let Some(entry) = state.entries.get_mut(key)
                        && entry.request == request.id
                    {
                        let old_bytes = image_bytes(&entry.image);
                        let delay = match loaded {
                            Loaded::Unchanged => REVALIDATE,
                            Loaded::Image(image, stamp) => {
                                entry.image = Some(image);
                                entry.stamp = Some(stamp);
                                REVALIDATE
                            }
                            // A removed/replacing executable must not blank a ready icon.
                            Loaded::Unavailable => RETRY,
                            Loaded::Changed => BUSY_RETRY,
                        };
                        entry.pending = false;
                        entry.retry = Instant::now() + delay;
                        let new_bytes = image_bytes(&entry.image);
                        state.bytes = state.bytes - old_bytes + new_bytes;
                        state.trim(Some(key), 0);
                    }
                }
            })
            .ok();
        Self { state, sender }
    }
    pub fn update_aliases(&self, generation: u64, keys: impl IntoIterator<Item = String>) {
        let paths: HashSet<_> = keys
            .into_iter()
            .filter_map(|key| local_path(&key).map(|path| path.to_lowercase()))
            .collect();
        if let Ok(state) = self.state.lock()
            && state.generation == generation
            && state.known_paths == paths
        {
            return;
        }
        let mut aliases: HashMap<String, Option<String>> = HashMap::new();
        for path in &paths {
            aliases
                .entry(basename(path))
                .and_modify(|existing| {
                    if existing.as_ref() != Some(path) {
                        *existing = None;
                    }
                })
                .or_insert(Some(path.clone()));
        }
        if let Ok(mut state) = self.state.lock() {
            state.generation = generation;
            state.aliases = aliases;
            state.known_paths = paths;
        }
    }
    pub fn image(&self, key: &str, pixels: u32) -> Option<Arc<RenderImage>> {
        let mut state = self.state.lock().ok()?;
        let path = local_path(key).or_else(|| {
            // A bare name may reuse a unique known path. Relative/remote paths never become aliases.
            if key.contains(['\\', '/', ':']) {
                return None;
            }
            state.aliases.get(&basename(key)).cloned().flatten()
        })?;
        let key = Key {
            path: path.to_lowercase(),
            pixels: pixels.clamp(16, 128),
        };
        state.clock = state.clock.wrapping_add(1);
        let used = state.clock;
        let now = Instant::now();
        if let Some(entry) = state.entries.get_mut(&key) {
            entry.used = used;
            if entry.pending || now < entry.retry {
                return entry.image.clone();
            }
        }
        let previous = state
            .entries
            .get(&key)
            .map(|entry| (entry.image.clone(), entry.stamp));
        if previous.is_none() {
            state.trim(None, 1);
        }
        let (image, stamp) = previous.unwrap_or((None, None));
        let queued = self
            .sender
            .try_send(Request {
                key: key.clone(),
                id: used,
                stamp,
                ready: image.is_some(),
            })
            .is_ok();
        state.entries.insert(
            key,
            Entry {
                image: image.clone(),
                pending: queued,
                retry: now + if queued { RETRY } else { BUSY_RETRY },
                used,
                request: used,
                stamp,
            },
        );
        image
    }
}
fn extract(path: &str, pixels: u32) -> Option<Arc<RenderImage>> {
    platform::extract(path, pixels)
}

fn icon_pixels(
    mut black: Vec<u8>,
    white: &[u8],
    source: u32,
    target: u32,
) -> Option<image::RgbaImage> {
    if black.len() != white.len() {
        return None;
    }
    // Keep premultiplied BGRA while filtering so transparent edges do not bleed black.
    for (pixel, white) in black
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(white.as_chunks::<4>().0)
    {
        let transparency = (0..3)
            .map(|channel| white[channel].saturating_sub(pixel[channel]))
            .max()
            .unwrap_or(0);
        pixel[3] = 255 - transparency;
        for channel in &mut pixel[..3] {
            *channel = (*channel).min(255 - transparency);
        }
    }
    let buffer = image::RgbaImage::from_raw(source, source, black)?;
    let mut buffer = if source == target {
        buffer
    } else {
        image::imageops::resize(
            &buffer,
            target,
            target,
            image::imageops::FilterType::Lanczos3,
        )
    };
    // RenderImage's ordinary image path expects straight BGRA, not premultiplied pixels.
    for pixel in buffer.pixels_mut() {
        let alpha = u16::from(pixel[3]);
        for channel in &mut pixel.0[..3] {
            *channel = (u16::from(*channel) * 255 + alpha / 2)
                .checked_div(alpha)
                .unwrap_or(0)
                .min(255) as u8;
        }
    }
    Some(buffer)
}

mod platform {
    use super::*;
    use std::{
        path::Path,
        ptr::{null, null_mut},
    };
    use windows_sys::Win32::{
        Graphics::Gdi::*,
        Storage::FileSystem::GetDriveTypeW,
        System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
        UI::{
            Shell::SHDefExtractIconW,
            WindowsAndMessaging::{DI_NORMAL, DestroyIcon, DrawIconEx, HICON},
        },
    };
    pub struct Com(bool);
    impl Com {
        pub fn new() -> Self {
            Self(unsafe { CoInitializeEx(null(), COINIT_APARTMENTTHREADED as u32) } >= 0)
        }
    }
    impl Drop for Com {
        fn drop(&mut self) {
            if self.0 {
                unsafe {
                    CoUninitialize();
                }
            }
        }
    }
    struct Icon(HICON);
    impl Drop for Icon {
        fn drop(&mut self) {
            unsafe {
                DestroyIcon(self.0);
            }
        }
    }
    struct Dib {
        dc: HDC,
        bitmap: HBITMAP,
        original: HGDIOBJ,
        bits: *mut u8,
        pixels: u32,
    }
    impl Dib {
        fn new(pixels: u32) -> Option<Self> {
            unsafe {
                let dc = CreateCompatibleDC(null_mut());
                if dc.is_null() {
                    return None;
                }
                let info = BITMAPINFO {
                    bmiHeader: BITMAPINFOHEADER {
                        biSize: size_of::<BITMAPINFOHEADER>() as u32,
                        biWidth: pixels as i32,
                        biHeight: -(pixels as i32),
                        biPlanes: 1,
                        biBitCount: 32,
                        biCompression: BI_RGB,
                        ..Default::default()
                    },
                    ..Default::default()
                };
                let mut bits = null_mut();
                let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
                if bitmap.is_null() || bits.is_null() {
                    if !bitmap.is_null() {
                        DeleteObject(bitmap);
                    }
                    DeleteDC(dc);
                    return None;
                }
                let original = SelectObject(dc, bitmap);
                if original.is_null() || original as isize == -1 {
                    DeleteObject(bitmap);
                    DeleteDC(dc);
                    return None;
                }
                Some(Self {
                    dc,
                    bitmap,
                    original,
                    bits: bits.cast(),
                    pixels,
                })
            }
        }
        fn draw(&self, icon: HICON, background: u8) -> Option<Vec<u8>> {
            unsafe {
                let len = self.pixels as usize * self.pixels as usize * 4;
                std::ptr::write_bytes(self.bits, background, len);
                if DrawIconEx(
                    self.dc,
                    0,
                    0,
                    icon,
                    self.pixels as i32,
                    self.pixels as i32,
                    0,
                    null_mut(),
                    DI_NORMAL,
                ) == 0
                    || GdiFlush() == 0
                {
                    return None;
                }
                Some(std::slice::from_raw_parts(self.bits, len).to_vec())
            }
        }
    }
    impl Drop for Dib {
        fn drop(&mut self) {
            unsafe {
                SelectObject(self.dc, self.original);
                DeleteObject(self.bitmap);
                DeleteDC(self.dc);
            }
        }
    }
    pub fn file_stamp(path: &str) -> Option<FileStamp> {
        let path = local_path(path)?;
        let root: Vec<u16> = format!("{}\\\0", &path[..2]).encode_utf16().collect();
        if unsafe { GetDriveTypeW(root.as_ptr()) } == 4 {
            return None;
        }
        let metadata = Path::new(&path).metadata().ok()?;
        metadata.is_file().then_some(FileStamp {
            length: metadata.len(),
            modified: metadata.modified().ok()?,
        })
    }
    pub fn extract(path: &str, pixels: u32) -> Option<Arc<RenderImage>> {
        unsafe {
            // Never follow UNC or mapped network paths while resolving a local application resource.
            file_stamp(path)?;
            let source = pixels.saturating_mul(2).clamp(32, 256);
            let wide: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
            let mut handle = null_mut();
            let result = SHDefExtractIconW(wide.as_ptr(), 0, 0, &mut handle, null_mut(), source);
            if result != 0 || handle.is_null() {
                if !handle.is_null() {
                    DestroyIcon(handle);
                }
                return None;
            }
            let icon = Icon(handle);
            let dib = Dib::new(source)?;
            let black = dib.draw(icon.0, 0)?;
            let white = dib.draw(icon.0, 255)?;
            let buffer = icon_pixels(black, &white, source, pixels)?;
            Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    fn fixture(pixels: u32) -> Arc<RenderImage> {
        Arc::new(RenderImage::new(vec![image::Frame::new(
            image::RgbaImage::new(pixels, pixels),
        )]))
    }
    fn wait_image(icons: &AppIcons, path: &str, pixels: u32) -> Arc<RenderImage> {
        let until = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(image) = icons.image(path, pixels) {
                return image;
            }
            assert!(Instant::now() < until, "icon worker did not complete");
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn expire(icons: &AppIcons) {
        for entry in icons.state.lock().unwrap().entries.values_mut() {
            entry.retry = Instant::now() - Duration::from_secs(1);
        }
    }
    fn settle(icons: &AppIcons) {
        let until = Instant::now() + Duration::from_secs(3);
        while icons
            .state
            .lock()
            .unwrap()
            .entries
            .values()
            .any(|entry| entry.pending)
        {
            assert!(Instant::now() < until, "icon refresh did not complete");
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    #[test]
    fn full_paths_and_ambiguous_names_do_not_share_identity() {
        let icons = AppIcons::with_loader(|_, _| None);
        icons.update_aliases(1, [r"C:\甲 路径\same.exe".into()]);
        assert_eq!(
            icons.state.lock().unwrap().aliases["same.exe"].as_deref(),
            Some(r"c:\甲 路径\same.exe")
        );
        icons.update_aliases(1, [r"C:\甲 路径\same.exe".into(), r"D:\乙\same.exe".into()]);
        assert!(icons.state.lock().unwrap().aliases["same.exe"].is_none());
        assert!(icons.image("same.exe", 24).is_none());
        assert!(icons.state.lock().unwrap().entries.is_empty());
        assert!(local_path(r"\\server\share\app.exe").is_none());
        assert!(local_path("app.exe").is_none());
        assert!(local_path("C:relative.exe").is_none());
        assert!(local_path("C:\\app\0.exe").is_none());
    }
    #[test]
    fn extraction_is_async_deduplicated_and_failures_are_cached() {
        let count = Arc::new(AtomicUsize::new(0));
        let calls = count.clone();
        let icons = AppIcons::with_loader(move |_, _| {
            calls.fetch_add(1, Ordering::SeqCst);
            None
        });
        for _ in 0..10 {
            assert!(icons.image(r"C:\missing.exe", 24).is_none());
        }
        let until = Instant::now() + Duration::from_secs(2);
        while count.load(Ordering::SeqCst) == 0 && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        for _ in 0..10 {
            icons.image(r"C:\missing.exe", 24);
        }
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn shell_icons_preserve_transparency_at_multiple_resolutions_and_unicode_paths() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("中文 folder.exe");
        std::fs::copy(std::env::current_exe().unwrap(), &path).unwrap();
        for pixels in [16, 24, 48] {
            let icon = extract(path.to_str().unwrap(), pixels).expect("embedded application icon");
            let bytes = icon.as_bytes(0).unwrap();
            assert_eq!(bytes.len(), pixels as usize * pixels as usize * 4);
            assert!(bytes.as_chunks::<4>().0.iter().any(|pixel| pixel[3] == 0));
            assert!(bytes.as_chunks::<4>().0.iter().any(|pixel| pixel[3] > 0));
            assert!(
                bytes
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .filter(|pixel| pixel[3] == 0)
                    .all(|pixel| pixel[..3] == [0, 0, 0])
            );
        }
        assert!(extract(temporary.path().join("absent.exe").to_str().unwrap(), 24).is_none());
    }
    #[test]
    fn translucent_colors_are_not_multiplied_by_alpha_twice() {
        let black = vec![0, 0, 128, 0, 0, 255, 0, 0, 0, 0, 0, 0, 64, 0, 0, 0];
        let white = [
            127, 127, 255, 0, 0, 255, 0, 0, 255, 255, 255, 0, 255, 191, 191, 0,
        ];
        let result = icon_pixels(black, &white, 2, 2).unwrap();
        assert_eq!(result.get_pixel(0, 0).0, [0, 0, 255, 128]);
        assert_eq!(result.get_pixel(1, 0).0, [0, 255, 0, 255]);
        assert_eq!(result.get_pixel(0, 1).0, [0, 0, 0, 0]);
        assert_eq!(result.get_pixel(1, 1).0, [255, 0, 0, 64]);
    }
    #[test]
    fn downsampling_keeps_translucent_edges_saturated() {
        let black = vec![0, 0, 255, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let white = [
            0, 0, 255, 0, 255, 255, 255, 0, 255, 255, 255, 0, 255, 255, 255, 0,
        ];
        let result = icon_pixels(black, &white, 2, 1).unwrap();
        let pixel = result.get_pixel(0, 0).0;
        assert_eq!(&pixel[..3], &[0, 0, 255]);
        assert!((62..=66).contains(&pixel[3]));
    }
    #[test]
    fn native_extraction_preserves_a_known_translucent_icon() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("半透明图标.ico");
        let mut source = image::RgbaImage::new(64, 64);
        for y in 16..48 {
            for x in 16..48 {
                source.put_pixel(x, y, image::Rgba([255, 0, 0, 128]));
            }
        }
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(source)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let png = png.into_inner();
        let mut ico = vec![0, 0, 1, 0, 1, 0, 64, 64, 0, 0, 1, 0, 32, 0];
        ico.extend_from_slice(&(png.len() as u32).to_le_bytes());
        ico.extend_from_slice(&22u32.to_le_bytes());
        ico.extend_from_slice(&png);
        std::fs::write(&path, ico).unwrap();
        for pixels in [16, 24, 48] {
            let icon = extract(path.to_str().unwrap(), pixels).unwrap();
            let bytes = icon.as_bytes(0).unwrap();
            let center = ((pixels / 2 * pixels + pixels / 2) * 4) as usize;
            // Native icon scaling/compositing can round a color by a few 8-bit levels.
            assert_eq!(&bytes[center..center + 2], &[0, 0]);
            assert!(bytes[center + 2] >= 250);
            assert!((120..=136).contains(&bytes[center + 3]));
            assert_eq!(&bytes[..4], &[0, 0, 0, 0]);
        }
    }
    #[test]
    fn refresh_checks_file_versions_without_blanking_or_reextracting_unchanged_icons() {
        let version = Arc::new(AtomicU64::new(1));
        let inspected = version.clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let loaded = calls.clone();
        let icons = AppIcons::with_source(
            move |_, pixels| {
                loaded.fetch_add(1, Ordering::SeqCst);
                Some(fixture(pixels))
            },
            move |_| {
                let version = inspected.load(Ordering::SeqCst);
                (version != 0).then_some(FileStamp {
                    length: version,
                    modified: SystemTime::UNIX_EPOCH,
                })
            },
        );
        let path = r"C:\replace.exe";
        let first = wait_image(&icons, path, 24);
        expire(&icons);
        assert!(Arc::ptr_eq(&first, &icons.image(path, 24).unwrap()));
        settle(&icons);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        version.store(2, Ordering::SeqCst);
        expire(&icons);
        assert!(Arc::ptr_eq(&first, &icons.image(path, 24).unwrap()));
        settle(&icons);
        let second = icons.image(path, 24).unwrap();
        assert!(!Arc::ptr_eq(&first, &second));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        version.store(0, Ordering::SeqCst);
        expire(&icons);
        assert!(Arc::ptr_eq(&second, &icons.image(path, 24).unwrap()));
        settle(&icons);
        assert!(Arc::ptr_eq(&second, &icons.image(path, 24).unwrap()));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        version.store(3, Ordering::SeqCst);
        expire(&icons);
        icons.image(path, 24);
        settle(&icons);
        assert!(!Arc::ptr_eq(&second, &icons.image(path, 24).unwrap()));
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }
    #[test]
    fn an_executable_replaced_during_extraction_is_retried() {
        let version = Arc::new(AtomicU64::new(1));
        let changed = version.clone();
        let inspected = version.clone();
        let icons = AppIcons::with_source(
            move |_, pixels| {
                changed.store(2, Ordering::SeqCst);
                Some(fixture(pixels))
            },
            move |_| {
                Some(FileStamp {
                    length: inspected.load(Ordering::SeqCst),
                    modified: SystemTime::UNIX_EPOCH,
                })
            },
        );
        let path = r"C:\changing.exe";
        assert!(icons.image(path, 24).is_none());
        settle(&icons);
        assert!(icons.image(path, 24).is_none());
        expire(&icons);
        wait_image(&icons, path, 24);
        assert_eq!(
            icons
                .state
                .lock()
                .unwrap()
                .entries
                .values()
                .next()
                .unwrap()
                .stamp
                .unwrap()
                .length,
            2
        );
    }
    #[test]
    fn cache_evicts_old_images_at_both_entry_and_byte_limits() {
        for (pixels, count) in [
            (16, CACHE_LIMIT + 2),
            (128, CACHE_BYTES / (128 * 128 * 4) + 2),
        ] {
            let icons = AppIcons::with_loader(|_, pixels| Some(fixture(pixels)));
            for index in 0..count {
                wait_image(&icons, &format!(r"C:\application-{index}.exe"), pixels);
                let state = icons.state.lock().unwrap();
                assert!(state.entries.len() <= CACHE_LIMIT);
                assert!(state.bytes <= CACHE_BYTES);
                assert_eq!(
                    state.bytes,
                    state
                        .entries
                        .values()
                        .map(|entry| image_bytes(&entry.image))
                        .sum::<usize>()
                );
            }
            let state = icons.state.lock().unwrap();
            assert!(!state.entries.contains_key(&Key {
                path: r"c:\application-0.exe".into(),
                pixels
            }));
            assert!(state.entries.contains_key(&Key {
                path: format!(r"c:\application-{}.exe", count - 1),
                pixels
            }));
        }
    }
    #[test]
    fn ready_images_are_reused_and_sizes_and_paths_remain_distinct() {
        let count = Arc::new(AtomicUsize::new(0));
        let calls = count.clone();
        let icons = AppIcons::with_loader(move |_, pixels| {
            calls.fetch_add(1, Ordering::SeqCst);
            Some(Arc::new(RenderImage::new(vec![image::Frame::new(
                image::RgbaImage::new(pixels, pixels),
            )])))
        });
        let wait = |path: &str, pixels| {
            let until = Instant::now() + Duration::from_secs(2);
            loop {
                if let Some(image) = icons.image(path, pixels) {
                    break image;
                }
                assert!(Instant::now() < until, "icon worker did not complete");
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        let first = wait(r"C:\one\same.exe", 24);
        assert!(Arc::ptr_eq(&first, &wait(r"C:\ONE\same.exe", 24)));
        assert!(!Arc::ptr_eq(&first, &wait(r"D:\two\same.exe", 24)));
        assert!(!Arc::ptr_eq(&first, &wait(r"C:\one\same.exe", 48)));
        assert_eq!(count.load(Ordering::SeqCst), 3);
    }
}
