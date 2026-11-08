//! A rounded wallpaper backdrop below GPUI's foreground composition target.
//! Unlike a DWM window backdrop, this visual can use the panel's exact radius.
use windows::{
    System::{DispatcherQueue, DispatcherQueueController},
    UI::Composition::{
        CompositionBackdropBrush, CompositionRoundedRectangleGeometry, Compositor, ContainerVisual,
        Desktop::DesktopWindowTarget, SpriteVisual,
    },
    Win32::{
        Foundation::HWND,
        System::WinRT::{
            Composition::ICompositorDesktopInterop, CreateDispatcherQueueController,
            DQTAT_COM_NONE, DQTYPE_THREAD_CURRENT, DispatcherQueueOptions, RO_INIT_SINGLETHREADED,
            RoInitialize, RoUninitialize,
        },
    },
    core::Interface,
};
use windows_numerics::{Vector2, Vector3};

pub const RADIUS: f32 = 14.;
pub const SHADOW_MARGIN: f32 = 8.;

thread_local! {
    static RUNTIME: std::cell::RefCell<Option<Runtime>> = const { std::cell::RefCell::new(None) };
}

struct Runtime {
    queue: Option<DispatcherQueueController>,
}
impl Drop for Runtime {
    fn drop(&mut self) {
        if let Some(queue) = &self.queue {
            let _ = queue.ShutdownQueueAsync();
        }
        unsafe {
            RoUninitialize();
        }
    }
}

fn ensure_runtime() -> windows::core::Result<()> {
    RUNTIME.with(|slot| {
        if slot.borrow().is_some() {
            return Ok(());
        }
        unsafe {
            RoInitialize(RO_INIT_SINGLETHREADED)?;
        }
        let mut runtime = Runtime { queue: None };
        if DispatcherQueue::GetForCurrentThread().is_err() {
            runtime.queue = Some(unsafe {
                CreateDispatcherQueueController(DispatcherQueueOptions {
                    dwSize: std::mem::size_of::<DispatcherQueueOptions>() as u32,
                    threadType: DQTYPE_THREAD_CURRENT,
                    apartmentType: DQTAT_COM_NONE,
                })?
            });
        }
        *slot.borrow_mut() = Some(runtime);
        Ok(())
    })
}

pub struct FloatingBackdrop {
    target: DesktopWindowTarget,
    root: ContainerVisual,
    wallpaper: SpriteVisual,
    source: CompositionBackdropBrush,
    geometry: CompositionRoundedRectangleGeometry,
    compositor: Compositor,
    size: (u32, u32, u32, bool),
    brush_theme: Option<bool>,
}

impl FloatingBackdrop {
    pub fn new(hwnd: isize) -> windows::core::Result<Self> {
        ensure_runtime()?;
        let compositor = Compositor::new()?;
        let interop: ICompositorDesktopInterop = compositor.cast()?;
        // GPUI owns the above-children target. Use the separate lower target.
        let target = unsafe { interop.CreateDesktopWindowTarget(HWND(hwnd as _), false)? };
        let root = compositor.CreateContainerVisual()?;
        let geometry = compositor.CreateRoundedRectangleGeometry()?;
        root.SetClip(&compositor.CreateGeometricClipWithGeometry(&geometry)?)?;
        let wallpaper = compositor.CreateSpriteVisual()?;
        let source = compositor.TryCreateBlurredWallpaperBackdropBrush()?;
        root.Children()?.InsertAtTop(&wallpaper)?;
        target.SetRoot(&root)?;
        Ok(Self {
            target,
            root,
            wallpaper,
            source,
            geometry,
            compositor,
            size: (0, 0, 0, false),
            brush_theme: None,
        })
    }

    pub fn resize(
        &mut self,
        width: f32,
        height: f32,
        scale: f32,
        dark: bool,
    ) -> windows::core::Result<()> {
        let key = (width.to_bits(), height.to_bits(), scale.to_bits(), dark);
        if self.size == key {
            return Ok(());
        }
        let inset = SHADOW_MARGIN * scale;
        let size = Vector2 {
            X: (width * scale - 2. * inset).max(1.),
            Y: (height * scale - 2. * inset).max(1.),
        };
        self.root.SetOffset(Vector3 {
            X: inset,
            Y: inset,
            Z: 0.,
        })?;
        self.root.SetSize(size)?;
        self.geometry.SetSize(size)?;
        self.geometry.SetCornerRadius(Vector2 {
            X: RADIUS * scale,
            Y: RADIUS * scale,
        })?;
        self.wallpaper.SetSize(size)?;
        if self.brush_theme != Some(dark) {
            self.wallpaper.SetBrush(&crate::mica_effect::brush(
                &self.compositor,
                &self.source.cast()?,
                dark,
            )?)?;
            self.brush_theme = Some(dark);
        }
        let _ = self.compositor.RequestCommitAsync()?;
        self.size = key;
        Ok(())
    }

    #[cfg(test)]
    pub fn test_source_color(&self, color: u32, dark: bool) -> windows::core::Result<()> {
        let source = self
            .compositor
            .CreateColorBrushWithColor(windows::UI::Color {
                A: 255,
                R: (color >> 16) as u8,
                G: (color >> 8) as u8,
                B: color as u8,
            })?;
        self.wallpaper.SetBrush(&crate::mica_effect::brush(
            &self.compositor,
            &source.cast()?,
            dark,
        )?)?;
        let _ = self.compositor.RequestCommitAsync()?;
        Ok(())
    }
}

impl Drop for FloatingBackdrop {
    fn drop(&mut self) {
        let _ = self.target.Close();
    }
}
