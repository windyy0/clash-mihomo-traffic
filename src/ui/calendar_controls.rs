use super::*;
use chrono::{Datelike, NaiveDate};
use history_state::CalendarAction;

pub(super) fn month_selection_style(
    element: Stateful<Div>,
    month: NaiveDate,
    range: (NaiveDate, NaiveDate),
    p: Palette,
) -> Stateful<Div> {
    let first = range.0.with_day(1).unwrap();
    let last = range.1.with_day(1).unwrap();
    let mut background = calendar::selection_color(p);
    background.a = if p.dark { 0.10 } else { 0.07 };
    let mut edge = calendar::selection_color(p);
    edge.a = 0.65;
    element.when(first < last && month >= first && month <= last, |d| {
        d.rounded(px(0.))
            .bg(background)
            .border_t_1()
            .border_b_1()
            .border_color(edge)
            .text_color(calendar::selection_color(p))
            .when(month == first, |d| d.border_l_1().rounded_l(px(5.)))
            .when(month == last, |d| d.border_r_1().rounded_r(px(5.)))
    })
}

impl TrafficApp {
    pub(super) fn history_month_navigation(&self, cx: &Context<Self>) -> Div {
        let p = self.p;
        let month = self.history_state.month;
        let range = self.history_state.selected_dates(&self.query);
        let today = chrono::Local::now().date_naive();
        row()
            .relative()
            .w_full()
            .gap(px(2.))
            .child(
                div()
                    .absolute()
                    .bottom_0()
                    .left_0()
                    .right_0()
                    .h(px(1.))
                    .bg(control_border(p)),
            )
            .child(
                self.button(
                    "history-previous-month",
                    "‹",
                    false,
                    Action::History(HistoryAction::Calendar(CalendarAction::Shift(-1))),
                    cx,
                )
                .w(px(16.))
                .h(px(32.))
                .px(px(0.)),
            )
            .child(
                row()
                    .gap(px(0.))
                    .flex_1()
                    .min_w_0()
                    .children((-3i32..=3).map(|delta| {
                        let date = if delta < 0 {
                            month.checked_sub_months(chrono::Months::new(delta.unsigned_abs()))
                        } else {
                            month.checked_add_months(chrono::Months::new(delta as u32))
                        };
                        self.button(
                            format!("history-strip-month-{delta}"),
                            date.map_or(String::new(), |d| format!("{:02}", d.month())),
                            false,
                            Action::History(HistoryAction::Calendar(if delta == 0 {
                                CalendarAction::ToggleMonth
                            } else {
                                CalendarAction::Shift(delta)
                            })),
                            cx,
                        )
                        .relative()
                        .when(delta == 0, |d| d.w(px(38.)).flex_none())
                        .when(delta != 0, |d| d.flex_1().min_w_0())
                        .h(px(28.))
                        .px(px(0.))
                        .gap(px(1.))
                        .text_size(px(if delta == 0 { 18. } else { 12. }))
                        .font_weight(if delta == 0 {
                            FontWeight::MEDIUM
                        } else {
                            FontWeight::NORMAL
                        })
                        .text_color(rgb(if delta == 0 { p.text } else { p.muted }))
                        .when_some(date, |d, date| month_selection_style(d, date, range, p))
                        .when(delta == 0, |d| {
                            d.child(label("月", p.muted).text_size(px(9.)).mt(px(4.)))
                                .child(
                                    div()
                                        .absolute()
                                        .bottom(px(-2.))
                                        .left(px(10.))
                                        .w(px(18.))
                                        .h(px(2.))
                                        .rounded_full()
                                        .bg(rgb(p.text)),
                                )
                        })
                        .when(date.is_none_or(|d| d > today), |d| d.opacity(0.25))
                    })),
            )
            .child(
                self.button(
                    "history-next-month",
                    "›",
                    false,
                    Action::History(HistoryAction::Calendar(CalendarAction::Shift(1))),
                    cx,
                )
                .w(px(16.))
                .h(px(32.))
                .px(px(0.))
                .when(
                    month
                        .checked_add_months(chrono::Months::new(1))
                        .is_none_or(|d| d > today),
                    |d| d.opacity(0.25),
                ),
            )
            .on_scroll_wheel(cx.listener(|app, event: &ScrollWheelEvent, window, cx| {
                let now = Instant::now();
                let h = &mut app.history_state;
                if h.month_wheel_at
                    .is_none_or(|at| now.duration_since(at) > Duration::from_millis(220))
                {
                    h.month_wheel = 0.;
                }
                h.month_wheel_at = Some(now);
                h.month_wheel += f32::from(event.delta.pixel_delta(px(20.)).y);
                let step = if h.month_wheel.abs() >= 40. {
                    let step = if h.month_wheel > 0. { -1 } else { 1 };
                    h.month_wheel = 0.;
                    step
                } else {
                    0
                };
                cx.stop_propagation();
                if step != 0 {
                    app.history_calendar_action(CalendarAction::Shift(step), window, cx);
                }
            }))
    }
    pub(super) fn history_calendar_help_button(&self, cx: &Context<Self>) -> Stateful<Div> {
        let p = self.p;
        let measured = self.history_state.help_trigger.clone();
        div()
            .id("history-calendar-help")
            .relative()
            .size(px(28.))
            .rounded(px(7.))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .text_color(rgb(p.muted))
            .hover(move |s| s.bg(state_surface(p)).text_color(rgb(p.text)))
            .child(
                Icon::default()
                    .path("icons/position-info.svg")
                    .size(px(14.)),
            )
            .child(
                canvas(move |r, _, _| measured.set(r), |_, _, _, _| {})
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
            )
            .tooltip(|window, cx| Tooltip::new("说明").build(window, cx))
            .on_click(cx.listener(|app, _, window, cx| {
                app.history_calendar_action(
                    CalendarAction::Help(!app.history_state.help_open),
                    window,
                    cx,
                )
            }))
    }

    pub(super) fn history_calendar_help(&self, window: &Window, cx: &Context<Self>) -> Div {
        let p = self.p;
        let h = &self.history_state;
        let mut surface = popup_surface(p);
        surface.a = 1.;
        let hint = |title: &str, action: &str| {
            column()
                .gap(px(3.))
                .child(
                    label(title.to_owned(), p.text)
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM),
                )
                .child(
                    label(action.to_owned(), p.muted)
                        .text_size(px(11.))
                        .whitespace_normal(),
                )
        };
        column()
            .absolute()
            .top(
                (h.help_trigger.get().bottom() + px(6.))
                    .min((window.viewport_size().height - px(360.)).max(px(60.))),
            )
            .left(
                (h.help_trigger.get().right() - px(252.))
                    .max(h.calendar_bounds.get().left() + px(12.))
                    .min((window.viewport_size().width - px(260.)).max(px(8.))),
            )
            .w(px(252.))
            .p(px(12.))
            .gap(px(10.))
            .bg(surface)
            .border_1()
            .border_color(control_border(p))
            .rounded(px(10.))
            .shadow_md()
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down_out(cx.listener(|app, event: &MouseDownEvent, _, cx| {
                if !app
                    .history_state
                    .help_trigger
                    .get()
                    .contains(&event.position)
                {
                    app.history_state.help_open = false;
                    cx.notify();
                }
            }))
            .child(
                self.button(
                    "history-help-close",
                    "×",
                    false,
                    Action::History(HistoryAction::Calendar(CalendarAction::Help(false))),
                    cx,
                )
                .absolute()
                .top(px(6.))
                .right(px(6.))
                .size(px(24.))
                .px(px(0.)),
            )
            .child(
                hint(
                    "选连续日期",
                    "拖动选择日期范围；或按住 Shift 点击结束日期。",
                )
                .pr(px(18.)),
            )
            .child(hint(
                "翻月与跳年",
                "月份选择器支持滚轮；单击年份选择，再次单击可输入年份。",
            ))
            .child(div().h(px(1.)).bg(control_border(p)))
            .child(
                label("翻月只浏览，不改变统计范围。", p.accent_text)
                    .text_size(px(11.))
                    .font_weight(FontWeight::MEDIUM),
            )
    }

    pub(super) fn history_calendar_legend(&self, annual: bool, cx: &Context<Self>) -> Div {
        let p = self.p;
        let scale = row()
            .gap(px(4.))
            .child(label("少", p.muted).text_size(px(11.)))
            .children((0..5).map(|level| {
                div()
                    .size(px(10.))
                    .rounded(px(2.))
                    .bg(calendar::heat(p, level))
            }))
            .child(label("多", p.muted).text_size(px(11.)));
        let tag = |marker: Div, text: &str| {
            row()
                .gap(px(4.))
                .child(marker)
                .child(label(text.to_owned(), p.muted).text_size(px(10.5)))
        };
        let missing = if annual {
            div()
                .size(px(12.))
                .rounded(px(3.))
                .bg(calendar::annual_missing(p))
        } else {
            calendar::missing_marker(p)
        };
        let today = if annual {
            div().size(px(12.)).relative().child(
                div()
                    .absolute()
                    .left(px(3.))
                    .bottom(px(1.))
                    .w(px(6.))
                    .h(px(1.5))
                    .rounded_full()
                    .bg(rgb(calendar::today_color(p))),
            )
        } else {
            calendar::today_marker(p)
        };
        let tags = row()
            .gap(px(8.))
            .justify_between()
            .child(tag(missing, "无记录"))
            .child(tag(
                div()
                    .size(px(12.))
                    .rounded(px(3.))
                    .border(px(1.))
                    .border_color(calendar::selection_color(p)),
                "已选",
            ))
            .child(tag(today, "今天"));
        let body = if annual {
            row().gap(px(18.)).flex_wrap().child(scale).child(tags)
        } else {
            let range = self.history_state.selected_dates(&self.query);
            let outside = self.history_state.month < range.0.with_day(1).unwrap()
                || self.history_state.month > range.1.with_day(1).unwrap();
            column()
                .gap(px(4.))
                .child(
                    row()
                        .h(px(16.))
                        .justify_between()
                        .child(scale)
                        .when(outside, |d| {
                            d.child(
                                self.button(
                                    "history-locate-range",
                                    "回到所选日期",
                                    false,
                                    Action::History(HistoryAction::Calendar(
                                        CalendarAction::Locate,
                                    )),
                                    cx,
                                )
                                .w(px(84.))
                                .h(px(16.))
                                .text_size(px(11.))
                                .px(px(4.)),
                            )
                        }),
                )
                .child(tags)
        };
        body.w_full()
            .border_t_1()
            .border_color(card_border(p))
            .pt(px(if annual { 8. } else { 6. }))
    }

    pub(super) fn history_calendar_footer(&self, cx: &Context<Self>) -> Div {
        self.history_calendar_legend(false, cx).mt_auto()
    }
}
