use crate::{
    model::Shared,
    taskbar_content::{Metric, TaskbarContent, TaskbarFontWeight, TaskbarItem},
    traffic_icon::{DirectionIcon, ICON_VIEWBOX},
};
use windows::{
    Win32::{
        Foundation::RECT,
        Graphics::{
            Direct2D::{
                Common::{
                    D2D_RECT_F, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F,
                    D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_END_CLOSED, D2D1_PIXEL_FORMAT,
                },
                D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_FACTORY_TYPE_SINGLE_THREADED,
                D2D1_QUADRATIC_BEZIER_SEGMENT, D2D1_RENDER_TARGET_PROPERTIES,
                D2D1_RENDER_TARGET_TYPE_SOFTWARE, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
                D2D1CreateFactory, ID2D1DCRenderTarget, ID2D1Factory, ID2D1PathGeometry,
                ID2D1SolidColorBrush,
            },
            DirectWrite::{
                DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_WEIGHT, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_MEASURING_MODE_NATURAL,
                DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_PIXEL_GEOMETRY_FLAT,
                DWRITE_RENDERING_MODE_NATURAL, DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_TEXT_METRICS,
                DWRITE_WORD_WRAPPING_NO_WRAP, DWriteCreateFactory, IDWriteFactory,
                IDWriteTextFormat,
            },
            Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
            Gdi::HDC,
        },
    },
    core::{PCWSTR, Result, w},
};

// The preview bar is 56 DIP tall; the standard taskbar is 48 DIP tall.
// Keep their text proportions similar instead of sharing the same font size.
pub const FONT_SIZE: f32 = 12.;
pub const PREVIEW_FONT_SIZE: f32 = 14.;
pub const PREVIEW_SCALE: f32 = PREVIEW_FONT_SIZE / FONT_SIZE;
#[cfg(test)]
pub const LINE_HEIGHT: f32 = 16.;
pub const LAYOUT_VERSION: u8 = 3;
const TEXT_PADDING: f32 = 2.;
// A cell has no outer text padding. Leave a DIP for fractional-DPI glyph
// coverage so DrawText's cell clip cannot cut off the last antialiased pixel.
const CELL_RASTER_PADDING: f32 = 1.;

fn rgb_color(color: u32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: ((color >> 16) & 255) as f32 / 255.,
        g: ((color >> 8) & 255) as f32 / 255.,
        b: (color & 255) as f32 / 255.,
        a: 1.,
    }
}

pub fn content(horizontal: bool, down: &str, up: &str) -> String {
    if horizontal {
        format!("↓ {down}   ↑ {up}")
    } else {
        format!("↓  {down}\n↑  {up}")
    }
}

fn text_format(
    factory: &IDWriteFactory,
    family: &str,
    size: f32,
    weight: DWRITE_FONT_WEIGHT,
) -> Result<IDWriteTextFormat> {
    let family: Vec<u16> = family.encode_utf16().chain([0]).collect();
    unsafe {
        let format = factory.CreateTextFormat(
            PCWSTR(family.as_ptr()),
            None,
            weight,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            w!("zh-CN"),
        )?;
        format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        Ok(format)
    }
}

fn measure(factory: &IDWriteFactory, format: &IDWriteTextFormat, text: &[u16]) -> Result<f32> {
    unsafe {
        let layout = factory.CreateTextLayout(text, format, 4096., 4096.)?;
        let mut metrics = DWRITE_TEXT_METRICS::default();
        layout.GetMetrics(&mut metrics)?;
        let overhang = layout.GetOverhangMetrics()?;
        Ok(metrics.widthIncludingTrailingWhitespace + overhang.right.max(0.) + TEXT_PADDING * 2.)
    }
}

/// Separate trailing punctuation for every metric, including CPU and memory.
pub fn split_prefix(text: &str) -> Option<(&str, &str)> {
    text.strip_suffix(':')
        .or_else(|| text.strip_suffix('：'))
        .map(|stem| (stem, &text[stem.len()..]))
}

struct PrefixLayout {
    stem: Vec<u16>,
    colon: Vec<u16>,
    stem_width: f32,
    colon_width: f32,
    stem_center_offset: f32,
    colon_center_offset: f32,
}

fn prefix_layout(
    factory: &IDWriteFactory,
    format: &IDWriteTextFormat,
    text: &str,
    font_size: f32,
) -> Result<Option<PrefixLayout>> {
    let Some((stem, colon)) = split_prefix(text) else {
        return Ok(None);
    };
    let stem: Vec<u16> = stem.encode_utf16().collect();
    let colon: Vec<u16> = colon.encode_utf16().collect();
    let measure_part = |text: &[u16]| -> Result<(f32, f32)> {
        if text.is_empty() {
            return Ok((0., 0.));
        }
        unsafe {
            let layout = factory.CreateTextLayout(text, format, 4096., font_size + 4.)?;
            let mut metrics = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut metrics)?;
            let ink = layout.GetOverhangMetrics()?;
            Ok((
                metrics.widthIncludingTrailingWhitespace + ink.right.max(0.),
                (ink.bottom - ink.top) / 2.,
            ))
        }
    };
    let (stem_width, stem_center_offset) = measure_part(&stem)?;
    let (colon_width, colon_center_offset) = measure_part(&colon)?;
    Ok(Some(PrefixLayout {
        stem,
        colon,
        stem_width,
        colon_width,
        stem_center_offset,
        colon_center_offset,
    }))
}

fn reserved_widths(factory: &IDWriteFactory, format: &IDWriteTextFormat) -> Result<[f32; 2]> {
    let mut widths = [0f32; 2];
    // Include the rounded 1024.0 boundary before each unit transition. The
    // highest unit can exceed this range and is handled by grow-only fitting.
    for sample in [
        "1023 B/s",
        "1024.0 KB/s",
        "1024.0 MB/s",
        "1024.0 GB/s",
        "1024.0 TB/s",
        "离线",
        "—",
    ] {
        for horizontal in [false, true] {
            let text: Vec<_> = content(horizontal, sample, sample).encode_utf16().collect();
            let index = usize::from(horizontal);
            widths[index] = widths[index].max(measure(factory, format, &text)?);
        }
    }
    Ok(widths)
}

fn measured_widths() -> Result<[f32; 2]> {
    let factory: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
    let format = text_format(&factory, "Segoe UI", 12., DWRITE_FONT_WEIGHT_NORMAL)?;
    reserved_widths(&factory, &format)
}

pub fn reserved_width(horizontal: bool) -> Result<f32> {
    Ok(measured_widths()?[usize::from(horizontal)])
}

pub fn reserved_content_width(content: &TaskbarContent) -> Result<f32> {
    Ok(TaskbarText::new()?
        .frame(content, &Shared::default())?
        .width)
}

#[derive(Clone, Debug)]
pub struct ContentCell {
    pub metric: Metric,
    pub text: String,
    pub x: f32,
    pub width: f32,
    pub segments: Vec<ContentSegment>,
    pub text_color: Option<u32>,
    pub background_color: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct ContentSegment {
    pub text: String,
    pub icon: Option<DirectionIcon>,
    pub x: f32,
    pub width: f32,
    pub right_aligned: bool,
}

#[derive(Clone, Copy, Debug, Default)]
struct PartWidths([f32; 3]);
impl PartWidths {
    fn merge(&mut self, other: Self) {
        for (width, other) in self.0.iter_mut().zip(other.0) {
            *width = width.max(other);
        }
    }
    fn total(self, prefix_gap: u8) -> f32 {
        self.0.iter().sum::<f32>()
            + if self.0[0] > 0. {
                f32::from(prefix_gap)
            } else {
                0.
            }
            + if self.0[2] > 0. { 3. } else { 0. }
    }
}

fn value_parts(value: &str) -> (&str, &str) {
    let start = value
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_ascii_alphabetic() || *c == '/' || *c == '%')
        .last()
        .map(|(index, _)| index)
        .unwrap_or(value.len());
    (value[..start].trim_end(), &value[start..])
}

#[derive(Clone, Debug)]
pub struct ContentFrame {
    pub rows: Vec<Vec<ContentCell>>,
    pub width: f32,
    pub font_family: String,
    pub font_size: f32,
    pub font_weight: TaskbarFontWeight,
    pub line_height: f32,
}

impl ContentFrame {
    pub fn metric_frame(&self, metric: Metric) -> Option<Self> {
        let mut cell = self
            .rows
            .iter()
            .flatten()
            .find(|cell| cell.metric == metric)?
            .clone();
        let shift = TEXT_PADDING - cell.x;
        cell.x = TEXT_PADDING;
        for segment in &mut cell.segments {
            segment.x += shift;
        }
        Some(Self {
            width: cell.width + TEXT_PADDING * 2.,
            rows: vec![vec![cell]],
            font_family: self.font_family.clone(),
            font_size: self.font_size,
            font_weight: self.font_weight,
            line_height: self.line_height,
        })
    }
}

/// Reuse DirectWrite resources and the same layout calculation as Explorer's
/// child window, with COM resources owned by the UI thread that created them.
#[cfg(test)]
pub fn preview_frame(content: &TaskbarContent, data: &Shared) -> ContentFrame {
    thread_local! {
        static RENDERER: std::cell::RefCell<Option<TaskbarText>> = const { std::cell::RefCell::new(None) };
    }
    RENDERER.with(|cache| cached_preview_frame(cache, content, data))
}

/// One independent grow-only cache per display/preview, all owned by the UI thread.
#[derive(Default)]
pub struct ContentMeasure(std::cell::RefCell<Option<TaskbarText>>);

impl ContentMeasure {
    pub fn frame(&self, content: &TaskbarContent, data: &Shared) -> ContentFrame {
        cached_preview_frame(&self.0, content, data)
    }
}

fn cached_preview_frame(
    cache: &std::cell::RefCell<Option<TaskbarText>>,
    content: &TaskbarContent,
    data: &Shared,
) -> ContentFrame {
    let mut cache = cache.borrow_mut();
    if cache.is_none() {
        *cache = TaskbarText::new().ok();
    }
    if let Some(renderer) = cache.as_mut()
        && let Ok(frame) = renderer.frame(content, data)
    {
        return frame;
    }
    // Keep the editor usable even if DirectWrite initialization failed.
    let mut widths = std::collections::HashMap::new();
    for item in content.rows.iter().flatten() {
        let mut parts = PartWidths::default();
        for value in item.samples().into_iter().chain([item.value(data)]) {
            let (number, unit) = value_parts(&value);
            let measure = |text: &str| {
                if text.is_empty() {
                    0.
                } else {
                    text.chars().count() as f32 * content.font_size * 0.75 + CELL_RASTER_PADDING
                }
            };
            parts.merge(PartWidths([
                if item.direction_icon().is_some() {
                    content.font_size + measure(&item.prefix_text())
                } else {
                    measure(&item.prefix_text())
                },
                measure(number),
                measure(unit),
            ]));
        }
        widths.insert(item.metric, parts);
    }
    arrange(content, data, &widths)
}

/// Inspect an existing cell without losing its shared column reservations.
/// Drafts use a separate cache so they cannot reset the main preview's widths.
#[cfg(test)]
pub fn example_frame(content: &TaskbarContent, data: &Shared, item: &TaskbarItem) -> ContentFrame {
    let frame = if content.item(item.metric).is_some() {
        preview_frame(content, data)
    } else {
        let mut draft = content.clone();
        draft.add(item.metric);
        *draft.item_mut(item.metric).unwrap() = item.clone();
        thread_local! {
            static EXAMPLE_RENDERER: std::cell::RefCell<Option<TaskbarText>> = const { std::cell::RefCell::new(None) };
        }
        EXAMPLE_RENDERER.with(|cache| cached_preview_frame(cache, &draft, data))
    };
    frame
        .metric_frame(item.metric)
        .expect("example includes its inspected metric")
}

fn arrange(
    content: &TaskbarContent,
    data: &Shared,
    widths: &std::collections::HashMap<Metric, PartWidths>,
) -> ContentFrame {
    let columns = content.rows.iter().map(Vec::len).max().unwrap_or(0);
    let column_widths: Vec<PartWidths> = (0..columns)
        .map(|column| {
            let mut parts = PartWidths::default();
            for row in &content.rows {
                if let Some(item) = row.get(column) {
                    parts.merge(widths[&item.metric]);
                }
            }
            parts
        })
        .collect();
    let rows = content
        .rows
        .iter()
        .map(|row| {
            let mut x = TEXT_PADDING;
            row.iter()
                .enumerate()
                .map(|(column, item)| {
                    let value = item.value(data);
                    let (number, unit) = value_parts(&value);
                    let parts = column_widths[column];
                    let number_x = x
                        + parts.0[0]
                        + if parts.0[0] > 0. {
                            f32::from(content.prefix_gap)
                        } else {
                            0.
                        };
                    let unit_x = number_x + parts.0[1] + if parts.0[2] > 0. { 3. } else { 0. };
                    let cell = ContentCell {
                        metric: item.metric,
                        text: item.text(&value, content.rows.len() == 1),
                        x,
                        width: parts.total(content.prefix_gap),
                        text_color: item.text_color,
                        background_color: item.background_color,
                        segments: vec![
                            ContentSegment {
                                text: item.prefix_text().into_owned(),
                                icon: item.direction_icon(),
                                x,
                                width: parts.0[0],
                                right_aligned: false,
                            },
                            ContentSegment {
                                text: number.into(),
                                icon: None,
                                x: number_x,
                                width: parts.0[1],
                                right_aligned: true,
                            },
                            ContentSegment {
                                text: unit.into(),
                                icon: None,
                                x: unit_x,
                                width: parts.0[2],
                                right_aligned: false,
                            },
                        ],
                    };
                    x += parts.total(content.prefix_gap) + f32::from(content.gap);
                    cell
                })
                .collect()
        })
        .collect();
    let width = TEXT_PADDING * 2.
        + column_widths
            .iter()
            .map(|parts| parts.total(content.prefix_gap))
            .sum::<f32>()
        + f32::from(content.gap) * columns.saturating_sub(1) as f32;
    ContentFrame {
        rows,
        width,
        font_family: content.font_family.clone(),
        font_size: content.font_size,
        font_weight: content.font_weight,
        line_height: (content.font_size + 4.).max(16.),
    }
}

/// Version 2 reserved each complete label/value string. Keep that measurement
/// solely for migrating a nonzero right-anchored manual coordinate.
pub fn previous_content_width(content: &TaskbarContent) -> Result<f32> {
    let renderer = TaskbarText::new()?;
    let format = text_format(
        &renderer.factory,
        "Segoe UI",
        12.,
        DWRITE_FONT_WEIGHT_NORMAL,
    )?;
    let horizontal = content.rows.len() == 1;
    let mut widths = std::collections::HashMap::new();
    for item in content.rows.iter().flatten() {
        let mut width = 0f32;
        for value in item
            .samples()
            .into_iter()
            .chain([item.value(&Shared::default())])
        {
            // This is the historical width before network-prefix punctuation.
            let text = if item.label.is_empty() {
                value
            } else {
                format!(
                    "{}{space}{value}",
                    item.label,
                    space = if horizontal { " " } else { "  " }
                )
            };
            let text: Vec<u16> = text.encode_utf16().collect();
            width = width.max(
                measure(&renderer.factory, &format, &text)? - TEXT_PADDING * 2.
                    + CELL_RASTER_PADDING,
            );
        }
        widths.insert(item.metric, width);
    }
    let columns = content.rows.iter().map(Vec::len).max().unwrap_or(0);
    let total = (0..columns)
        .map(|column| {
            content
                .rows
                .iter()
                .filter_map(|row| row.get(column))
                .map(|item| widths[&item.metric])
                .fold(0., f32::max)
        })
        .sum::<f32>();
    Ok(TEXT_PADDING * 2. + total + f32::from(content.gap) * columns.saturating_sub(1) as f32)
}

fn icon_geometry(factory: &ID2D1Factory, icon: DirectionIcon) -> Result<ID2D1PathGeometry> {
    unsafe {
        let geometry = factory.CreatePathGeometry()?;
        let sink = geometry.Open()?;
        let point =
            |p: crate::traffic_icon::IconPoint| windows_numerics::Vector2 { X: p.x, Y: p.y };
        for outline in icon.outlines() {
            sink.BeginFigure(point(outline[0].start), D2D1_FIGURE_BEGIN_FILLED);
            for corner in outline {
                sink.AddLine(point(corner.start));
                sink.AddQuadraticBezier(&D2D1_QUADRATIC_BEZIER_SEGMENT {
                    point1: point(corner.control),
                    point2: point(corner.end),
                });
            }
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
        }
        sink.Close()?;
        Ok(geometry)
    }
}

/// Shell-thread-owned drawing resources, including cached vector icons.
/// Direct2D writes antialiased, premultiplied pixels directly.
pub struct TaskbarText {
    target: ID2D1DCRenderTarget,
    factory: IDWriteFactory,
    format: IDWriteTextFormat,
    right_format: IDWriteTextFormat,
    brush: ID2D1SolidColorBrush,
    icons: [ID2D1PathGeometry; 3],
    prefix_layouts: std::collections::HashMap<String, PrefixLayout>,
    #[cfg(test)]
    widths: [f32; 2],
    #[cfg(test)]
    last_text: Option<(bool, Vec<u16>)>,
    content: Option<TaskbarContent>,
    cell_widths: std::collections::HashMap<Metric, PartWidths>,
    cell_texts: std::collections::HashMap<Metric, String>,
}

impl TaskbarText {
    pub fn new() -> Result<Self> {
        unsafe {
            let factory: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let target = factory.CreateDCRenderTarget(&D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.,
                dpiY: 96.,
                ..Default::default()
            })?;
            target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            let write_factory: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let defaults = write_factory.CreateRenderingParams()?;
            let rendering = write_factory.CreateCustomRenderingParams(
                defaults.GetGamma(),
                defaults.GetEnhancedContrast(),
                0.,
                DWRITE_PIXEL_GEOMETRY_FLAT,
                DWRITE_RENDERING_MODE_NATURAL,
            )?;
            // Keep small taskbar text vertically grid-fitted. Symmetric
            // smoothing softens horizontal strokes as the size increases;
            // natural mode retains fractional sizing and horizontal advances.
            target.SetTextRenderingParams(&rendering);
            let format = text_format(&write_factory, "Segoe UI", 12., DWRITE_FONT_WEIGHT_NORMAL)?;
            let right_format =
                text_format(&write_factory, "Segoe UI", 12., DWRITE_FONT_WEIGHT_NORMAL)?;
            right_format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_TRAILING)?;
            #[cfg(test)]
            let widths = reserved_widths(&write_factory, &format)?;
            let brush = target.CreateSolidColorBrush(&D2D1_COLOR_F::default(), None)?;
            let icons = [
                icon_geometry(&factory, DirectionIcon::Upload)?,
                icon_geometry(&factory, DirectionIcon::Download)?,
                icon_geometry(&factory, DirectionIcon::TotalSpeed)?,
            ];
            Ok(Self {
                target,
                factory: write_factory,
                format,
                right_format,
                brush,
                icons,
                prefix_layouts: Default::default(),
                #[cfg(test)]
                widths,
                #[cfg(test)]
                last_text: None,
                content: None,
                cell_widths: Default::default(),
                cell_texts: Default::default(),
            })
        }
    }

    #[cfg(test)]
    pub fn width(&mut self, horizontal: bool, text: &[u16]) -> Result<f32> {
        let index = usize::from(horizontal);
        if self
            .last_text
            .as_ref()
            .is_none_or(|(previous_layout, previous)| {
                *previous_layout != horizontal || previous != text
            })
        {
            // Ordinary speed changes stay within the measured reservation. Only
            // unusually long values grow it, and falling speeds never shrink it.
            self.widths[index] =
                self.widths[index].max(measure(&self.factory, &self.format, text)?);
            self.last_text = Some((horizontal, text.to_vec()));
        }
        Ok(self.widths[index])
    }

    fn part_widths(&self, item: &TaskbarItem, value: &str, font_size: f32) -> Result<PartWidths> {
        let (number, unit) = value_parts(value);
        let width = |text: &str, format: &IDWriteTextFormat| -> Result<f32> {
            if text.is_empty() {
                return Ok(0.);
            }
            let text: Vec<u16> = text.encode_utf16().collect();
            Ok(measure(&self.factory, format, &text)? - TEXT_PADDING * 2. + CELL_RASTER_PADDING)
        };
        Ok(PartWidths([
            if let Some(prefix) = self.prefix_layouts.get(item.prefix_text().as_ref()) {
                (if item.direction_icon().is_some() {
                    font_size
                } else {
                    prefix.stem_width
                }) + prefix.colon_width
                    + CELL_RASTER_PADDING
            } else if item.direction_icon().is_some() {
                font_size + width(&item.prefix_text(), &self.format)?
            } else {
                width(&item.prefix_text(), &self.format)?
            },
            width(number, &self.right_format)?,
            width(unit, &self.format)?,
        ]))
    }

    pub fn frame(&mut self, content: &TaskbarContent, data: &Shared) -> Result<ContentFrame> {
        if self.content.as_ref() != Some(content) {
            if self.content.as_ref().is_none_or(|previous| {
                previous.font_family != content.font_family
                    || previous.font_size != content.font_size
                    || previous.font_weight != content.font_weight
            }) {
                let format = text_format(
                    &self.factory,
                    &content.font_family,
                    content.font_size,
                    DWRITE_FONT_WEIGHT(i32::from(content.font_weight.value())),
                )?;
                let right_format = text_format(
                    &self.factory,
                    &content.font_family,
                    content.font_size,
                    DWRITE_FONT_WEIGHT(i32::from(content.font_weight.value())),
                )?;
                unsafe {
                    right_format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_TRAILING)?;
                }
                self.format = format;
                self.right_format = right_format;
            }
            self.cell_widths.clear();
            self.cell_texts.clear();
            self.prefix_layouts.clear();
            for item in content.rows.iter().flatten() {
                let text = item.prefix_text();
                if let Some(prefix) =
                    prefix_layout(&self.factory, &self.format, &text, content.font_size)?
                {
                    self.prefix_layouts.insert(text.into_owned(), prefix);
                }
                let mut parts = PartWidths::default();
                for value in item.samples() {
                    parts.merge(self.part_widths(item, &value, content.font_size)?);
                }
                self.cell_widths.insert(item.metric, parts);
            }
            self.content = Some(content.clone());
        }
        for item in content.rows.iter().flatten() {
            let value = item.value(data);
            if self.cell_texts.get(&item.metric) == Some(&value) {
                continue;
            }
            let parts = self.part_widths(item, &value, content.font_size)?;
            self.cell_widths.get_mut(&item.metric).unwrap().merge(parts);
            self.cell_texts.insert(item.metric, value);
        }
        Ok(arrange(content, data, &self.cell_widths))
    }

    /// Draw all cells in one transparent surface. Cell widths include glyph
    /// overhang; column positions match the editor's preview calculation.
    pub unsafe fn draw_frame(
        &self,
        dc: windows_sys::Win32::Graphics::Gdi::HDC,
        bounds: windows_sys::Win32::Foundation::RECT,
        dpi: u32,
        frame: &ContentFrame,
        color: u32,
    ) -> Result<()> {
        unsafe {
            let bounds = RECT {
                left: bounds.left,
                top: bounds.top,
                right: bounds.right,
                bottom: bounds.bottom,
            };
            let dpi = dpi.max(96) as f32;
            self.target.SetDpi(dpi, dpi);
            self.target.BindDC(HDC(dc), &bounds)?;
            let height = (bounds.bottom - bounds.top) as f32 * 96. / dpi;
            let line_height = frame
                .line_height
                .min(height / frame.rows.len().max(1) as f32);
            let top = (height - line_height * frame.rows.len() as f32) / 2.;
            self.target.BeginDraw();
            self.target.Clear(Some(&D2D1_COLOR_F::default()));
            for (row_index, row) in frame.rows.iter().enumerate() {
                for cell in row {
                    let cell_top = top + row_index as f32 * line_height;
                    if let Some(background) = cell.background_color {
                        self.brush.SetColor(&rgb_color(background));
                        self.target.FillRectangle(
                            &D2D_RECT_F {
                                left: cell.x,
                                right: cell.x + cell.width,
                                top: cell_top,
                                bottom: cell_top + line_height,
                            },
                            &self.brush,
                        );
                    }
                    self.brush
                        .SetColor(&rgb_color(cell.text_color.unwrap_or(color)));
                    for (segment_index, segment) in cell.segments.iter().enumerate() {
                        let mut text_left = segment.x;
                        if let Some(icon) = segment.icon {
                            let scale = frame.font_size / ICON_VIEWBOX;
                            self.target.SetTransform(&windows_numerics::Matrix3x2 {
                                M11: scale,
                                M22: scale,
                                M31: segment.x,
                                M32: cell_top + (line_height - frame.font_size) / 2.,
                                ..Default::default()
                            });
                            self.target
                                .FillGeometry(&self.icons[icon.index()], &self.brush, None);
                            self.target
                                .SetTransform(&windows_numerics::Matrix3x2::identity());
                            text_left += frame.font_size;
                        }
                        if segment_index == 0 {
                            let measured;
                            let prefix =
                                if let Some(prefix) = self.prefix_layouts.get(&segment.text) {
                                    Some(prefix)
                                } else {
                                    measured = prefix_layout(
                                        &self.factory,
                                        &self.format,
                                        &segment.text,
                                        frame.font_size,
                                    )?;
                                    measured.as_ref()
                                };
                            if let Some(prefix) = prefix {
                                let stem_width = if segment.icon.is_some() {
                                    frame.font_size
                                } else {
                                    prefix.stem_width
                                };
                                for (text, left, right, offset) in [
                                    (
                                        &prefix.stem,
                                        segment.x,
                                        segment.x + stem_width,
                                        prefix.stem_center_offset,
                                    ),
                                    (
                                        &prefix.colon,
                                        segment.x + stem_width,
                                        segment.x + segment.width,
                                        prefix.colon_center_offset,
                                    ),
                                ] {
                                    if !text.is_empty() {
                                        self.target.DrawText(
                                            text,
                                            &self.format,
                                            &D2D_RECT_F {
                                                left,
                                                right: right + CELL_RASTER_PADDING,
                                                top: cell_top - offset,
                                                bottom: cell_top + line_height - offset,
                                            },
                                            &self.brush,
                                            D2D1_DRAW_TEXT_OPTIONS_CLIP,
                                            DWRITE_MEASURING_MODE_NATURAL,
                                        );
                                    }
                                }
                                continue;
                            }
                        }
                        if segment.text.is_empty() {
                            continue;
                        }
                        let text: Vec<u16> = segment.text.encode_utf16().collect();
                        let rect = D2D_RECT_F {
                            left: text_left,
                            right: segment.x + segment.width,
                            top: cell_top,
                            bottom: cell_top + line_height,
                        };
                        self.target.DrawText(
                            &text,
                            if segment.right_aligned {
                                &self.right_format
                            } else {
                                &self.format
                            },
                            &rect,
                            &self.brush,
                            D2D1_DRAW_TEXT_OPTIONS_CLIP,
                            DWRITE_MEASURING_MODE_NATURAL,
                        );
                    }
                }
            }
            self.target.EndDraw(None, None)
        }
    }

    /// The caller owns a valid 32-bit top-down DIB selected into `dc`, and keeps
    /// it alive until drawing and UpdateLayeredWindow have completed.
    #[cfg(test)]
    pub unsafe fn draw(
        &self,
        dc: windows_sys::Win32::Graphics::Gdi::HDC,
        bounds: windows_sys::Win32::Foundation::RECT,
        dpi: u32,
        text: &[u16],
        color: u32,
    ) -> Result<()> {
        unsafe {
            let bounds = RECT {
                left: bounds.left,
                top: bounds.top,
                right: bounds.right,
                bottom: bounds.bottom,
            };
            let dpi = dpi.max(96) as f32;
            self.target.SetDpi(dpi, dpi);
            self.target.BindDC(HDC(dc), &bounds)?;
            self.brush.SetColor(&D2D1_COLOR_F {
                r: ((color >> 16) & 255) as f32 / 255.,
                g: ((color >> 8) & 255) as f32 / 255.,
                b: (color & 255) as f32 / 255.,
                a: 1.,
            });
            let layout = D2D_RECT_F {
                left: TEXT_PADDING,
                top: 0.,
                right: (bounds.right - bounds.left) as f32 * 96. / dpi - TEXT_PADDING,
                bottom: (bounds.bottom - bounds.top) as f32 * 96. / dpi,
            };
            self.target.BeginDraw();
            self.target.Clear(Some(&D2D1_COLOR_F::default()));
            self.target.DrawText(
                text,
                &self.format,
                &layout,
                &self.brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
            self.target.EndDraw(None, None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr::null_mut;
    use windows_sys::Win32::{Foundation::RECT, Graphics::Gdi::*};

    #[test]
    fn numbers_and_units_have_separate_shared_column_edges() {
        assert_eq!(value_parts("99%"), ("99", "%"));
        assert_eq!(value_parts("1.0 MB/s"), ("1.0", "MB/s"));
        assert_eq!(value_parts("8.0 / 16.0 GB"), ("8.0 / 16.0", "GB"));
        assert_eq!(value_parts("1.2M"), ("1.2", "M"));
        assert_eq!(value_parts("离线"), ("离线", ""));
        let mut renderer = TaskbarText::new().unwrap();
        let data = Shared {
            connected: true,
            up: 1024.,
            down: 128. * 1024f64.powi(2),
            system: crate::model::SystemMetrics {
                cpu_percent: Some(9.),
                memory: Some((9, 10)),
                ..Default::default()
            },
            ..Default::default()
        };
        let frame = renderer.frame(&TaskbarContent::default(), &data).unwrap();
        for (top, bottom) in frame.rows[0].iter().zip(&frame.rows[1]) {
            let top_number = &top.segments[1];
            let bottom_number = &bottom.segments[1];
            assert!(top_number.right_aligned && bottom_number.right_aligned);
            assert_eq!(
                top_number.x + top_number.width,
                bottom_number.x + bottom_number.width
            );
            assert_eq!(top.segments[2].x, bottom.segments[2].x);
            assert!(top_number.x + top_number.width < top.segments[2].x);
        }
        assert_eq!(frame.rows[0][0].segments[2].text, "%");
        assert_eq!(frame.rows[1][0].segments[2].text, "%");
        assert_eq!(frame.rows[0][1].segments[2].text, "K/s");
        assert_eq!(frame.rows[1][1].segments[2].text, "M/s");
    }

    struct Bitmap {
        dc: windows_sys::Win32::Graphics::Gdi::HDC,
        bitmap: HBITMAP,
        previous: HGDIOBJ,
        bits: *mut u8,
    }

    impl Bitmap {
        fn new(width: i32, height: i32) -> Self {
            unsafe {
                let dc = CreateCompatibleDC(null_mut());
                assert!(!dc.is_null());
                let info = BITMAPINFO {
                    bmiHeader: BITMAPINFOHEADER {
                        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                        biWidth: width,
                        biHeight: -height,
                        biPlanes: 1,
                        biBitCount: 32,
                        biCompression: BI_RGB,
                        ..Default::default()
                    },
                    ..Default::default()
                };
                let mut bits = null_mut();
                let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
                assert!(!bitmap.is_null());
                let previous = SelectObject(dc, bitmap);
                assert!(!previous.is_null());
                Self {
                    dc,
                    bitmap,
                    previous,
                    bits: bits.cast(),
                }
            }
        }
    }

    impl Drop for Bitmap {
        fn drop(&mut self) {
            unsafe {
                SelectObject(self.dc, self.previous);
                DeleteObject(self.bitmap);
                DeleteDC(self.dc);
            }
        }
    }

    #[test]
    fn network_icons_preserve_prefix_settings_and_column_alignment() {
        let mut renderer = TaskbarText::new().unwrap();
        let mut content = TaskbarContent::default();
        let serialized = toml::to_string(&content).unwrap();
        content = toml::from_str(&serialized).unwrap();
        let data = Shared {
            connected: true,
            up: 128. * 1024.,
            down: 8.6 * 1024f64.powi(2),
            ..Default::default()
        };
        for (family, weight) in [("Segoe UI", 400), ("Microsoft YaHei", 700)] {
            content.font_family = family.into();
            content.font_weight = TaskbarFontWeight::new(weight).unwrap();
            let frame = renderer.frame(&content, &data).unwrap();
            let up = &frame.rows[0][1];
            let down = &frame.rows[1][1];
            assert_eq!(up.segments[0].icon, Some(DirectionIcon::Upload));
            assert_eq!(down.segments[0].icon, Some(DirectionIcon::Download));
            assert!(up.segments[0].width > content.font_size + CELL_RASTER_PADDING);
            assert_eq!(up.segments[0].text, ":");
            assert_eq!(down.segments[0].text, ":");
            assert_eq!(up.segments[0].width, down.segments[0].width);
            assert_eq!(up.segments[1].x, down.segments[1].x);
            assert_eq!(up.segments[2].x, down.segments[2].x);
        }
        content.item_mut(Metric::Upload).unwrap().label = "上传:".into();
        content.item_mut(Metric::Download).unwrap().label.clear();
        let frame = renderer.frame(&content, &data).unwrap();
        assert_eq!(frame.rows[0][1].segments[0].text, "上传:");
        assert!(frame.rows[0][1].segments[0].icon.is_none());
        assert!(frame.rows[1][1].segments[0].text.is_empty());
        assert!(frame.rows[1][1].segments[0].icon.is_none());
        content.item_mut(Metric::Upload).unwrap().label.clear();
        let frame = renderer.frame(&content, &data).unwrap();
        assert_eq!(frame.rows[0][1].segments[0].width, 0.);
        assert_eq!(frame.rows[0][1].segments[1].x, frame.rows[0][1].x);
    }

    #[test]
    fn network_icons_render_at_fractional_sizes_without_font_dependency() {
        let samples =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/direction-icons");
        std::fs::create_dir_all(&samples).unwrap();
        let mut renderer = TaskbarText::new().unwrap();
        let mut content = TaskbarContent::default();
        content.add(Metric::TotalSpeed);
        let mut data = Shared {
            connected: true,
            up: 128. * 1024.,
            down: 8.6 * 1024f64.powi(2),
            ..Default::default()
        };
        data.system.cpu_percent = Some(38.);
        data.system.memory = Some((81, 100));
        for size in [8., 12., 12.5, 13., 20.] {
            content.font_size = size;
            for dpi in [96, 120, 144, 192] {
                let scale = dpi as f32 / 96.;
                let width = ((size + 4.) * scale).ceil() as i32;
                let height = ((size + 8.) * scale).ceil() as i32;
                let mut reference = Vec::new();
                for (family, weight) in [("Segoe UI", 400), ("Microsoft YaHei", 900)] {
                    content.font_family = family.into();
                    content.font_weight = TaskbarFontWeight::new(weight).unwrap();
                    let frame = renderer.frame(&content, &data).unwrap();
                    let mut pair = Vec::new();
                    for metric in [Metric::Upload, Metric::Download] {
                        let mut icon = frame.metric_frame(metric).unwrap();
                        // Isolate the arrow for shape and font comparisons.
                        icon.rows[0][0].segments[0].text.clear();
                        for segment in &mut icon.rows[0][0].segments[1..] {
                            segment.text.clear();
                        }
                        let bitmap = Bitmap::new(width, height);
                        let pixels = unsafe {
                            renderer
                                .draw_frame(
                                    bitmap.dc,
                                    RECT {
                                        left: 0,
                                        top: 0,
                                        right: width,
                                        bottom: height,
                                    },
                                    dpi,
                                    &icon,
                                    0x3498db,
                                )
                                .unwrap();
                            assert_ne!(GdiFlush(), 0);
                            std::slice::from_raw_parts(bitmap.bits, (width * height * 4) as usize)
                                .to_vec()
                        };
                        let alpha = pixels
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .map(|pixel| {
                                assert!(pixel[..3].iter().all(|&channel| channel <= pixel[3]));
                                pixel[3]
                            })
                            .collect::<Vec<_>>();
                        let peak_alpha = alpha.iter().copied().max().unwrap_or(0);
                        assert!(
                            peak_alpha >= 128,
                            "icon is too faint at {size}px / {dpi} DPI: {peak_alpha}"
                        );
                        assert!(alpha.iter().any(|&a| a > 0 && a < 255));
                        let sample = |x: f32, y: f32| {
                            let x =
                                ((TEXT_PADDING + size * x / ICON_VIEWBOX) * scale).floor() as usize;
                            let y = ((height as f32 - size * scale) / 2.
                                + size * y / ICON_VIEWBOX * scale)
                                .floor() as usize;
                            alpha[y * width as usize + x]
                        };
                        let shaft_y = if metric == Metric::Upload { 15. } else { 9. };
                        let shaft_alpha = sample(12., shaft_y);
                        let side_alpha = sample(6., shaft_y);
                        assert!(
                            shaft_alpha > side_alpha.saturating_add(40),
                            "solid arrow missing at {size}px / {dpi} DPI"
                        );
                        assert_eq!(side_alpha, 0, "the arrow must have no surrounding tile");
                        if size * scale >= 24. {
                            assert_eq!(shaft_alpha, 255, "arrow shaft must be solid");
                        }
                        for row in [0, height - 1] {
                            assert!(
                                alpha[(row * width) as usize..((row + 1) * width) as usize]
                                    .iter()
                                    .all(|&a| a == 0)
                            );
                        }
                        pair.push(alpha);
                    }
                    let mirror_error: u32 = pair[0]
                        .iter()
                        .enumerate()
                        .map(|(index, &a)| {
                            let x = index % width as usize;
                            let y = index / width as usize;
                            u32::from(
                                a.abs_diff(pair[1][(height as usize - y - 1) * width as usize + x]),
                            )
                        })
                        .sum();
                    assert!(
                        mirror_error < (width * height * 2) as u32,
                        "asymmetric icons at {size}px / {dpi} DPI: {mirror_error}"
                    );
                    if reference.is_empty() {
                        reference = pair;
                    } else {
                        assert_eq!(
                            pair, reference,
                            "icon changed with font at {size}px / {dpi} DPI"
                        );
                    }
                }
                if size == 12. || size == 13. {
                    let mut preview = content.clone();
                    preview.font_family = "Microsoft YaHei".into();
                    preview.font_weight = TaskbarFontWeight::NORMAL;
                    let frame = renderer.frame(&preview, &data).unwrap();
                    let w = (frame.width * scale).ceil() as i32;
                    let h = (48. * scale).ceil() as i32;
                    let bitmap = Bitmap::new(w, h);
                    unsafe {
                        renderer
                            .draw_frame(
                                bitmap.dc,
                                RECT {
                                    left: 0,
                                    top: 0,
                                    right: w,
                                    bottom: h,
                                },
                                dpi,
                                &frame,
                                0x24272d,
                            )
                            .unwrap();
                        assert_ne!(GdiFlush(), 0);
                        let pixels = std::slice::from_raw_parts(bitmap.bits, (w * h * 4) as usize);
                        let image = image::RgbaImage::from_fn(w as u32, h as u32, |x, y| {
                            let index = ((y * w as u32 + x) * 4) as usize;
                            let pixel = &pixels[index..index + 4];
                            let composite = |channel: u8| {
                                (u16::from(channel)
                                    + (245 * (255 - u16::from(pixel[3])) + 127) / 255)
                                    .min(255) as u8
                            };
                            image::Rgba([
                                composite(pixel[2]),
                                composite(pixel[1]),
                                composite(pixel[0]),
                                255,
                            ])
                        });
                        image
                            .save(samples.join(format!("rounded-solid-{size}-{dpi}.png")))
                            .unwrap();
                    }
                }
            }
        }
    }

    #[test]
    fn all_prefix_colons_match_text_and_center_cpu_memory_and_network_icons() {
        let mut renderer = TaskbarText::new().unwrap();
        let mut content = TaskbarContent::default();
        for metric in Metric::ALL {
            content.add(metric);
        }
        for family in ["Segoe UI", "Microsoft YaHei"] {
            content.font_family = family.into();
            for size in [8., 12., 12.5, 13., 20.] {
                content.font_size = size;
                for weight in [400, 900] {
                    content.font_weight = TaskbarFontWeight::new(weight).unwrap();
                    for dpi in [96, 144, 192] {
                        let scale = dpi as f32 / 96.;
                        let frame = renderer.frame(&content, &Shared::default()).unwrap();
                        for (metric, colon) in Metric::ALL
                            .into_iter()
                            .flat_map(|metric| [":", "："].map(|colon| (metric, colon)))
                        {
                            let mut prefix = frame.metric_frame(metric).unwrap();
                            let segment = &mut prefix.rows[0][0].segments[0];
                            let (stem, _) = split_prefix(&segment.text).unwrap();
                            segment.text = format!("{stem}{colon}");
                            let parts = prefix_layout(
                                &renderer.factory,
                                &renderer.format,
                                &segment.text,
                                size,
                            )
                            .unwrap()
                            .unwrap();
                            let stem_width = if segment.icon.is_some() {
                                size
                            } else {
                                parts.stem_width
                            };
                            segment.width = stem_width + parts.colon_width + CELL_RASTER_PADDING;
                            for segment in &mut prefix.rows[0][0].segments[1..] {
                                segment.text.clear();
                            }
                            let width = ((prefix.rows[0][0].segments[0].width + TEXT_PADDING * 2.)
                                * scale)
                                .ceil() as i32;
                            let height = ((size + 8.) * scale).ceil() as i32;
                            let bitmap = Bitmap::new(width, height);
                            let alpha = unsafe {
                                renderer
                                    .draw_frame(
                                        bitmap.dc,
                                        RECT {
                                            left: 0,
                                            top: 0,
                                            right: width,
                                            bottom: height,
                                        },
                                        dpi,
                                        &prefix,
                                        0x24272d,
                                    )
                                    .unwrap();
                                assert_ne!(GdiFlush(), 0);
                                std::slice::from_raw_parts(
                                    bitmap.bits,
                                    (width * height * 4) as usize,
                                )
                                .as_chunks::<4>()
                                .0
                                .iter()
                                .map(|pixel| pixel[3])
                                .collect::<Vec<_>>()
                            };
                            let split = ((TEXT_PADDING + stem_width) * scale).ceil() as usize;
                            // Render the same suffix as ordinary text, exactly
                            // as CPU/memory punctuation is rendered. The colon
                            // pixels must match, including font and weight.
                            let suffix = &mut prefix.rows[0][0].segments[0];
                            suffix.text = colon.into();
                            suffix.icon = None;
                            suffix.x += stem_width;
                            suffix.width -= stem_width;
                            let colon_alpha = unsafe {
                                renderer
                                    .draw_frame(
                                        bitmap.dc,
                                        RECT {
                                            left: 0,
                                            top: 0,
                                            right: width,
                                            bottom: height,
                                        },
                                        dpi,
                                        &prefix,
                                        0x24272d,
                                    )
                                    .unwrap();
                                assert_ne!(GdiFlush(), 0);
                                let pixels = std::slice::from_raw_parts(
                                    bitmap.bits,
                                    (width * height * 4) as usize,
                                );
                                let reference = pixels
                                    .as_chunks::<4>()
                                    .0
                                    .iter()
                                    .map(|pixel| pixel[3])
                                    .collect::<Vec<_>>();
                                for y in 0..height as usize {
                                    for x in split..width as usize {
                                        let index = y * width as usize + x;
                                        // A CJK stem's raster fringe can touch
                                        // the first column beyond its advance.
                                        if x == split {
                                            assert!(alpha[index] >= reference[index]);
                                        } else {
                                            assert_eq!(
                                                alpha[index], reference[index],
                                                "colon must use the text renderer: {metric:?} {family} {size}px / {dpi} DPI {colon} at {x},{y}"
                                            );
                                        }
                                    }
                                }
                                reference
                            };
                            let center = |alpha: &[u8], start: usize, end: usize| {
                                let rows = (0..height as usize)
                                    .filter(|&y| {
                                        (start..end).any(|x| alpha[y * width as usize + x] > 16)
                                    })
                                    .collect::<Vec<_>>();
                                assert!(
                                    !rows.is_empty(),
                                    "missing prefix ink: {metric:?} {size}px / {dpi} DPI x={start}..{end}, peak={}",
                                    (0..height as usize)
                                        .flat_map(|y| (start..end).map(move |x| (y, x)))
                                        .map(|(y, x)| alpha[y * width as usize + x])
                                        .max()
                                        .unwrap_or(0)
                                );
                                (rows[0] + rows[rows.len() - 1] + 1) as f32 / 2.
                            };
                            assert!(
                                // Text is vertically grid-fitted; the vector
                                // bounds remain fractional. Each edge can
                                // differ by up to one raster pixel.
                                (center(&alpha, 0, split)
                                    - center(&colon_alpha, split, width as usize))
                                .abs()
                                    <= 1.5,
                                "prefix/colon centers differ: {metric:?} {family} {size}px / {dpi} DPI weight={weight} {colon}, prefix={}, colon={}",
                                center(&alpha, 0, split),
                                center(&colon_alpha, split, width as usize)
                            );
                            if metric == Metric::TotalSpeed && size * scale >= 24. {
                                let row_ink = |left: f32, right: f32, y: f32| {
                                    let row = ((height as f32 - size * scale) / 2.
                                        + y * size / ICON_VIEWBOX * scale)
                                        .floor()
                                        as usize;
                                    let start = ((TEXT_PADDING + left * size / ICON_VIEWBOX)
                                        * scale)
                                        .floor()
                                        as usize;
                                    let end = ((TEXT_PADDING + right * size / ICON_VIEWBOX) * scale)
                                        .ceil()
                                        as usize;
                                    (start..end)
                                        .filter(|&x| alpha[row * width as usize + x] > 64)
                                        .count()
                                };
                                assert!(
                                    row_ink(2., 12., 6.) > row_ink(2., 12., 15.),
                                    "left arrow must point up"
                                );
                                assert!(
                                    row_ink(12., 22., 18.) > row_ink(12., 22., 9.),
                                    "right arrow must point down"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn natural_taskbar_text_preserves_layout_and_transparent_edges() {
        use windows::Win32::Graphics::DirectWrite::DWRITE_RENDERING_MODE_NATURAL_SYMMETRIC;
        let samples =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/taskbar-render-quality");
        std::fs::create_dir_all(&samples).unwrap();
        let mut renderer = TaskbarText::new().unwrap();
        let natural = unsafe { renderer.target.GetTextRenderingParams().unwrap() };
        let defaults = unsafe { renderer.factory.CreateRenderingParams().unwrap() };
        let symmetric = unsafe {
            renderer
                .factory
                .CreateCustomRenderingParams(
                    defaults.GetGamma(),
                    defaults.GetEnhancedContrast(),
                    0.,
                    DWRITE_PIXEL_GEOMETRY_FLAT,
                    DWRITE_RENDERING_MODE_NATURAL_SYMMETRIC,
                )
                .unwrap()
        };
        let mut content = TaskbarContent::default();
        let mut data = Shared {
            connected: true,
            up: 1024.,
            down: 9.2 * 1024.,
            ..Default::default()
        };
        data.system.cpu_percent = Some(10.);
        data.system.memory = Some((81, 100));
        let mut changed = 0;
        let mut vertical_sharpness = [0.; 2];
        for (family_index, family) in ["Segoe UI", "Microsoft YaHei"].into_iter().enumerate() {
            content.font_family = family.into();
            for size in [12., 12.5, 13., 14., 16., 20.] {
                content.font_size = size;
                let frame = renderer.frame(&content, &data).unwrap();
                for dpi in [96, 120, 144, 192] {
                    let scale = dpi as f32 / 96.;
                    let width = (frame.width * scale).ceil() as i32;
                    let height = (48. * scale).ceil() as i32;
                    for (theme, foreground, background) in [
                        ("light", 0x303030, [237, 241, 247]),
                        ("dark", 0xe8e8e8, [30, 33, 38]),
                    ] {
                        let mut old_pixels = None;
                        for (variant, params) in
                            [symmetric.clone(), natural.clone()].into_iter().enumerate()
                        {
                            let bitmap = Bitmap::new(width, height);
                            let pixels = unsafe {
                                renderer.target.SetTextRenderingParams(&params);
                                renderer
                                    .draw_frame(
                                        bitmap.dc,
                                        RECT {
                                            left: 0,
                                            top: 0,
                                            right: width,
                                            bottom: height,
                                        },
                                        dpi,
                                        &frame,
                                        foreground,
                                    )
                                    .unwrap();
                                assert_ne!(GdiFlush(), 0);
                                std::slice::from_raw_parts(
                                    bitmap.bits,
                                    (width * height * 4) as usize,
                                )
                                .to_vec()
                            };
                            assert!(
                                pixels
                                    .as_chunks::<4>()
                                    .0
                                    .iter()
                                    .any(|pixel| pixel[3] == 255)
                            );
                            assert!(
                                pixels
                                    .as_chunks::<4>()
                                    .0
                                    .iter()
                                    .any(|pixel| pixel[3] > 0 && pixel[3] < 255)
                            );
                            assert!(pixels.as_chunks::<4>().0.iter().all(|pixel| {
                                pixel[..3].iter().all(|&channel| channel <= pixel[3])
                            }));
                            for row in [0, height - 1] {
                                let start = (row * width * 4) as usize;
                                assert!(
                                    pixels[start..start + (width * 4) as usize]
                                        .as_chunks::<4>()
                                        .0
                                        .iter()
                                        .all(|pixel| pixel[3] == 0)
                                );
                            }
                            let mut energy = 0.;
                            let mut ink = 0.;
                            let mut preview = image::RgbImage::new(width as u32, height as u32);
                            for (index, (pixel, output)) in pixels
                                .as_chunks::<4>()
                                .0
                                .iter()
                                .zip(preview.pixels_mut())
                                .enumerate()
                            {
                                let alpha = f64::from(pixel[3]);
                                ink += alpha;
                                if index >= width as usize {
                                    energy += (alpha
                                        - f64::from(pixels[(index - width as usize) * 4 + 3]))
                                    .powi(2);
                                }
                                let blend = |channel: u8, bg: u8| {
                                    (u32::from(channel)
                                        + (u32::from(bg) * u32::from(255 - pixel[3]) + 127) / 255)
                                        .min(255) as u8
                                };
                                *output = image::Rgb([
                                    blend(pixel[2], background[0]),
                                    blend(pixel[1], background[1]),
                                    blend(pixel[0], background[2]),
                                ]);
                            }
                            vertical_sharpness[variant] += energy / ink;
                            let name = if variant == 0 { "before" } else { "after" };
                            preview
                                .save(samples.join(format!(
                                    "{dpi}-f{family_index}-s{}-{theme}-{name}.png",
                                    (size * 100.) as u32
                                )))
                                .unwrap();
                            if let Some(before) = old_pixels.as_ref() {
                                changed += usize::from(before != &pixels);
                            } else {
                                old_pixels = Some(pixels);
                            }
                        }
                        let repeated = renderer.frame(&content, &data).unwrap();
                        assert_eq!(
                            frame.width, repeated.width,
                            "rendering mode changed reserved width"
                        );
                        assert_eq!(frame.line_height, repeated.line_height);
                    }
                }
            }
        }
        assert!(
            changed > 0,
            "taskbar text still uses the old symmetric rasterization"
        );
        println!(
            "Taskbar raster pairs changed={changed}, normalized vertical edge energy before/after={vertical_sharpness:?}; samples={}",
            samples.display()
        );
    }

    #[test]
    fn font_weight_changes_ink_and_restores_normal_layout() {
        let mut renderer = TaskbarText::new().unwrap();
        let mut content = TaskbarContent::default();
        let mut data = Shared {
            connected: true,
            ..Default::default()
        };
        data.system.cpu_percent = Some(38.);
        data.system.memory = Some((50, 100));
        for dpi in [96, 144] {
            let mut coverage = Vec::new();
            let mut normal = None;
            for weight in TaskbarFontWeight::ALL
                .into_iter()
                .chain([350, 450, 550].map(|value| TaskbarFontWeight::new(value).unwrap()))
                .chain([TaskbarFontWeight::NORMAL])
            {
                content.font_weight = weight;
                let frame = renderer.frame(&content, &data).unwrap();
                let scale = dpi as f32 / 96.;
                let width = (frame.width * scale).ceil() as i32;
                let height = (48. * scale) as i32;
                let bitmap = Bitmap::new(width, height);
                let pixels = unsafe {
                    renderer
                        .draw_frame(
                            bitmap.dc,
                            RECT {
                                left: 0,
                                top: 0,
                                right: width,
                                bottom: height,
                            },
                            dpi,
                            &frame,
                            0x303030,
                        )
                        .unwrap();
                    assert_ne!(GdiFlush(), 0);
                    std::slice::from_raw_parts(bitmap.bits, (width * height * 4) as usize).to_vec()
                };
                coverage.push(
                    pixels
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|p| u64::from(p[3]))
                        .sum::<u64>(),
                );
                if weight == TaskbarFontWeight::NORMAL {
                    if let Some((normal_width, normal_pixels)) = &normal {
                        assert_eq!(
                            frame.width, *normal_width,
                            "stale width after changing weight"
                        );
                        assert_eq!(&pixels, normal_pixels, "normal font was not restored");
                    } else {
                        normal = Some((frame.width, pixels));
                    }
                }
            }
            assert!(
                coverage[0] < coverage[1] && coverage[1] < coverage[2],
                "font weights did not change actual ink at {dpi} DPI: {coverage:?}"
            );
        }
    }

    #[test]
    fn custom_font_size_colors_and_hidden_unit_reach_the_rasterizer() {
        let mut renderer = TaskbarText::new().unwrap();
        let mut content = TaskbarContent {
            rows: vec![vec![TaskbarItem::new(Metric::Cpu)]],
            ..Default::default()
        };
        let mut data = Shared::default();
        data.system.cpu_percent = Some(38.);
        let original = renderer.frame(&content, &data).unwrap();
        assert_eq!(original.rows[0][0].segments[2].text, "%");
        content.item_mut(Metric::Cpu).unwrap().hide_percent = true;
        let without_unit = renderer.frame(&content, &data).unwrap();
        assert!(without_unit.width < original.width);
        assert_eq!(without_unit.rows[0][0].segments[2].width, 0.);
        for family in ["Segoe UI", "Consolas"] {
            content.font_family = family.into();
            let mut previous_width = 0.;
            for size in [8., 12., 12.25, 12.5, 12.75, 13., 20.] {
                content.font_size = size;
                let frame = renderer.frame(&content, &data).unwrap();
                assert!(frame.width > previous_width);
                assert_eq!(frame.font_family, family);
                assert_eq!(frame.font_size, size);
                previous_width = frame.width;
                for (foreground, background) in [(0xff0000, 0x000000), (0x000000, 0xffffff)] {
                    let item = content.item_mut(Metric::Cpu).unwrap();
                    item.text_color = Some(foreground);
                    item.background_color = Some(background);
                    let frame = renderer.frame(&content, &data).unwrap();
                    for dpi in [96, 144] {
                        let scale = dpi as f32 / 96.;
                        let width = (frame.width * scale).ceil() as i32;
                        let height = (48. * scale) as i32;
                        let bitmap = Bitmap::new(width, height);
                        unsafe {
                            renderer
                                .draw_frame(
                                    bitmap.dc,
                                    RECT {
                                        left: 0,
                                        top: 0,
                                        right: width,
                                        bottom: height,
                                    },
                                    dpi,
                                    &frame,
                                    0x00ff00,
                                )
                                .unwrap();
                            assert_ne!(GdiFlush(), 0);
                            let pixels = std::slice::from_raw_parts(
                                bitmap.bits,
                                (width * height * 4) as usize,
                            );
                            let bg = [
                                (background & 255) as u8,
                                ((background >> 8) & 255) as u8,
                                ((background >> 16) & 255) as u8,
                                255,
                            ];
                            assert!(
                                pixels.as_chunks::<4>().0.contains(&bg),
                                "custom background missing"
                            );
                            assert!(
                                pixels.as_chunks::<4>().0.iter().any(|pixel| pixel[3] == 0),
                                "outer padding must remain transparent"
                            );
                            assert!(
                                pixels.as_chunks::<4>().0.iter().any(|pixel| {
                                    pixel[3] == 255
                                        && if foreground == 0 {
                                            pixel[0] < 220 && pixel[1] < 220 && pixel[2] < 220
                                        } else {
                                            pixel[2] > 30 && pixel[0] == 0 && pixel[1] == 0
                                        }
                                }),
                                "custom foreground missing for {family} {size}px at {dpi} DPI"
                            );
                        }
                    }
                }
            }
        }
        content.font_family = original.font_family.clone();
        content.font_size = original.font_size;
        content.item_mut(Metric::Cpu).unwrap().hide_percent = false;
        let restored = renderer.frame(&content, &data).unwrap();
        assert_eq!(restored.width, original.width);
    }

    #[test]
    fn custom_cells_align_stay_stable_and_render_without_clipping() {
        use crate::{model::SystemMetrics, taskbar_content::TaskbarItem};
        let mut renderer = TaskbarText::new().unwrap();
        let samples =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/taskbar-text-check");
        std::fs::create_dir_all(&samples).unwrap();
        let mut content = TaskbarContent::default();
        for metric in Metric::ALL {
            content.add(metric);
        }
        content.item_mut(Metric::Download).unwrap().label = "下行".into();
        content.item_mut(Metric::Upload).unwrap().label.clear();
        content.item_mut(Metric::Memory).unwrap().detailed = true;
        let mut data = Shared {
            connected: true,
            down: 1024f64.powi(3) - 1.,
            up: 1024f64.powi(4) - 1.,
            system: SystemMetrics {
                cpu_percent: Some(100.),
                memory: Some((1024u64.pow(4), 1024u64.pow(4))),
                ..Default::default()
            },
            ..Shared::default()
        };
        data.today.up = 1024u64.pow(4) - 1;
        for row_count in [2, 1] {
            content.set_rows(row_count);
            let frame = renderer.frame(&content, &data).unwrap();
            if row_count == 2 {
                for (top, bottom) in frame.rows[0].iter().zip(&frame.rows[1]) {
                    assert_eq!((top.x, top.width), (bottom.x, bottom.width));
                }
            }
            let width = frame.width;
            let mut offline = data.clone();
            offline.connected = false;
            offline.system = SystemMetrics::default();
            assert_eq!(renderer.frame(&content, &offline).unwrap().width, width);
            for dpi in [96, 120, 144, 192] {
                let scale = dpi as f32 / 96.;
                let w = (width * scale).ceil() as i32;
                let h = (48. * scale).round() as i32;
                let compact = Bitmap::new(w, h);
                let reference = Bitmap::new(w + 96, h);
                let mut wider = frame.clone();
                for cell in wider.rows.iter_mut().flatten() {
                    cell.width += 64.;
                    for segment in &mut cell.segments {
                        if segment.right_aligned {
                            segment.x -= 64.;
                        }
                        segment.width += 64.;
                    }
                }
                unsafe {
                    renderer
                        .draw_frame(
                            compact.dc,
                            RECT {
                                left: 0,
                                top: 0,
                                right: w,
                                bottom: h,
                            },
                            dpi,
                            &frame,
                            0x233349,
                        )
                        .unwrap();
                    renderer
                        .draw_frame(
                            reference.dc,
                            RECT {
                                left: 0,
                                top: 0,
                                right: w + 96,
                                bottom: h,
                            },
                            dpi,
                            &wider,
                            0x233349,
                        )
                        .unwrap();
                    assert_ne!(GdiFlush(), 0);
                    let compact_pixels =
                        std::slice::from_raw_parts(compact.bits, w as usize * h as usize * 4);
                    let reference_pixels = std::slice::from_raw_parts(
                        reference.bits,
                        (w + 96) as usize * h as usize * 4,
                    );
                    let mut image = image::RgbaImage::new(w as u32, h as u32);
                    for (out, pixel) in image
                        .pixels_mut()
                        .zip(compact_pixels.as_chunks::<4>().0.iter())
                    {
                        // Composite the premultiplied surface onto a light paper.
                        let bg = 248u32 * (255 - pixel[3]) as u32 / 255;
                        *out = image::Rgba([
                            (pixel[2] as u32 + bg).min(255) as u8,
                            (pixel[1] as u32 + bg).min(255) as u8,
                            (pixel[0] as u32 + bg).min(255) as u8,
                            255,
                        ]);
                    }
                    image
                        .save(samples.join(format!("custom-{row_count}-rows-{dpi}.png")))
                        .unwrap();
                    for (actual, reference) in compact_pixels
                        .chunks_exact(w as usize * 4)
                        .zip(reference_pixels.chunks_exact((w + 96) as usize * 4))
                    {
                        assert!(
                            actual == &reference[..w as usize * 4],
                            "cell text clipped or overlapped at {dpi} DPI in {row_count} rows"
                        );
                        assert!(
                            reference[w as usize * 4..]
                                .as_chunks::<4>()
                                .0
                                .iter()
                                .all(|pixel| pixel[3] == 0)
                        );
                    }
                    assert!(
                        compact_pixels
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .any(|pixel| pixel[3] > 0 && pixel[3] < 255)
                    );
                }
            }
        }
        // A shorter label resets its reservation instead of retaining stale width.
        content.rows = vec![vec![TaskbarItem {
            metric: Metric::Cpu,
            label: "很长的自定义文案".into(),
            detailed: true,
            ..TaskbarItem::default()
        }]];
        let wide = renderer.frame(&content, &data).unwrap().width;
        content.rows[0][0].label.clear();
        assert!(renderer.frame(&content, &data).unwrap().width < wide);
    }

    #[test]
    fn transparent_text_is_antialiased_and_scales_with_dpi() {
        let mut renderer = TaskbarText::new().unwrap();
        let samples =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/taskbar-text-check");
        std::fs::create_dir_all(&samples).unwrap();
        let mut reference_heights = std::collections::HashMap::new();
        for dpi in [96, 120, 144, 192] {
            for horizontal in [true, false] {
                let scale = dpi as f32 / 96.;
                let width = (renderer.widths[usize::from(horizontal)] * scale).ceil() as i32;
                let height = (40. * scale).round() as i32;
                let surface = Bitmap::new(width, height);
                let bounds = RECT {
                    left: 0,
                    top: 0,
                    right: width,
                    bottom: height,
                };
                let text: Vec<u16> = content(horizontal, "1024.0 MB/s", "1024.0 MB/s")
                    .encode_utf16()
                    .collect();
                renderer.width(horizontal, &text).unwrap();
                for (theme, color) in [("light", 0x233349), ("dark", 0xecf3fc)] {
                    let byte_len = width as usize * height as usize * 4;
                    let pixels = unsafe {
                        // A transparent clear must replace even nonzero reused pixels.
                        std::ptr::write_bytes(surface.bits, 0xaa, byte_len);
                        renderer
                            .draw(surface.dc, bounds, dpi, &text, color)
                            .unwrap();
                        assert_ne!(GdiFlush(), 0);
                        std::slice::from_raw_parts(surface.bits, byte_len)
                    };
                    let mut ink = 0;
                    let mut edge = 0;
                    let mut ink_top = height;
                    let mut ink_bottom = 0;
                    let mut alpha_levels = std::collections::HashSet::new();
                    let mut image = image::RgbaImage::new(width as u32, height as u32);
                    for (index, (pixel, output)) in pixels
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .zip(image.pixels_mut())
                        .enumerate()
                    {
                        let alpha = pixel[3];
                        ink += usize::from(alpha > 0);
                        edge += usize::from(alpha > 0 && alpha < 255);
                        if alpha > 0 {
                            let y = index as i32 / width;
                            ink_top = ink_top.min(y);
                            ink_bottom = ink_bottom.max(y);
                        }
                        alpha_levels.insert(alpha);
                        assert!(pixel[..3].iter().all(|&channel| channel <= alpha));
                        let straight = |channel: u8| {
                            if alpha == 0 {
                                0
                            } else {
                                (channel as u32 * 255 / alpha as u32).min(255) as u8
                            }
                        };
                        *output = image::Rgba([
                            straight(pixel[2]),
                            straight(pixel[1]),
                            straight(pixel[0]),
                            alpha,
                        ]);
                    }
                    let layout = if horizontal { "horizontal" } else { "stacked" };
                    image
                        .save(samples.join(format!("{dpi}-{theme}-{layout}.png")))
                        .unwrap();
                    let background = if theme == "light" {
                        image::Rgba([248, 249, 251, 255])
                    } else {
                        image::Rgba([27, 29, 32, 255])
                    };
                    let mut preview =
                        image::RgbaImage::from_pixel(width as u32, height as u32, background);
                    image::imageops::overlay(&mut preview, &image, 0, 0);
                    preview
                        .save(samples.join(format!("{dpi}-{theme}-{layout}-preview.png")))
                        .unwrap();
                    println!(
                        "{dpi} DPI {theme} {layout}: ink={ink}, edge={edge}, alpha_levels={}",
                        alpha_levels.len()
                    );
                    assert!(ink > 100 && ink < width as usize * height as usize / 2);
                    // Grayscale rasterizers may quantize coverage; require real
                    // intermediate alpha edges, rather than a specific bit depth.
                    assert!(
                        edge > ink / 4 && alpha_levels.len() > 2,
                        "missing smooth coverage at {dpi} DPI"
                    );
                    let ink_height = (ink_bottom - ink_top + 1) as f32 / scale;
                    if dpi == 96 {
                        reference_heights.insert((horizontal, theme), ink_height);
                    } else {
                        assert!(
                            (ink_height - reference_heights[&(horizontal, theme)]).abs() <= 2.,
                            "glyph size did not track {dpi} DPI"
                        );
                    }
                    assert!(
                        pixels[..width as usize * 4]
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .all(|p| p[3] == 0)
                    );
                    assert!(
                        pixels[(height as usize - 1) * width as usize * 4..]
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .all(|p| p[3] == 0)
                    );
                }
            }
        }
        println!("Rendered taskbar samples: {}", samples.display());
    }

    #[test]
    fn compact_width_stays_stable_and_renders_without_clipping() {
        let mut renderer = TaskbarText::new().unwrap();
        for horizontal in [false, true] {
            let base = renderer.widths[usize::from(horizontal)];
            let old_width = if horizontal { 224. } else { 124. };
            assert!(
                base < old_width - 16.,
                "compact width did not remove excess space"
            );
            println!(
                "{}: {old_width} -> {base:.2} DIP",
                if horizontal { "horizontal" } else { "stacked" }
            );
            for value in [
                0.,
                1023.,
                1024.,
                1024f64.powi(2) - 1.,
                1024f64.powi(3) - 1.,
                1024f64.powi(4) - 1.,
                1024f64.powi(5) - 1.,
                u64::MAX as f64,
            ] {
                let speed = crate::model::speed(value);
                let text: Vec<_> = content(horizontal, &speed, &speed).encode_utf16().collect();
                let fitted = renderer.width(horizontal, &text).unwrap();
                if value < 1024f64.powi(5) {
                    assert_eq!(
                        fitted, base,
                        "ordinary speed {speed} changed the reserved width"
                    );
                }
                for dpi in [96, 120, 144, 192] {
                    let scale = dpi as f32 / 96.;
                    let width = (fitted * scale).ceil() as i32;
                    let height = (48. * scale).round() as i32;
                    let compact = Bitmap::new(width, height);
                    let reference = Bitmap::new(width + 96, height);
                    unsafe {
                        renderer
                            .draw(
                                compact.dc,
                                RECT {
                                    left: 0,
                                    top: 0,
                                    right: width,
                                    bottom: height,
                                },
                                dpi,
                                &text,
                                0x233349,
                            )
                            .unwrap();
                        renderer
                            .draw(
                                reference.dc,
                                RECT {
                                    left: 0,
                                    top: 0,
                                    right: width + 96,
                                    bottom: height,
                                },
                                dpi,
                                &text,
                                0x233349,
                            )
                            .unwrap();
                        assert_ne!(GdiFlush(), 0);
                        let compact_pixels = std::slice::from_raw_parts(
                            compact.bits,
                            width as usize * height as usize * 4,
                        );
                        let reference_pixels = std::slice::from_raw_parts(
                            reference.bits,
                            (width + 96) as usize * height as usize * 4,
                        );
                        for (compact_row, reference_row) in compact_pixels
                            .chunks_exact(width as usize * 4)
                            .zip(reference_pixels.chunks_exact((width + 96) as usize * 4))
                        {
                            assert_eq!(
                                compact_row,
                                &reference_row[..width as usize * 4],
                                "raster changed at {dpi} DPI for {speed}"
                            );
                            assert!(
                                reference_row[width as usize * 4..]
                                    .as_chunks::<4>()
                                    .0
                                    .iter()
                                    .all(|pixel| pixel[3] == 0),
                                "speed text clipped at {dpi} DPI for {speed}"
                            );
                        }
                    }
                }
            }
            let widest = renderer.widths[usize::from(horizontal)];
            let offline: Vec<_> = content(horizontal, "离线", "—").encode_utf16().collect();
            assert_eq!(
                renderer.width(horizontal, &offline).unwrap(),
                widest,
                "falling speeds shrank the layout"
            );
        }
    }
}
#[test]
fn inspector_example_keeps_live_column_segments_and_draft_settings() {
    use crate::taskbar_content::{ByteUnit, UnitStyle};
    let content = TaskbarContent::default();
    let data = Shared {
        connected: true,
        up: 128. * 1024.,
        down: 2. * 1024f64.powi(2),
        ..Default::default()
    };
    let actual = preview_frame(&content, &data);
    for metric in [
        Metric::Upload,
        Metric::Download,
        Metric::Cpu,
        Metric::Memory,
    ] {
        let item = content.item(metric).unwrap();
        let example = example_frame(&content, &data, item);
        let source = actual
            .rows
            .iter()
            .flatten()
            .find(|cell| cell.metric == metric)
            .unwrap();
        assert_eq!(example.rows.len(), 1);
        let cell = &example.rows[0][0];
        assert_eq!(cell.width, source.width);
        for (segment, source_segment) in cell.segments.iter().zip(&source.segments) {
            assert_eq!(segment.text, source_segment.text);
            assert_eq!(segment.width, source_segment.width);
            assert_eq!(segment.right_aligned, source_segment.right_aligned);
            assert!(((segment.x - cell.x) - (source_segment.x - source.x)).abs() < 0.001);
        }
    }
    let mut draft = TaskbarItem::new(Metric::TotalSpeed);
    draft.label = "合计".into();
    draft.unit = Some(ByteUnit::K);
    draft.unit_style = UnitStyle::Short;
    draft.precision = Some(2);
    let example = example_frame(&content, &data, &draft);
    let cell = &example.rows[0][0];
    assert_eq!(cell.segments[0].text, "合计:");
    assert_eq!(cell.segments[1].text, "2176.00");
    assert_eq!(cell.segments[2].text, "K/s");
    assert!(cell.segments[1].right_aligned && !cell.segments[2].right_aligned);
    assert_eq!(content, TaskbarContent::default());
    assert!(actual.metric_frame(Metric::TotalSpeed).is_none());
}

#[test]
fn spacing_changes_preserve_numeric_alignment_and_omit_empty_prefix_gaps() {
    let mut content = TaskbarContent::default();
    let data = Shared::default();
    let original = preview_frame(&content, &data);
    let prefix_delta = 18. - f32::from(content.prefix_gap);
    let previous_gap = f32::from(content.gap);
    content.prefix_gap = 18;
    content.gap = 0;
    let resized = preview_frame(&content, &data);
    assert!((resized.width - original.width - (prefix_delta * 2. - previous_gap)).abs() < 0.001);
    for column in 0..2 {
        let top = &resized.rows[0][column];
        let bottom = &resized.rows[1][column];
        assert_eq!(top.segments[1].x, bottom.segments[1].x);
        assert_eq!(top.segments[1].width, bottom.segments[1].width);
        assert_eq!(top.segments[2].x, bottom.segments[2].x);
        assert!(
            (top.segments[1].x - top.segments[0].x - top.segments[0].width - 18.).abs() < 0.001
        );
    }
    assert!((resized.rows[0][1].x - resized.rows[0][0].x - resized.rows[0][0].width).abs() < 0.001);
    for item in content.rows.iter_mut().flatten() {
        item.label.clear();
    }
    content.prefix_gap = 0;
    let without = preview_frame(&content, &data);
    content.prefix_gap = 32;
    let with = preview_frame(&content, &data);
    assert_eq!(without.width, with.width);
    for (a, b) in without
        .rows
        .iter()
        .flatten()
        .zip(with.rows.iter().flatten())
    {
        assert_eq!(a.segments[1].x, a.x);
        assert_eq!(a.segments[1].x, b.segments[1].x);
    }
}
