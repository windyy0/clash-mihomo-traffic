use super::*;
use chrono::{Datelike, NaiveDate};
use history_state::{AnnualScope, CalendarAction, CalendarView};

impl TrafficApp {
    pub(super) fn history_calendar_heading(&self, cx: &Context<Self>) -> Div {
        row()
            .w(px(242.))
            .flex_none()
            .justify_between()
            .child(self.history_view_switch(cx))
            .child(
                row()
                    .gap(px(4.))
                    .child(self.history_year_button(cx))
                    .child(self.history_calendar_help_button(cx)),
            )
    }
    pub(super) fn history_year_button(&self, cx: &Context<Self>) -> Stateful<Div> {
        let h = &self.history_state;
        let p = self.p;
        let year = match (h.view, h.annual_scope) {
            (CalendarView::Year, AnnualScope::Year(year)) => year,
            _ => h.month.year(),
        };
        let measured = h.year_trigger.clone();
        row()
            .id("history-month-title")
            .relative()
            .w(px(72.))
            .flex_none()
            .justify_center()
            .h(px(28.))
            .px(px(7.))
            .gap(px(3.))
            .rounded(px(7.))
            .cursor_pointer()
            .text_color(rgb(p.muted))
            .hover(move |d| d.bg(state_surface(p)).text_color(rgb(p.text)))
            .child(label(format!("{year}年"), p.muted).text_size(px(12.)))
            .child(Icon::default().path("icons/chevron-down.svg").size(px(10.)))
            .child(
                canvas(move |r, _, _| measured.set(r), |_, _, _, _| {})
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
            )
            .tooltip(|window, cx| {
                Tooltip::new("选择年月 · 再次单击编辑年份 · 滚轮换年").build(window, cx)
            })
            .on_click(cx.listener(|app, _, window, cx| {
                cx.stop_propagation();
                app.history_year_title_clicked(false, window, cx);
            }))
            .on_scroll_wheel(cx.listener(|app, event: &ScrollWheelEvent, window, cx| {
                app.history_year_wheel(event, window, cx)
            }))
    }
    pub(super) fn history_year_title_clicked(
        &mut self,
        in_popup: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let h = &self.history_state;
        let action = if !h.month_open {
            CalendarAction::ToggleMonth
        } else if !in_popup || h.year_grid {
            CalendarAction::EditYear
        } else {
            CalendarAction::YearGrid
        };
        self.history_calendar_action(action, window, cx);
    }
    pub(super) fn history_year_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        if self.history_state.year_edit {
            return;
        }
        let now = Instant::now();
        let h = &mut self.history_state;
        if h.year_wheel_at
            .is_none_or(|at| now.duration_since(at) > Duration::from_millis(220))
        {
            h.year_wheel = 0.;
        }
        h.year_wheel_at = Some(now);
        h.year_wheel += f32::from(event.delta.pixel_delta(px(20.)).y);
        if h.year_wheel.abs() >= 40. {
            let delta = if h.year_wheel > 0. { -1 } else { 1 };
            h.year_wheel = 0.;
            self.history_calendar_action(CalendarAction::YearStep(delta), window, cx);
        }
    }
    pub(super) fn history_month_popup(&self, window: &Window, cx: &Context<Self>) -> Div {
        let h = &self.history_state;
        let p = self.p;
        let today = chrono::Local::now().date_naive();
        let year = h.picker_year;
        let caption = div()
            .id("history-picker-year-title")
            .flex_1()
            .h(px(32.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(7.))
            .cursor_pointer()
            .hover(move |d| d.bg(state_surface(p)))
            .child(
                label(format!("{year}年"), p.text)
                    .text_size(px(15.))
                    .font_weight(FontWeight::MEDIUM),
            )
            .on_click(cx.listener(|app, _, window, cx| {
                cx.stop_propagation();
                app.history_year_title_clicked(true, window, cx);
            }));
        let header = if h.year_edit {
            div().flex_1().child(self.text_input(&h.year_input))
        } else {
            div().flex_1().child(caption)
        };
        let mut panel = column()
            .absolute()
            .top(
                (h.year_trigger.get().bottom() + px(6.))
                    .min((window.viewport_size().height - px(328.)).max(px(60.))),
            )
            .left(
                (h.year_trigger.get().right() - px(248.))
                    .max(h.calendar_bounds.get().left() + px(12.))
                    .min((window.viewport_size().width - px(256.)).max(px(8.))),
            )
            .w(px(248.))
            .p(px(12.))
            .gap(px(8.))
            .rounded(px(10.))
            .bg(popup_surface(p))
            .border_1()
            .border_color(control_border(p))
            .shadow_md()
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down_out(cx.listener(|app, event: &MouseDownEvent, _, cx| {
                if !app
                    .history_state
                    .year_trigger
                    .get()
                    .contains(&event.position)
                {
                    app.history_state.month_open = false;
                    app.history_state.year_edit = false;
                    app.history_state.error.clear();
                    cx.notify();
                }
            }))
            .on_scroll_wheel(cx.listener(|app, event: &ScrollWheelEvent, window, cx| {
                app.history_year_wheel(event, window, cx)
            }))
            .child(
                row()
                    .gap(px(4.))
                    .child(
                        self.button(
                            "history-picker-previous-year",
                            "‹",
                            false,
                            Action::History(HistoryAction::Calendar(CalendarAction::YearStep(-1))),
                            cx,
                        )
                        .w(px(24.))
                        .px(px(0.)),
                    )
                    .child(header)
                    .child(
                        self.button(
                            "history-picker-next-year",
                            "›",
                            false,
                            Action::History(HistoryAction::Calendar(CalendarAction::YearStep(1))),
                            cx,
                        )
                        .w(px(24.))
                        .px(px(0.))
                        .when(year >= today.year(), |d| d.opacity(0.3)),
                    ),
            );
        if h.year_edit {
            panel =
                panel.child(label("输入年份，Enter 确认 · Esc 取消", p.muted).text_size(px(11.)));
        } else {
            let first_year = (year - 1) / 12 * 12 + 1;
            for row_index in 0..3 {
                panel = panel.child(row().gap(px(4.)).children((0..4).map(|col| {
                    let index = row_index * 4 + col;
                    let (title, selected, future, action) = if h.year_grid {
                        let y = first_year + index;
                        (
                            y.to_string(),
                            y == year,
                            y > today.year(),
                            if y == year {
                                CalendarAction::EditYear
                            } else {
                                CalendarAction::Year(y)
                            },
                        )
                    } else {
                        let month = index as u32 + 1;
                        (
                            format!("{month}月"),
                            year == h.month.year() && month == h.month.month(),
                            NaiveDate::from_ymd_opt(year, month, 1).is_none_or(|d| d > today),
                            CalendarAction::Month(month),
                        )
                    };
                    self.button(
                        format!("history-picker-cell-{index}"),
                        title,
                        selected,
                        Action::History(HistoryAction::Calendar(action)),
                        cx,
                    )
                    .flex_1()
                    .h(px(32.))
                    .px(px(0.))
                    .text_size(px(12.))
                    .when(future, |d| d.opacity(0.3))
                })));
            }
            if !h.year_grid {
                panel =
                    panel.child(label("点击年份选年份 · 滚轮换年", p.muted).text_size(px(10.5)));
            }
        }
        panel.when(!h.error.is_empty(), |d| {
            d.child(
                label(h.error.clone(), p.amber)
                    .text_size(px(11.))
                    .whitespace_normal(),
            )
        })
    }
}
