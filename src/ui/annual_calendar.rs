use super::*;
use chrono::{Datelike, NaiveDate};
use history_state::{CalendarAction, CalendarView};

const ANNUAL_HOVER_WIDTH: f32 = 144.;

fn annual_hover_anchor(date: Bounds<Pixels>, viewport: Size<Pixels>) -> Point<Pixels> {
    let right = date.right() + px(10.);
    let left = if right + px(ANNUAL_HOVER_WIDTH) <= viewport.width - px(8.) {
        right
    } else {
        date.left() - px(ANNUAL_HOVER_WIDTH + 10.)
    };
    point(
        left.max(px(8.))
            .min((viewport.width - px(ANNUAL_HOVER_WIDTH + 8.)).max(px(8.))),
        date.top()
            .max(px(60.))
            .min((viewport.height - px(144.)).max(px(60.))),
    )
}

impl TrafficApp {
    pub(super) fn history_annual_hover(&self, window: &Window) -> Option<AnyElement> {
        let h = &self.history_state;
        if h.view != CalendarView::Year
            || h.month_open
            || h.time_open
            || h.help_open
            || h.drag.is_some()
        {
            return None;
        }
        let date = h.annual_hover?;
        let today = chrono::Local::now().date_naive();
        let (from, to) = h.annual_scope.bounds(today);
        if date < from || date > to {
            return None;
        }
        let bounds = *h.day_bounds.borrow().get(&date)?;
        let p = self.p;
        let anchor = annual_hover_anchor(bounds, window.viewport_size());
        let month = h.cached_month(date);
        let day = month.and_then(|m| {
            m.days
                .iter()
                .find(|d| crate::history::local_date(d.from) == date)
        });
        let mut panel = column()
            .absolute()
            .left(anchor.x)
            .top(anchor.y)
            .w(px(ANNUAL_HOVER_WIDTH))
            .p(px(8.))
            .gap(px(5.))
            .rounded(px(8.))
            .bg(popup_surface(p))
            .border_1()
            .border_color(control_border(p))
            .shadow_md()
            .child(
                label(
                    format!(
                        "{}{}",
                        date.format("%Y-%m-%d"),
                        if date == today { " · 今天" } else { "" }
                    ),
                    p.text,
                )
                .text_size(px(13.))
                .font_weight(FontWeight::MEDIUM),
            );
        if let Some(day) = day.filter(|d| d.observed) {
            for (name, bytes, color) in [
                ("下载", day.bytes.down, p.download_text),
                ("上传", day.bytes.up, p.upload_text),
                ("合计", day.bytes.total(), p.text),
            ] {
                panel = panel.child(
                    row()
                        .justify_between()
                        .child(label(name, p.muted).text_size(px(12.)))
                        .child(label(human_bytes(bytes), color).text_size(px(12.))),
                );
            }
        } else {
            panel = panel.child(
                label(
                    if date > today {
                        "未来日期"
                    } else if month.is_none() {
                        "正在读取日历…"
                    } else {
                        "此时段没有采集记录"
                    },
                    p.muted,
                )
                .text_size(px(12.))
                .whitespace_normal(),
            );
        }
        Some(panel.into_any_element())
    }
    pub(super) fn history_view_switch(&self, cx: &Context<Self>) -> Div {
        row()
            .gap(px(2.))
            .p(px(2.))
            .h(px(32.))
            .rounded(px(10.))
            .border_1()
            .border_color(surface_border(self.p))
            .children(
                [
                    (CalendarView::Month, "月历", "history-view-month"),
                    (CalendarView::Year, "年览", "history-view-year"),
                ]
                .map(|(view, title, id)| {
                    self.settings_segment(
                        id,
                        title,
                        self.history_state.view == view,
                        Action::History(HistoryAction::Calendar(CalendarAction::View(view))),
                        cx,
                    )
                    .min_w(px(0.))
                    .h(px(26.))
                    .px(px(10.))
                }),
            )
    }
    pub(super) fn history_annual_calendar(
        &self,
        window: &Window,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let p = self.p;
        let today = chrono::Local::now().date_naive();
        let h = &self.history_state;
        let (from, to) = h.annual_scope.bounds(today);
        let available = window.viewport_size().width - self.sidebar_width() - px(48. + 32. + 22.);
        let split = available < px(540.);
        let range = h.selected_dates(&self.query);
        let max = h
            .cache
            .iter()
            .flat_map(|m| m.days.iter())
            .filter(|b| {
                b.observed
                    && b.from >= crate::history::midnight(from).unwrap()
                    && b.from <= crate::history::midnight(to).unwrap()
            })
            .map(|b| b.bytes.total())
            .max()
            .unwrap_or(0);
        let measured = h.calendar_bounds.clone();
        let mut result = self
            .card()
            .p(px(16.))
            .gap(px(10.))
            .w_full()
            .id("history-annual-calendar")
            .relative()
            .track_focus(&h.calendar_focus)
            .tab_stop(true)
            .child(
                canvas(move |r, _, _| measured.set(r), |_, _, _, _| {})
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
            )
            .child(
                row()
                    .justify_between()
                    .gap(px(8.))
                    .child(self.history_calendar_heading(cx))
                    .when(range.1 < from || range.0 > to, |d| {
                        d.child(
                            self.button(
                                "history-annual-locate",
                                "回到所选日期",
                                false,
                                Action::History(HistoryAction::Calendar(CalendarAction::Locate)),
                                cx,
                            )
                            .text_size(px(11.)),
                        )
                    }),
            );
        let middle = from.checked_add_months(chrono::Months::new(6)).unwrap();
        if split {
            result = result
                .child(self.annual_block(
                    from,
                    middle.pred_opt().unwrap(),
                    available,
                    max,
                    window,
                    cx,
                ))
                .child(self.annual_block(middle, to, available, max, window, cx));
        } else {
            result = result.child(self.annual_block(from, to, available, max, window, cx));
        }
        result
            .child(self.history_calendar_legend(true, cx))
            .when(!h.month_error.is_empty(), |d| {
                d.child(
                    label(h.month_error.clone(), p.amber)
                        .text_size(px(11.))
                        .whitespace_normal(),
                )
            })
            .on_mouse_down_out(cx.listener(|app, _, window, _| {
                if app.history_state.calendar_focus.is_focused(window) {
                    window.blur();
                }
            }))
    }
    fn annual_block(
        &self,
        from: NaiveDate,
        to: NaiveDate,
        width: Pixels,
        maximum: u64,
        window: &Window,
        cx: &Context<Self>,
    ) -> Div {
        let p = self.p;
        let range = self.history_state.selected_dates(&self.query);
        let monday = from
            .checked_sub_days(chrono::Days::new(
                from.weekday().num_days_from_monday() as u64
            ))
            .unwrap();
        let weeks = ((to - monday).num_days() as usize + 1).div_ceil(7);
        let gap = 3.;
        let side = ((width - px(gap * (weeks - 1) as f32)) / weeks as f32)
            .max(px(5.))
            .min(px(14.));
        let grid_width = side * weeks as f32 + px(gap * (weeks - 1) as f32);
        let mut labels = div().relative().w(grid_width).h(px(22.));
        let mut month = from.with_day(1).unwrap();
        while month <= to {
            let next_month = month.checked_add_months(chrono::Months::new(1)).unwrap();
            let col = ((month - monday).num_days() / 7) as f32;
            let next_col = if next_month > to {
                weeks as f32
            } else {
                ((next_month - monday).num_days() / 7) as f32
            };
            let measured = self.history_state.month_bounds.clone();
            labels = labels.child(
                div()
                    .id(SharedString::from(format!("history-annual-month-{month}")))
                    .absolute()
                    .left((side + px(gap)) * col)
                    .w(((side + px(gap)) * (next_col - col))
                        .min(grid_width - (side + px(gap)) * col))
                    .h(px(22.))
                    .px(px(2.))
                    .rounded(px(4.))
                    .cursor_pointer()
                    .text_size(px(11.))
                    .text_color(rgb(p.muted))
                    .hover(move |s| s.bg(state_surface(p)).text_color(rgb(p.text)))
                    .map(|d| calendar_controls::month_selection_style(d, month, range, p))
                    .child(format!("{}月", month.month()))
                    .child(
                        canvas(
                            move |r, _, _| {
                                measured.borrow_mut().insert(month, r);
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full(),
                    )
                    .when(month > chrono::Local::now().date_naive(), |d| {
                        d.opacity(0.3)
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |app, event: &MouseDownEvent, window, cx| {
                            if month <= chrono::Local::now().date_naive() {
                                window.prevent_default();
                                cx.stop_propagation();
                                app.begin_history_month_drag(
                                    month,
                                    event.modifiers.shift,
                                    window,
                                    cx,
                                );
                            }
                        }),
                    ),
            );
            month = next_month;
        }
        let mut grid = column().relative().gap(px(gap)).w(grid_width);
        for day in 0..7 {
            let mut line = row().gap(px(gap)).h(side);
            for week in 0..weeks {
                let date = monday
                    .checked_add_days(chrono::Days::new((week * 7 + day) as u64))
                    .unwrap();
                if date < from || date > to {
                    line = line.child(div().size(side).flex_none());
                } else {
                    line = line.child(self.annual_day(date, side, maximum, window, cx));
                }
            }
            grid = grid.child(line);
        }
        let today = chrono::Local::now().date_naive();
        let today_index =
            (today >= from && today <= to).then(|| (today - monday).num_days() as usize);
        grid = grid.child(
            canvas(
                |_, _, _| (),
                move |rect, _, window, _| {
                    if let Some(index) = today_index {
                        let width = (side * 0.5).clamp(px(3.), px(6.));
                        let tick = Bounds::new(
                            rect.origin
                                + point(
                                    (side + px(gap)) * (index / 7) as f32 + (side - width) / 2.,
                                    (side + px(gap)) * (index % 7) as f32 + side + px(0.75),
                                ),
                            size(width, px(1.5)),
                        );
                        window.paint_quad(
                            fill(tick, rgb(calendar::today_color(p))).corner_radii(px(0.75)),
                        );
                    }
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        );
        let rail = column()
            .pt(px(22.))
            .gap(px(gap))
            .w(px(15.))
            .flex_none()
            .children((0..7).map(|i| {
                label(
                    match i {
                        0 => "一",
                        2 => "三",
                        4 => "五",
                        6 => "日",
                        _ => "",
                    },
                    p.muted,
                )
                .text_size(px(10.))
                .h(side)
                .flex()
                .items_center()
            }));
        row()
            .gap(px(7.))
            .items_start()
            .child(rail)
            .child(column().gap(px(0.)).child(labels).child(grid))
    }
    fn annual_day(
        &self,
        date: NaiveDate,
        side: Pixels,
        maximum: u64,
        window: &Window,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let h = &self.history_state;
        let p = self.p;
        let today = chrono::Local::now().date_naive();
        let future = date > today;
        let month = h.cached_month(date);
        let day = month.and_then(|m| {
            m.days
                .iter()
                .find(|b| crate::history::local_date(b.from) == date)
        });
        let observed = day.is_some_and(|b| b.observed);
        let bytes = day.map(|b| b.bytes).unwrap_or_default();
        let level = if maximum == 0 || bytes.total() == 0 {
            0
        } else {
            ((bytes.total() as u128 * 4).div_ceil(maximum as u128) as usize).clamp(1, 4)
        };
        let (from, to) = h.selected_dates(&self.query);
        let active = date >= from && date <= to;
        let focused =
            self.keyboard_navigation && h.calendar_focus.is_focused(window) && h.cursor == date;
        let measured = h.day_bounds.clone();
        div()
            .id(SharedString::from(format!("history-annual-day-{date}")))
            .size(side)
            .flex_none()
            .relative()
            .rounded((side * 0.22).min(px(3.)))
            .bg(if observed && !future {
                calendar::heat(p, level)
            } else {
                calendar::annual_missing(p)
            })
            .when(future, |d| d.opacity(0.3))
            .when(!future, |d| d.cursor_pointer())
            .child(
                canvas(
                    move |r, _, _| {
                        measured.borrow_mut().insert(date, r);
                    },
                    move |rect, _, window, _| {
                        if active || focused {
                            window.paint_quad(
                                outline(
                                    rect,
                                    if focused {
                                        rgb(p.accent)
                                    } else {
                                        calendar::selection_color(p)
                                    },
                                    BorderStyle::Solid,
                                )
                                .corner_radii((side * 0.22).min(px(3.)))
                                .border_widths(px(1.)),
                            );
                        }
                    },
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .on_hover(cx.listener(move |app, hovered: &bool, _, cx| {
                if *hovered && app.history_state.drag.is_none() {
                    app.history_state.annual_hover = Some(date);
                    cx.notify();
                } else if app.history_state.annual_hover == Some(date) {
                    app.history_state.annual_hover = None;
                    cx.notify();
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |app, event: &MouseDownEvent, window, cx| {
                    if !future {
                        window.prevent_default();
                        cx.stop_propagation();
                        app.history_state.month_open = false;
                        app.begin_history_drag(date, event.modifiers.shift, window, cx);
                    }
                }),
            )
    }
}

#[cfg(test)]
mod hover_position_tests {
    use super::{ANNUAL_HOVER_WIDTH, annual_hover_anchor};
    use gpui::{Bounds, point, px, size};
    #[test]
    fn hover_stays_beside_the_date_and_switches_at_the_right_edge() {
        let viewport = size(px(900.), px(700.));
        let left = Bounds::new(point(px(250.), px(150.)), size(px(14.), px(14.)));
        let right = Bounds::new(point(px(860.), px(150.)), size(px(14.), px(14.)));
        assert_eq!(
            annual_hover_anchor(left, viewport),
            point(px(274.), px(150.))
        );
        let anchor = annual_hover_anchor(right, viewport);
        assert!(anchor.x + px(ANNUAL_HOVER_WIDTH) < right.left());
        assert_eq!(anchor.y, right.top());
        let low = Bounds::new(point(px(250.), px(680.)), size(px(14.), px(14.)));
        assert!(annual_hover_anchor(low, viewport).y + px(136.) <= viewport.height - px(8.));
    }
}
