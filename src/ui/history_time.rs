use super::*;
use chrono::{Datelike, NaiveDate};
use history_state::{CalendarAction, TimeQuickRange, TimeUnit, TimeWheelTarget};

const TIME_INPUT_HINT: &str = "点击数字输入／粘贴；悬浮滚轮微调，聚焦后可用↑↓。";

#[derive(Clone, Copy)]
struct TimeCalendarRender {
    today: NaiveDate,
    selected: Option<(NaiveDate, NaiveDate)>,
    focused: bool,
}

fn range_apply_colors(p: Palette) -> (Rgba, Rgba, Rgba) {
    let fill = rgb(p.accent_text);
    let foreground = rgb(p.panel);
    let mut overlay = rgb(p.text);
    overlay.a = 0.075;
    (fill, foreground, fill.blend(overlay))
}

impl TrafficApp {
    pub(super) fn time_range_editor(&self, window: &Window, cx: &Context<Self>) -> Div {
        let h = &self.history_state;
        let p = self.p;
        let available = window.viewport_size().width - self.sidebar_width() - px(48.);
        let available = available - px(22.);
        let stack = available < px(550.);
        let narrow = available < px(740.);
        let valid = self.time_range_result(cx).is_ok();
        let enabled = valid && self.time_range_dirty(cx);
        let endpoints = row()
            .gap(px(10.))
            .flex_1()
            .min_w_0()
            .w_full()
            .when(stack, |r| r.flex_col())
            .child(
                self.time_range_endpoint(0, window, cx)
                    .when(stack, |d| d.w_full()),
            )
            .child(
                svg()
                    .path(if stack {
                        "icons/arrow-down.svg"
                    } else {
                        "icons/arrow-right.svg"
                    })
                    .size(px(18.))
                    .flex_none()
                    .text_color(rgb(p.muted)),
            )
            .child(
                self.time_range_endpoint(1, window, cx)
                    .when(stack, |d| d.w_full()),
            );
        let toggle = row()
            .id("range-calendar-toggle")
            .justify_center()
            .text_size(px(13.))
            .cursor_pointer()
            .bg(if h.time_expanded {
                state_surface(p)
            } else {
                rgba(0)
            })
            .text_color(rgb(if h.time_expanded {
                p.accent_text
            } else {
                p.muted
            }))
            .hover(move |d| d.bg(state_surface(p)).text_color(rgb(p.accent_text)))
            .h(px(36.))
            .px(px(9.))
            .gap(px(4.))
            .flex_none()
            .rounded(px(7.))
            .border_1()
            .border_color(control_border(p))
            .font_weight(FontWeight::NORMAL)
            .track_focus(&h.time_buttons[0])
            .tab_stop(true)
            .child(
                svg()
                    .path("icons/calendar.svg")
                    .size(px(15.))
                    .text_color(rgb(p.muted)),
            )
            .child("日历")
            .child(
                svg()
                    .path(if h.time_expanded {
                        "icons/chevron-up.svg"
                    } else {
                        "icons/chevron-down.svg"
                    })
                    .size(px(11.))
                    .text_color(rgb(p.muted)),
            )
            .on_click(cx.listener(|app, _, window, cx| {
                app.history_calendar_action(CalendarAction::TimeCalendar, window, cx);
            }));
        let (fill, foreground, hover) = range_apply_colors(p);
        // This button owns its hover style. The generic secondary button would
        // replace the solid fill while leaving its foreground unchanged.
        let apply = div()
            .id("range-apply")
            .flex()
            .items_center()
            .justify_center()
            .h(px(36.))
            .px(px(13.))
            .flex_none()
            .rounded(px(7.))
            .track_focus(&h.time_buttons[2])
            .tab_stop(enabled)
            .text_size(px(13.))
            .font_weight(FontWeight::NORMAL)
            .bg(if enabled { fill } else { state_surface(p) })
            .text_color(if enabled { foreground } else { rgb(p.muted) })
            .child("应用")
            .when(enabled, |d| {
                d.cursor_pointer()
                    .hover(move |d| d.bg(hover).text_color(foreground))
                    .active(move |d| d.bg(fill).text_color(foreground))
                    .on_click(cx.listener(|app, _, window, cx| {
                        if app.time_range_dirty(cx) && app.time_range_result(cx).is_ok() {
                            app.act(
                                if app.page == Page::Apps {
                                    Action::ApplyRange
                                } else {
                                    Action::History(HistoryAction::ApplyRange)
                                },
                                window,
                                cx,
                            );
                        }
                    }))
            });
        let action_bounds = h.time_action_bounds.clone();
        let actions = row()
            .relative()
            .gap(px(8.))
            .flex_none()
            .child(toggle)
            .child(apply)
            .child(
                canvas(
                    move |bounds, _, _| action_bounds.set(bounds),
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            );
        let main = row()
            .gap(px(10.))
            .when(narrow, |r| r.flex_col())
            .child(endpoints)
            .child(actions.when(narrow, |r| r.w_full().justify_end()));
        let bounds = h.time_editor_bounds.clone();
        let mut editor = column()
            .relative()
            .w_full()
            .p(px(10.))
            .rounded(px(12.))
            .border_1()
            .border_color(control_border(p))
            .gap(px(8.))
            .bg(self.card_surface())
            .track_focus(&h.time_focus)
            .child(main)
            .child(
                canvas(
                    move |rect, _, _| bounds.set(rect.dilate(px(1.))),
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            );
        if let Some(error) = self.time_range_error(window, cx) {
            editor = editor.child(label(error, p.amber).text_size(px(12.)).whitespace_normal());
        }
        editor.when(h.time_expanded, |d| {
            let calendar = self.time_range_calendar(available, window, cx);
            if native::client_area_animations_enabled() {
                d.child(
                    div()
                        .w_full()
                        .min_h_0()
                        .overflow_hidden()
                        .child(calendar)
                        .with_animation(
                            "range-calendar-expand",
                            Animation::new(Duration::from_millis(150))
                                .with_easing(ease_out_quint()),
                            move |d, t| {
                                let ceiling = if available < px(472.) {
                                    1100.
                                } else if available < px(760.) {
                                    640.
                                } else {
                                    320.
                                };
                                d.max_h(px(ceiling * t))
                            },
                        ),
                )
            } else {
                d.child(calendar)
            }
        })
    }

    fn time_range_endpoint(
        &self,
        end: usize,
        window: &Window,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let p = self.p;
        let active = self.history_state.time_expanded
            && (self.history_state.time_pick_target == Some(end)
                || end == 1 && self.history_state.time_anchor.is_some());
        let date = row()
            .gap(px(0.))
            .flex_none()
            .font_family("Segoe UI")
            .child(self.time_number_field(end * 5, window, cx))
            .child(
                label("-", p.text)
                    .text_size(px(14.))
                    .w(px(6.))
                    .flex_none()
                    .whitespace_nowrap()
                    .text_center(),
            )
            .child(self.time_number_field(end * 5 + 1, window, cx))
            .child(
                label("-", p.text)
                    .text_size(px(14.))
                    .w(px(6.))
                    .flex_none()
                    .whitespace_nowrap()
                    .text_center(),
            )
            .child(self.time_number_field(end * 5 + 2, window, cx));
        let date_hint = div()
            .id(("range-endpoint-date-info", end))
            .size(px(24.))
            .flex_none()
            .rounded(px(4.))
            .flex()
            .items_center()
            .justify_center()
            .cursor_default()
            .child(
                svg()
                    .path("icons/date.svg")
                    .size(px(15.))
                    .text_color(rgb(p.muted)),
            )
            .tooltip(|window, cx| Tooltip::new(TIME_INPUT_HINT).build(window, cx));
        let date_area = row()
            .gap(px(6.))
            .flex_1()
            .min_w_0()
            .child(
                label(if end == 0 { "从" } else { "至" }, p.muted)
                    .text_size(px(12.))
                    .flex_none(),
            )
            .child(date)
            .child(div().flex_1())
            .child(date_hint);
        let clock = div()
            .id(("range-endpoint-time-info", end))
            .size(px(24.))
            .flex_none()
            .rounded(px(4.))
            .flex()
            .items_center()
            .justify_center()
            .cursor_default()
            .child(
                svg()
                    .path("icons/clock.svg")
                    .size(px(15.))
                    .text_color(rgb(p.muted)),
            )
            .tooltip(|window, cx| Tooltip::new(TIME_INPUT_HINT).build(window, cx));
        let time = row()
            .gap(px(0.))
            .w(px(80.))
            .flex_none()
            .font_family("Segoe UI")
            .child(self.time_number_field(end * 5 + 3, window, cx))
            .child(
                label(":", p.muted)
                    .text_size(px(14.))
                    .w(px(4.))
                    .flex_none()
                    .text_center(),
            )
            .child(self.time_number_field(end * 5 + 4, window, cx))
            .child(div().flex_1())
            .child(clock);
        row()
            .id(("range-endpoint", end))
            .gap(px(8.))
            .flex_1()
            .min_w_0()
            .min_h(px(46.))
            .px(px(9.))
            .py(px(7.))
            .rounded(px(10.))
            .border_1()
            .border_color(if active {
                calendar::selection_color(p)
            } else {
                control_border(p)
            })
            .bg(self.card_surface())
            .child(date_area)
            .child(div().w(px(1.)).h(px(26.)).flex_none().bg(control_border(p)))
            .child(time)
    }

    fn time_number_field(
        &self,
        index: usize,
        window: &Window,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let p = self.p;
        let field = self.history_state.time_parts[index].clone();
        let digits = if index.is_multiple_of(5) {
            "0000"
        } else {
            "00"
        };
        let mut style = window.text_style();
        style.font_family = "Segoe UI".into();
        style.font_weight = FontWeight::NORMAL;
        let text_width = window
            .text_system()
            .shape_line(digits.into(), px(14.), &[style.to_run(digits.len())], None)
            .width;
        let width = text_width + px(4.);
        let content = if self.history_state.time_drag.is_some()
            && self.history_state.time_dragged
            && index % 5 < 3
        {
            label(self.time_part_strings(cx)[index].clone(), p.text)
                .h(px(28.))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(14.))
                .into_any_element()
        } else {
            TextInput::new(&field)
                .appearance(false)
                .focus_bordered(false)
                .px(px(0.))
                .pl(px(2.))
                .py(px(0.))
                .h(px(28.))
                // InputState keeps 10 DIP after the caret. An inner overhang
                // provides that room while the clipped viewport stays compact;
                // the remaining 2 DIP keep the caret visible at the right edge.
                .w(width + px(8.))
                .absolute()
                .top_0()
                .left_0()
                .text_size(px(14.))
                .text_left()
                .text_color(rgb(p.text))
                .into_any_element()
        };
        let weak = cx.entity().downgrade();
        let part_bounds = self.history_state.time_part_bounds[index].clone();
        let hit = canvas(
            move |bounds, window, _| {
                part_bounds.set(bounds);
                window.insert_hitbox(bounds, HitboxBehavior::Normal)
            },
            move |_, hitbox, window, _| {
                let hitbox = hitbox.clone();
                let weak = weak.clone();
                window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
                    if phase != DispatchPhase::Capture || !hitbox.should_handle_scroll(window) {
                        return;
                    }
                    let delta = event.delta.pixel_delta(window.line_height());
                    if delta.y == px(0.) || delta.x.abs() > delta.y.abs() {
                        return;
                    }
                    let _ = weak.update(cx, |app, cx| {
                        let step = app.time_wheel_step(TimeWheelTarget::Part(index), event.delta);
                        if step != 0 {
                            app.step_time_part(index, step, window, cx);
                        }
                    });
                    cx.stop_propagation();
                });
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        div()
            .id(("range-part", index))
            .relative()
            .w(width)
            .h(px(28.))
            .overflow_hidden()
            .flex_none()
            .font_family("Segoe UI")
            .font_weight(FontWeight::NORMAL)
            .track_focus(&field.focus_handle(cx))
            .rounded(px(3.))
            .when(self.keyboard_navigation, |d| {
                d.focus(move |d| d.bg(state_surface(p)))
            })
            .child(content)
            .child(hit)
    }

    fn time_range_calendar(&self, available: Pixels, window: &Window, cx: &Context<Self>) -> Div {
        let h = &self.history_state;
        let p = self.p;
        let style = TimeCalendarRender {
            today: chrono::Local::now().date_naive(),
            selected: self.history_time_selection(cx),
            focused: false,
        };
        h.time_bounds
            .borrow_mut()
            .retain(|(side, date), _| match h.time_unit {
                TimeUnit::Day => date.with_day(1) == Some(h.time_months[*side]),
                TimeUnit::Month => *side == 0 && date.year() == h.time_year,
                TimeUnit::Year => {
                    *side == 0
                        && date.year() >= (h.time_year - 9).max(1)
                        && date.year() <= (h.time_year - 9).max(1) + 11
                }
            });
        let mut months = row()
            .w_full()
            .max_w(px(532.))
            .gap(px(12.))
            .items_start()
            .when(available < px(472.), |d| d.flex_col().items_start());
        if h.time_unit == TimeUnit::Day {
            for side in 0..2 {
                months = months.child(self.time_range_month(
                    h.time_months[side],
                    side,
                    style,
                    window,
                    cx,
                ));
            }
        } else {
            let legacy_style = TimeCalendarRender {
                focused: self.keyboard_navigation && h.time_grids[0].is_focused(window),
                ..style
            };
            let mut grid = column()
                .w(px(208.))
                .gap(px(6.))
                .child(self.time_month_heading(h.time_months[0], 0, cx));
            for line in 0..3 {
                grid = grid.child(row().gap(px(4.)).children((0..4).map(|col| {
                    let index = line * 4 + col;
                    let date = if h.time_unit == TimeUnit::Month {
                        NaiveDate::from_ymd_opt(h.time_year, index + 1, 1).unwrap()
                    } else {
                        NaiveDate::from_ymd_opt((h.time_year - 9).max(1) + index as i32, 1, 1)
                            .unwrap()
                    };
                    let text = if h.time_unit == TimeUnit::Month {
                        format!("{}月", date.month())
                    } else {
                        date.year().to_string()
                    };
                    self.time_range_tile(0, date, text, legacy_style, cx)
                        .flex_1()
                        .h(px(36.))
                })));
            }
            months = months.child(
                self.time_calendar_panel(0)
                    .child(grid)
                    .track_focus(&h.time_grids[0])
                    .tab_stop(true),
            );
        }
        let wide = available >= px(760.);
        let content = stretch()
            .w_full()
            .max_w(px(if wide { 760. } else { 532. }))
            .gap(px(12.))
            .when(!wide, |d| d.flex_col().items_start())
            .child(months.flex_1().min_w_0())
            .child(self.time_tools(style, wide, cx));
        let bounds = h.time_calendar_bounds.clone();
        column()
            .relative()
            .w_full()
            .gap(px(0.))
            .pt(px(8.))
            .border_t_1()
            .border_color(control_border(p))
            .child(content)
            .child(
                canvas(move |rect, _, _| bounds.set(rect), |_, _, _, _| {})
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
            )
    }

    fn time_calendar_panel(&self, side: usize) -> Div {
        let bounds = self.history_state.time_panel_bounds[side].clone();
        column()
            .relative()
            .w_full()
            .min_w(px(230.))
            .max_w(px(260.))
            .flex_1()
            .gap(px(0.))
            .items_center()
            .p(px(10.))
            .border_1()
            .border_color(control_border(self.p))
            .rounded(px(8.))
            .child(
                canvas(
                    move |rect, _, _| bounds.set(rect.dilate(px(1.))),
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
    }

    fn time_tool_button(
        &self,
        id: impl Into<SharedString>,
        text: &'static str,
        icon: Option<&'static str>,
        action: CalendarAction,
        enabled: bool,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let p = self.p;
        let mut accent_border = rgb(p.accent_text);
        accent_border.a = 0.5;
        div()
            .id(id.into())
            .flex()
            .items_center()
            .justify_center()
            .gap(px(6.))
            .h(px(30.))
            .px(px(6.))
            .rounded(px(6.))
            .border_1()
            .border_color(control_border(p))
            .bg(control_surface(p))
            .text_color(rgb(p.text))
            .text_size(px(12.))
            .font_weight(FontWeight::NORMAL)
            .whitespace_nowrap()
            .tab_stop(enabled)
            .when_some(icon, |d, path| {
                d.child(
                    svg()
                        .path(path)
                        .size(px(14.))
                        .flex_none()
                        .text_color(rgb(p.accent_text)),
                )
            })
            .child(text)
            .when(enabled, |d| {
                d.cursor_pointer()
                    .hover(move |d| {
                        d.bg(state_surface(p))
                            .border_color(accent_border)
                            .text_color(rgb(p.accent_text))
                    })
                    .active(move |d| d.bg(state_surface(p)))
                    .when(self.keyboard_navigation, |d| {
                        d.focus(move |d| d.border_color(rgb(p.accent_text)))
                    })
                    .on_click(cx.listener(move |app, _, window, cx| {
                        app.history_calendar_action(action.clone(), window, cx);
                    }))
            })
            .when(!enabled, |d| d.opacity(0.38).cursor_default())
    }

    fn time_tools(&self, style: TimeCalendarRender, wide: bool, cx: &Context<Self>) -> Div {
        let h = &self.history_state;
        let p = self.p;
        let mut accent_border = rgb(p.accent_text);
        accent_border.a = 0.5;
        let selected = self.active_time_quick(style.today, cx);
        let changed = self.time_range_dirty(cx);
        let all_day = self.time_all_day_enabled(cx);
        let count = style
            .selected
            .map(|(a, b)| (b - a).num_days() + 1)
            .filter(|n| *n > 0);
        let status = row()
            .gap(px(4.))
            .when_some(count, |d, n| {
                d.child(label("已选", p.muted).text_size(px(11.)))
                    .child(
                        label(n.to_string(), p.accent_text)
                            .text_size(px(18.))
                            .font_weight(FontWeight::MEDIUM),
                    )
                    .child(label("天", p.muted).text_size(px(11.)))
            })
            .when(count.is_none(), |d| {
                d.child(label("未选择", p.muted).text_size(px(12.)))
            });
        let mut presets = column()
            .w_full()
            .gap(px(6.))
            .child(label("常用范围", p.muted).text_size(px(11.)).h(px(14.)));
        for line in 0..3 {
            presets = presets.child(row().w_full().gap(px(4.)).children((0..2).map(|col| {
                let index = line * 2 + col;
                let quick = TimeQuickRange::ALL[index];
                let bounds = h.time_quick_bounds[index].clone();
                self.time_tool_button(
                    format!("range-quick-{index}"),
                    quick.label(),
                    None,
                    CalendarAction::TimeQuick(quick),
                    true,
                    cx,
                )
                .relative()
                .flex_1()
                .min_w_0()
                .track_focus(&h.time_quick_buttons[index])
                .when(selected == Some(quick), |d| {
                    d.bg(state_surface(p))
                        .border_color(accent_border)
                        .text_color(rgb(p.accent_text))
                        .font_weight(FontWeight::MEDIUM)
                })
                .child(
                    canvas(move |rect, _, _| bounds.set(rect), |_, _, _, _| {})
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full(),
                )
            })));
        }
        let operations = column()
            .w_full()
            .gap(px(4.))
            .pt(px(8.))
            .border_t_1()
            .border_color(control_border(p))
            .child(
                row()
                    .w_full()
                    .gap(px(4.))
                    .child(
                        self.time_tool_button(
                            "range-all-day",
                            "全天",
                            Some("icons/clock.svg"),
                            CalendarAction::TimeAllDay,
                            all_day,
                            cx,
                        )
                        .flex_1()
                        .h(px(28.))
                        .track_focus(&h.time_all_day),
                    )
                    .child(
                        self.time_tool_button(
                            "range-reset",
                            "还原修改",
                            Some("icons/undo.svg"),
                            CalendarAction::TimeReset,
                            changed,
                            cx,
                        )
                        .flex_1()
                        .h(px(28.))
                        .track_focus(&h.time_buttons[1]),
                    ),
            )
            .child(
                self.time_tool_button(
                    "range-locate",
                    "定位选区",
                    Some("icons/locate.svg"),
                    CalendarAction::TimeBrowse(4),
                    style.selected.is_some(),
                    cx,
                )
                .w_full()
                .h(px(28.))
                .when(style.selected.is_some(), |d| {
                    d.bg(state_surface(p)).text_color(rgb(p.accent_text))
                })
                .track_focus(&h.time_controls[6])
                .tooltip(|window, cx| Tooltip::new("让左右月历重新显示所选范围").build(window, cx)),
            );
        let bounds = h.time_panel_bounds[2].clone();
        column()
            .relative()
            .w(px(216.))
            .flex_none()
            .gap(px(8.))
            .p(px(10.))
            .when(wide, |d| d.border_l_1())
            .when(!wide, |d| d.border_t_1())
            .border_color(control_border(p))
            .child(
                row()
                    .h(px(24.))
                    .w_full()
                    .gap(px(4.))
                    .justify_between()
                    .child(status)
                    .when(
                        h.time_anchor.is_some() || h.time_pick_target == Some(1),
                        |d| d.child(label("选结束日", p.muted).text_size(px(10.))),
                    ),
            )
            .child(presets)
            .child(operations)
            .child(
                canvas(
                    move |rect, _, _| bounds.set(rect.dilate(px(1.))),
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
    }

    fn time_month_heading(
        &self,
        month: NaiveDate,
        side: usize,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let h = &self.history_state;
        let p = self.p;
        let weak = cx.entity().downgrade();
        let hit = canvas(
            |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
            move |_, hitbox, window, _| {
                let weak = weak.clone();
                let hitbox = hitbox.clone();
                window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
                    if phase != DispatchPhase::Capture || !hitbox.should_handle_scroll(window) {
                        return;
                    }
                    let delta = event.delta.pixel_delta(window.line_height());
                    if delta.y == px(0.) || delta.y.abs() < delta.x.abs() {
                        return;
                    }
                    let _ = weak.update(cx, |app, cx| {
                        app.scroll_time_heading(side, event.position, event.delta, window, cx);
                    });
                    cx.stop_propagation();
                });
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        let mut title = row()
            .gap(px(0.))
            .justify_center()
            .child(self.time_browse_field(side * 2, 44.))
            .child(label("年", p.muted).text_size(px(11.)));
        if h.time_unit == TimeUnit::Day {
            title = title
                .child(self.time_browse_field(side * 2 + 1, 26.))
                .child(label("月", p.muted).text_size(px(11.)));
        }
        row()
            .id(("range-month-heading", side))
            .relative()
            .h(px(28.))
            .flex_none()
            .gap(px(2.))
            .justify_between()
            .child(
                self.button(
                    format!("range-prev-{side}"),
                    "‹",
                    false,
                    Action::History(HistoryAction::Calendar(CalendarAction::TimeShift(side, -1))),
                    cx,
                )
                .size(px(24.))
                .px(px(0.))
                .track_focus(&h.time_nav[side][0])
                .when(!self.can_shift_time_calendar(side, -1), |d| {
                    d.opacity(0.35).cursor_default()
                }),
            )
            .child(title)
            .child(
                self.button(
                    format!("range-next-{side}"),
                    "›",
                    false,
                    Action::History(HistoryAction::Calendar(CalendarAction::TimeShift(side, 1))),
                    cx,
                )
                .size(px(24.))
                .px(px(0.))
                .track_focus(&h.time_nav[side][1])
                .when(!self.can_shift_time_calendar(side, 1), |d| {
                    d.opacity(0.35).cursor_default()
                }),
            )
            .tooltip(move |window, cx| {
                Tooltip::new(format!(
                    "{}年{}月 · 悬浮年月滚轮调整，点击可输入",
                    month.year(),
                    month.month()
                ))
                .build(window, cx)
            })
            .child(hit)
    }

    fn time_browse_field(&self, index: usize, width: f32) -> Div {
        let p = self.p;
        let field = if index == 0 {
            &self.history_state.year_input
        } else {
            &self.history_state.time_browse[index - 1]
        };
        let bounds = self.history_state.time_browse_bounds[index].clone();
        div()
            .relative()
            .w(px(width))
            .h(px(26.))
            .flex_none()
            .rounded(px(4.))
            .child(
                TextInput::new(field)
                    .appearance(false)
                    .focus_bordered(false)
                    .px(px(1.))
                    .py(px(0.))
                    .h(px(26.))
                    .text_size(px(12.))
                    .text_center()
                    .text_color(rgb(p.text)),
            )
            .child(
                canvas(move |rect, _, _| bounds.set(rect), |_, _, _, _| {})
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
            )
    }

    fn time_range_month(
        &self,
        month: NaiveDate,
        side: usize,
        style: TimeCalendarRender,
        window: &Window,
        cx: &Context<Self>,
    ) -> Div {
        let style = TimeCalendarRender {
            focused: self.keyboard_navigation
                && self.history_state.time_grids[side].is_focused(window),
            ..style
        };
        let first = month.weekday().num_days_from_monday();
        let count = month
            .checked_add_months(chrono::Months::new(1))
            .unwrap()
            .pred_opt()
            .unwrap()
            .day();
        let mut grid = column()
            .w(px(208.))
            .flex_none()
            .gap(px(2.))
            .child(self.time_month_heading(month, side, cx))
            .child(row().w_full().h(px(18.)).flex_none().gap(px(2.)).children(
                ["一", "二", "三", "四", "五", "六", "日"].map(|text| {
                    label(text, self.p.muted)
                        .w(px(28.))
                        .flex_none()
                        .text_center()
                        .text_size(px(11.))
                }),
            ));
        for week in 0..6 {
            let mut line = row().w_full().h(px(28.)).flex_none().gap(px(2.));
            for col in 0..7 {
                let slot = week * 7 + col;
                line = if slot < first || slot >= first + count {
                    line.child(div().size(px(28.)).flex_none())
                } else {
                    let date = month.with_day(slot - first + 1).unwrap();
                    line.child(
                        self.time_range_tile(side, date, date.day().to_string(), style, cx)
                            .size(px(28.))
                            .flex_none(),
                    )
                };
            }
            grid = grid.child(line);
        }
        self.time_calendar_panel(side)
            .track_focus(&self.history_state.time_grids[side])
            .tab_stop(true)
            .child(grid)
    }

    fn time_range_tile(
        &self,
        side: usize,
        date: NaiveDate,
        text: String,
        style: TimeCalendarRender,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let p = self.p;
        let h = &self.history_state;
        let span = h.time_unit.span(date, style.today);
        let active = span
            .zip(style.selected)
            .is_some_and(|((a, b), (from, to))| a <= to && b >= from);
        let edge = span
            .zip(style.selected)
            .is_some_and(|((a, b), (from, to))| a == from || b == to);
        let focused = style.focused && h.time_cursors[side] == date;
        let bounds = h.time_bounds.clone();
        div()
            .id(SharedString::from(format!("range-date-{side}-{date}")))
            .relative()
            .min_w_0()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(12.))
            .font_weight(FontWeight::NORMAL)
            .rounded(px(if edge { 5. } else { 0. }))
            .bg(if edge {
                rgb(p.accent_text)
            } else if active {
                rgb(p.tint)
            } else {
                rgba(0)
            })
            .text_color(rgb(if edge {
                p.panel
            } else if date == style.today {
                p.accent_text
            } else {
                p.text
            }))
            .border_1()
            .border_color(if focused {
                calendar::selection_color(p)
            } else {
                rgba(0)
            })
            .child(text)
            .when(span.is_none(), |d| d.opacity(0.25))
            .when(span.is_some(), |d| {
                d.cursor_pointer()
                    .hover(move |d| {
                        if edge || active {
                            d
                        } else {
                            d.bg(state_surface(p))
                        }
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |app, event: &MouseDownEvent, window, cx| {
                            app.begin_time_drag(side, date);
                            app.history_state.time_grids[side].focus(window);
                            if event.modifiers.shift {
                                app.history_state.time_anchor = None;
                                app.history_state.time_pick_target = Some(1);
                            }
                            cx.notify();
                        }),
                    )
                    .on_click(cx.listener(move |app, _, window, cx| {
                        app.history_state.time_side = side;
                        app.pick_history_time(date, window, cx)
                    }))
            })
            .child(
                canvas(
                    move |rect, _, _| {
                        bounds
                            .borrow_mut()
                            .insert((side, date), rect.dilate(px(1.)));
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn luminance(c: Rgba) -> f32 {
        let linear = |n: f32| {
            if n <= 0.04045 {
                n / 12.92
            } else {
                ((n + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(c.r) + 0.7152 * linear(c.g) + 0.0722 * linear(c.b)
    }
    #[::core::prelude::v1::test]
    fn primary_range_action_keeps_readable_foreground_during_hover() {
        for theme in ColorTheme::ALL {
            for dark in [false, true] {
                let p = Palette::new(theme, dark).main_window();
                let (fill, foreground, hover) = range_apply_colors(p);
                for background in [fill, hover] {
                    let a = luminance(background);
                    let b = luminance(foreground);
                    assert!(
                        (a.max(b) + 0.05) / (a.min(b) + 0.05) >= 4.5,
                        "unreadable range action: {theme:?} {dark}"
                    );
                }
            }
        }
    }
}
