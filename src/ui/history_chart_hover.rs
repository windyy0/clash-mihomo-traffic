use super::*;
use crate::history::{Grain, Trend};
use std::sync::Arc;

const CARD_PADDING: f32 = 8.;
const CARD_GAP: f32 = 3.;
const CARD_MAX_WIDTH: f32 = 192.;
const YEAR_CARD_MAX_WIDTH: f32 = 160.;
const STATUS_FONT_SIZE: f32 = 12.;
const STATUS_LINE_HEIGHT: f32 = 18.;
const CARD_CLOSE_WIDTH: f32 = 16.;
const CARD_HEADER_GAP: f32 = 6.;
const SMALL_LINE: f32 = 16.;

#[derive(Clone, PartialEq)]
struct MetricsKey {
    scope: (i64, i64),
    grain: Grain,
    revision: u64,
    font: Font,
    scale: u32,
    view: history_state::CalendarView,
}

#[derive(Clone, Copy)]
pub(super) struct HoverRowLayout {
    pub size: Size<Pixels>,
    header_height: Pixels,
    label_width: Pixels,
    total_vertical: bool,
    total_font_size: f32,
}

pub(super) struct HoverMetrics {
    key: MetricsKey,
    snapshot: Arc<Trend>,
    width: Pixels,
    rows: Vec<HoverRowLayout>,
}

#[derive(Clone, Copy)]
pub(super) struct TrendHover {
    pub index: usize,
    pub scope: (i64, i64),
    pub bucket: (i64, i64),
    pub source_revision: u64,
    pub visible: bool,
    pub pinned: bool,
    pub sequence: u64,
}

fn range_caption(from: i64, to: i64) -> String {
    use chrono::{Datelike, Timelike};
    let Some(a) = chrono::DateTime::from_timestamp_millis(from) else {
        return String::new();
    };
    let Some(b) = chrono::DateTime::from_timestamp_millis(to - 1) else {
        return String::new();
    };
    let a = a.with_timezone(&chrono::Local);
    let b = b.with_timezone(&chrono::Local);
    let whole_days = a.hour() == 0 && a.minute() == 0 && b.hour() == 23 && b.minute() == 59;
    let dates = if a.date_naive() == b.date_naive() {
        format!("{}年 · {}月{}日", a.year(), a.month(), a.day())
    } else if a.year() != b.year() {
        format!("{}–{}", a.format("%Y年%m月%d日"), b.format("%Y年%m月%d日"))
    } else if a.month() != b.month() {
        format!(
            "{}年 · {}月{}日–{}月{}日",
            a.year(),
            a.month(),
            a.day(),
            b.month(),
            b.day()
        )
    } else {
        format!("{}年 · {}月{}–{}日", a.year(), a.month(), a.day(), b.day())
    };
    if whole_days {
        dates
    } else if a.date_naive() == b.date_naive() {
        format!("{dates}  {}–{}", a.format("%H:%M"), b.format("%H:%M"))
    } else {
        format!(
            "{}–{}",
            a.format("%Y年%m月%d日 %H:%M"),
            b.format("%Y年%m月%d日 %H:%M")
        )
    }
}

fn drill_hint(grain: Grain) -> Option<&'static str> {
    match grain {
        Grain::Minute => Some("右键查看前后5分钟"),
        Grain::Hour => Some("右键查看这一小时"),
        Grain::Day => Some("右键查看这一天"),
        Grain::Week => Some("右键查看这一周"),
        Grain::Months(1) => Some("右键查看这一个月"),
        Grain::Months(_) => Some("右键查看此时段"),
    }
}

fn clock_range(from: i64, to: i64) -> String {
    use chrono::Timelike;
    let local =
        |t| chrono::DateTime::from_timestamp_millis(t).map(|d| d.with_timezone(&chrono::Local));
    let (Some(a), Some(b)) = (local(from), local(to)) else {
        return String::new();
    };
    let end = if b.date_naive() != a.date_naive() && b.hour() == 0 && b.minute() == 0 {
        "24:00".into()
    } else {
        b.format("%H:%M").to_string()
    };
    format!("{}–{end}", a.format("%H:%M"))
}

fn bucket_caption(from: i64, to: i64, grain: Grain) -> String {
    if grain == Grain::Minute {
        use chrono::Timelike;
        return chrono::DateTime::from_timestamp_millis(from)
            .map(|d| {
                let d = d.with_timezone(&chrono::Local);
                format!("{}:{:02}", d.hour(), d.minute())
            })
            .unwrap_or_default();
    }
    if grain == Grain::Hour {
        clock_range(from, to)
    } else {
        short_dates(from, to)
    }
}

fn short_dates(from: i64, to: i64) -> String {
    let caption = range_caption(from, to);
    caption
        .split_once("年 · ")
        .map_or(caption.clone(), |(_, dates)| dates.to_owned())
}

fn overview_title(from: i64, to: i64, period: TimePeriod) -> String {
    match period {
        TimePeriod::LastSevenDays => "7天总览".into(),
        TimePeriod::LastThreeMonths => "3个月总览".into(),
        _ => short_dates(from, to),
    }
}

fn bucket_at(plot: Bounds<Pixels>, count: usize, cursor: Point<Pixels>) -> Option<usize> {
    if count == 0
        || plot.size.width <= px(0.)
        || !plot.contains(&cursor)
        || cursor.x >= plot.right()
    {
        return None;
    }
    Some(
        ((f32::from(cursor.x - plot.left()) / f32::from(plot.size.width) * count as f32) as usize)
            .min(count - 1),
    )
}

fn hover_anchor(
    plot: Bounds<Pixels>,
    index: usize,
    count: usize,
    viewport: Size<Pixels>,
    card: Size<Pixels>,
) -> Point<Pixels> {
    let slot = plot.size.width / count.max(1) as f32;
    let left = plot.left() + slot * index as f32;
    let right = left + slot;
    let x = if right + card.width + px(17.) <= viewport.width {
        right + px(8.)
    } else {
        left - card.width - px(8.)
    };
    point(
        x.max(px(8.))
            .min((viewport.width - card.width - px(9.)).max(px(8.))),
        (plot.top() + px(6.))
            .max(px(60.))
            .min((viewport.height - card.height - px(14.)).max(px(60.))),
    )
}

impl TrafficApp {
    pub(super) fn history_hover_layout(
        &self,
        data: &Arc<Trend>,
        index: usize,
        window: &Window,
    ) -> HoverRowLayout {
        let mut style = window.text_style();
        style.font_family = "Microsoft YaHei UI".into();
        style.font_weight = FontWeight::NORMAL;
        let key = MetricsKey {
            scope: (data.from, data.to),
            grain: data.grain,
            revision: self.data.storage_revision,
            font: style.font(),
            scale: window.scale_factor().to_bits(),
            view: self.history_state.view,
        };
        let mut cache = self.history_state.chart_hover_metrics.borrow_mut();
        if let Some(cached) = cache
            .as_ref()
            .filter(|c| c.key == key && Arc::ptr_eq(&c.snapshot, data))
        {
            return cached.rows[index];
        }
        let text_width = |text: &str, font_size: f32, weight: FontWeight| {
            let mut style = style.clone();
            style.font_weight = weight;
            window
                .text_system()
                .shape_line(
                    text.to_owned().into(),
                    px(font_size),
                    &[style.to_run(text.len())],
                    None,
                )
                .width
        };
        let direction_width = |name: &str, bytes| {
            text_width(name, 11., FontWeight::NORMAL)
                + px(6.)
                + text_width(&human_bytes(bytes), 11., FontWeight::NORMAL)
        };
        let label_width = ["合计", "下载", "上传"]
            .into_iter()
            .map(|name| text_width(name, 11., FontWeight::NORMAL))
            .fold(px(0.), Pixels::max)
            .ceil();
        let mut needed = px(0.);
        for bucket in &data.buckets {
            let caption =
                history_range::display_text(&bucket_caption(bucket.from, bucket.to, data.grain));
            needed = needed.max(
                text_width(&caption, 11., FontWeight::MEDIUM)
                    + px(CARD_HEADER_GAP + CARD_CLOSE_WIDTH),
            );
            if bucket.observed {
                needed = needed.max(
                    label_width
                        + px(6.)
                        + text_width(&human_bytes(bucket.bytes.total()), 18., FontWeight::MEDIUM),
                );
                needed = needed.max(
                    direction_width("下载", bucket.bytes.down)
                        .max(direction_width("上传", bucket.bytes.up)),
                );
            } else {
                needed = needed.max(text_width(
                    if bucket.from < data.generated_at {
                        "无记录"
                    } else {
                        "时段未开始"
                    },
                    STATUS_FONT_SIZE,
                    FontWeight::NORMAL,
                ));
            }
            if let Some(hint) = drill_hint(data.grain).filter(|_| bucket.from < data.generated_at) {
                needed = needed.max(text_width(hint, 11., FontWeight::NORMAL));
            }
        }
        let old_width = cache
            .as_ref()
            .filter(|c| c.key == key)
            .map_or(px(0.), |c| c.width);
        let max_width = if self.history_state.view == history_state::CalendarView::Year {
            YEAR_CARD_MAX_WIDTH
        } else {
            CARD_MAX_WIDTH
        };
        let width = (needed + px(CARD_PADDING * 2. + 2.))
            .ceil()
            .max(old_width)
            .min(px(max_width));
        let content_width = width - px(CARD_PADDING * 2. + 2.);
        let rows = data
            .buckets
            .iter()
            .map(|bucket| {
                let caption = history_range::display_text(&bucket_caption(
                    bucket.from,
                    bucket.to,
                    data.grain,
                ));
                let mut header_style = style.clone();
                header_style.font_weight = FontWeight::MEDIUM;
                let header_height = window
                    .text_system()
                    .shape_text(
                        caption.clone().into(),
                        px(11.),
                        &[header_style.to_run(caption.len())],
                        Some((content_width - px(CARD_HEADER_GAP + CARD_CLOSE_WIDTH)).max(px(1.))),
                        None,
                    )
                    .map(|lines| {
                        lines
                            .iter()
                            .map(|line| line.size(px(SMALL_LINE)).height)
                            .fold(px(0.), |total, height| total + height)
                    })
                    .unwrap_or(px(SMALL_LINE))
                    .max(px(SMALL_LINE));
                let total_width =
                    text_width(&human_bytes(bucket.bytes.total()), 18., FontWeight::MEDIUM);
                let total_vertical = label_width + px(6.) + total_width > content_width;
                let total_font_size = if total_width > content_width {
                    (18. * f32::from(content_width) / f32::from(total_width))
                        .floor()
                        .clamp(12., 18.)
                } else {
                    18.
                };
                let total_height = if total_vertical {
                    27. + SMALL_LINE + CARD_GAP
                } else {
                    27.
                };
                let mut heights = vec![
                    header_height,
                    px(if bucket.observed {
                        total_height
                    } else {
                        STATUS_LINE_HEIGHT
                    }),
                ];
                if bucket.observed {
                    heights.extend([px(SMALL_LINE), px(SMALL_LINE)]);
                }
                if drill_hint(data.grain).is_some() && bucket.from < data.generated_at {
                    heights.extend([px(1.), px(SMALL_LINE)]);
                }
                let height = heights
                    .iter()
                    .copied()
                    .fold(px(0.), |total, height| total + height)
                    + px(CARD_GAP * (heights.len() - 1) as f32 + CARD_PADDING * 2. + 2.);
                HoverRowLayout {
                    size: size(width, height),
                    header_height,
                    label_width,
                    total_vertical,
                    total_font_size,
                }
            })
            .collect::<Vec<_>>();
        let result = rows[index];
        *cache = Some(HoverMetrics {
            key,
            snapshot: data.clone(),
            width,
            rows,
        });
        result
    }
    pub(super) fn track_history_chart_pointer(
        &mut self,
        cursor: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if self.seconds.scope.is_some() {
            return;
        }
        if self.valid_history_chart_hover().is_some_and(|h| h.pinned) {
            return;
        }
        let h = &self.history_state;
        let index = self
            .report
            .history
            .as_ref()
            .filter(|_| {
                self.page == Page::History
                    && h.tab == HistoryTab::Records
                    && !h.help_open
                    && !h.month_open
                    && !h.time_expanded
                    && h.drag.is_none()
                    && self.report.query.as_ref() == Some(&self.query)
            })
            .and_then(|data| {
                let plot = h.chart_plot_bounds.get();
                if h.chart_motion.borrow().animating() {
                    if plot.size.width <= px(0.) || !plot.contains(&cursor) {
                        return None;
                    }
                    h.chart_motion.borrow().hit(
                        f32::from(cursor.x - plot.left()) / f32::from(plot.size.width),
                        data.buckets.len(),
                        (self.query.from, self.query.to),
                        self.data.storage_revision,
                    )
                } else {
                    bucket_at(plot, data.buckets.len(), cursor)
                }
            });
        if let Some(index) = index {
            self.hover_history_chart(index, true, cx);
        } else {
            self.close_history_chart_hover(cx);
        }
    }

    pub(super) fn history_chart_card_right_click(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(hover) = self.valid_history_chart_hover().filter(|h| h.visible) else {
            return false;
        };
        if event.button != MouseButton::Right
            || !self
                .history_state
                .chart_hover_bounds
                .get()
                .contains(&event.position)
        {
            return false;
        }
        window.prevent_default();
        cx.stop_propagation();
        if let Some(data) = &self.report.history {
            let b = &data.buckets[hover.index];
            if b.from < data.generated_at {
                self.act(
                    Action::History(HistoryAction::Bucket(b.from, b.to, data.grain)),
                    window,
                    cx,
                );
            }
        }
        true
    }
    pub(super) fn valid_history_chart_hover(&self) -> Option<TrendHover> {
        self.history_state.chart_hover.filter(|h| {
            self.page == Page::History
                && self.history_state.tab == HistoryTab::Records
                && h.scope == (self.query.from, self.query.to)
                && h.source_revision == self.data.storage_revision
                && self.report.query.as_ref() == Some(&self.query)
                && self
                    .report
                    .history
                    .as_ref()
                    .and_then(|d| d.buckets.get(h.index))
                    .is_some_and(|b| h.bucket == (b.from, b.to))
        })
    }
    pub(super) fn hover_history_chart(
        &mut self,
        index: usize,
        entered: bool,
        cx: &mut Context<Self>,
    ) {
        if self.valid_history_chart_hover().is_some_and(|h| h.pinned) {
            return;
        }
        if entered {
            if self
                .valid_history_chart_hover()
                .is_some_and(|h| h.index == index && h.visible)
            {
                return;
            }
            let sequence = if let Some(h) = self.valid_history_chart_hover().filter(|h| h.visible) {
                h.sequence
            } else {
                self.history_state.chart_hover_sequence += 1;
                self.history_state.chart_hover_sequence
            };
            let Some(bucket) = self
                .report
                .history
                .as_ref()
                .and_then(|d| d.buckets.get(index))
            else {
                return;
            };
            self.history_state.chart_hover = Some(TrendHover {
                index,
                scope: (self.query.from, self.query.to),
                bucket: (bucket.from, bucket.to),
                source_revision: self.data.storage_revision,
                visible: true,
                pinned: false,
                sequence,
            });
            cx.notify();
        } else if self
            .valid_history_chart_hover()
            .is_some_and(|h| h.index == index && h.visible)
        {
            self.close_history_chart_hover(cx);
        }
    }
    pub(super) fn close_history_chart_hover(&mut self, cx: &mut Context<Self>) {
        let Some(mut hover) = self.history_state.chart_hover else {
            return;
        };
        if !hover.visible {
            return;
        }
        self.history_state.chart_hover_sequence += 1;
        hover.visible = false;
        hover.pinned = false;
        hover.sequence = self.history_state.chart_hover_sequence;
        self.history_state.chart_hover = Some(hover);
        cx.notify();
        // Only removal waits for the exit animation. Entry has no tooltip delay.
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(160))
                .await;
            let _ = this.update(cx, |app, cx| {
                if app
                    .history_state
                    .chart_hover
                    .is_some_and(|h| h.sequence == hover.sequence && !h.visible)
                {
                    app.history_state.chart_hover = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }
    pub(super) fn pin_history_chart(&mut self, index: usize, cx: &mut Context<Self>) {
        if self
            .valid_history_chart_hover()
            .is_some_and(|h| h.index == index && h.pinned)
        {
            self.close_history_chart_hover(cx);
            return;
        }
        if self
            .valid_history_chart_hover()
            .is_some_and(|h| h.index == index && h.visible)
        {
            if let Some(h) = &mut self.history_state.chart_hover {
                h.pinned = true;
            }
            cx.notify();
            return;
        }
        self.history_state.chart_hover = None;
        self.hover_history_chart(index, true, cx);
        if let Some(h) = &mut self.history_state.chart_hover {
            h.pinned = true;
        }
    }
    pub(super) fn history_chart_hover(
        &self,
        window: &Window,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let hover = self.valid_history_chart_hover()?;
        let h = &self.history_state;
        if h.help_open || h.month_open || h.time_expanded || h.drag.is_some() {
            return None;
        }
        let data = self.report.history.as_ref()?;
        let bucket = data.buckets.get(hover.index)?;
        let p = self.p;
        let layout = self.history_hover_layout(data, hover.index, window);
        let plot = h.chart_plot_bounds.get();
        let slot = h.chart_motion.borrow().slot(
            hover.index,
            (self.query.from, self.query.to),
            self.data.storage_revision,
        );
        let (anchor_plot, anchor_index, anchor_count) = if let Some((left, width)) = slot {
            (
                Bounds::new(
                    point(plot.left() + plot.size.width * left, plot.top()),
                    size(plot.size.width * width, plot.size.height),
                ),
                0,
                1,
            )
        } else {
            (plot, hover.index, data.buckets.len())
        };
        let anchor = hover_anchor(
            anchor_plot,
            anchor_index,
            anchor_count,
            window.viewport_size(),
            layout.size,
        );
        let measured = h.chart_hover_bounds.clone();
        let from = bucket.from;
        let to = bucket.to;
        let grain = data.grain;
        let can_enter = from < data.generated_at;
        let hint = can_enter.then(|| drill_hint(grain)).flatten();
        let caption = bucket_caption(from, to, grain);
        let mut panel = column()
            .absolute()
            .left(anchor.x)
            .top(anchor.y)
            .w(layout.size.width)
            .p(px(CARD_PADDING))
            .gap(px(CARD_GAP))
            .font_family("Microsoft YaHei UI")
            .rounded(px(8.))
            .bg(popup_surface(p))
            .border_1()
            .border_color(control_border(p))
            .shadow_md()
            .overflow_hidden()
            .id("history-chart-hover")
            .when(hover.visible && hover.pinned, |d| d.occlude())
            .child(
                // An absolutely positioned 100% canvas measures the padding
                // box. Include the 1 DIP border for accurate pointer hit tests.
                canvas(
                    move |rect, _, _| {
                        measured.set(Bounds::new(
                            rect.origin - point(px(1.), px(1.)),
                            rect.size + size(px(2.), px(2.)),
                        ))
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .child(
                row()
                    .justify_between()
                    .items_start()
                    .h(layout.header_height)
                    .gap(px(CARD_HEADER_GAP))
                    .child(
                        history_range::label(caption, Some(p.text), FontWeight::MEDIUM, true)
                            .flex_1()
                            .min_w_0(),
                    )
                    .child(
                        div()
                            .id("history-chart-unpin")
                            .w(px(CARD_CLOSE_WIDTH))
                            .h(px(SMALL_LINE))
                            .flex_none()
                            .when(hover.pinned, |d| {
                                d.cursor_pointer()
                                    .flex()
                                    .justify_center()
                                    .items_center()
                                    .child(
                                        label("×", p.muted)
                                            .text_size(px(13.))
                                            .line_height(px(SMALL_LINE)),
                                    )
                                    .on_click(cx.listener(|app, _, _, cx| {
                                        app.close_history_chart_hover(cx)
                                    }))
                            }),
                    ),
            );
        if bucket.observed {
            panel = panel
                .child(
                    row()
                        .gap(px(if layout.total_vertical { CARD_GAP } else { 6. }))
                        .when(layout.total_vertical, |d| d.flex_col().items_start())
                        .child(
                            label("合计", p.muted)
                                .text_size(px(11.))
                                .line_height(px(SMALL_LINE))
                                .w(layout.label_width)
                                .flex_none(),
                        )
                        .child(
                            label(human_bytes(bucket.bytes.total()), p.text)
                                .text_size(px(layout.total_font_size))
                                .line_height(px(27.))
                                .whitespace_nowrap()
                                .font_weight(FontWeight::MEDIUM),
                        ),
                )
                .children(
                    [
                        ("下载", bucket.bytes.down, p.download_text),
                        ("上传", bucket.bytes.up, p.upload_text),
                    ]
                    .map(|(title, bytes, color)| {
                        row()
                            .gap(px(6.))
                            .child(
                                label(title, p.muted)
                                    .text_size(px(11.))
                                    .line_height(px(SMALL_LINE))
                                    .w(layout.label_width)
                                    .flex_none()
                                    .whitespace_nowrap(),
                            )
                            .child(
                                label(human_bytes(bytes), color)
                                    .text_size(px(11.))
                                    .line_height(px(SMALL_LINE))
                                    .whitespace_nowrap(),
                            )
                    }),
                );
        } else {
            panel = panel.child(
                label(
                    if can_enter {
                        "无记录"
                    } else {
                        "时段未开始"
                    },
                    p.muted,
                )
                .text_size(px(STATUS_FONT_SIZE))
                .line_height(px(STATUS_LINE_HEIGHT))
                .whitespace_nowrap(),
            );
        }
        if let Some(hint) = hint {
            panel = panel
                .child(div().h(px(1.)).w_full().bg(control_border(p)))
                .child(
                    row().justify_end().child(
                        label(hint, p.muted)
                            .text_size(px(11.))
                            .line_height(px(SMALL_LINE))
                            .whitespace_nowrap(),
                    ),
                );
        }
        let visible = hover.visible;
        Some(
            panel
                .with_animation(
                    SharedString::from(format!("history-chart-hover-{}", hover.sequence)),
                    Animation::new(Duration::from_millis(150)).with_easing(ease_out_quint()),
                    move |panel, delta| {
                        let progress = if visible { delta } else { 1. - delta };
                        panel
                            .top(anchor.y + px(5. * (1. - progress)))
                            .opacity(if visible { 1. } else { progress })
                    },
                )
                .into_any_element(),
        )
    }
    pub(super) fn history_chart_path(&self) -> Vec<(String, Option<usize>)> {
        if self.history_state.range_stack.is_empty() {
            return vec![(range_caption(self.query.from, self.query.to), None)];
        }
        let mut path = self
            .history_state
            .range_stack
            .iter()
            .enumerate()
            .map(|(index, &(from, to, period))| {
                (
                    if index == 0 {
                        overview_title(from, to, period)
                    } else if to - from <= 60 * crate::history::MINUTE {
                        clock_range(from, to)
                    } else {
                        short_dates(from, to)
                    },
                    Some(index),
                )
            })
            .collect::<Vec<_>>();
        let from = self.query.from;
        let to = self.query.to;
        if to - from <= 60 * crate::history::MINUTE {
            path.push((clock_range(from, to), None));
        } else {
            path.push((short_dates(from, to), None));
        }
        path
    }

    pub(super) fn history_chart_heading(&self, compact: bool, cx: &Context<Self>) -> Div {
        let p = self.p;
        let caption = range_caption(self.query.from, self.query.to);
        let trend = if self.history_state.seconds_level {
            self.seconds.data.as_ref().and_then(|d| d.trend.as_ref())
        } else {
            self.report.history.as_deref()
        };
        let full_caption = trend
            .filter(|_| {
                self.history_state.seconds_level || self.report.query.as_ref() == Some(&self.query)
            })
            .map_or_else(
                || caption.clone(),
                |trend| {
                    format!(
                        "{caption}\n图表范围 {}\n图表合计 {}",
                        range_caption(trend.from, trend.to),
                        human_bytes(trend.total().total())
                    )
                },
            );
        let back_title = self
            .history_state
            .range_stack
            .last()
            .map(|&(from, to, period)| {
                if self.history_state.seconds_level {
                    "返回上级"
                } else if self.query.history_context.is_some() || to - from <= crate::history::DAY {
                    "返回全天"
                } else {
                    match period {
                        TimePeriod::LastSevenDays => "返回7天总览",
                        TimePeriod::LastThreeMonths => "返回3个月总览",
                        _ => "返回所选范围",
                    }
                }
            });
        let back_bounds = self.history_state.chart_back_bounds.clone();
        let legend = |title, color| {
            row()
                .gap(px(5.))
                .child(div().w(px(9.)).h(px(4.)).rounded(px(2.)).bg(rgb(color)))
                .child(label(title, p.muted).text_size(px(11.)))
        };
        let path = self.history_chart_path();
        let last = path.len() - 1;
        let drilled = !self.history_state.range_stack.is_empty();
        let mut crumbs = row().min_w_0().gap(px(3.)).h(px(24.));
        for (n, (text, target)) in path.into_iter().enumerate() {
            if n > 0 {
                crumbs = crumbs.child(label("›", p.muted).text_size(px(13.)).flex_none());
            }
            crumbs = if let Some(index) = target {
                crumbs.child(
                    self.button(
                        format!("history-chart-ancestor-{index}"),
                        "",
                        false,
                        Action::History(HistoryAction::BackTo(index)),
                        cx,
                    )
                    .h(px(24.))
                    .px(px(2.))
                    .min_w_0()
                    .text_size(px(11.))
                    .child(history_range::label(
                        text,
                        None,
                        FontWeight::NORMAL,
                        false,
                    )),
                )
            } else {
                crumbs.child(
                    history_range::label(
                        text,
                        Some(if drilled && n == last {
                            p.accent_text
                        } else {
                            p.muted
                        }),
                        if drilled && n == last {
                            FontWeight::MEDIUM
                        } else {
                            FontWeight::NORMAL
                        },
                        false,
                    )
                    .min_w_0()
                    .truncate(),
                )
            };
        }
        let crumbs = crumbs.id("history-chart-range").tooltip(move |window, cx| {
            let text = full_caption.clone();
            Tooltip::element(move |_, _| {
                history_range::label(text.clone(), Some(p.text), FontWeight::NORMAL, true)
                    .max_w(px(340.))
            })
            .build(window, cx)
        });
        let controls = row()
            .flex_none()
            .gap(px(10.))
            .when_some(back_title, |d, title| {
                d.child(
                    self.button(
                        "history-range-back",
                        title,
                        false,
                        Action::History(HistoryAction::Calendar(
                            history_state::CalendarAction::Back,
                        )),
                        cx,
                    )
                    .relative()
                    .h(px(26.))
                    .px(px(6.))
                    .text_size(px(11.))
                    .border_1()
                    .border_color(control_border(p))
                    .rounded(px(6.))
                    .child(
                        canvas(move |rect, _, _| back_bounds.set(rect), |_, _, _, _| {})
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full(),
                    ),
                )
            })
            .child(legend("下载", p.download))
            .child(legend("上传", p.upload));
        if compact {
            column()
                .h(px(52.))
                .gap(px(2.))
                .child(crumbs.w_full())
                .child(row().w_full().justify_end().child(controls))
        } else {
            row()
                .h(px(28.))
                .gap(px(8.))
                .justify_between()
                .child(crumbs.flex_1())
                .child(controls)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn minute(y: i32, m: u32, d: u32, h: i64, min: i64) -> i64 {
        crate::history::midnight(chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap()).unwrap()
            + (h * 60 + min) * crate::history::MINUTE
    }
    #[::core::prelude::v1::test]
    fn range_labels_keep_inclusive_minutes_and_calendar_boundaries() {
        assert_eq!(
            range_caption(minute(2024, 2, 19, 0, 0), minute(2024, 2, 26, 0, 0)),
            "2024年 · 2月19–25日"
        );
        assert_eq!(
            range_caption(minute(2024, 2, 29, 0, 0), minute(2024, 3, 2, 0, 0)),
            "2024年 · 2月29日–3月1日"
        );
        assert_eq!(
            range_caption(minute(2024, 12, 31, 0, 0), minute(2025, 1, 2, 0, 0)),
            "2024年12月31日–2025年01月01日"
        );
        assert_eq!(
            range_caption(minute(2024, 2, 20, 9, 15), minute(2024, 2, 20, 9, 16)),
            "2024年 · 2月20日  09:15–09:15"
        );
    }
    #[::core::prelude::v1::test]
    fn preview_changes_sides_and_stays_in_the_viewport() {
        let plot = Bounds::new(point(px(500.), px(300.)), size(px(350.), px(166.)));
        let viewport = size(px(900.), px(700.));
        let card = size(px(150.), px(83.));
        let left = hover_anchor(plot, 0, 7, viewport, card);
        let right = hover_anchor(plot, 6, 7, viewport, card);
        assert!(left.x > plot.left());
        assert!(right.x + card.width < plot.right());
        let bottom = hover_anchor(
            Bounds::new(point(px(10.), px(670.)), plot.size),
            0,
            60,
            viewport,
            card,
        );
        assert!(bottom.y + card.height + px(5.) <= viewport.height - px(8.));
    }
    #[::core::prelude::v1::test]
    fn pointer_tracking_keeps_one_bucket_and_popup_outside_its_band() {
        let plot = Bounds::new(point(px(500.), px(300.)), size(px(350.), px(166.)));
        let viewport = size(px(900.), px(700.));
        for count in [7, 24, 60] {
            for n in 0..count {
                let left = plot.left() + plot.size.width * n as f32 / count as f32;
                let right = plot.left() + plot.size.width * (n + 1) as f32 / count as f32;
                for fraction in [0.01, 0.25, 0.5, 0.99] {
                    assert_eq!(
                        bucket_at(
                            plot,
                            count,
                            point(left + (right - left) * fraction, plot.center().y)
                        ),
                        Some(n)
                    );
                }
                for card in [
                    size(px(70.), px(66.)),
                    size(px(160.), px(83.)),
                    size(px(192.), px(145.)),
                ] {
                    let anchor = hover_anchor(plot, n, count, viewport, card);
                    assert!(
                        anchor.x >= right || anchor.x + card.width <= left,
                        "popup overlaps active bucket {n}/{count}"
                    );
                    assert!(anchor.x >= px(8.) && anchor.x + card.width <= viewport.width - px(8.));
                    assert!(anchor.y + card.height + px(5.) <= viewport.height - px(8.));
                }
            }
        }
        assert_eq!(bucket_at(plot, 0, plot.center()), None);
        assert_eq!(
            bucket_at(plot, 24, point(plot.right(), plot.center().y)),
            None
        );
        assert_eq!(
            bucket_at(plot, 24, point(plot.left(), plot.top() - px(1.))),
            None
        );
    }
    #[::core::prelude::v1::test]
    fn clock_ranges_show_bucket_endpoints_and_minutes_offer_seconds_drill() {
        assert_eq!(
            clock_range(minute(2024, 2, 20, 14, 0), minute(2024, 2, 20, 15, 0)),
            "14:00–15:00"
        );
        assert_eq!(
            clock_range(minute(2024, 2, 20, 14, 17), minute(2024, 2, 20, 14, 18)),
            "14:17–14:18"
        );
        assert_eq!(
            clock_range(minute(2024, 2, 20, 23, 0), minute(2024, 2, 21, 0, 0)),
            "23:00–24:00"
        );
        assert_eq!(drill_hint(Grain::Minute), Some("右键查看前后5分钟"));
        assert_eq!(
            bucket_caption(
                minute(2024, 2, 20, 1, 19),
                minute(2024, 2, 20, 1, 20),
                Grain::Minute
            ),
            "1:19"
        );
        assert_eq!(
            bucket_caption(
                minute(2024, 2, 20, 23, 59),
                minute(2024, 2, 21, 0, 0),
                Grain::Minute
            ),
            "23:59"
        );
        assert_eq!(
            bucket_caption(
                minute(2024, 2, 20, 13, 0),
                minute(2024, 2, 20, 14, 0),
                Grain::Hour
            ),
            "13:00–14:00"
        );
    }
}
