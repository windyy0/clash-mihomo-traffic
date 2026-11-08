use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ColorTheme {
    #[default]
    CloudBlue,
    SunlitApricot,
    PorcelainLilac,
    SoftWhite,
    #[serde(alias = "unified-white")]
    Mica,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn traffic_direction_text_is_readable_on_each_theme_surface() {
        for theme in ColorTheme::ALL {
            for dark in [false, true] {
                let palette = Palette::new(theme, dark);
                for palette in [palette, palette.main_window(), palette.floating_window()] {
                    for background in [palette.bg, palette.panel] {
                        for color in [palette.download_text, palette.upload_text] {
                            assert!(
                                contrast(color, background) >= 4.5,
                                "traffic text contrast below 4.5: {theme:?} dark={dark}, color={color:x}, background={background:x}"
                            );
                        }
                    }
                }
            }
        }
    }
}

impl ColorTheme {
    pub const ALL: [Self; 5] = [
        Self::Mica,
        Self::CloudBlue,
        Self::SunlitApricot,
        Self::PorcelainLilac,
        Self::SoftWhite,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::CloudBlue => "云白浅蓝",
            Self::SunlitApricot => "日光杏白",
            Self::PorcelainLilac => "瓷白浅紫",
            Self::SoftWhite => "柔白细线",
            Self::Mica => "一体云母",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::CloudBlue => "cloud-blue",
            Self::SunlitApricot => "sunlit-apricot",
            Self::PorcelainLilac => "porcelain-lilac",
            Self::SoftWhite => "soft-white",
            Self::Mica => "mica",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub dark: bool,
    pub mica: bool,
    pub bg: u32,
    pub panel: u32,
    pub side: u32,
    pub text: u32,
    pub muted: u32,
    pub line: u32,
    pub accent: u32,
    pub accent_text: u32,
    pub download: u32,
    pub download_text: u32,
    pub upload: u32,
    pub upload_text: u32,
    pub tint: u32,
    pub wash: u32,
    pub blue: u32,
    pub blue_text: u32,
    pub soft: u32,
    pub amber: u32,
    pub radius: u8,
}

/// Blend a small amount of color into a neutral surface.
fn mix(color: u32, surface: u32, percent: u32) -> u32 {
    let channel = |shift: u32| -> u32 {
        (((color >> shift) & 255u32) * percent + ((surface >> shift) & 255u32) * (100 - percent))
            / 100
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

fn luminance(color: u32) -> f64 {
    let channel = |shift: u32| {
        let value = ((color >> shift) & 255u32) as f64 / 255.;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
}

fn contrast(a: u32, b: u32) -> f64 {
    let a = luminance(a);
    let b = luminance(b);
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

fn readable_accent(color: u32, background: u32, text: u32) -> u32 {
    for percent in (0..=100).step_by(10) {
        let candidate = mix(text, color, percent);
        if contrast(candidate, background) >= 4.5 {
            return candidate;
        }
    }
    text
}

/// Keep the wallpaper hue vivid enough for controls; resolve readable text separately.
fn wallpaper_accent(color: u32, dark: bool, background: u32) -> u32 {
    let r = ((color >> 16) & 255) as f64 / 255.;
    let g = ((color >> 8) & 255) as f64 / 255.;
    let b = (color & 255) as f64 / 255.;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let chroma = max - min;
    let lightness = (max + min) / 2.;
    let saturation = if chroma > 0. {
        chroma / (1. - (2. * lightness - 1.).abs())
    } else {
        0.
    };
    // Near-neutral wallpapers should not acquire a hue from rounding noise.
    if chroma < 0.025 || saturation < 0.04 {
        return if dark { 0xbdbdbd } else { 0x646464 };
    }
    let hue = if max == r {
        ((g - b) / chroma).rem_euclid(6.)
    } else if max == g {
        (b - r) / chroma + 2.
    } else {
        (r - g) / chroma + 4.
    };
    let saturation = (0.32 + saturation * 0.75).clamp(0.40, 0.68);
    let resolve = |lightness: f64| {
        let chroma = (1. - (2. * lightness - 1.).abs()) * saturation;
        let x = chroma * (1. - (hue.rem_euclid(2.) - 1.).abs());
        let (r, g, b) = match hue as u32 {
            0 => (chroma, x, 0.),
            1 => (x, chroma, 0.),
            2 => (0., chroma, x),
            3 => (0., x, chroma),
            4 => (x, 0., chroma),
            _ => (chroma, 0., x),
        };
        let channel = |value: f64| ((value + lightness - chroma / 2.) * 255.).round() as u32;
        (channel(r) << 16) | (channel(g) << 8) | channel(b)
    };
    // Preserve colorfulness when a bright hue needs more contrast against the base surface.
    for step in 0..=50 {
        let lightness = if dark {
            (0.76 + step as f64 * 0.01).min(0.96)
        } else {
            0.58 - step as f64 * 0.01
        };
        let candidate = resolve(lightness);
        if contrast(candidate, background) >= 3. {
            return candidate;
        }
    }
    if dark { 0xededed } else { 0x303030 }
}

impl Palette {
    pub fn new(color: ColorTheme, dark: bool) -> Self {
        use ColorTheme::*;
        let (bg, panel, side, text, muted, accent, accent_text, blue, tint, soft) =
            match (color, dark) {
                (CloudBlue, false) => (
                    0xf8f9fb, 0xffffff, 0xffffff, 0x233349, 0x53657b, 0x4b99dc, 0x286a9f, 0x5b9d90,
                    0xeaf5ff, 0xf8fbfe,
                ),
                (SunlitApricot, false) => (
                    0xfbfaf8, 0xffffff, 0xffffff, 0x3f342a, 0x6d5f52, 0xe1a35f, 0x95602a, 0x638fb8,
                    0xfff2de, 0xfffcf7,
                ),
                (PorcelainLilac, false) => (
                    0xf9f9fc, 0xffffff, 0xffffff, 0x30334f, 0x61647a, 0x9791df, 0x6e63af, 0xb77390,
                    0xf1efff, 0xfbfaff,
                ),
                (SoftWhite, false) => (
                    0xf7f7f6, 0xffffff, 0xf2f2f1, 0x2d3035, 0x626771, 0x738198, 0x526070, 0x8793a4,
                    0xe8e9eb, 0xf8f8f8,
                ),
                (Mica, false) => (
                    0xf3f3f3, 0xffffff, 0xf3f3f3, 0x303030, 0x606060, 0x646464, 0x505050, 0x4c83b8,
                    0xe9e9e9, 0xf6f6f6,
                ),
                (CloudBlue, true) => (
                    0x19212d, 0x222e3d, 0x1d2836, 0xecf3fc, 0xb2c4da, 0x80b8ed, 0xa0cfff, 0x90d2c5,
                    0x2b425a, 0x263548,
                ),
                (SunlitApricot, true) => (
                    0x241e18, 0x302820, 0x27211c, 0xfff1e2, 0xd0bca8, 0xedb57b, 0xf6c995, 0x97c6ef,
                    0x4c3926, 0x382e24,
                ),
                (PorcelainLilac, true) => (
                    0x201e2e, 0x2b283d, 0x252235, 0xf0ecff, 0xbdb6db, 0xbbb1f2, 0xd0c7ff, 0xebb8cd,
                    0x3c3558, 0x312c46,
                ),
                (SoftWhite, true) => (
                    0x1b1d20, 0x23262a, 0x202226, 0xe6e8eb, 0xa1a6af, 0xa6b4c9, 0xc3cede, 0x919cae,
                    0x33373e, 0x2a2d32,
                ),
                (Mica, true) => (
                    0x202020, 0x292929, 0x202020, 0xededed, 0xb5b5b5, 0xbdbdbd, 0xd0d0d0, 0x7fb4eb,
                    0x3c3c3c, 0x2e2e2e,
                ),
            };
        Self {
            dark,
            mica: color == Mica,
            bg,
            panel,
            side,
            text,
            muted,
            accent,
            accent_text,
            download: if dark { 0x78b0ff } else { 0x3b82f6 },
            download_text: if dark { 0xa6c8ff } else { 0x2563b8 },
            upload: if dark { 0x66d8ad } else { 0x2db78b },
            upload_text: if dark { 0x9ee7c8 } else { 0x1c775a },
            blue,
            blue_text: if dark {
                blue
            } else {
                match color {
                    CloudBlue => 0x2d7167,
                    SunlitApricot => 0x356f9f,
                    PorcelainLilac => 0x97516f,
                    SoftWhite => 0x58677b,
                    Mica => 0x346b9c,
                }
            },
            tint,
            soft,
            line: mix(text, panel, if dark { 15 } else { 13 }),
            wash: mix(accent, panel, if dark { 8 } else { 5 }),
            amber: if dark { 0xe9bd84 } else { 0x9b652f },
            radius: match color {
                SunlitApricot => 12,
                PorcelainLilac | SoftWhite => 9,
                CloudBlue | Mica => 10,
            },
        }
    }

    pub fn with_wallpaper_accent(mut self, color: Option<u32>) -> Self {
        if self.mica
            && let Some(color) = color
        {
            self.accent = wallpaper_accent(color, self.dark, self.bg);
            self.accent_text = readable_accent(self.accent, self.bg, self.text);
            self.tint = mix(self.accent, self.bg, if self.dark { 18 } else { 12 });
        }
        self
    }

    pub fn accent_foreground(self) -> u32 {
        if contrast(self.accent, 0xffffff) >= contrast(self.accent, 0) {
            0xffffff
        } else {
            0
        }
    }

    pub fn accent_hover(self) -> u32 {
        let shade = if self.accent_foreground() == 0xffffff {
            0
        } else {
            0xffffff
        };
        mix(shade, self.accent, 10)
    }

    pub fn switch_thumb(self) -> u32 {
        if contrast(self.accent, 0xffffff).min(contrast(self.accent_hover(), 0xffffff)) >= 3. {
            0xffffff
        } else {
            0x202020
        }
    }

    pub fn floating_window(mut self) -> Self {
        self.panel = if self.mica {
            mix(
                self.accent,
                if self.dark { self.bg } else { 0xfafafa },
                if self.dark { 12 } else { 8 },
            )
        } else {
            mix(self.tint, self.bg, if self.dark { 35 } else { 25 })
        };
        self.line = mix(self.text, self.panel, if self.dark { 15 } else { 10 });
        self
    }

    /// Resolve main-window surfaces separately from the native taskbar and floating window.
    pub fn main_window(mut self) -> Self {
        self.panel = self.bg;
        self.soft = self.bg;
        self.line = mix(self.text, self.panel, if self.dark { 10 } else { 6 });
        self.wash = self.bg;
        self
    }
}
