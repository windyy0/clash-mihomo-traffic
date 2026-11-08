use super::super::history_state::CalendarAction;
use super::*;
use chrono::{Datelike, NaiveDate};

fn date(month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, month, day).unwrap()
}
async fn key(cx: &mut AsyncWindowContext, key: &str) -> anyhow::Result<()> {
    cx.update(|window, cx| {
        window.dispatch_keystroke(Keystroke::parse(key).unwrap(), cx);
    })?;
    Ok(())
}
async fn pointer(
    this: &WeakEntity<TrafficApp>,
    cx: &mut AsyncWindowContext,
    p: Point<Pixels>,
    down: bool,
    shift: bool,
) -> anyhow::Result<()> {
    this.update_in(cx, |app, window, cx| {
        if down {
            let date = app
                .history_state
                .day_bounds
                .borrow()
                .iter()
                .find(|(_, b)| b.contains(&p))
                .map(|(date, _)| *date)
                .unwrap();
            app.begin_history_drag(date, shift, window, cx);
        } else {
            app.finish_history_drag(cx);
            app.finish_history_drag(cx);
        }
    })?;
    Ok(())
}
async fn center(
    this: &WeakEntity<TrafficApp>,
    cx: &mut AsyncWindowContext,
    d: NaiveDate,
) -> anyhow::Result<Point<Pixels>> {
    this.update_in(cx, |app, _, _| {
        let b = app.history_state.day_bounds.borrow();
        let rect = b
            .get(&d)
            .ok_or_else(|| anyhow::anyhow!("calendar date is not laid out: {d}"))?;
        Ok(rect.center())
    })?
}
fn scope(app: &TrafficApp) -> (i64, i64) {
    (app.query.from, app.query.to)
}
fn day_scope(a: NaiveDate, b: NaiveDate) -> (i64, i64) {
    (
        crate::history::day_bounds(a).unwrap().0,
        crate::history::day_bounds(b).unwrap().1,
    )
}

pub(super) async fn exercise(
    this: &WeakEntity<TrafficApp>,
    cx: &mut AsyncWindowContext,
) -> anyhow::Result<()> {
    for theme in ["light", "dark"] {
        for width in [900., 1240.] {
            this.update_in(cx, |app, window, cx| {
                window.resize(size(px(width), px(700.)));
                app.act(Action::Theme(theme), window, cx);
                app.act(Action::Nav(Page::History), window, cx);
                app.act(
                    Action::History(HistoryAction::Tab(HistoryTab::Records)),
                    window,
                    cx,
                );
                app.history_state.restore_offset = None;
                app.history_state.month = date(2, 1);
                app.history_state.time_open = false;
                app.history_state.month_open = false;
                app.picker = None;
                app.commit_history_dates(date(2, 12), date(2, 12), date(2, 12));
                app.page_scroll.set_offset(point(px(0.), px(0.)));
                seed(app, false, false);
                cx.notify();
            })?;
            settle_layout(this, cx).await?;
            this.update_in(cx, |app, window, _| {
                let a = app.history_state.calendar_bounds.get();
                let b = app.history_state.hero_bounds.get();
                anyhow::ensure!(
                    a.size.width >= px(240.) && b.size.width > px(260.) && a.right() <= b.left(),
                    "calendar/chart must remain beside each other: {theme} {width} {a:?} {b:?}"
                );
                anyhow::ensure!(
                    (a.top() - b.top()).abs() <= px(1.)
                        && a.bottom() < window.viewport_size().height
                        && b.bottom() < window.viewport_size().height,
                    "history hero must fit first screen: {a:?} {b:?}"
                );
                anyhow::ensure!(
                    app.page_scroll.max_offset().width <= px(1.),
                    "calendar page horizontally overflows"
                );
                Ok(())
            })??;
            let mut stable_layout = None;
            for (year, month, row_count) in [(2021, 2, 4), (2024, 2, 5), (2024, 9, 6)] {
                this.update_in(cx, |app, _, cx| {
                    app.history_state.month = NaiveDate::from_ymd_opt(year, month, 1).unwrap();
                    app.history_state.day_bounds.borrow_mut().clear();
                    seed(app, false, false);
                    cx.notify();
                })?;
                settle_layout(this, cx).await?;
                this.update_in(cx, |app, window, _| {
                    let calendar = app.history_state.calendar_bounds.get();
                    let trend = app.history_state.hero_bounds.get();
                    let layout = (calendar, trend, app.history_state.help_trigger.get(), app.history_state.year_trigger.get());
                    if let Some(previous) = stable_layout {
                        anyhow::ensure!(layout == previous, "switching four/five/six-row months moved the calendar, trend or header: {theme} {width}");
                    }
                    stable_layout = Some(layout);
                    anyhow::ensure!(
                        calendar.size.height <= px(if width >= 1240. { 452. } else { 512. }),
                        "fixed calendar and trend cards must stay compact: {theme} {width} {calendar:?} {trend:?}"
                    );
                    anyhow::ensure!(
                        (calendar.top() - trend.top()).abs() <= px(1.)
                            && (calendar.bottom() - trend.bottom()).abs() <= px(1.),
                        "calendar and trend must remain aligned: {theme} {width} {calendar:?} {trend:?}"
                    );
                    let bounds = app.history_state.day_bounds.borrow();
                    let mut rows = vec![];
                    for (date, rect) in bounds.iter() {
                        anyhow::ensure!(
                            (rect.size.width - px(28.)).abs() < px(0.1)
                                && (rect.size.height - px(28.)).abs() < px(0.1),
                            "calendar date is not a 28 DIP square: {date} {rect:?}"
                        );
                        if !rows.contains(&rect.top()) {
                            rows.push(rect.top());
                        }
                        if date.weekday().num_days_from_monday() < 6
                            && let Some(next) = date.succ_opt().and_then(|d| bounds.get(&d))
                        {
                            anyhow::ensure!(
                                (next.left() - rect.right() - px(4.)).abs() < px(0.1),
                                "calendar column gap changed"
                            );
                        }
                        if let Some(next) = date
                            .checked_add_days(chrono::Days::new(7))
                            .and_then(|d| bounds.get(&d))
                        {
                            anyhow::ensure!(
                                (next.top() - rect.bottom() - px(4.)).abs() < px(0.1),
                                "calendar row gap changed"
                            );
                        }
                    }
                    anyhow::ensure!(
                        rows.len() == row_count,
                        "calendar month row count differs: {year}-{month}"
                    );
                    let last_date_bottom = bounds.values().map(|rect| rect.bottom()).max().unwrap();
                    anyhow::ensure!(
                        last_date_bottom + px(40.) < calendar.bottom(),
                        "last calendar row overlaps the footer: {year}-{month}"
                    );
                    anyhow::ensure!(
                        app.history_state.calendar_bounds.get().bottom()
                            < window.viewport_size().height
                            && app.history_state.hero_bounds.get().bottom()
                                < window.viewport_size().height,
                        "month does not fit first screen: {theme} {width} {year}-{month}"
                    );
                    anyhow::ensure!(
                        app.page_scroll.max_offset().width <= px(1.),
                        "calendar month overflows horizontally"
                    );
                    Ok(())
                })??;
            }
            this.update_in(cx, |app, _, cx| {
                app.history_state.month = date(2, 1);
                app.history_state.day_bounds.borrow_mut().clear();
                seed(app, false, false);
                cx.notify();
            })?;
            settle_layout(this, cx).await?;
            let a = center(this, cx, date(2, 12)).await?;
            let b = center(this, cx, date(2, 18)).await?;
            let id = this.update_in(cx, |app, _, _| app.query_id)?;
            pointer(this, cx, a, true, false).await?;
            this.update_in(cx, |app, _, cx| app.move_history_drag(b, cx))?;
            this.update_in(cx, |app, _, _| {
                anyhow::ensure!(
                    app.query_id == id
                        && app
                            .history_state
                            .drag
                            .is_some_and(|d| d.over == date(2, 18)),
                    "drag preview queried or failed to extend"
                );
                Ok(())
            })??;
            pointer(this, cx, b, false, false).await?;
            this.update_in(cx, |app, _, _| {
                anyhow::ensure!(
                    app.query_id == id + 1
                        && scope(app) == day_scope(date(2, 12), date(2, 18))
                        && app.history_state.drag.is_none(),
                    "drag release must submit exactly once"
                );
                Ok(())
            })??;
            settle_layout(this, cx).await?;
            let a = center(this, cx, date(2, 20)).await?;
            pointer(this, cx, a, true, false).await?;
            pointer(this, cx, a, false, false).await?;
            let b = center(this, cx, date(2, 12)).await?;
            pointer(this, cx, b, true, true).await?;
            pointer(this, cx, b, false, true).await?;
            this.update_in(cx, |app, window, cx| {
                anyhow::ensure!(
                    scope(app) == day_scope(date(2, 12), date(2, 20))
                        && app.history_state.anchor == date(2, 20),
                    "reverse Shift selection lost its anchor"
                );
                let id = app.query_id;
                app.act(
                    Action::History(HistoryAction::Calendar(CalendarAction::Shift(-1))),
                    window,
                    cx,
                );
                anyhow::ensure!(
                    app.query_id == id && app.history_state.anchor == date(2, 20),
                    "browsing month changed the query or anchor"
                );
                seed(app, false, false);
                cx.notify();
                Ok(())
            })??;
            settle_layout(this, cx).await?;
            let a = center(this, cx, date(1, 31)).await?;
            pointer(this, cx, a, true, true).await?;
            pointer(this, cx, a, false, true).await?;
            this.update_in(cx, |app, _, _| {
                anyhow::ensure!(
                    scope(app) == day_scope(date(1, 31), date(2, 20)),
                    "cross-month Shift selection failed"
                );
                Ok(())
            })??;
            key(cx, "shift-right").await?;
            this.update_in(cx, |app, _, cx| {
                anyhow::ensure!(
                    scope(app) == day_scope(date(2, 1), date(2, 20))
                        && app.history_state.month == date(2, 1),
                    "keyboard extension did not cross the month boundary"
                );
                seed(app, false, false);
                cx.notify();
                Ok(())
            })??;
            settle_layout(this, cx).await?;
            let a = center(this, cx, date(2, 12)).await?;
            let id = this.update_in(cx, |app, _, _| app.query_id)?;
            pointer(this, cx, a, true, false).await?;
            key(cx, "escape").await?;
            pointer(this, cx, a, false, false).await?;
            this.update_in(cx, |app, window, cx| {
                anyhow::ensure!(
                    app.query_id == id && app.history_state.drag.is_none(),
                    "Escape committed a cancelled drag"
                );
                app.act(
                    Action::History(HistoryAction::Calendar(CalendarAction::Week(date(1, 29)))),
                    window,
                    cx,
                );
                anyhow::ensure!(
                    scope(app) == day_scope(date(1, 29), date(2, 4)),
                    "week selection must include Monday through Sunday"
                );
                app.begin_history_month_drag(date(2, 1), false, window, cx);
                app.finish_history_drag(cx);
                anyhow::ensure!(
                    scope(app) == day_scope(date(2, 1), date(2, 29)),
                    "whole leap month selection failed"
                );
                app.act(
                    Action::History(HistoryAction::Calendar(CalendarAction::OpenTime)),
                    window,
                    cx,
                );
                app.history_state.time_parts[0]
                    .focus_handle(cx)
                    .focus(window);
                Ok(())
            })??;
            settle_layout(this, cx).await?;
            for _ in 0..7 {
                key(cx, "tab").await?;
            }
            this.update_in(cx, |app, window, cx| {
                anyhow::ensure!(
                    app.history_state.time_focus.contains_focused(window, cx),
                    "time modal tab order escaped the modal"
                );
                Ok(())
            })??;
            let original = this.update_in(cx, |app, _, _| scope(app))?;
            key(cx, "escape").await?;
            this.update_in(cx, |app, window, cx| {
                anyhow::ensure!(
                    !app.history_state.time_expanded && scope(app) == original,
                    "cancelled time modal changed query"
                );
                app.act(
                    Action::History(HistoryAction::Calendar(CalendarAction::OpenTime)),
                    window,
                    cx,
                );
                app.history_state.time_precision = true;
                app.set_time_values(["2024-02-20", "10:15", "2024-02-20", "10:14"], window, cx);
                app.act(Action::History(HistoryAction::ApplyRange), window, cx);
                anyhow::ensure!(
                    app.history_state.time_open
                        && !app.history_state.error.is_empty()
                        && scope(app) == original,
                    "reversed minute range was accepted"
                );
                app.history_state.time_parts[9].update(cx, |s, cx| s.set_value("15", window, cx));
                Ok(())
            })??;
            key(cx, "enter").await?;
            this.update_in(cx, |app, window, cx| {
                anyhow::ensure!(
                    app.history_state.time_open
                        && !app.history_state.time_expanded
                        && app.query.to - app.query.from == 60_000,
                    "exact minute must be inclusive"
                );
                app.commit_history_dates(date(2, 20), date(2, 20), date(2, 20));
                seed(app, false, false);
                let full = scope(app);
                let hour = app.report.history.as_ref().unwrap().buckets[2].clone();
                let id = app.query_id;
                app.act(
                    Action::History(HistoryAction::Bucket(
                        hour.from,
                        hour.to,
                        crate::history::Grain::Hour,
                    )),
                    window,
                    cx,
                );
                anyhow::ensure!(
                    app.query_id == id + 1
                        && app.query.history_context.is_none()
                        && scope(app) == (hour.from, hour.to),
                    "hour drill queried twice or retained the day chart"
                );
                seed(app, false, false);
                anyhow::ensure!(
                    app.report.history.as_ref().unwrap().from == hour.from
                        && app.report.history.as_ref().unwrap().grain
                            == crate::history::Grain::Minute
                        && app.application_detail_query().is_none(),
                    "hour view did not show minute data for the actual detail scope"
                );
                app.act(
                    Action::History(HistoryAction::Calendar(CalendarAction::Back)),
                    window,
                    cx,
                );
                anyhow::ensure!(
                    scope(app) == full && app.query.history_context.is_none(),
                    "restore day failed"
                );
                app.analysis.snapshot = None;
                app.analysis.pending = None;
                app.act(
                    Action::History(HistoryAction::Tab(HistoryTab::Analysis)),
                    window,
                    cx,
                );
                anyhow::ensure!(
                    app.history_state.analysis_range == full,
                    "first comparison did not inherit the actual record range"
                );
                let request = crate::analysis::AnalysisRequest::historical(
                    800,
                    app.data.storage_revision,
                    full.0,
                    full.1,
                    chrono::Utc::now().timestamp_millis(),
                )?;
                app.analysis.snapshot = Some(std::sync::Arc::new(
                    crate::analysis::Snapshot::fixture(request),
                ));
                app.analysis.pending = None;
                app.analysis.error.clear();
                app.analysis.anchor = None;
                app.page_scroll.set_offset(point(px(0.), px(0.)));
                cx.notify();
                Ok(())
            })??;
            settle_layout(this, cx).await?;
            key(cx, "tab").await?;
            this.update_in(cx, |app, window, cx| {
                anyhow::ensure!(
                    app.history_state.tab == HistoryTab::Analysis,
                    "Tab left the comparison page"
                );
                app.toggle_picker(
                    "history-range",
                    vec![(
                        "今天".into(),
                        Action::History(HistoryAction::Period(TimePeriod::Today)),
                        true,
                    )],
                    window,
                    cx,
                );
                Ok(())
            })??;
            key(cx, "escape").await?;
            this.update_in(cx, |app, _, _| {
                anyhow::ensure!(
                    app.picker.is_none() && app.history_state.tab == HistoryTab::Analysis,
                    "Escape left comparison before closing its range menu"
                );
                Ok(())
            })??;
            key(cx, "escape").await?;
            this.update_in(cx, |app, _, cx| {
                anyhow::ensure!(
                    app.history_state.tab == HistoryTab::Records
                        && scope(app) == day_scope(date(2, 20), date(2, 20)),
                    "comparison close lost history context"
                );
                let month = app.history_state.visible_month().unwrap().clone();
                let mut wanted = month.request.clone();
                wanted.id = 900;
                app.history_state.month_pending = Some(wanted.clone());
                let mut stale = wanted.clone();
                stale.id = 899;
                *app.backend.calendar_result.lock().unwrap() =
                    Some(crate::backend::CalendarReply {
                        request: stale,
                        result: Ok(month.clone()),
                    });
                app.poll_history_calendar(false, cx);
                anyhow::ensure!(
                    app.history_state.month_pending.as_ref() == Some(&wanted)
                        && std::sync::Arc::ptr_eq(
                            app.history_state.visible_month().unwrap(),
                            &month
                        ),
                    "stale calendar reply replaced current month"
                );
                *app.backend.calendar_result.lock().unwrap() =
                    Some(crate::backend::CalendarReply {
                        request: wanted.clone(),
                        result: Err("test refresh failed".into()),
                    });
                app.poll_history_calendar(false, cx);
                anyhow::ensure!(
                    std::sync::Arc::ptr_eq(app.history_state.visible_month().unwrap(), &month)
                        && !app.history_state.month_error.is_empty(),
                    "calendar failure discarded old heatmap"
                );
                app.backend.shared.lock().unwrap().storage_revision += 1;
                app.poll_history_calendar(false, cx);
                anyhow::ensure!(
                    app.history_state.cache.is_empty(),
                    "old source calendar survived source revision change"
                );
                app.backend.shared.lock().unwrap().storage_revision = 0;
                *app.backend.calendar_result.lock().unwrap() = None;
                seed(app, false, false);
                Ok(())
            })??;
            println!(
                "calendar live input passed: {theme} {width}x700, drag once, Shift, keyboard, modal, drill, comparison page and source isolation"
            );
        }
    }
    this.update_in(cx, |app, window, cx| {
        window.resize(size(px(720.), px(700.)));
        app.history_state.month = date(2, 1);
        app.history_state.day_bounds.borrow_mut().clear();
        app.history_state.restore_offset = None;
        app.page_scroll.set_offset(point(px(0.), px(0.)));
        seed(app, false, false);
        cx.notify();
    })?;
    settle_layout(this, cx).await?;
    this.update_in(cx, |app, window, _| {
        let calendar = app.history_state.calendar_bounds.get();
        let hero = app.history_state.hero_bounds.get();
        anyhow::ensure!(
            window.viewport_size().width < px(780.),
            "narrow calendar test did not resize"
        );
        anyhow::ensure!(
            hero.top() >= calendar.bottom(),
            "narrow calendar/chart did not stack"
        );
        let dates = app.history_state.day_bounds.borrow();
        let first = dates.get(&date(2, 1)).unwrap();
        let grid_left = first.left() - px(22. + 3. * 32.);
        anyhow::ensure!(
            (grid_left + px(121.) - calendar.center().x).abs() < px(0.6),
            "fixed calendar grid is not centered in narrow card"
        );
        anyhow::ensure!(
            dates
                .values()
                .all(|r| (r.size.width - px(28.)).abs() < px(0.1)
                    && (r.size.height - px(28.)).abs() < px(0.1)),
            "narrow calendar stretched the date cells"
        );
        anyhow::ensure!(
            app.page_scroll.max_offset().width <= px(1.),
            "narrow calendar overflows horizontally"
        );
        Ok(())
    })??;
    println!("calendar squares passed: four/five/six rows, 4 DIP gaps and centered narrow grid");
    exercise_trend_input(this, cx).await?;
    exercise_dual_view(this, cx).await?;
    Ok(())
}

async fn trend_move(
    this: &WeakEntity<TrafficApp>,
    cx: &mut AsyncWindowContext,
    position: Point<Pixels>,
) -> anyhow::Result<()> {
    this.update_in(cx, |app, _, cx| {
        let plot = app.history_state.chart_plot_bounds.get();
        let count = app.report.history.as_ref().unwrap().buckets.len();
        let index = (f32::from(position.x - plot.left()) / f32::from(plot.size.width)
            * count as f32) as usize;
        app.hover_history_chart(index, true, cx);
    })?;
    Ok(())
}

async fn trend_click(
    this: &WeakEntity<TrafficApp>,
    cx: &mut AsyncWindowContext,
    position: Point<Pixels>,
    button: MouseButton,
) -> anyhow::Result<()> {
    this.update_in(cx, |app, window, cx| {
        if button == MouseButton::Left
            && !app.history_state.range_stack.is_empty()
            && app
                .history_state
                .chart_back_bounds
                .get()
                .contains(&position)
        {
            app.act(
                Action::History(HistoryAction::Calendar(CalendarAction::Back)),
                window,
                cx,
            );
            if app.history_state.range_stack.is_empty() {
                app.history_state.record_period = TimePeriod::Custom;
            }
        } else {
            let plot = app.history_state.chart_plot_bounds.get();
            let data = app.report.history.as_ref().unwrap();
            let index = (f32::from(position.x - plot.left()) / f32::from(plot.size.width)
                * data.buckets.len() as f32) as usize;
            if button == MouseButton::Left {
                app.pin_history_chart(index, cx);
            } else {
                let b = &data.buckets[index];
                let range = (b.from, b.to, data.grain);
                // This suite uses February 2024. Capture the seven-day preset
                // only in the drill action, so the live clock cannot replace
                // the fixed fixture while testing previews and exit timers.
                if app.history_state.range_stack.is_empty() {
                    app.history_state.record_period = TimePeriod::LastSevenDays;
                }
                app.act(
                    Action::History(HistoryAction::Bucket(range.0, range.1, range.2)),
                    window,
                    cx,
                );
            }
        }
    })?;
    Ok(())
}

async fn exercise_trend_input(
    this: &WeakEntity<TrafficApp>,
    cx: &mut AsyncWindowContext,
) -> anyhow::Result<()> {
    for theme in ["light", "dark"] {
        for width in [900., 1240.] {
            this.update_in(cx, |app, window, cx| {
                window.resize(size(px(width), px(700.)));
                app.act(Action::Theme(theme), window, cx);
                app.history_state.view = super::super::history_state::CalendarView::Month;
                app.history_state.month = date(2, 1);
                app.history_state.time_open = false;
                app.history_state.month_open = false;
                app.history_state.help_open = false;
                app.picker = None;
                app.commit_history_dates(date(2, 19), date(2, 25), date(2, 19));
                app.history_state.record_period = TimePeriod::Custom;
                app.page_scroll.set_offset(point(px(0.), px(0.)));
                seed(app, false, false);
                cx.notify();
            })?;
            settle_layout(this, cx).await?;
            let (id, full, hero, position, day) = this.update_in(cx, |app, _, _| {
                let plot = app.history_state.chart_plot_bounds.get();
                let data = app.report.history.as_ref().unwrap();
                (
                    app.query_id,
                    scope(app),
                    app.history_state.hero_bounds.get(),
                    point(
                        plot.left() + plot.size.width / data.buckets.len() as f32 / 2.,
                        plot.bottom() - px(4.),
                    ),
                    (data.buckets[0].from, data.buckets[0].to),
                )
            })?;
            this.update_in(cx, |app, _, _| {
                anyhow::ensure!(
                    app.history_chart_path() == vec![("2024年 · 2月19–25日".into(), None)],
                    "root date should have one full caption without an overview prefix"
                );
                Ok(())
            })??;
            this.update_in(cx, |app, window, cx| {
                // Assert entry in the same turn, before the real desktop
                // cursor can deliver a different hover during a later frame.
                app.history_state.chart_hover = None;
                app.track_history_chart_pointer(position, cx);
                anyhow::ensure!(
                    app.valid_history_chart_hover()
                        .is_some_and(|h| h.visible && h.index == 0),
                    "trend hover was not immediate"
                );
                anyhow::ensure!(
                    app.history_chart_hover(window, cx).is_some(),
                    "trend preview was unavailable on entry"
                );
                let sequence = app.history_state.chart_hover.unwrap().sequence;
                let plot = app.history_state.chart_plot_bounds.get();
                let data = app.report.history.as_ref().unwrap().clone();
                let count = data.buckets.len();
                let common_width = app.history_hover_layout(&data, 0, window).size.width;
                anyhow::ensure!(
                    common_width <= px(192.),
                    "popup width exceeds the compact limit"
                );
                for pass in [false, true] {
                    for n in 0..count {
                        let i = if pass { count - n - 1 } else { n };
                        let cursor = point(
                            plot.left() + plot.size.width * (i as f32 + 0.5) / count as f32,
                            plot.center().y,
                        );
                        for _ in 0..4 {
                            app.track_history_chart_pointer(cursor, cx);
                        }
                        anyhow::ensure!(
                            app.history_hover_layout(&data, i, window).size.width == common_width,
                            "same-level popup widths differ"
                        );
                        anyhow::ensure!(
                            app.valid_history_chart_hover().is_some_and(|h| h.index == i
                                && h.visible
                                && h.sequence == sequence),
                            "moving across the plot flickered or restarted the animation"
                        );
                    }
                }
                app.track_history_chart_pointer(position, cx);
                app.pin_history_chart(0, cx);
                Ok(())
            })??;
            settle_layout(this, cx).await?;
            this.update_in(cx, |app, window, _| {
                anyhow::ensure!(
                    app.valid_history_chart_hover().is_some_and(|h| h.pinned),
                    "left click did not pin the preview"
                );
                anyhow::ensure!(
                    app.query_id == id && scope(app) == full,
                    "preview changed the query"
                );
                anyhow::ensure!(
                    app.history_state.hero_bounds.get() == hero,
                    "preview changed the chart layout"
                );
                let data = app.report.history.as_ref().unwrap();
                let expected = app.history_hover_layout(data, 0, window).size;
                let actual = app.history_state.chart_hover_bounds.get().size;
                anyhow::ensure!((actual.width - expected.width).abs() <= px(1.) && (actual.height - expected.height).abs() <= px(1.), "popup measured size differs from its natural layout: {actual:?} vs {expected:?}");
                Ok(())
            })??;
            key(cx, "escape").await?;
            this.update_in(cx, |app, _, cx| {
                anyhow::ensure!(
                    !app.history_state.chart_hover.is_some_and(|h| h.pinned),
                    "Escape did not release the pinned preview"
                );
                app.history_state.chart_hover = None;
                app.pin_history_chart(0, cx);
                Ok(())
            })??;
            Timer::after(Duration::from_millis(180)).await;
            this.update_in(cx, |app, _, _| {
                anyhow::ensure!(
                    app.valid_history_chart_hover().is_some_and(|h| h.visible),
                    "old exit animation removed a newer hover"
                );
                Ok(())
            })??;
            trend_click(this, cx, position, MouseButton::Right).await?;
            this.update_in(cx, |app, _, cx| {
                anyhow::ensure!(
                    app.query_id == id + 1
                        && scope(app) == day
                        && app.history_state.range_stack.len() == 1,
                    "right click must drill exactly once"
                );
                seed(app, false, false);
                cx.notify();
                Ok(())
            })??;
            exercise_chart_motion(this, cx, hero).await?;
            settle_layout(this, cx).await?;
            let (hour_position, hour) = this.update_in(cx, |app, _, _| {
                let current = app.history_state.hero_bounds.get();
                anyhow::ensure!(
                    current == hero,
                    "drill inserted a row or moved the trend: {hero:?} -> {current:?}"
                );
                let back = app.history_state.chart_back_bounds.get();
                anyhow::ensure!(
                    back.top() > hero.top() && back.right() < hero.right() - px(80.),
                    "back is not beside the legend"
                );
                let plot = app.history_state.chart_plot_bounds.get();
                let data = app.report.history.as_ref().unwrap();
                Ok((
                    point(
                        plot.left() + plot.size.width * 2.5 / data.buckets.len() as f32,
                        plot.bottom() - px(4.),
                    ),
                    (data.buckets[2].from, data.buckets[2].to),
                ))
            })??;
            trend_move(this, cx, hour_position).await?;
            trend_click(this, cx, hour_position, MouseButton::Right).await?;
            this.update_in(cx, |app, _, cx| {
                anyhow::ensure!(
                    scope(app) == hour
                        && app.query.history_context.is_none()
                        && app.history_state.range_stack.len() == 2,
                    "hour drill retained the parent chart"
                );
                seed(app, false, false);
                cx.notify();
                Ok(())
            })??;
            settle_layout(this, cx).await?;
            this.update_in(cx, |app, _, _| {
                let trend = app.report.history.as_ref().unwrap();
                anyhow::ensure!(
                    trend.grain == crate::history::Grain::Minute
                        && trend.buckets.len() == 60
                        && (trend.from, trend.to) == hour,
                    "selected hour did not render 60 minute buckets"
                );
                let path = app.history_chart_path();
                anyhow::ensure!(
                    path.len() == 3
                        && path[0].0 == "7天总览"
                        && path[1].0 == "2月19日"
                        && path[2].0 == "02:00–03:00",
                    "wrong drill breadcrumb: {path:?}"
                );
                Ok(())
            })??;
            let before_seconds = this.update_in(cx, |app, _, _| {
                (
                    app.history_state.hero_bounds.get(),
                    app.history_state.calendar_bounds.get(),
                )
            })?;
            trend_click(this, cx, hour_position, MouseButton::Right).await?;
            this.update_in(cx, |app, _, cx| {
                anyhow::ensure!(
                    app.query_id == id + 3
                        && app.history_state.range_stack.len() == 3
                        && app.history_state.seconds_level
                        && app.seconds.inline_history,
                    "minute drill did not become an inline history level"
                );
                anyhow::ensure!(
                    app.seconds.scope.is_some()
                        && app.seconds.view.is_some_and(|v| v.seconds
                            && v.to - v.from <= crate::speed_stats::WINDOW
                            && (v.from, v.to) == scope(app)),
                    "inline range differs from history query"
                );
                anyhow::ensure!(!app.seconds.is_modal(), "inline history opened a modal");
                seed(app, false, false);
                let request = app.seconds.pending.take().unwrap();
                let view = app.seconds.view.unwrap();
                app.seconds.loaded_request = Some(request.clone());
                app.seconds.displayed = Some((request.generation, request.revision));
                app.seconds.data = Some(crate::speed_stats::SecondsData {
                    from: view.from,
                    to: view.to,
                    first: Some(view.from),
                    last: Some(view.to),
                    trend: app.report.history.as_deref().cloned(),
                    samples: (0..600)
                        .map(|i| crate::speed_stats::SecondSample {
                            seq: i,
                            from: view.from + i * 1000,
                            to: view.from + (i + 1) * 1000,
                            duration_ms: 1000,
                            down: ((i as f64 / 11.).sin() + 1.) * 1_000_000.,
                            up: ((i as f64 / 17.).cos() + 1.) * 300_000.,
                            valid: true,
                        })
                        .collect(),
                });
                cx.notify();
                Ok::<_, anyhow::Error>(())
            })??;
            settle_layout(this, cx).await?;
            this.update_in(cx, |app, window, cx| {
                anyhow::ensure!(
                    app.seconds_overlay(window, cx).is_none(),
                    "unexpected modal"
                );
                let hero = app.history_state.hero_bounds.get();
                let calendar=app.history_state.calendar_bounds.get();
                anyhow::ensure!((hero.size.width-before_seconds.0.size.width).abs()<px(1.) && (hero.size.height-before_seconds.0.size.height).abs()<px(1.) && calendar==before_seconds.1,"ten-minute drill resized the original cards: {before_seconds:?} / {hero:?} {calendar:?}");
                let minute=app.seconds.minute_bounds.get();let second=app.seconds.second_bounds.get();
                anyhow::ensure!((minute.left()-second.left()).abs()<px(1.) && (minute.right()-second.right()).abs()<px(1.) && minute==second && second.size.height==px(144.),"compact plots are not aligned: {minute:?} {second:?}");
                anyhow::ensure!(hero.right()<=window.viewport_size().width && app.page_scroll.max_offset().width<=px(1.),"compact history overflows");
                Ok::<_, anyhow::Error>(())
            })??;
            this.update_in(cx, |app, window, cx| {
                app.seconds.focus.focus(window);
                app.hover_drill(crate::traffic_drill::Target::Second(2), true, true, cx);
            })?;
            cx.update(|window, cx| {
                window.dispatch_keystroke(Keystroke::parse("right").unwrap(), cx)
            })?;
            this.update_in(cx, |app, _, _| {
                anyhow::ensure!(
                    app.seconds.pinned
                        && app.seconds.hover == Some(crate::traffic_drill::Target::Second(3)),
                    "right did not select the next raw sample"
                );
                Ok::<_, anyhow::Error>(())
            })??;
            cx.update(|window, cx| {
                window.dispatch_keystroke(Keystroke::parse("left").unwrap(), cx)
            })?;
            this.update_in(cx, |app, window, _| {
                anyhow::ensure!(
                    app.seconds.hover == Some(crate::traffic_drill::Target::Second(2)),
                    "left did not restore raw sample"
                );
                window.blur();
                Ok::<_, anyhow::Error>(())
            })??;
            cx.update(|window, cx| {
                window.dispatch_keystroke(Keystroke::parse("right").unwrap(), cx)
            })?;
            this.update_in(cx, |app, _, _| {
                anyhow::ensure!(
                    app.seconds.hover == Some(crate::traffic_drill::Target::Second(2)),
                    "arrows outside plot focus changed sample"
                );
                Ok::<_, anyhow::Error>(())
            })??;
            super::drill_time_tests::exercise(this, cx).await?;
            if std::env::var_os("MIHOMO_COMBO_PREVIEW").is_some()
                && theme == "light"
                && width == 1240.
            {
                this.update_in(cx, |app, _, cx| {
                    app.seconds.hover = Some(crate::traffic_drill::Target::Second(2));
                    app.seconds.pinned = true;
                    let from = app.seconds.data.as_ref().unwrap().from;
                    app.seconds.hover_data = Some(crate::traffic_drill::HoverData {
                        from: from + 2000,
                        to: from + 3000,
                        duration_ms: 1000,
                        down: 5000.,
                        up: 1000.,
                        observed: true,
                        applications: (0..8)
                            .map(|i| Row {
                                key: format!("预览应用{i}.exe"),
                                bytes: Bytes { down: 500, up: 100 },
                            })
                            .collect(),
                        ..Default::default()
                    });
                    cx.notify();
                })?;
                settle_layout(this, cx).await?;
                println!("combined inline preview ready");
                Timer::after(Duration::from_secs(45)).await;
            }
            let locate_request = this.update_in(cx, |app, window, cx| {
                app.act(Action::Drill(DrillAction::DrillLocateToggle), window, cx);
                app.seconds.locator.parts[5].focus_handle(cx).focus(window);
                (app.query_id, app.seconds.serial)
            })?;
            cx.update(|window, cx| {
                window.dispatch_keystroke(Keystroke::parse("enter").unwrap(), cx)
            })?;
            this.update_in(cx, |app, _, cx| {
                anyhow::ensure!(
                    (app.query_id == locate_request.0 || app.query_id == locate_request.0 + 1)
                        && app.seconds.serial == locate_request.1 + 1
                        && app.seconds.inline_history
                        && !app.seconds.locate_open,
                    "inline Enter did not locate exactly once: query {}->{}, seconds {}->{}, inline {}, open {}, error {}",
                    locate_request.0, app.query_id, locate_request.1, app.seconds.serial,
                    app.seconds.inline_history, app.seconds.locate_open, app.seconds.locator.error
                );
                app.history_back_to(2);
                cx.notify();
                Ok::<_, anyhow::Error>(())
            })??;
            settle_layout(this, cx).await?;
            for expected in [day, full] {
                let back = this.update_in(cx, |app, _, _| {
                    app.history_state.chart_back_bounds.get().center()
                })?;
                trend_click(this, cx, back, MouseButton::Left).await?;
                this.update_in(cx, |app, _, cx| {
                    anyhow::ensure!(
                        scope(app) == expected,
                        "back did not restore the previous range"
                    );
                    anyhow::ensure!(
                        app.report.query.as_ref() == Some(&app.query)
                            && app
                                .report
                                .history
                                .as_ref()
                                .is_some_and(|trend| (trend.from, trend.to) == expected)
                            && !app.report.rows.is_empty()
                            && app.pending_query.as_ref() == Some(&app.query),
                        "back cleared the parent chart, totals or records while refreshing"
                    );
                    cx.notify();
                    Ok(())
                })??;
                // Leave the backend response pending for the whole transition.
                // The parent snapshot must render immediately, without a blank loader.
                exercise_chart_motion(this, cx, hero).await?;
                this.update_in(cx, |app, _, cx| {
                    anyhow::ensure!(
                        app.report.query.as_ref() == Some(&app.query)
                            && !app.report.rows.is_empty(),
                        "parent snapshot disappeared before the refresh arrived"
                    );
                    seed(app, false, false);
                    cx.notify();
                    Ok(())
                })??;
                settle_layout(this, cx).await?;
            }
            this.update_in(cx, |app, _, cx| {
                anyhow::ensure!(
                    app.history_state.hero_bounds.get() == hero
                        && app.page_scroll.max_offset().width <= px(1.),
                    "return shifted or overflowed the chart: {hero:?} -> {:?}, overflow {:?}",
                    app.history_state.hero_bounds.get(),
                    app.page_scroll.max_offset()
                );
                app.hover_history_chart(0, true, cx);
                let source = app.data.storage_revision;
                app.data.storage_revision += 1;
                anyhow::ensure!(
                    app.valid_history_chart_hover().is_none(),
                    "old source preview survived"
                );
                app.data.storage_revision = source;
                app.query_id += 1;
                anyhow::ensure!(
                    app.valid_history_chart_hover().is_some(),
                    "same-range background refresh dismissed the preview"
                );
                let from = app.query.from;
                app.query.from += 60_000;
                anyhow::ensure!(
                    app.valid_history_chart_hover().is_none(),
                    "old-range preview survived a new request"
                );
                app.query.from = from;
                app.history_state.chart_hover = None;
                app.report.query = None;
                cx.notify();
                Ok(())
            })??;
            settle_layout(this, cx).await?;
            this.update_in(cx, |app, _, cx| {
                anyhow::ensure!(
                    app.history_state.hero_bounds.get() == hero,
                    "loading shifted the chart layout"
                );
                seed(app, false, false);
                cx.notify();
                Ok(())
            })??;
            this.update_in(cx, |app, window, cx| {
                let day = app.report.history.as_ref().unwrap().buckets[0].clone();
                app.act(
                    Action::History(HistoryAction::Bucket(
                        day.from,
                        day.to,
                        crate::history::Grain::Day,
                    )),
                    window,
                    cx,
                );
                seed(app, false, false);
                let hour = app.report.history.as_ref().unwrap().buckets[0].clone();
                app.act(
                    Action::History(HistoryAction::Bucket(
                        hour.from,
                        hour.to,
                        crate::history::Grain::Hour,
                    )),
                    window,
                    cx,
                );
                app.act(Action::History(HistoryAction::BackTo(0)), window, cx);
                anyhow::ensure!(
                    scope(app) == full
                        && app.history_state.range_stack.is_empty()
                        && app.report.query.as_ref() == Some(&app.query)
                        && !app.report.rows.is_empty(),
                    "breadcrumb did not restore the root without blanking its records"
                );
                seed(app, false, false);
                cx.notify();
                Ok(())
            })??;
            exercise_compact_card_layout(this, cx).await?;
            println!(
                "trend hover/right-click passed: {theme} {width}x700, instant preview, pin/Esc, stale exit/source/request, one query, fixed layout and two-level return"
            );
        }
    }
    Ok(())
}

async fn exercise_chart_motion(
    this: &WeakEntity<TrafficApp>,
    cx: &mut AsyncWindowContext,
    hero: Bounds<Pixels>,
) -> anyhow::Result<()> {
    if !crate::native::client_area_animations_enabled() {
        return Ok(());
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        Timer::after(Duration::from_millis(20)).await;
        let complete = this.update_in(cx, |app, _, _| {
            let motion = app.history_state.chart_motion.borrow();
            anyhow::ensure!(
                app.history_state.hero_bounds.get() == hero,
                "chart animation changed the card layout"
            );
            if motion.observed_frames.last().is_some_and(|frame| !frame.0) {
                let first = &motion.observed_frames.first().unwrap().1;
                let last = &motion.observed_frames.last().unwrap().1;
                anyhow::ensure!(
                    motion.observed_frames.len() >= 3,
                    "chart transition did not paint intermediate frames"
                );
                anyhow::ensure!(
                    first[1].left != last[1].left && first[1].width != last[1].width,
                    "chart transition only faded instead of moving and resizing columns"
                );
                anyhow::ensure!(
                    motion
                        .observed_frames
                        .iter()
                        .any(|(_, bars)| bars[1].left != first[1].left
                            && bars[1].left != last[1].left),
                    "chart transition skipped the intermediate column geometry"
                );
                return Ok(true);
            }
            Ok(false)
        })??;
        if complete {
            return Ok(());
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "chart transition did not finish painting"
        );
    }
}

async fn exercise_compact_card_layout(
    this: &WeakEntity<TrafficApp>,
    cx: &mut AsyncWindowContext,
) -> anyhow::Result<()> {
    this.update_in(cx, |_, window, _| {
        let mut style = window.text_style();
        style.font_family = "Microsoft YaHei UI".into();
        for weight in [FontWeight::NORMAL, FontWeight::MEDIUM] {
            style.font_weight = weight;
            for caption in ["00:00–01:00", "13:00–14:00", "23:00–24:00"] {
                let text = history_range::display_text(caption);
                let line = window.text_system().shape_line(
                    text.clone().into(),
                    px(11.),
                    &[style.to_run(text.len())],
                    None,
                );
                let rules = history_range::separator_rules(&text, None, &style, window);
                anyhow::ensure!(rules.len() == 1, "time range lost its separator");
                anyhow::ensure!(
                    rules[0].size.height == px(0.85),
                    "separator did not retain the lighter design stroke"
                );
                let dash = text.find('–').unwrap();
                let before = text[..dash]
                    .char_indices()
                    .rev()
                    .find(|(_, ch)| *ch == '0')
                    .unwrap()
                    .0;
                let end = dash + '–'.len_utf8();
                let after = end
                    + text[end..]
                        .char_indices()
                        .find(|(_, ch)| ch.is_ascii_digit())
                        .unwrap()
                        .0;
                let ink = window.text_system().typographic_bounds(
                    line.font_id_for_index(before).unwrap(),
                    px(11.),
                    '0',
                )?;
                let right_ink = window.text_system().typographic_bounds(
                    line.font_id_for_index(after).unwrap(),
                    px(11.),
                    text[after..].chars().next().unwrap(),
                )?;
                let baseline = (px(16.) - line.ascent - line.descent) / 2. + line.ascent;
                let digit_center = baseline - ink.origin.y - ink.size.height / 2.;
                let optical_center = digit_center - px(0.25);
                anyhow::ensure!(
                    (rules[0].center().y - optical_center).abs() < px(0.01),
                    "separator did not retain its optical lift: {caption}"
                );
                let origin = point(px(517.3333), px(220.6667));
                let painted = history_range::align_rule_to_pixels(
                    Bounds::new(origin + rules[0].origin, rules[0].size),
                    window.scale_factor(),
                );
                anyhow::ensure!(
                    (painted.center().y - origin.y - optical_center).abs() * window.scale_factor()
                        <= px(0.501),
                    "pixel alignment moved the separator away from its optical center: {caption}"
                );
                for edge in [
                    painted.left(),
                    painted.right(),
                    painted.top(),
                    painted.bottom(),
                ] {
                    let device = f32::from(edge) * window.scale_factor();
                    anyhow::ensure!(
                        (device - device.round()).abs() < 0.001,
                        "separator edge is between device pixels: {caption}"
                    );
                }
                let left_gap = painted.left()
                    - origin.x
                    - (line.x_for_index(before) + ink.origin.x + ink.size.width);
                let right_gap =
                    origin.x + line.x_for_index(after) + right_ink.origin.x - painted.right();
                anyhow::ensure!(
                    left_gap * window.scale_factor() >= px(1.)
                        && right_gap * window.scale_factor() >= px(1.),
                    "separator touches a digit: {caption}: {left_gap:?}/{right_gap:?}"
                );
            }
            for caption in [
                "2026年 · 10月19–25日",
                "2024年12月29日–2025年01月04日",
                "2026年 · 10月29日  13:00–14:00\n图表范围 13:00–14:00",
            ] {
                let text = history_range::display_text(caption);
                let rules = history_range::separator_rules(&text, Some(px(152.)), &style, window);
                anyhow::ensure!(
                    rules.len() == text.matches('–').count()
                        && rules.iter().all(|r| r.left() >= px(0.)
                            && r.right() <= px(152.)
                            && r.top() >= px(0.)),
                    "wrapped range separator escaped its label: {caption}: {rules:?}"
                );
            }
        }
        Ok(())
    })??;
    let period = this.update_in(cx, |app, _, _| {
        let period = app.history_state.record_period;
        app.history_state.record_period = TimePeriod::Custom;
        period
    })?;
    let original = this.update_in(cx, |app, _, _| app.report.history.as_ref().unwrap().clone())?;
    let mut last_width = px(0.);
    for state in 0..4 {
        this.update_in(cx, |app, _, cx| {
            let mut trend = (*original).clone();
            let bucket = &mut trend.buckets[0];
            bucket.bytes = if state == 1 {
                Bytes {
                    down: u64::MAX / 2,
                    up: u64::MAX / 2,
                }
            } else {
                Bytes::default()
            };
            bucket.observed = state < 2;
            if state == 3 {
                trend.generated_at = bucket.from;
            }
            app.report.history = Some(std::sync::Arc::new(trend));
            app.backend.shared.lock().unwrap().report = app.report.clone().into();
            app.received_report_id = app.query_id;
            app.history_state.chart_hover = None;
            app.pin_history_chart(0, cx);
            cx.notify();
        })?;
        settle_layout(this, cx).await?;
        last_width = this.update_in(cx, |app, window, _| {
            let data = app.report.history.as_ref().unwrap();
            if state == 1 {
                anyhow::ensure!(
                    data.buckets[0].bytes.down == u64::MAX / 2,
                    "desktop polling replaced the extreme-value fixture"
                );
            }
            let layout = app.history_hover_layout(data, 0, window);
            let actual = app.history_state.chart_hover_bounds.get();
            anyhow::ensure!(
                layout.size.width >= last_width && layout.size.width <= px(192.),
                "same-range refresh shrank or overgrew the popup"
            );
            anyhow::ensure!(
                (actual.size.width - layout.size.width).abs() <= px(1.)
                    && (actual.size.height - layout.size.height).abs() <= px(1.),
                "natural card size differs for state={state}: {:?} vs {:?}",
                actual.size,
                layout.size
            );
            anyhow::ensure!(
                actual.left() >= px(8.)
                    && actual.right() <= window.viewport_size().width - px(8.)
                    && actual.bottom() <= window.viewport_size().height - px(8.),
                "compact popup left the viewport"
            );
            if state == 1 {
                anyhow::ensure!(
                    layout.size.height >= px(125.),
                    "extreme values clipped the card rows: {:?}",
                    layout.size
                );
                let previous_view = app.history_state.view;
                app.history_state.view = super::super::history_state::CalendarView::Year;
                let annual = app.history_hover_layout(data, 0, window);
                anyhow::ensure!(
                    annual.size.width <= px(160.) && annual.size.width < layout.size.width,
                    "year trend hover did not shrink or reused the month width cache"
                );
                app.history_state.view = previous_view;
                anyhow::ensure!(
                    app.history_hover_layout(data, 0, window).size == layout.size,
                    "restoring month view retained the year hover metrics"
                );
            } else if state >= 2 {
                anyhow::ensure!(
                    layout.size.height <= px(if state == 2 { 84. } else { 64. }),
                    "missing/future trend status retained the old large text height: {:?}",
                    layout.size
                );
            }
            Ok(layout.size.width)
        })??;
    }
    this.update_in(cx, |app, window, cx| {
        let from =
            crate::history::midnight(NaiveDate::from_ymd_opt(2024, 12, 29).unwrap()).unwrap();
        let to = crate::history::midnight(NaiveDate::from_ymd_opt(2025, 1, 5).unwrap()).unwrap();
        let mut long = (*original).clone();
        long.from = from;
        long.to = to;
        long.grain = crate::history::Grain::Week;
        long.buckets = vec![crate::history::Bucket {
            from,
            to,
            bytes: Bytes { down: 1, up: 2 },
            observed: true,
            expected_ms: to - from,
            covered_ms: to - from,
        }];
        let long = std::sync::Arc::new(long);
        let layout = app.history_hover_layout(&long, 0, window);
        anyhow::ensure!(
            layout.size.height > px(125.),
            "cross-year title did not wrap within the compact width"
        );
        app.report.history = Some(original);
        app.history_state.record_period = period;
        app.backend.shared.lock().unwrap().report = app.report.clone().into();
        app.history_state.chart_hover = None;
        cx.notify();
        Ok(())
    })??;
    Ok(())
}

pub(super) async fn exercise_dual_view(
    this: &WeakEntity<TrafficApp>,
    cx: &mut AsyncWindowContext,
) -> anyhow::Result<()> {
    use super::super::history_state::{AnnualScope, CalendarView, TimeUnit};
    for theme in ["light", "dark"] {
        for width in [900., 1240.] {
            this.update_in(cx, |app, window, cx| {
                window.resize(size(px(width), px(700.)));
                app.act(Action::Theme(theme), window, cx);
                app.act(Action::Nav(Page::History), window, cx);
                app.act(
                    Action::History(HistoryAction::Tab(HistoryTab::Records)),
                    window,
                    cx,
                );
                app.history_state.restore_offset = None;
                app.page_scroll.set_offset(point(px(0.), px(0.)));
                app.act(
                    Action::History(HistoryAction::Period(TimePeriod::LastSevenDays)),
                    window,
                    cx,
                );
                seed(app, false, false);
                let query = app.query.clone();
                let id = app.query_id;
                let anchor = app.history_state.anchor;
                app.act(
                    Action::History(HistoryAction::Calendar(CalendarAction::View(
                        CalendarView::Year,
                    ))),
                    window,
                    cx,
                );
                seed(app, false, false);
                anyhow::ensure!(
                    app.query == query && app.query_id == id && app.history_state.anchor == anchor,
                    "changing calendar view changed the query or anchor"
                );
                anyhow::ensure!(
                    app.history_state
                        .calendar_months(chrono::Local::now().date_naive())
                        .iter()
                        .all(|m| app.history_state.cached_month(*m).is_some()),
                    "year fixture lost a required month"
                );
                Ok(())
            })??;
            settle_layout(this, cx).await?;
            this.update_in(cx, |app,window,cx| {
                let calendar=app.history_state.calendar_bounds.get();
                let chart=app.history_state.hero_bounds.get();
                anyhow::ensure!(chart.top()>=calendar.bottom()&&(chart.size.width-calendar.size.width).abs()<px(1.),"annual calendar and trend did not use full-width stacked layout: {calendar:?} {chart:?}");
                anyhow::ensure!(chart.bottom()<window.viewport_size().height-px(4.),"annual calendar and trend do not fit the first screen: {theme} {width} {chart:?}");
                anyhow::ensure!(app.page_scroll.max_offset().width<=px(1.),"annual calendar overflows horizontally");
                let query=app.query.clone();
                let id=app.query_id;
                app.act(Action::History(HistoryAction::Calendar(CalendarAction::Annual(AnnualScope::Year(2024)))),window,cx);
                seed(app,false,false);
                anyhow::ensure!(app.query==query&&app.query_id==id,"browsing a year changed the traffic query");
                anyhow::ensure!(app.history_state.cache.len()==12,"natural year needs twelve monthly results");
                // Prefer evicting an out-of-view month over rereading the visible year.
                let january=app.history_state.cached_month(date(1,1)).unwrap().clone();
                app.history_state.cache.retain(|m|m.request.month!=1);
                let mut outside=(*january).clone(); outside.request.year=2023;
                app.history_state.cache.push_back(std::sync::Arc::new(outside));
                let mut incoming=(*january).clone(); incoming.request.id=40001;
                let request=incoming.request.clone();
                app.history_state.month_pending=Some(request.clone());
                *app.backend.calendar_result.lock().unwrap()=Some(crate::backend::CalendarReply{request,result:Ok(std::sync::Arc::new(incoming))});
                app.poll_history_calendar(false,cx);
                anyhow::ensure!(app.history_state.calendar_months(chrono::Local::now().date_naive()).iter().all(|m|app.history_state.cached_month(*m).is_some())&&app.history_state.cache.len()==12,"calendar cache evicted a visible month");
                app.act(Action::History(HistoryAction::Calendar(CalendarAction::View(CalendarView::Month))),window,cx);
                anyhow::ensure!(app.query==query&&app.query_id==id&&app.history_state.month==crate::history::local_date(query.to-1).with_day(1).unwrap(),"return to month view lost selected range");
                app.begin_history_month_drag(date(2,1), false, window, cx);
                app.finish_history_drag(cx);
                anyhow::ensure!(scope(app)==day_scope(date(2,1),date(2,29)),"annual month shortcut lost leap day");
                seed(app,false,false);
                app.act(Action::History(HistoryAction::Calendar(CalendarAction::OpenTime)),window,cx);
                app.act(Action::History(HistoryAction::Calendar(CalendarAction::TimeUnit(TimeUnit::Month))),window,cx);
                app.history_state.year_input.update(cx,|s,cx|s.set_value("2023",window,cx));
                app.act(Action::History(HistoryAction::Calendar(CalendarAction::JumpYear)),window,cx);
                let before=app.query_id;
                app.act(Action::History(HistoryAction::Calendar(CalendarAction::TimePick(NaiveDate::from_ymd_opt(2023,11,1).unwrap()))),window,cx);
                app.history_state.year_input.update(cx,|s,cx|s.set_value("2024",window,cx));
                app.act(Action::History(HistoryAction::Calendar(CalendarAction::JumpYear)),window,cx);
                app.act(Action::History(HistoryAction::Calendar(CalendarAction::TimePick(date(2,1)))),window,cx);
                anyhow::ensure!(app.query_id==before,"draft range selection submitted a query");
                app.act(Action::History(HistoryAction::ApplyRange),window,cx);
                anyhow::ensure!(app.query_id==before+1&&scope(app)==day_scope(NaiveDate::from_ymd_opt(2023,11,1).unwrap(),date(2,29)),"cross-year monthly range did not apply once");
                seed(app,false,false);
                app.act(Action::History(HistoryAction::Calendar(CalendarAction::OpenTime)),window,cx);
                app.act(Action::History(HistoryAction::Calendar(CalendarAction::TimeUnit(TimeUnit::Year))),window,cx);
                app.act(Action::History(HistoryAction::Calendar(CalendarAction::TimeCalendar)),window,cx);
                app.history_state.year_input.update(cx,|s,cx|s.set_value("2020",window,cx));
                app.history_state.year_input.focus_handle(cx).focus(window);
                Ok(())
            })??;
            settle_layout(this, cx).await?;
            let before = this.update_in(cx, |app, _, _| (app.query_id, scope(app)))?;
            key(cx, "enter").await?;
            this.update_in(cx, |app, window, cx| {
                anyhow::ensure!(
                    app.history_state.time_open
                        && app.history_state.time_year == 2020
                        && app.query_id == before.0,
                    "year input Enter applied instead of browsing"
                );
                app.act(
                    Action::History(HistoryAction::Calendar(CalendarAction::TimePick(
                        NaiveDate::from_ymd_opt(2020, 1, 1).unwrap(),
                    ))),
                    window,
                    cx,
                );
                app.act(
                    Action::History(HistoryAction::Calendar(CalendarAction::TimePick(
                        NaiveDate::from_ymd_opt(2021, 1, 1).unwrap(),
                    ))),
                    window,
                    cx,
                );
                app.act(
                    Action::History(HistoryAction::Calendar(CalendarAction::TimeReset)),
                    window,
                    cx,
                );
                anyhow::ensure!(
                    scope(app) == before.1 && app.query_id == before.0,
                    "cancelled year draft changed history"
                );
                app.history_state.time_unit = TimeUnit::Day;
                app.act(
                    Action::History(HistoryAction::Calendar(CalendarAction::View(
                        CalendarView::Year,
                    ))),
                    window,
                    cx,
                );
                app.act(
                    Action::History(HistoryAction::Calendar(CalendarAction::Annual(
                        AnnualScope::Year(2020),
                    ))),
                    window,
                    cx,
                );
                app.act(
                    Action::History(HistoryAction::Period(TimePeriod::Today)),
                    window,
                    cx,
                );
                let bounds = app
                    .history_state
                    .annual_scope
                    .bounds(chrono::Local::now().date_naive());
                anyhow::ensure!(
                    bounds.0 <= chrono::Local::now().date_naive()
                        && bounds.1 >= chrono::Local::now().date_naive(),
                    "choosing today left annual view on an unrelated year"
                );
                app.act(
                    Action::History(HistoryAction::Calendar(CalendarAction::View(
                        CalendarView::Month,
                    ))),
                    window,
                    cx,
                );
                seed(app, false, false);
                Ok(())
            })??;
            println!(
                "dual calendar view, yearly cache, full-screen layout and long-range picker passed: {theme} {width}x700"
            );
        }
    }
    this.update_in(cx, |app, window, cx| {
        app.act(
            Action::History(HistoryAction::Calendar(CalendarAction::View(
                CalendarView::Year,
            ))),
            window,
            cx,
        );
        app.act(
            Action::History(HistoryAction::Calendar(CalendarAction::Annual(
                AnnualScope::Year(2024),
            ))),
            window,
            cx,
        );
        seed(app, false, false);
    })?;
    settle_layout(this, cx).await?;
    let before_cancel = this.update_in(cx, |app, window, cx| {
        let endpoint = app.history_state.month_bounds.borrow()[&date(8, 1)].center();
        let id = app.query_id;
        app.begin_history_month_drag(date(5, 1), false, window, cx);
        app.move_history_drag(endpoint, cx);
        anyhow::ensure!(
            app.query_id == id
                && app.history_state.selected_dates(&app.query) == (date(5, 1), date(8, 31)),
            "month drag preview submitted a query or lost month endpoints"
        );
        app.finish_history_drag(cx);
        app.finish_history_drag(cx);
        anyhow::ensure!(
            app.query_id == id + 1 && scope(app) == day_scope(date(5, 1), date(8, 31)),
            "month drag did not submit exactly once"
        );
        let endpoint = app.history_state.month_bounds.borrow()[&date(6, 1)].center();
        app.begin_history_month_drag(date(11, 1), false, window, cx);
        app.move_history_drag(endpoint, cx);
        anyhow::ensure!(
            app.history_state.selected_dates(&app.query) == (date(6, 1), date(11, 30)),
            "reverse month drag lost the range"
        );
        Ok((app.query_id, scope(app)))
    })??;
    key(cx, "escape").await?;
    this.update_in(cx, |app, window, cx| {
        anyhow::ensure!(
            app.history_state.drag.is_none() && (app.query_id, scope(app)) == before_cancel,
            "cancelling month drag changed the query"
        );
        app.act(
            Action::History(HistoryAction::Calendar(CalendarAction::Help(true))),
            window,
            cx,
        );
        Ok(())
    })??;
    key(cx, "escape").await?;
    this.update_in(cx, |app, _, _| {
        anyhow::ensure!(
            !app.history_state.help_open && (app.query_id, scope(app)) == before_cancel,
            "help Escape changed history context"
        );
        Ok(())
    })??;
    this.update_in(cx, |app, window, cx| {
        app.act(
            Action::History(HistoryAction::Calendar(CalendarAction::View(
                CalendarView::Month,
            ))),
            window,
            cx,
        );
    })?;
    println!(
        "annual month-label drag passed: forward, reverse, one commit, cancel and help Escape"
    );
    exercise_year_controls(this, cx).await?;
    Ok(())
}

async fn exercise_year_controls(
    this: &WeakEntity<TrafficApp>,
    cx: &mut AsyncWindowContext,
) -> anyhow::Result<()> {
    use super::super::history_state::{AnnualScope, CalendarView};
    let saved = this.update_in(cx, |app, window, cx| {
        let before = (app.query_id, scope(app), app.history_state.anchor);
        let year = app.history_state.month.year();
        app.history_year_title_clicked(false, window, cx);
        anyhow::ensure!(
            app.history_state.month_open && !app.history_state.year_edit,
            "first year click must open selection"
        );
        app.history_year_title_clicked(false, window, cx);
        anyhow::ensure!(
            app.history_state.year_edit
                && app
                    .history_state
                    .year_input
                    .focus_handle(cx)
                    .is_focused(window),
            "another single year click must focus the editor"
        );
        app.history_state.year_edit = false;
        app.history_year_title_clicked(true, window, cx);
        anyhow::ensure!(
            app.history_state.year_grid && !app.history_state.year_edit,
            "first popup title click must select years"
        );
        app.act(
            Action::History(HistoryAction::Calendar(CalendarAction::YearStep(-1))),
            window,
            cx,
        );
        anyhow::ensure!(
            app.history_state.year_grid
                && app.history_state.month_open
                && app.history_state.picker_year == year - 1,
            "year scrolling lost the picker stage"
        );
        app.history_year_title_clicked(true, window, cx);
        anyhow::ensure!(
            app.history_state.year_edit,
            "another single popup title click must enter year editing"
        );
        app.history_state
            .year_input
            .update(cx, |s, cx| s.set_value("2024", window, cx));
        Ok(before)
    })??;
    settle_layout(this, cx).await?;
    key(cx, "enter").await?;
    this.update_in(cx, |app, _, _| {
        anyhow::ensure!(
            !app.history_state.year_edit
                && !app.history_state.year_grid
                && app.history_state.month.year() == 2024
                && app.history_state.month_open,
            "editing the year did not return to month selection"
        );
        anyhow::ensure!(
            (app.query_id, scope(app), app.history_state.anchor) == saved,
            "browsing years changed query context"
        );
        Ok(())
    })??;
    key(cx, "escape").await?;
    this.update_in(cx, |app, window, cx| {
        app.act(
            Action::History(HistoryAction::Calendar(CalendarAction::View(
                CalendarView::Year,
            ))),
            window,
            cx,
        );
        anyhow::ensure!(
            app.history_state.annual_scope == AnnualScope::Year(2024),
            "annual view did not use the shared calendar year"
        );
        app.act(
            Action::History(HistoryAction::Calendar(CalendarAction::YearStep(1))),
            window,
            cx,
        );
        seed(app, false, false);
        Ok(())
    })??;
    settle_layout(this, cx).await?;
    this.update_in(cx, |app, window, cx| {
        app.history_state.annual_hover = Some(NaiveDate::from_ymd_opt(2025, 1, 1).unwrap());
        anyhow::ensure!(
            app.history_annual_hover(window).is_some(),
            "annual hover card was not available immediately"
        );
        app.act(
            Action::History(HistoryAction::Calendar(CalendarAction::ToggleMonth)),
            window,
            cx,
        );
        anyhow::ensure!(
            app.history_annual_hover(window).is_none(),
            "hover card covered the year picker"
        );
        app.act(
            Action::History(HistoryAction::Calendar(CalendarAction::ToggleMonth)),
            window,
            cx,
        );
        app.act(
            Action::History(HistoryAction::Calendar(CalendarAction::View(
                CalendarView::Month,
            ))),
            window,
            cx,
        );
        anyhow::ensure!(
            (app.query_id, scope(app), app.history_state.anchor) == saved,
            "annual year navigation changed the query"
        );
        seed(app, false, false);
        Ok(())
    })??;
    println!(
        "shared year picker passed: stages, scroll, edit/Enter, natural year, immediate hover and query preservation"
    );
    Ok(())
}
