use super::*;
use crate::ui::history_state::{CalendarAction, TimeQuickRange, TimeWheelTarget};
use chrono::{Datelike, NaiveDate};

async fn hover_wheel(
    this: &WeakEntity<TrafficApp>,
    cx: &mut AsyncWindowContext,
    target: TimeWheelTarget,
    amount: f32,
) -> anyhow::Result<()> {
    // Target only the isolated test window; leave the system cursor untouched.
    this.update_in(cx, |app, window, _| {
        use windows_sys::Win32::{
            Foundation::POINT,
            Graphics::Gdi::ClientToScreen,
            UI::WindowsAndMessaging::{PostMessageW, WM_MOUSEMOVE, WM_MOUSEWHEEL},
        };
        let center = match target {
            TimeWheelTarget::Part(i) => app.history_state.time_part_bounds[i].get().center(),
            TimeWheelTarget::Browse(i) => app.history_state.time_browse_bounds[i].get().center(),
            TimeWheelTarget::Calendar(_) | TimeWheelTarget::DrillPart(_) => unreachable!(),
        };
        let mut position = POINT {
            x: (f32::from(center.x) * window.scale_factor()) as i32,
            y: (f32::from(center.y) * window.scale_factor()) as i32,
        };
        let coordinates = |p: &POINT| (p.x as u16 as u32 | ((p.y as u16 as u32) << 16)) as isize;
        unsafe {
            let hwnd = app.main_hwnd as _;
            anyhow::ensure!(
                PostMessageW(hwnd, WM_MOUSEMOVE, 0, coordinates(&position)) != 0,
                "test hover message failed"
            );
            anyhow::ensure!(
                ClientToScreen(hwnd, &mut position) != 0,
                "test wheel coordinates unavailable"
            );
            let delta: i16 = if amount > 0. { 120 } else { -120 };
            anyhow::ensure!(
                PostMessageW(
                    hwnd,
                    WM_MOUSEWHEEL,
                    ((delta as u16 as u32) << 16) as usize,
                    coordinates(&position)
                ) != 0,
                "test wheel message failed"
            );
        }
        Ok(())
    })??;
    Timer::after(Duration::from_millis(65)).await;
    settle_layout(this, cx).await
}

async fn click_calendar(
    this: &WeakEntity<TrafficApp>,
    cx: &mut AsyncWindowContext,
    side: usize,
    date: NaiveDate,
) -> anyhow::Result<()> {
    this.update_in(cx, |app, window, _| {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            PostMessageW, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
        };
        let center = app.history_state.time_bounds.borrow()[&(side, date)].center();
        let x = (f32::from(center.x) * window.scale_factor()) as i32;
        let y = (f32::from(center.y) * window.scale_factor()) as i32;
        let position = (x as u16 as u32 | ((y as u16 as u32) << 16)) as isize;
        unsafe {
            let hwnd = app.main_hwnd as _;
            for (event, buttons) in [(WM_MOUSEMOVE, 0), (WM_LBUTTONDOWN, 1), (WM_LBUTTONUP, 0)] {
                anyhow::ensure!(
                    PostMessageW(hwnd, event, buttons, position) != 0,
                    "isolated calendar click failed"
                );
            }
        }
        Ok(())
    })??;
    Timer::after(Duration::from_millis(65)).await;
    settle_layout(this, cx).await
}

async fn exercise_compact_field_hits(
    this: &WeakEntity<TrafficApp>,
    cx: &mut AsyncWindowContext,
) -> anyhow::Result<()> {
    let original = this.update_in(cx, |app, _, _| (app.query_id, app.query.clone()))?;
    for index in 0..10 {
        for end in [false, true] {
            this.update_in(cx, |app, window, _| {
                use windows_sys::Win32::UI::WindowsAndMessaging::{
                    PostMessageW, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
                };
                let bounds = app.history_state.time_part_bounds[index].get();
                let point = point(
                    if end {
                        bounds.right() - px(2.)
                    } else {
                        bounds.left() + px(1.)
                    },
                    bounds.center().y,
                );
                let x = (f32::from(point.x) * window.scale_factor()) as i32;
                let y = (f32::from(point.y) * window.scale_factor()) as i32;
                let position = (x as u16 as u32 | ((y as u16 as u32) << 16)) as isize;
                unsafe {
                    for (event, buttons) in
                        [(WM_MOUSEMOVE, 0), (WM_LBUTTONDOWN, 1), (WM_LBUTTONUP, 0)]
                    {
                        anyhow::ensure!(
                            PostMessageW(app.main_hwnd as _, event, buttons, position) != 0,
                            "isolated field click failed"
                        );
                    }
                }
                Ok(())
            })??;
            Timer::after(Duration::from_millis(65)).await;
            settle_layout(this, cx).await?;
            this.update_in(cx, |app, window, cx| {
                let field = &app.history_state.time_parts[index];
                anyhow::ensure!(
                    field.focus_handle(cx).is_focused(window)
                        && (!end || field.read(cx).cursor() == field.read(cx).value().len())
                        && app.query_id == original.0
                        && app.query == original.1,
                    "compact field {index} leaked a hit target or lost its last digit"
                );
                Ok(())
            })??;
        }
    }
    this.update_in(cx, |app, window, _| {
        app.history_state.time_focus.focus(window)
    })?;
    settle_layout(this, cx).await
}

async fn exercise_partial_input(
    this: &WeakEntity<TrafficApp>,
    cx: &mut AsyncWindowContext,
) -> anyhow::Result<()> {
    let original = this.update_in(cx, |app, _, _| (app.query_id, app.query.clone()))?;
    for (index, message) in [(0, "请填写开始年份"), (3, "请填写开始小时")] {
        this.update_in(cx, |app, window, cx| {
            app.history_state.time_parts[index]
                .focus_handle(cx)
                .focus(window);
        })?;
        settle_layout(this, cx).await?;
        cx.update(|window, cx| {
            window.dispatch_action(Box::new(gpui_component::input::SelectAll), cx);
            window.dispatch_keystroke(Keystroke::parse("backspace").unwrap(), cx);
        })?;
        settle_layout(this, cx).await?;
        this.update_in(cx, |app, window, cx| {
            anyhow::ensure!(
                app.history_state.time_parts[index]
                    .read(cx)
                    .value()
                    .is_empty()
                    && app.time_range_result(cx).is_err()
                    && app.time_range_error(window, cx).is_none(),
                "deleting a focused part must keep an incomplete draft without a message"
            );
            app.history_state.time_focus.focus(window);
            Ok(())
        })??;
        settle_layout(this, cx).await?;
        this.update_in(cx, |app, window, cx| {
            anyhow::ensure!(
                app.time_range_error(window, cx).as_deref() == Some(message),
                "blur must explain which part is missing"
            );
            app.history_state.time_parts[index]
                .focus_handle(cx)
                .focus(window);
            Ok(())
        })??;
        settle_layout(this, cx).await?;
        cx.update(|window, cx| {
            window.dispatch_keystroke(Keystroke::parse("enter").unwrap(), cx);
        })?;
        settle_layout(this, cx).await?;
        this.update_in(cx, |app, window, cx| {
            anyhow::ensure!(
                app.time_range_error(window, cx).as_deref() == Some(message)
                    && app.query_id == original.0
                    && app.query == original.1,
                "incomplete Enter must explain the field without querying"
            );
            app.set_time_values(["2024-02-12", "00:00", "2024-02-20", "23:59"], window, cx);
            app.history_state.error.clear();
            app.history_state.time_focus.focus(window);
            Ok(())
        })??;
        settle_layout(this, cx).await?;
    }
    Ok(())
}

async fn exercise_independent_calendars(
    this: &WeakEntity<TrafficApp>,
    cx: &mut AsyncWindowContext,
) -> anyhow::Result<()> {
    let date = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap();
    let original = this.update_in(cx, |app, window, cx| {
        let original = (app.query_id, app.query.clone());
        app.history_state.time_months = [date(2023, 12, 1), date(2024, 1, 1)];
        app.shift_time_calendar(0, 1, window, cx);
        anyhow::ensure!(
            app.history_state.time_months == [date(2024, 1, 1); 2],
            "left navigation moved both calendars"
        );
        app.shift_time_calendar(1, -1, window, cx);
        app.shift_time_calendar(0, 1, window, cx);
        anyhow::ensure!(
            app.history_state.time_months == [date(2024, 1, 1); 2]
                && !app.can_shift_time_calendar(0, 1)
                && !app.can_shift_time_calendar(1, -1),
            "navigation crossed the other calendar"
        );
        app.history_state.time_browse[1].update(cx, |s, cx| s.set_value("2023", window, cx));
        app.edit_time_browse(2, window, cx);
        anyhow::ensure!(
            app.history_state.time_months == [date(2024, 1, 1); 2]
                && !app.history_state.error.is_empty(),
            "year entry bypassed month order"
        );
        app.history_state.time_browse[2].update(cx, |s, cx| s.set_value("03", window, cx));
        app.edit_time_browse(3, window, cx);
        anyhow::ensure!(
            app.history_state.time_months == [date(2024, 1, 1), date(2024, 3, 1)],
            "right month entry moved left calendar"
        );
        app.history_state.time_months = [date(2024, 2, 1); 2];
        app.shift_time_calendar(0, 0, window, cx);
        app.history_state.time_cursors = [date(2024, 2, 20); 2];
        app.set_time_values(["2024-02-10", "09:00", "2024-02-20", "18:00"], window, cx);
        app.history_state.time_anchor = None;
        app.history_state.time_pick_target = None;
        cx.notify();
        Ok(original)
    })??;
    settle_layout(this, cx).await?;
    this.update_in(cx, |app, _, _| {
        let bounds = app.history_state.time_bounds.borrow();
        let left = bounds[&(0, date(2024, 2, 19))];
        let right = bounds[&(1, date(2024, 2, 19))];
        anyhow::ensure!(
            left.right() < right.left()
                && bounds.keys().filter(|(side, _)| *side == 0).count() == 29
                && bounds.keys().filter(|(side, _)| *side == 1).count() == 29,
            "duplicate months lost a hit region"
        );
        Ok(())
    })??;
    click_calendar(this, cx, 0, date(2024, 2, 19)).await?;
    click_calendar(this, cx, 1, date(2024, 2, 22)).await?;
    this.update_in(cx, |app, window, cx| {
        anyhow::ensure!(
            app.history_time_selection(cx) == Some((date(2024, 2, 19), date(2024, 2, 22)))
                && app.history_state.time_grids[1].is_focused(window),
            "clicking both identical months did not select a shared range"
        );
        let target = app.history_state.time_bounds.borrow()[&(1, date(2024, 2, 18))].center();
        app.begin_time_drag(0, date(2024, 2, 25));
        app.move_time_drag(target, window, cx);
        app.finish_time_drag(window, cx);
        anyhow::ensure!(
            app.history_time_selection(cx) == Some((date(2024, 2, 18), date(2024, 2, 25)))
                && app.history_state.time_grids[1].is_focused(window),
            "reverse cross-panel drag failed"
        );
        app.history_state.time_cursors[0] = date(2024, 2, 29);
        app.history_state.time_grids[0].focus(window);
        Ok(())
    })??;
    cx.update(|window, cx| {
        window.dispatch_keystroke(Keystroke::parse("right").unwrap(), cx);
    })?;
    this.update_in(cx, |app, window, cx| {
        anyhow::ensure!(
            app.history_state.time_months == [date(2024, 2, 1); 2]
                && app.history_state.time_cursors[0] == date(2024, 2, 29),
            "keyboard cursor crossed the other calendar"
        );
        app.shift_time_calendar(1, 1, window, cx);
        app.history_state.time_grids[0].focus(window);
        Ok(())
    })??;
    cx.update(|window, cx| {
        window.dispatch_keystroke(Keystroke::parse("right").unwrap(), cx);
    })?;
    this.update_in(cx, |app, window, cx| {
        anyhow::ensure!(
            app.history_state.time_months == [date(2024, 3, 1); 2]
                && app.history_state.time_cursors[0] == date(2024, 3, 1),
            "keyboard could not enter the allowed month"
        );
        let months = app.history_state.time_months;
        app.history_calendar_action(CalendarAction::TimeCalendar, window, cx);
        app.history_calendar_action(CalendarAction::TimeCalendar, window, cx);
        anyhow::ensure!(
            app.history_state.time_months == months,
            "collapse reset independent browsing"
        );
        app.locate_time_calendar(window, cx);
        let months = app.history_state.time_months;
        let selection = app.history_time_selection(cx);
        app.history_state.time_parts[3].update(cx, |s, cx| s.set_value("99", window, cx));
        anyhow::ensure!(
            app.time_all_day_enabled(cx),
            "all-day must repair invalid time with valid dates"
        );
        app.set_time_all_day(window, cx);
        let values = app.time_part_strings(cx);
        anyhow::ensure!(
            [
                values[3].as_str(),
                values[4].as_str(),
                values[8].as_str(),
                values[9].as_str()
            ] == ["00", "00", "23", "59"]
                && app.history_time_selection(cx) == selection
                && app.history_state.time_months == months
                && !app.time_all_day_enabled(cx),
            "all-day changed dates or browsing"
        );
        anyhow::ensure!(
            app.query_id == original.0 && app.query == original.1,
            "calendar browsing, dragging or all-day queried"
        );
        app.set_time_values(["2024-02-12", "00:00", "2024-02-20", "23:59"], window, cx);
        app.locate_time_calendar(window, cx);
        app.history_state.time_focus.focus(window);
        cx.notify();
        Ok(())
    })??;
    settle_layout(this, cx).await
}

async fn exercise_quick_ranges(
    this: &WeakEntity<TrafficApp>,
    cx: &mut AsyncWindowContext,
) -> anyhow::Result<()> {
    let original = this.update_in(cx, |app, _, _| (app.query_id, app.query.clone()))?;
    for quick in TimeQuickRange::ALL {
        this.update_in(cx, |app, window, cx| {
            app.history_state.time_anchor = Some((
                NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
                NaiveDate::from_ymd_opt(2024, 2, 2).unwrap(),
            ));
            app.select_time_quick(quick, window, cx);
            let today = chrono::Local::now().date_naive();
            let (a, b) = quick.dates(today).unwrap();
            anyhow::ensure!(
                app.time_range_result(cx)?
                    == (
                        crate::history::midnight(a)?,
                        crate::history::midnight(b.succ_opt().unwrap())?
                    ),
                "shortcut bounds differ from natural days"
            );
            anyhow::ensure!(
                app.active_time_quick(today, cx) == Some(quick)
                    && app.history_state.time_anchor.is_none()
                    && app.history_state.time_drag.is_none()
                    && app.history_state.time_expanded,
                "shortcut selection or anchor state incorrect"
            );
            anyhow::ensure!(
                app.query_id == original.0 && app.query == original.1,
                "shortcut submitted a query"
            );
            Ok(())
        })??;
    }
    this.update_in(cx, |app, window, _| {
        app.history_state.time_grids[1].focus(window);
    })?;
    for (key, expected) in [
        ("tab", None),
        ("enter", Some(TimeQuickRange::Today)),
        ("tab", None),
        ("space", Some(TimeQuickRange::Yesterday)),
    ] {
        cx.update(|window, cx| {
            window.dispatch_keystroke(Keystroke::parse(key).unwrap(), cx);
        })?;
        if let Some(expected) = expected {
            this.update_in(cx, |app, _, cx| {
                anyhow::ensure!(
                    app.active_time_quick(chrono::Local::now().date_naive(), cx) == Some(expected),
                    "shortcut keyboard activation failed: {key}"
                );
                Ok(())
            })??;
        }
    }
    this.update_in(cx, |app, window, cx| {
        app.act(Action::Period(TimePeriod::Custom), window, cx);
        app.act(Action::Period(TimePeriod::Custom), window, cx);
        anyhow::ensure!(
            app.active_time_quick(chrono::Local::now().date_naive(), cx)
                == Some(TimeQuickRange::Yesterday)
                && !app.history_state.time_expanded,
            "collapse/reopen lost shortcut draft"
        );
        app.history_calendar_action(CalendarAction::TimeCalendar, window, cx);
        app.step_time_part(4, 1, window, cx);
        anyhow::ensure!(
            app.active_time_quick(chrono::Local::now().date_naive(), cx)
                .is_none(),
            "edited time retained shortcut highlight"
        );
        app.reset_time_range(window, cx);
        anyhow::ensure!(
            app.history_state.time_quick.is_none(),
            "reset retained shortcut selection"
        );
        app.set_time_values(["2024-02-12", "00:00", "2024-02-20", "23:59"], window, cx);
        app.locate_time_calendar(window, cx);
        app.history_state.time_focus.focus(window);
        anyhow::ensure!(
            app.query_id == original.0 && app.query == original.1,
            "shortcut keyboard or reset queried"
        );
        cx.notify();
        Ok(())
    })??;
    settle_layout(this, cx).await
}

pub(super) async fn exercise(
    this: &WeakEntity<TrafficApp>,
    cx: &mut AsyncWindowContext,
) -> anyhow::Result<()> {
    for page in [Page::Apps, Page::History] {
        let page_name = page.label();
        for theme in ["light", "dark"] {
            for width in [900., 1240., 1600.] {
                let original = this.update_in(cx, |app, window, cx| {
                    window.resize(size(px(width), px(870.)));
                    app.act(Action::Theme(theme), window, cx);
                    app.act(Action::Nav(page), window, cx);
                    app.act(
                        if page == Page::Apps {
                            Action::Period(TimePeriod::LastSevenDays)
                        } else {
                            Action::History(HistoryAction::Period(TimePeriod::LastSevenDays))
                        },
                        window,
                        cx,
                    );
                    let original = (app.query_id, app.query.clone());
                    app.act(
                        if page == Page::Apps {
                            Action::Period(TimePeriod::Custom)
                        } else {
                            Action::History(HistoryAction::QuickPeriod(TimePeriod::Custom))
                        },
                        window,
                        cx,
                    );
                    anyhow::ensure!(
                        app.history_state.time_open && !app.history_state.time_expanded,
                        "custom must open compact"
                    );
                    anyhow::ensure!(
                        app.history_state.time_focus.is_focused(window)
                            && !app.history_state.time_parts[0]
                                .focus_handle(cx)
                                .is_focused(window),
                        "opening Custom must not select the year"
                    );
                    app.set_time_values(["2024-02-12", "00:00", "2024-02-20", "23:59"], window, cx);
                    app.history_state.time_parts[0].update(cx, |s, cx| {
                        s.set_value("2024/02/12 – 2024/02/20", window, cx)
                    });
                    app.page_scroll.set_offset(point(px(0.), px(0.)));
                    seed(app, false, false);
                    cx.notify();
                    Ok(original)
                })??;
                settle_layout(this, cx).await?;
                let action_top = this.update_in(cx, |app, window, _| {
                    let card = app.history_state.time_editor_bounds.get();
                    let expected = window.viewport_size().width - app.sidebar_width() - px(48.);
                    anyhow::ensure!(
                        card.size.height >= px(46.)
                            && card.size.height <= px(if width < 1000. { 130. } else { 80. })
                            && (card.size.width - expected).abs() <= px(2.)
                            && (card.left() - app.sidebar_width() - px(24.)).abs() <= px(2.),
                        "compact editor has incorrect height: {card:?}"
                    );
                    for index in 0..10 {
                        let bounds = app.history_state.time_part_bounds[index].get();
                        let text = if index.is_multiple_of(5) { "8888" } else { "88" };
                        let mut style = window.text_style();
                        style.font_family = "Segoe UI".into();
                        style.font_weight = FontWeight::NORMAL;
                        let line = window.text_system().shape_line(
                            text.into(), px(14.), &[style.to_run(text.len())], None,
                        );
                        anyhow::ensure!(
                            (bounds.size.width - line.width - px(4.)).abs() <= px(1.)
                                && card.contains(&bounds.origin)
                                && card.right() >= bounds.right(),
                            "date field lacks caret room or is clipped: {page_name} {theme} {width} {bounds:?} {card:?}"
                        );
                    }
                    anyhow::ensure!(
                        app.query_id == original.0 && app.query == original.1,
                        "draft submitted query"
                    );
                    Ok(app.history_state.time_action_bounds.get().top())
                })??;
                cx.update(|window, cx| {
                    window.dispatch_keystroke(Keystroke::parse("tab").unwrap(), cx);
                })?;
                this.update_in(cx, |app, window, cx| {
                    anyhow::ensure!(
                        app.history_state.time_parts[0]
                            .focus_handle(cx)
                            .is_focused(window),
                        "first Tab must enter start year"
                    );
                    app.history_state.time_focus.focus(window);
                    Ok(())
                })??;
                settle_layout(this, cx).await?;
                if page == Page::Apps && theme == "light" && width == 900. {
                    exercise_partial_input(this, cx).await?;
                    exercise_compact_field_hits(this, cx).await?;
                    for i in 0..10 {
                        let before = this.update_in(cx, |app, _, cx| app.time_part_strings(cx))?;
                        hover_wheel(this, cx, TimeWheelTarget::Part(i), 40.).await?;
                        this.update_in(cx, |app, window, cx| {
                            anyhow::ensure!(
                                app.time_part_strings(cx) != before,
                                "unfocused part {i} did not change"
                            );
                            anyhow::ensure!(
                                app.history_state.time_focus.is_focused(window),
                                "hover stole input focus"
                            );
                            anyhow::ensure!(
                                app.query_id == original.0 && app.page_scroll.offset().y == px(0.),
                                "hover queried or scrolled page"
                            );
                            Ok(())
                        })??;
                        hover_wheel(this, cx, TimeWheelTarget::Part(i), -40.).await?;
                        this.update_in(cx, |app, _, cx| {
                            anyhow::ensure!(
                                app.time_part_strings(cx) == before,
                                "reverse wheel did not restore part {i}"
                            );
                            Ok(())
                        })??;
                    }
                    this.update_in(cx, |app, _, _| {
                        let delta = |n| ScrollDelta::Pixels(point(px(0.), px(n)));
                        anyhow::ensure!(
                            app.time_wheel_step(TimeWheelTarget::Part(0), delta(20.)) == 0,
                            "small wheel delta stepped"
                        );
                        anyhow::ensure!(
                            app.time_wheel_step(TimeWheelTarget::Part(1), delta(20.)) == 0,
                            "wheel remainder crossed fields"
                        );
                        anyhow::ensure!(
                            app.time_wheel_step(TimeWheelTarget::Part(1), delta(-20.)) == 0,
                            "wheel reversal retained remainder"
                        );
                        anyhow::ensure!(
                            app.time_wheel_step(TimeWheelTarget::Part(1), delta(-20.)) == -1,
                            "same-target deltas did not accumulate"
                        );
                        Ok(())
                    })??;
                }
                this.update_in(cx, |app, window, cx| {
                    app.act(
                        Action::History(HistoryAction::Calendar(CalendarAction::TimeCalendar)),
                        window,
                        cx,
                    )
                })?;
                Timer::after(Duration::from_millis(220)).await;
                settle_layout(this, cx).await?;
                if page == Page::Apps && theme == "light" && width == 900. {
                    exercise_independent_calendars(this, cx).await?;
                    exercise_quick_ranges(this, cx).await?;
                }
                this.update_in(cx, |app, window, cx| {
                    anyhow::ensure!(
                        app.history_state.time_action_bounds.get().top() == action_top,
                        "calendar expansion moved input actions"
                    );
                    let date = NaiveDate::from_ymd_opt(2024, 2, 15).unwrap();
                    let bounds = *app
                        .history_state
                        .time_bounds
                        .borrow()
                        .get(&(1, date))
                        .ok_or_else(|| anyhow::anyhow!("date was not painted"))?;
                    let card = app.history_state.time_editor_bounds.get();
                    anyhow::ensure!(
                        (bounds.size.height - px(28.)).abs() <= px(1.)
                            && (bounds.size.width - px(28.)).abs() <= px(1.)
                            && (bounds.size.width - bounds.size.height).abs() <= px(1.)
                            && card.contains(&bounds.origin)
                            && card.right() >= bounds.right(),
                        "calendar date clipped: {bounds:?} {card:?}"
                    );
                    let calendar = app.history_state.time_calendar_bounds.get();
                    anyhow::ensure!(
                        card.contains(&calendar.origin)
                            && calendar.right() <= card.right()
                            && calendar.bottom() <= card.bottom(),
                        "calendar must stay inside shared frame"
                    );
                    let quick = app
                        .history_state
                        .time_quick_bounds
                        .each_ref()
                        .map(|b| b.get());
                    let dates_right = app
                        .history_state
                        .time_bounds
                        .borrow()
                        .values()
                        .map(|b| b.right())
                        .max()
                        .unwrap();
                    let dates_bottom = app
                        .history_state
                        .time_bounds
                        .borrow()
                        .values()
                        .map(|b| b.bottom())
                        .max()
                        .unwrap();
                    for rect in quick {
                        anyhow::ensure!(
                            calendar.contains(&rect.origin)
                                && rect.right() <= calendar.right()
                                && rect.bottom() <= calendar.bottom()
                                && rect.bottom() <= card.bottom(),
                            "quick range clipped: {page_name} {theme} {width} {rect:?}"
                        );
                    }
                    if width >= 1240. {
                        anyhow::ensure!(
                            quick[0].left() >= dates_right && quick[4].top() > quick[0].top(),
                            "wide quick ranges must form a side column"
                        );
                    } else {
                        anyhow::ensure!(
                            quick[0].top() >= dates_bottom,
                            "narrow quick ranges must follow calendars"
                        );
                    }
                    app.history_state.time_parts[2]
                        .focus_handle(cx)
                        .focus(window);
                    Ok(())
                })??;
                cx.update(|window, cx| {
                    window.dispatch_keystroke(Keystroke::parse("up").unwrap(), cx);
                })?;
                settle_layout(this, cx).await?;
                this.update_in(cx, |app, window, cx| {
                    anyhow::ensure!(
                        app.history_time_selection(cx).unwrap().0.day() == 13,
                        "native up key failed"
                    );
                    app.history_state.time_focus.focus(window);
                    Ok(())
                })??;
                hover_wheel(this, cx, TimeWheelTarget::Part(2), 40.).await?;
                if page == Page::Apps && theme == "light" && width == 900. {
                    this.update_in(cx, |app, window, cx| {
                        app.history_state.time_months = [
                            NaiveDate::from_ymd_opt(2022, 1, 1).unwrap(),
                            NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
                        ];
                        app.shift_time_calendar(0, 0, window, cx);
                    })?;
                    settle_layout(this, cx).await?;
                    for i in 0..4 {
                        let before = this.update_in(cx, |app, _, cx| {
                            (app.history_state.time_months, app.time_part_strings(cx))
                        })?;
                        hover_wheel(this, cx, TimeWheelTarget::Browse(i), 40.).await?;
                        this.update_in(cx, |app, window, cx| {
                            let months = if i.is_multiple_of(2) { 12 } else { 1 };
                            anyhow::ensure!(
                                app.history_state.time_months[i / 2]
                                    == before.0[i / 2]
                                        .checked_add_months(chrono::Months::new(months))
                                        .unwrap()
                                    && app.history_state.time_months[1 - i / 2]
                                        == before.0[1 - i / 2],
                                "header field {i} changed wrong unit"
                            );
                            anyhow::ensure!(
                                app.time_part_strings(cx) == before.1
                                    && app.query_id == original.0
                                    && app.history_state.time_focus.is_focused(window),
                                "header hover altered draft, query or focus"
                            );
                            Ok(())
                        })??;
                        hover_wheel(this, cx, TimeWheelTarget::Browse(i), -40.).await?;
                    }
                    let month = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
                    this.update_in(cx, |app, window, cx| {
                        app.history_state.time_months = [month; 2];
                        app.shift_time_calendar(0, 0, window, cx);
                    })?;
                    settle_layout(this, cx).await?;
                    for (i, amount) in [(0, 40.), (1, 40.), (2, -40.), (3, -40.)] {
                        hover_wheel(this, cx, TimeWheelTarget::Browse(i), amount).await?;
                        this.update_in(cx, |app, _, _| {
                            anyhow::ensure!(
                                app.history_state.time_months == [month; 2],
                                "hover wheel field {i} bypassed month order"
                            );
                            Ok(())
                        })??;
                    }
                }
                this.update_in(cx, |app, window, cx| {
                    anyhow::ensure!(
                        app.history_time_selection(cx).unwrap().0.day() == 14,
                        "unfocused wheel increment failed"
                    );
                    app.act(
                        if page == Page::Apps {
                            Action::Period(TimePeriod::Custom)
                        } else {
                            Action::History(HistoryAction::QuickPeriod(TimePeriod::Custom))
                        },
                        window,
                        cx,
                    );
                    anyhow::ensure!(
                        !app.history_state.time_open
                            && !app.history_state.time_expanded
                            && app.history_time_selection(cx).unwrap().0.day() == 14,
                        "clicking Custom again must collapse and keep the draft"
                    );
                    app.act(
                        if page == Page::Apps {
                            Action::Period(TimePeriod::Custom)
                        } else {
                            Action::History(HistoryAction::QuickPeriod(TimePeriod::Custom))
                        },
                        window,
                        cx,
                    );
                    anyhow::ensure!(
                        app.history_state.time_open
                            && !app.history_state.time_expanded
                            && app.history_state.time_focus.is_focused(window)
                            && app.history_time_selection(cx).unwrap().0.day() == 14,
                        "reopening Custom lost the draft"
                    );
                    app.act(
                        Action::History(HistoryAction::Calendar(CalendarAction::TimeCalendar)),
                        window,
                        cx,
                    );
                    app.act(
                        Action::History(HistoryAction::Calendar(CalendarAction::TimeCalendar)),
                        window,
                        cx,
                    );
                    anyhow::ensure!(
                        !app.history_state.time_expanded
                            && app.history_time_selection(cx).unwrap().0.day() == 14,
                        "collapse lost draft"
                    );
                    app.act(
                        Action::History(HistoryAction::Calendar(CalendarAction::TimeCalendar)),
                        window,
                        cx,
                    );
                    app.history_state.time_pick_target = None;
                    app.pick_history_time(
                        NaiveDate::from_ymd_opt(2024, 2, 17).unwrap(),
                        window,
                        cx,
                    );
                    app.pick_history_time(
                        NaiveDate::from_ymd_opt(2024, 2, 19).unwrap(),
                        window,
                        cx,
                    );
                    let raw = app
                        .history_state
                        .time_parts
                        .iter()
                        .map(|s| s.read(cx).value())
                        .collect::<Vec<_>>();
                    let target = app.history_state.time_bounds.borrow()
                        [&(1, NaiveDate::from_ymd_opt(2024, 2, 22).unwrap())]
                        .center();
                    app.begin_time_drag(1, NaiveDate::from_ymd_opt(2024, 2, 18).unwrap());
                    app.move_time_drag(target, window, cx);
                    anyhow::ensure!(
                        app.history_time_selection(cx)
                            == Some((
                                NaiveDate::from_ymd_opt(2024, 2, 18).unwrap(),
                                NaiveDate::from_ymd_opt(2024, 2, 22).unwrap()
                            )),
                        "drag preview did not follow pointer"
                    );
                    anyhow::ensure!(
                        app.history_state
                            .time_parts
                            .iter()
                            .map(|s| s.read(cx).value())
                            .collect::<Vec<_>>()
                            == raw,
                        "drag preview rewrote input entities"
                    );
                    app.finish_time_drag(window, cx);
                    anyhow::ensure!(
                        app.history_state.time_parts[2].read(cx).value() == "18"
                            && app.history_state.time_parts[7].read(cx).value() == "22"
                            && app.history_state.time_cursors[1]
                                == NaiveDate::from_ymd_opt(2024, 2, 22).unwrap(),
                        "drag release did not update input fields"
                    );
                    anyhow::ensure!(app.query_id == original.0, "calendar queried before apply");
                    app.history_calendar_action(
                        CalendarAction::TimeQuick(TimeQuickRange::LastMonth),
                        window,
                        cx,
                    );
                    let quick_range = app.time_range_result(cx)?;
                    anyhow::ensure!(
                        app.query_id == original.0 && app.history_state.time_expanded,
                        "quick range submitted early or collapsed calendar"
                    );
                    app.act(
                        if page == Page::Apps {
                            Action::ApplyRange
                        } else {
                            Action::History(HistoryAction::ApplyRange)
                        },
                        window,
                        cx,
                    );
                    anyhow::ensure!(
                        app.query_id == original.0 + 1
                            && app.history_state.time_open
                            && !app.history_state.time_expanded,
                        "apply must query once, keep bar and collapse calendar"
                    );
                    anyhow::ensure!(!app.time_range_dirty(cx), "applied range remains dirty");
                    anyhow::ensure!(
                        (app.query.from, app.query.to) == quick_range,
                        "quick range was not applied"
                    );
                    let app_range = (app.query.from, app.query.to);
                    app.act(Action::Nav(Page::Overview), window, cx);
                    anyhow::ensure!(
                        app.period != TimePeriod::Custom && !app.history_state.time_open,
                        "overview retained custom range"
                    );
                    if page == Page::Apps {
                        app.act(Action::Nav(Page::Apps), window, cx);
                        anyhow::ensure!(
                            (app.query.from, app.query.to) == app_range,
                            "overview erased application's custom range"
                        );
                    }
                    app.act(Action::Period(TimePeriod::Today), window, cx);
                    app.app_custom_range = None;
                    app.history_state.time_open = false;
                    app.history_state.time_expanded = false;
                    cx.notify();
                    Ok(())
                })??;
                println!("shared compact time editor passed: {page_name} {theme} {width}");
            }
        }
    }
    for theme in ["light", "dark"] {
        for width in [900., 1240., 1600.] {
            let original = this.update_in(cx, |app, window, cx| {
                window.resize(size(px(width), px(870.)));
                app.act(Action::Theme(theme), window, cx);
                app.act(Action::Nav(Page::History), window, cx);
                app.act(
                    Action::History(HistoryAction::Tab(HistoryTab::Analysis)),
                    window,
                    cx,
                );
                app.open_history_time(window, cx);
                app.set_time_values(["2024-02-12", "09:00", "2024-02-20", "18:00"], window, cx);
                app.history_calendar_action(CalendarAction::TimeCalendar, window, cx);
                seed(app, false, false);
                cx.notify();
                (app.history_state.analysis_range, app.query.clone())
            })?;
            Timer::after(Duration::from_millis(220)).await;
            settle_layout(this, cx).await?;
            this.update_in(cx, |app, window, _| {
                let frame = app.history_state.time_editor_bounds.get();
                let calendar = app.history_state.time_calendar_bounds.get();
                for part in &app.history_state.time_part_bounds {
                    let rect = part.get();
                    anyhow::ensure!(
                        frame.contains(&rect.origin) && frame.right() >= rect.right(),
                        "comparison date field clipped: {theme} {width}"
                    );
                }
                anyhow::ensure!(
                    frame.contains(&calendar.origin) && frame.bottom() >= calendar.bottom(),
                    "comparison calendar escaped shared frame"
                );
                app.history_state.time_focus.focus(window);
                Ok(())
            })??;
            hover_wheel(this, cx, TimeWheelTarget::Part(4), 40.).await?;
            this.update_in(cx, |app, window, cx| {
                anyhow::ensure!(
                    app.history_state.time_parts[4].read(cx).value() == "01",
                    "comparison hover did not edit minute"
                );
                anyhow::ensure!(
                    app.history_state.analysis_range == original.0
                        && app.query == original.1
                        && app.history_state.time_focus.is_focused(window),
                    "comparison draft changed query or focus"
                );
                app.select_time_quick(TimeQuickRange::LastWeek, window, cx);
                let quick = app.time_range_result(cx)?;
                anyhow::ensure!(
                    app.history_state.analysis_range == original.0 && app.query == original.1,
                    "comparison shortcut submitted before apply"
                );
                app.apply_history_dates(cx);
                anyhow::ensure!(
                    app.history_state.analysis_range == quick
                        && app.query == original.1
                        && !app.history_state.time_expanded,
                    "comparison shortcut did not apply independently"
                );
                app.act(
                    Action::History(HistoryAction::Tab(HistoryTab::Records)),
                    window,
                    cx,
                );
                Ok(())
            })??;
            println!("shared time editor comparison passed: {theme} {width}");
        }
    }
    Ok(())
}
