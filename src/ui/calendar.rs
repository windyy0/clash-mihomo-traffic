use super::*;
use crate::history::{CalendarMonth, Coverage};
use chrono::{Datelike, NaiveDate};
use history_page::{CalendarAction, HistoryTab};

const DAY_SIDE: f32 = 28.;
const GRID_WIDTH: f32 = 242.;

fn mix(a: u32, b: u32, percent: u32) -> u32 {
    let channel = |s: u32| {
        ((((a >> s) & 255u32) * (100 - percent) + ((b >> s) & 255u32) * percent) / 100) << s
    };
    channel(16) | channel(8) | channel(0)
}
pub(super) fn heat(p: Palette, level: usize) -> Rgba {
    rgb(mix(
        p.bg,
        p.download,
        if p.dark {
            [6, 14, 24, 33, 42][level]
        } else {
            [6, 14, 29, 48, 70][level]
        },
    ))
}
fn cal_action(action: CalendarAction) -> Action {
    Action::History(HistoryAction::Calendar(action))
}
pub(super) fn annual_missing(p: Palette) -> Rgba {
    rgb(mix(p.bg, p.text, if p.dark { 12 } else { 8 }))
}
fn tile_path(rect: Bounds<Pixels>, radius: Pixels, mut path: PathBuilder) -> PathBuilder {
    let left = rect.left();
    let right = rect.right();
    let top = rect.top();
    let bottom = rect.bottom();
    path.move_to(point(left + radius, top));
    path.line_to(point(right - radius, top));
    path.curve_to(point(right, top + radius), point(right, top));
    path.line_to(point(right, bottom - radius));
    path.curve_to(point(right - radius, bottom), point(right, bottom));
    path.line_to(point(left + radius, bottom));
    path.curve_to(point(left, bottom - radius), point(left, bottom));
    path.line_to(point(left, top + radius));
    path.curve_to(point(left + radius, top), point(left, top));
    path.close();
    path
}
pub(super) fn paint_missing_outline(
    rect: Bounds<Pixels>,
    p: Palette,
    inset: f32,
    window: &mut Window,
) {
    let rect = Bounds::new(
        rect.origin + point(px(inset), px(inset)),
        rect.size - size(px(inset * 2.), px(inset * 2.)),
    );
    let compact = rect.size.width < px(16.);
    let dash = if compact { px(1.5) } else { px(2.) };
    let path = tile_path(
        rect,
        (rect.size.width * 0.22).min(px(3.)),
        PathBuilder::stroke(if compact { px(0.75) } else { px(1.) }).dash_array(&[dash, dash]),
    );
    if let Ok(path) = path.build() {
        let mut color = rgb(p.muted);
        color.a = match (compact, p.dark) {
            (true, true) => 0.3,
            (true, false) => 0.24,
            (false, true) => 0.85,
            (false, false) => 0.7,
        };
        window.paint_path(path, color);
    }
}
pub(super) fn missing_marker(p: Palette) -> Div {
    div().size(px(12.)).flex_none().child(
        canvas(
            |_, _, _| (),
            move |r, _, window, _| paint_missing_outline(r, p, 1., window),
        )
        .size_full(),
    )
}
pub(super) fn today_marker(p: Palette) -> Div {
    label(chrono::Local::now().day().to_string(), today_color(p))
        .size(px(14.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(12.))
        .font_weight(FontWeight::NORMAL)
}
pub(super) fn today_color(p: Palette) -> u32 {
    if p.dark {
        mix(p.accent, 0xffffff, 12)
    } else {
        p.accent
    }
}
pub(super) fn selection_color(p: Palette) -> Rgba {
    rgb(if p.dark { 0x9acbff } else { 0x195fc2 })
}
pub(super) fn paint_range_outline(
    origin: Point<Pixels>,
    contours: &[Vec<(f32, f32)>],
    p: Palette,
    window: &mut Window,
) {
    let mut halo = selection_color(p);
    halo.a = 0.06;
    for contour in contours {
        let points: Vec<_> = contour
            .iter()
            .map(|&(x, y)| origin + point(px(x), px(y)))
            .collect();
        for (width, color) in [(3., halo), (1., selection_color(p))] {
            let mut path = PathBuilder::stroke(px(width));
            for (i, &current) in points.iter().enumerate() {
                let previous = points[(i + points.len() - 1) % points.len()];
                let next = points[(i + 1) % points.len()];
                let length =
                    |a: Point<Pixels>, b: Point<Pixels>| (a.x - b.x).abs() + (a.y - b.y).abs();
                let radius = px(5.)
                    .min(px(5.))
                    .min(length(current, previous) / 2.)
                    .min(length(current, next) / 2.);
                let toward = |target: Point<Pixels>| {
                    current
                        + point(
                            if target.x < current.x {
                                -radius
                            } else if target.x > current.x {
                                radius
                            } else {
                                px(0.)
                            },
                            if target.y < current.y {
                                -radius
                            } else if target.y > current.y {
                                radius
                            } else {
                                px(0.)
                            },
                        )
                };
                let entry = toward(previous);
                if i == 0 {
                    path.move_to(entry);
                } else {
                    path.line_to(entry);
                }
                path.curve_to(toward(next), current);
            }
            path.close();
            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        }
    }
}

impl TrafficApp {
    pub(super) fn history(&self, window: &Window, cx: &Context<Self>) -> Div {
        let p = self.p;
        let available = window.viewport_size().width - self.sidebar_width() - px(48.);
        let narrow = available < px(590.);
        let annual = self.history_state.view == history_state::CalendarView::Year;
        let calendar = if annual {
            self.history_annual_calendar(window, cx)
        } else {
            self.history_calendar(window, cx)
        }
        .when(!narrow && !annual, |d| d.w(px(276.)).flex_none());
        let top = stretch()
            .when(narrow || annual, |d| d.flex_col())
            .gap(px(16.))
            .child(calendar)
            .child(
                self.history_hero(
                    if narrow || annual {
                        available
                    } else {
                        available - px(292.)
                    } < px(480.),
                    window,
                    cx,
                )
                .min_w_0()
                .when(narrow || annual, |d| d.w_full())
                .when(!narrow && !annual, |d| d.flex_1()),
            );
        column()
            .gap(px(16.))
            .when(
                !self.history_state.error.is_empty() && !self.history_state.time_open,
                |d| d.child(label(self.history_state.error.clone(), p.amber).whitespace_normal()),
            )
            .child(top)
            .child(self.history_record_table(window, cx))
            .child(self.history_quality(window, cx))
    }
    fn history_hero(&self, compact: bool, window: &Window, cx: &Context<Self>) -> Div {
        let p = self.p;
        let inline = self.history_state.seconds_level;
        let trend = if inline {
            self.seconds.data.as_ref().and_then(|d| d.trend.as_ref())
        } else {
            self.report.history.as_deref()
        };
        let loading = if inline {
            self.seconds.pending.is_some() || trend.is_none()
        } else {
            self.report.query.as_ref() != Some(&self.query)
        };
        let observed = trend.is_some_and(|h| {
            h.buckets
                .iter()
                .any(|b| b.from < self.query.to && b.to > self.query.from && b.observed)
        });
        let total = if inline {
            trend.map_or_else(Bytes::default, |h| h.total())
        } else {
            self.report.total
        };
        let bounds = self.history_state.hero_bounds.clone();
        let value = |bytes| {
            if loading || !observed {
                "—".into()
            } else {
                human_bytes(bytes)
            }
        };
        let direction = |title, bytes, color, icon| {
            let caption = row()
                .gap(px(5.))
                .text_color(rgb(color))
                .child(Icon::default().path(icon).size(px(14.)))
                .child(label(title, p.muted).text_size(px(12.)));
            div()
                .flex()
                .min_w_0()
                .flex_1()
                .when(compact, |d| d.items_center().justify_between().gap(px(8.)))
                .when(!compact, |d| d.flex_col().gap(px(5.)))
                .child(caption)
                .child(
                    label(value(bytes), color)
                        .text_size(px(if compact { 15. } else { 22. }))
                        .font_weight(FontWeight::MEDIUM)
                        .whitespace_nowrap(),
                )
        };
        let mut icon_tint = rgb(p.download);
        icon_tint.a = if p.dark { 0.13 } else { 0.07 };
        let mut separator = rgb(p.line);
        separator.a = 0.65;
        self.card()
            .p(px(16.))
            .pt(px(12.))
            .pb(px(8.))
            .gap(px(6.))
            .relative()
            .child(
                canvas(move |r, _, _| bounds.set(r), |_, _, _, _| {})
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
            )
            .child(
                row()
                    .justify_between()
                    .gap(px(8.))
                    .child(
                        row()
                            .gap(px(8.))
                            .child(
                                div()
                                    .size(px(28.))
                                    .flex_none()
                                    .rounded(px(8.))
                                    .bg(icon_tint)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_color(rgb(p.download_text))
                                    .child(
                                        Icon::default()
                                            .path("icons/history-trend.svg")
                                            .size(px(16.)),
                                    ),
                            )
                            .child(label("时段用量趋势", p.text).font_weight(FontWeight::MEDIUM)),
                    )
                    .child(
                        row()
                            .gap(px(4.))
                            .when(inline, |d| d.child(self.history_seconds_tools(cx)))
                            .child(
                                self.button(
                                    "history-open-comparison",
                                    "时段对比",
                                    false,
                                    Action::History(HistoryAction::Tab(HistoryTab::Analysis)),
                                    cx,
                                )
                                .h(px(32.))
                                .px(px(10.))
                                .gap(px(5.))
                                .rounded(px(8.))
                                .border_1()
                                .border_color(card_border(p))
                                .text_size(px(12.))
                                .child(Icon::default().path("icons/arrow-right.svg").size(px(13.))),
                            ),
                    ),
            )
            .child(
                row()
                    .gap(px(16.))
                    .py(px(
                        if self.history_state.view == history_state::CalendarView::Year {
                            0.
                        } else {
                            2.
                        },
                    ))
                    .child(
                        column()
                            .flex_1()
                            .min_w_0()
                            .gap(px(4.))
                            .child(label("时段合计", p.muted).text_size(px(12.)))
                            .child(
                                label(value(total.total()), p.text)
                                    .text_size(px(27.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .whitespace_nowrap(),
                            ),
                    )
                    .child(div().w(px(1.)).h(px(42.)).flex_none().bg(separator))
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .gap(px(if compact { 8. } else { 16. }))
                            .when(compact, |d| d.flex_col())
                            .child(direction(
                                "下载",
                                total.down,
                                p.download_text,
                                "icons/arrow-down.svg",
                            ))
                            .child(direction(
                                "上传",
                                total.up,
                                p.upload_text,
                                "icons/arrow-up.svg",
                            )),
                    ),
            )
            .child(div().h(px(1.)).w_full().bg(separator))
            .child(if self.history_state.seconds_level {
                column()
                    .gap(px(8.))
                    .child(self.history_chart_heading(compact, cx))
                    .child(self.history_seconds_content(window, cx))
            } else {
                self.history_histogram(
                    if loading {
                        None
                    } else {
                        self.report.history.clone()
                    },
                    compact,
                    cx,
                )
            })
            .when(!self.report.error.is_empty(), |d| {
                d.child(
                    label(self.report.error.clone(), p.amber)
                        .text_size(px(11.))
                        .whitespace_normal(),
                )
            })
    }
    fn history_calendar(&self, window: &Window, cx: &Context<Self>) -> Stateful<Div> {
        let p = self.p;
        let h = &self.history_state;
        let month = h.month;
        let today = chrono::Local::now().date_naive();
        let snapshot = h.visible_month().cloned();
        let first = month.weekday().num_days_from_monday() as usize;
        let next = month.checked_add_months(chrono::Months::new(1)).unwrap();
        let count = (next - month).num_days() as usize;
        let rows = (first + count).div_ceil(7);
        let (from, to) = h.selected_dates(&self.query);
        let mut week_surface = state_surface(p);
        week_surface.a *= 0.65;
        let measured = h.calendar_bounds.clone();
        let card = self
            .card()
            .p(px(16.))
            .pt(px(12.))
            .pb(px(8.))
            .gap(px(6.))
            .id("history-calendar")
            .relative()
            .w_full()
            .items_center()
            .child(self.history_calendar_heading(cx))
            .track_focus(&h.calendar_focus)
            .tab_stop(true)
            .child(
                canvas(move |r, _, _| measured.set(r), |_, _, _, _| {})
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
            )
            .on_mouse_down_out(cx.listener(|app, _, window, _| {
                if app.history_state.calendar_focus.is_focused(window) {
                    window.blur();
                }
            }));
        let mut content = column()
            .w(px(GRID_WIDTH))
            .flex_none()
            .flex_grow()
            .gap(px(4.))
            .child(self.history_month_navigation(cx));
        let mut grid = column().mt(px(4.)).gap(px(4.)).child(
            row()
                .gap(px(4.))
                .child(
                    div()
                        .id("history-week-heading")
                        .w(px(18.))
                        .flex_none()
                        .text_center()
                        .text_size(px(10.5))
                        .text_color(rgb(p.accent_text))
                        .font_weight(FontWeight::MEDIUM)
                        .child("周")
                        .tooltip(|window, cx| {
                            Tooltip::new("全年周次（周一开始）；点击下方周次选择整周")
                                .build(window, cx)
                        }),
                )
                .children(["一", "二", "三", "四", "五", "六", "日"].map(|name| {
                    label(name, p.muted)
                        .w(px(DAY_SIDE))
                        .flex_none()
                        .text_center()
                        .text_size(px(11.))
                })),
        );
        // Reserve six weeks so browsing shorter months never moves either card.
        let mut days = column().relative().gap(px(4.)).h(px(188.)).flex_none();
        let mut bands = Vec::new();
        for week in 0..rows {
            let week_from = month
                .checked_add_signed(chrono::Duration::days(week as i64 * 7 - first as i64))
                .unwrap();
            let mut line = row().relative().gap(px(4.)).child(
                self.button(
                    format!("history-week-{week}"),
                    week_from.iso_week().week().to_string(),
                    false,
                    cal_action(CalendarAction::Week(week_from)),
                    cx,
                )
                .w(px(18.))
                .flex_none()
                .px(px(0.))
                .h(px(20.))
                .rounded(px(4.))
                .bg(week_surface)
                .text_color(rgb(p.accent_text))
                .font_weight(FontWeight::MEDIUM)
                .text_size(px(10.5))
                .justify_center()
                .tooltip(move |window, cx| {
                    let to = week_from.checked_add_days(chrono::Days::new(6)).unwrap();
                    Tooltip::new(format!(
                        "{}年第{}周 · {} 至 {}，点击选择整周",
                        week_from.iso_week().year(),
                        week_from.iso_week().week(),
                        week_from.format("%Y-%m-%d"),
                        to.format("%Y-%m-%d")
                    ))
                    .build(window, cx)
                })
                .when(week_from > today, |d| d.opacity(0.3)),
            );
            for col in 0..7 {
                let index = week * 7 + col;
                if index < first || index >= first + count {
                    line = line.child(div().size(px(DAY_SIDE)).flex_none());
                    continue;
                }
                let day = month
                    .checked_add_days(chrono::Days::new((index - first) as u64))
                    .unwrap();
                line = line.child(self.history_day(
                    day,
                    day >= from && day <= to,
                    snapshot.as_deref(),
                    cx,
                ));
            }
            let visible_first = month.max(week_from);
            let visible_last = next
                .pred_opt()
                .unwrap()
                .min(week_from.checked_add_days(chrono::Days::new(6)).unwrap());
            let selected_first = from.max(visible_first);
            let selected_last = to.min(visible_last);
            let selected = (selected_first <= selected_last).then(|| {
                (
                    selected_first.weekday().num_days_from_monday(),
                    selected_last.weekday().num_days_from_monday(),
                )
            });
            if let Some((first, last)) = selected {
                bands.push((week, first as usize, last as usize));
            }
            let focused = (self.keyboard_navigation
                && h.calendar_focus.is_focused(window)
                && h.cursor >= visible_first
                && h.cursor <= visible_last)
                .then(|| h.cursor.weekday().num_days_from_monday());
            line = line.child(
                canvas(
                    |_, _, _| (),
                    move |rect, _, window, _| {
                        let cell = |col: u32| {
                            Bounds::new(
                                point(rect.left() + px(22. + col as f32 * 32.), rect.top()),
                                size(px(DAY_SIDE), px(DAY_SIDE)),
                            )
                        };
                        if let Some(col) = focused {
                            let cell = cell(col);
                            let inset = Bounds::new(
                                cell.origin + point(px(0.5), px(0.5)),
                                cell.size - size(px(1.), px(1.)),
                            );
                            if let Ok(path) =
                                tile_path(inset, px(3.5), PathBuilder::stroke(px(1.))).build()
                            {
                                window.paint_path(path, rgb(p.accent));
                            }
                        }
                    },
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            );
            days = days.child(line);
        }
        let contours = super::calendar_outline::selection_contours(&bands);
        grid = grid.child(
            days.child(
                canvas(
                    |_, _, _| (),
                    move |rect, _, window, _| {
                        paint_range_outline(
                            rect.origin + point(px(22.), px(0.)),
                            &contours,
                            p,
                            window,
                        );
                    },
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            ),
        );
        content = content
            .child(grid)
            .child(self.history_calendar_footer(cx))
            .when(!h.month_error.is_empty(), |d| {
                d.child(
                    label(h.month_error.clone(), p.amber)
                        .text_size(px(11.))
                        .whitespace_normal(),
                )
            });
        card.child(content)
    }
    fn history_day(
        &self,
        day: NaiveDate,
        active: bool,
        month: Option<&CalendarMonth>,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let p = self.p;
        let today = chrono::Local::now().date_naive();
        let future = day > today;
        let data = month.and_then(|m| {
            m.days
                .iter()
                .find(|b| crate::history::local_date(b.from) == day)
        });
        let status = if future {
            Coverage::Future
        } else {
            data.map_or(Coverage::Missing, |b| {
                b.coverage(month.unwrap().request.as_of)
            })
        };
        let color = if data.is_some_and(|b| b.observed) && !future {
            heat(p, month.unwrap().level(data.unwrap()))
        } else {
            rgba(0)
        };
        let tooltip = format!(
            "{}{}\n{}",
            day.format("%Y-%m-%d"),
            if day == today { " · 今天" } else { "" },
            data.filter(|b| b.observed).map_or_else(
                || if future {
                    "未来日期".into()
                } else if month.is_none() {
                    "正在读取月历".into()
                } else {
                    "无记录".into()
                },
                |b| format!(
                    "合计 {}\n上传 {}\n下载 {}",
                    human_bytes(b.bytes.total()),
                    human_bytes(b.bytes.up),
                    human_bytes(b.bytes.down)
                )
            ),
        );
        let compact_tooltip = future || (status == Coverage::Missing && month.is_some());
        let bounds = self.history_state.day_bounds.clone();
        let measured = bounds.clone();
        div()
            .id(SharedString::from(format!("history-day-{day}")))
            .relative()
            .size(px(DAY_SIDE))
            .flex_none()
            .rounded(px(4.))
            .bg(color)
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(11.))
            .text_color(rgb(p.text))
            .when(future, |d| d.opacity(0.25))
            .when(!future, |d| d.cursor_pointer())
            .child(
                canvas(
                    move |r, _, _| {
                        measured.borrow_mut().insert(day, r);
                    },
                    move |rect, _, window, _| {
                        if status == Coverage::Missing && !future {
                            paint_missing_outline(rect, p, if active { 3. } else { 1. }, window);
                        }
                    },
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .child(
                label(
                    day.day().to_string(),
                    if day == today { today_color(p) } else { p.text },
                )
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(if day == today { 14. } else { 13. }))
                .font_weight(FontWeight::NORMAL)
                .when(active && day != today, |d| {
                    d.font_weight(FontWeight::MEDIUM)
                }),
            )
            .tooltip(move |window, cx| {
                Tooltip::new(tooltip.clone())
                    .when(compact_tooltip, |t| t.text_size(px(10.)))
                    .build(window, cx)
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |app, event: &MouseDownEvent, window, cx| {
                    if !future {
                        window.prevent_default();
                        cx.stop_propagation();
                        app.history_state.month_open = false;
                        app.begin_history_drag(day, event.modifiers.shift, window, cx);
                    }
                }),
            )
    }
    pub(super) fn history_overlays(&self, window: &Window, cx: &Context<Self>) -> Vec<AnyElement> {
        if self.page != Page::History {
            return vec![];
        }
        let mut result = Vec::new();
        if let Some(hover) = self.history_chart_hover(window, cx) {
            result.push(hover);
        }
        if let Some(hover) = self.history_annual_hover(window) {
            result.push(hover);
        }
        if self.history_state.help_open
            && !self.history_state.time_expanded
            && self.history_state.tab == HistoryTab::Records
        {
            result.push(self.history_calendar_help(window, cx).into_any_element());
        }
        if self.history_state.month_open
            && !self.history_state.time_expanded
            && self.history_state.tab == HistoryTab::Records
        {
            result.push(self.history_month_popup(window, cx).into_any_element());
        }
        result
    }
    pub(super) fn history_comparison(&self, window: &Window, cx: &Context<Self>) -> Div {
        let available = window.viewport_size().width - self.sidebar_width() - px(48.);
        column()
            .gap(px(12.))
            .child(
                row()
                    .justify_between()
                    .flex_wrap()
                    .gap(px(8.))
                    .child(
                        row()
                            .gap(px(12.))
                            .child(self.button(
                                "history-comparison-back",
                                "← 返回",
                                false,
                                Action::History(HistoryAction::Tab(HistoryTab::Records)),
                                cx,
                            ))
                            .child(heading("时段对比", self.p)),
                    )
                    .child(
                        row()
                            .gap(px(8.))
                            .child(self.history_compare_tools(cx))
                            .child(self.analysis_refresh_button(cx)),
                    ),
            )
            .when(self.history_state.time_open, |d| {
                d.child(self.time_range_editor(window, cx))
            })
            .when(!self.history_state.error.is_empty(), |d| {
                d.child(label(self.history_state.error.clone(), self.p.amber).whitespace_normal())
            })
            .child(self.analysis_panel(available < px(850.), cx))
    }
    fn history_compare_tools(&self, cx: &Context<Self>) -> impl IntoElement {
        let period = self.history_state.analysis_period;
        let mut options: Vec<_> = TimePeriod::HISTORY
            .into_iter()
            .map(|p| {
                (
                    p.label().into(),
                    Action::History(HistoryAction::Period(p)),
                    period == Some(p),
                )
            })
            .collect();
        options.extend(crate::analysis::AnalysisRange::ALL.into_iter().map(|r| {
            (
                r.label().into(),
                Action::Analysis(AnalysisAction::Range(r)),
                period.is_none() && self.analysis.range == r,
            )
        }));
        self.taskbar_picker(
            "history-range",
            period
                .map_or(self.analysis.range.label(), TimePeriod::label)
                .to_string(),
            options,
            cx,
        )
    }
}
