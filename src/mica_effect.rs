//! Wallpaper luminosity and color blends, without an opaque surface wash.
#![allow(non_snake_case)]
use windows::{
    Foundation::{IPropertyValue, PropertyValue},
    Graphics::Effects::{
        IGraphicsEffect, IGraphicsEffect_Impl, IGraphicsEffectSource, IGraphicsEffectSource_Impl,
    },
    UI::Composition::{
        CompositionBrush, CompositionEffectBrush, CompositionEffectSourceParameter, Compositor,
    },
    Win32::{
        Foundation::E_INVALIDARG,
        Graphics::Direct2D::{
            CLSID_D2D1Blend, CLSID_D2D1Flood,
            Common::{D2D1_BLEND_MODE_COLOR, D2D1_BLEND_MODE_LUMINOSITY},
        },
        System::WinRT::Graphics::Direct2D::{
            GRAPHICS_EFFECT_PROPERTY_MAPPING, IGraphicsEffectD2D1Interop,
            IGraphicsEffectD2D1Interop_Impl,
        },
    },
    core::{Error, GUID, HSTRING, Interface, PCWSTR, Result, implement},
};

enum Value {
    Color([f32; 4]),
    Mode(u32),
}

#[implement(IGraphicsEffect, IGraphicsEffectSource, IGraphicsEffectD2D1Interop)]
struct Effect {
    id: GUID,
    value: Value,
    sources: Vec<IGraphicsEffectSource>,
    name: std::sync::Mutex<HSTRING>,
}

impl Effect {
    fn make(id: GUID, value: Value, sources: Vec<IGraphicsEffectSource>) -> IGraphicsEffectSource {
        let effect: IGraphicsEffect = Self {
            id,
            value,
            sources,
            name: Default::default(),
        }
        .into();
        effect
            .cast()
            .expect("effect implements IGraphicsEffectSource")
    }
    fn color(channel: f32, opacity: f32) -> IGraphicsEffectSource {
        Self::make(
            CLSID_D2D1Flood,
            Value::Color([channel, channel, channel, opacity]),
            vec![],
        )
    }
    fn blend(
        mode: u32,
        foreground: IGraphicsEffectSource,
        background: IGraphicsEffectSource,
    ) -> IGraphicsEffectSource {
        Self::make(
            CLSID_D2D1Blend,
            Value::Mode(mode),
            vec![foreground, background],
        )
    }
}
impl IGraphicsEffectSource_Impl for Effect_Impl {}
impl IGraphicsEffect_Impl for Effect_Impl {
    fn Name(&self) -> Result<HSTRING> {
        Ok(self.name.lock().unwrap().clone())
    }
    fn SetName(&self, name: &HSTRING) -> Result<()> {
        *self.name.lock().unwrap() = name.clone();
        Ok(())
    }
}
impl IGraphicsEffectD2D1Interop_Impl for Effect_Impl {
    fn GetEffectId(&self) -> Result<GUID> {
        Ok(self.id)
    }
    fn GetNamedPropertyMapping(
        &self,
        _: &PCWSTR,
        _: *mut u32,
        _: *mut GRAPHICS_EFFECT_PROPERTY_MAPPING,
    ) -> Result<()> {
        Err(Error::from(E_INVALIDARG))
    }
    fn GetPropertyCount(&self) -> Result<u32> {
        Ok(1)
    }
    fn GetProperty(&self, index: u32) -> Result<IPropertyValue> {
        if index != 0 {
            return Err(Error::from(E_INVALIDARG));
        }
        match self.value {
            Value::Color(color) => PropertyValue::CreateSingleArray(&color)?.cast(),
            Value::Mode(mode) => PropertyValue::CreateUInt32(mode)?.cast(),
        }
    }
    fn GetSource(&self, index: u32) -> Result<IGraphicsEffectSource> {
        self.sources
            .get(index as usize)
            .cloned()
            .ok_or_else(|| Error::from(E_INVALIDARG))
    }
    fn GetSourceCount(&self) -> Result<u32> {
        Ok(self.sources.len() as u32)
    }
}

pub fn brush(
    compositor: &Compositor,
    wallpaper: &CompositionBrush,
    dark: bool,
) -> Result<CompositionEffectBrush> {
    let source: IGraphicsEffectSource =
        CompositionEffectSourceParameter::Create(&HSTRING::from("Wallpaper"))?.cast()?;
    let channel = if dark { 32. / 255. } else { 243. / 255. };
    // Direct2D's first input is the foreground. Apply wallpaper hue at
    // the theme's luminosity, then blend in the neutral theme tint.
    let luminosity = Effect::blend(
        D2D1_BLEND_MODE_COLOR.0 as u32,
        source,
        Effect::color(channel, 1.),
    );
    let effect: IGraphicsEffect = Effect::blend(
        D2D1_BLEND_MODE_LUMINOSITY.0 as u32,
        luminosity,
        Effect::color(channel, if dark { 0.8 } else { 0.5 }),
    )
    .cast()?;
    let brush = compositor.CreateEffectFactory(&effect)?.CreateBrush()?;
    brush.SetSourceParameter(&HSTRING::from("Wallpaper"), wallpaper)?;
    Ok(brush)
}
