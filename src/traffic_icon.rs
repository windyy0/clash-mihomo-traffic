/// Shared rounded solid arrows for the native taskbar and GPUI readouts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectionIcon {
    Upload,
    Download,
    TotalSpeed,
}

#[derive(Clone, Copy)]
pub struct IconPoint {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy)]
pub struct RoundedCorner {
    pub start: IconPoint,
    pub control: IconPoint,
    pub end: IconPoint,
}

pub const ICON_VIEWBOX: f32 = 24.;
// Arrow dimensions in the shared viewbox, scaled with the font size.
const SHAFT_WIDTH: f32 = 2.5;
const SHAFT_HEIGHT: f32 = 12.;
const HEAD_WIDTH: f32 = 10.;
const HEAD_HEIGHT: f32 = 6.;
const CORNER_CUT: f32 = 1.2;

impl DirectionIcon {
    pub fn asset_path(self) -> &'static str {
        match self {
            Self::Upload => "icons/traffic-upload.svg",
            Self::Download => "icons/traffic-download.svg",
            Self::TotalSpeed => "icons/traffic-total.svg",
        }
    }

    /// Keep GPUI's small-icon SVG rasterizer and Direct2D on the exact same
    /// rounded outlines. The SVG is cached by GPUI at each size and DPI.
    pub fn svg(self) -> Vec<u8> {
        use std::fmt::Write;
        let mut path = String::new();
        for outline in self.outlines() {
            write!(path, "M{} {}", outline[0].start.x, outline[0].start.y).unwrap();
            for corner in outline {
                write!(
                    path,
                    "L{} {}Q{} {} {} {}",
                    corner.start.x,
                    corner.start.y,
                    corner.control.x,
                    corner.control.y,
                    corner.end.x,
                    corner.end.y
                )
                .unwrap();
            }
            path.push('Z');
        }
        format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24"><path fill="black" d="{path}"/></svg>"#).into_bytes()
    }

    pub fn index(self) -> usize {
        match self {
            Self::Upload => 0,
            Self::Download => 1,
            Self::TotalSpeed => 2,
        }
    }

    /// The combined metric reuses the same arrows, arranged like two opposing
    /// lanes: upward on the left, downward on the right, slightly staggered.
    pub fn outlines(self) -> impl Iterator<Item = [RoundedCorner; 7]> {
        let top = (ICON_VIEWBOX - HEAD_HEIGHT - SHAFT_HEIGHT) / 2.;
        let outlines = match self {
            Self::Upload => [Some(Self::arrow(12., top, HEAD_WIDTH, false)), None],
            Self::Download => [Some(Self::arrow(12., top, HEAD_WIDTH, true)), None],
            Self::TotalSpeed => [
                Some(Self::arrow(6., 2., HEAD_WIDTH, false)),
                Some(Self::arrow(18., 2., HEAD_WIDTH, true)),
            ],
        };
        outlines.into_iter().flatten()
    }

    fn arrow(center: f32, top: f32, head_width: f32, down: bool) -> [RoundedCorner; 7] {
        let shaft_left = center - SHAFT_WIDTH / 2.;
        let shaft_right = center + SHAFT_WIDTH / 2.;
        let head_base = top + HEAD_HEIGHT;
        let bottom = head_base + SHAFT_HEIGHT;
        let arrow = [
            (center, top),
            (center + head_width / 2., head_base),
            (shaft_right, head_base),
            (shaft_right, bottom),
            (shaft_left, bottom),
            (shaft_left, head_base),
            (center - head_width / 2., head_base),
        ];
        Self::rounded(arrow, CORNER_CUT, down)
    }

    fn rounded<const N: usize>(
        vertices: [(f32, f32); N],
        cut: f32,
        down: bool,
    ) -> [RoundedCorner; N] {
        let transform = |(x, y)| IconPoint {
            x,
            y: if down { ICON_VIEWBOX - y } else { y },
        };
        std::array::from_fn(|index| {
            let vertex = vertices[index];
            let towards = |other: (f32, f32)| {
                let dx = other.0 - vertex.0;
                let dy = other.1 - vertex.1;
                let fraction = cut / dx.hypot(dy);
                transform((vertex.0 + dx * fraction, vertex.1 + dy * fraction))
            };
            RoundedCorner {
                start: towards(vertices[(index + vertices.len() - 1) % vertices.len()]),
                control: transform(vertex),
                end: towards(vertices[(index + 1) % vertices.len()]),
            }
        })
    }
}
