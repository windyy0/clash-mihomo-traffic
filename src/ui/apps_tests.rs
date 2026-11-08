use super::activity_page::ActivityTab;
use super::*;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

pub static RUN: AtomicBool = AtomicBool::new(false);
static COMPARISON_ONLY: AtomicBool = AtomicBool::new(false);
static OUTCOME: Mutex<Option<Result<(), String>>> = Mutex::new(None);
#[path = "comparison_tests.rs"]
mod comparison_tests;
#[path = "drill_time_tests.rs"]
mod drill_time_tests;
#[path = "history_tests.rs"]
mod history_tests;
#[path = "speed_tests.rs"]
mod speed_tests;
#[path = "time_editor_tests.rs"]
mod time_editor_tests;

fn seed(app: &mut TrafficApp, offline: bool, empty: bool) {
    let mut data = Shared {
        connected: !offline,
        status: if offline {
            "等待连接".into()
        } else {
            "采集正常".into()
        },
        up: 20480.,
        down: 2_097_152.,
        ..Default::default()
    };
    let mut report = Report {
        total: Bytes {
            down: 5_368_709_120,
            up: 268_435_456,
        },
        observed: Bytes {
            down: 5_368_000_000,
            up: 268_000_000,
        },
        app_count: 61,
        row_count: 62,
        ..Default::default()
    };
    if !empty {
        for (index, name) in [
            "浏览器.exe",
            "同步服务.exe",
            "音乐.exe",
            "开发工具.exe",
            "idle.exe",
            "未知进程",
        ]
        .into_iter()
        .enumerate()
        {
            let key = format!(
                r"C:\应用程序 data\{}\{name}",
                "long-application-path-".repeat(8)
            );
            std::sync::Arc::make_mut(&mut data.connections).push(Connection {
                metadata: Metadata {
                    process_path: key.clone(),
                    ..Default::default()
                },
                ..Default::default()
            });
            if index < 2 {
                data.app_rates.insert(
                    key.clone(),
                    Bytes {
                        down: 100_000 * (index as u64 + 1),
                        up: 5000 * (6 - index as u64),
                    },
                );
            }

            report.rows.push(Row {
                key,
                bytes: Bytes {
                    down: 1_000_000 * (6 - index as u64),
                    up: 100_000 * (index as u64 + 1),
                },
            });
        }
        for index in 0..15 {
            let key = format!(r"D:\测试应用\helper-{index:02}.exe");
            std::sync::Arc::make_mut(&mut data.connections).push(Connection {
                metadata: Metadata {
                    process_path: key.clone(),
                    ..Default::default()
                },
                ..Default::default()
            });
            report.rows.push(Row {
                key,
                bytes: Bytes {
                    up: 50_000,
                    down: 500_000,
                },
            });
        }

        report.apps = report.rows.clone();
        report.row_count = report.rows.len();
        report.rows = report
            .rows
            .into_iter()
            .skip(app.query.page * app.query.page_size)
            .take(app.query.page_size)
            .collect();
    } else {
        report = Report::default();
    }
    let anchor = chrono::Utc::now().timestamp_millis();
    for index in 0..300 {
        data.live.push_back(LiveSample {
            from: anchor - (300 - index) * 1000,
            to: anchor - (299 - index) * 1000,
            down: ((index % 17) as f64 + 1.) * 18000.,
            up: (index % 7) as f64 * 8000.,
            valid: true,
        });
    }
    report.query = Some(app.query.clone());
    let mut detail = report.clone();
    detail.query = Some(Query {
        app: app
            .active_detail_context()
            .map(|context| context.key)
            .unwrap_or_else(|| {
                report
                    .rows
                    .first()
                    .map(|row| row.key.clone())
                    .unwrap_or_default()
            }),
        group_apps: app
            .active_detail_context()
            .is_none_or(|context| context.group_apps),
        dimension: app.detail_dimension.into(),
        sort: TrafficSort::Total,
        ascending: false,
        page_size: 50,
        ..app.query.clone()
    });
    detail.rows = (0..9)
        .map(|index| Row {
            key: format!("{}-{index}.example.com", "a-very-long-domain-".repeat(10)),
            bytes: Bytes {
                down: (10 - index) * 1_000_000,
                up: 1000,
            },
        })
        .collect();
    detail.domains = detail.rows.clone();
    detail.routes = vec![Row {
        key: "DIRECT".into(),
        bytes: detail.total,
    }];
    detail.sources = vec![Row {
        key: "192.0.2.1".into(),
        bytes: detail.total,
    }];
    detail.series = data
        .live
        .iter()
        .map(|sample| SeriesPoint {
            time: sample.to,
            down: sample.down,
            up: sample.up,
        })
        .collect();
    let mut histogram =
        crate::app_trend::Builder::new(app.query.from, app.query.to.min(anchor), app.query.to);
    for point in &detail.series {
        histogram
            .add(
                point.time,
                Bytes {
                    up: point.up as u64,
                    down: point.down as u64,
                },
            )
            .unwrap();
    }
    detail.histogram = histogram.finish().unwrap();

    detail.archive_daily = app.period == TimePeriod::LastThreeMonths;
    if app.page == Page::History {
        let now = chrono::Utc::now().timestamp_millis();
        let (from, to) = app
            .query
            .history_context
            .unwrap_or((app.query.from, app.query.to));
        let mut trend = crate::history::fixture(from, to, now);
        if empty {
            for bucket in &mut trend.buckets {
                bucket.observed = false;
                bucket.bytes = Bytes::default();
                bucket.covered_ms = 0;
            }
        }
        report.total = if empty {
            Bytes::default()
        } else {
            crate::history::fixture(app.query.from, app.query.to, now).total()
        };
        report.history = Some(std::sync::Arc::new(trend));
        let month = app.history_state.month;
        use chrono::Datelike;
        let request = crate::history::CalendarRequest {
            id: 0,
            generation: data.storage_revision,
            year: month.year(),
            month: month.month(),
            as_of: now,
        };
        let (a, b) = crate::history::month_bounds(request.year, request.month).unwrap();
        let mut days = crate::history::fixture(a, b, now).buckets;
        if let Some(day) = days.get_mut(4) {
            day.observed = false;
            day.covered_ms = 0;
            day.bytes = Bytes::default();
        }
        if let Some(day) = days.get_mut(7) {
            day.bytes = Bytes::default();
        }
        if let Some(day) = days.get_mut(17) {
            day.covered_ms /= 2;
        }
        app.history_state.source_revision = data.storage_revision;
        app.history_state.month_pending = None;
        app.history_state.month_error.clear();
        app.history_state.month_retry = None;
        app.history_state.cache.clear();
        app.history_state
            .cache
            .push_front(std::sync::Arc::new(crate::history::CalendarMonth {
                request,
                days,
            }));
        if app.history_state.view == super::history_state::CalendarView::Year {
            let months = app
                .history_state
                .calendar_months(chrono::Local::now().date_naive());
            app.history_state.cache.retain(|m| {
                months
                    .iter()
                    .any(|d| d.year() == m.request.year && d.month() == m.request.month)
            });
            for month in months {
                if app.history_state.cached_month(month).is_some() {
                    continue;
                }
                let (from, to) = crate::history::month_bounds(month.year(), month.month()).unwrap();
                let request = crate::history::CalendarRequest {
                    id: 0,
                    generation: data.storage_revision,
                    year: month.year(),
                    month: month.month(),
                    as_of: chrono::Utc::now().timestamp_millis(),
                };
                let trend = crate::history::fixture(from, to, request.as_of);
                app.history_state.cache.push_back(std::sync::Arc::new(
                    crate::history::CalendarMonth {
                        request,
                        days: trend.buckets,
                    },
                ));
            }
        }
    }
    report.row_speeds = report
        .rows
        .iter()
        .map(|r| {
            (
                r.key.clone(),
                crate::speed_stats::SpeedStats {
                    bytes: r.bytes,
                    covered_ms: 180_000,
                    expected_ms: 360_000,
                    first: Some(app.query.from),
                    as_of: Some(app.query.from + 360_000),
                    down_peak: Some(crate::speed_stats::Peak {
                        rate: r.bytes.down as f64 / 2.,
                        at: app.query.from + 1000,
                    }),
                    up_peak: Some(crate::speed_stats::Peak {
                        rate: r.bytes.up as f64 / 3.,
                        at: app.query.from + 2000,
                    }),
                },
            )
        })
        .collect();
    data.report = report.clone().into();
    data.report_id = app.query_id;
    data.detail = detail.clone().into();
    data.detail_id = app.detail_id;
    *app.backend.shared.lock().unwrap() = data.clone();
    app.data = data;
    app.report = report;
    app.detail = detail;
}

async fn settle_layout(
    this: &WeakEntity<TrafficApp>,
    cx: &mut AsyncWindowContext,
) -> anyhow::Result<()> {
    for _ in 0..3 {
        let painted = Rc::new(Cell::new(false));
        let flag = painted.clone();
        this.update_in(cx, |_, window, cx| {
            window.activate_window();
            window.on_next_frame(move |_, _| flag.set(true));
            cx.notify();
        })?;
        let deadline = Instant::now() + Duration::from_secs(5);
        while !painted.get() {
            Timer::after(Duration::from_millis(20)).await;
            anyhow::ensure!(
                Instant::now() < deadline,
                "window did not produce a layout frame"
            );
        }
    }
    Ok(())
}

async fn exercise_usage_detail(
    this: &WeakEntity<TrafficApp>,
    cx: &mut AsyncWindowContext,
) -> anyhow::Result<()> {
    for theme in ["light", "dark"] {
        for (width, docked) in [(1240., true), (900., false)] {
            this.update_in(cx, |app, window, cx| {
                window.resize(size(px(width), px(700.)));
                app.act(Action::Theme(theme), window, cx);
                app.act(Action::Nav(Page::Apps), window, cx);
                seed(app, false, false);
                let main_id = app.query_id;
                let key = app.report.rows[0].key.clone();
                app.act(
                    Action::Application(ApplicationAction::OpenUsageDetail(key)),
                    window,
                    cx,
                );
                anyhow::ensure!(
                    app.selected.is_empty() && app.usage_detail_open(),
                    "usage click entered the full detail page"
                );
                anyhow::ensure!(
                    app.query_id == main_id && app.app_usage_anchor_pending,
                    "opening detail refreshed or failed to anchor the list"
                );
                seed(app, false, false);
                Ok::<_, anyhow::Error>(())
            })??;
            // The pane can start below the viewport. Wait for the anchor frame
            // and its subsequent paint before reading canvas bounds.
            for _ in 0..50 {
                Timer::after(Duration::from_millis(100)).await;
                if this.update_in(cx, |app, _, _| {
                    !app.app_usage_anchor_pending
                        && app.usage_detail.bounds.get().size.width > px(0.)
                })? {
                    break;
                }
            }
            settle_layout(this, cx).await?;
            this.update_in(cx, |app, window, cx| {
                anyhow::ensure!(
                    app.usage_detail_docked(window) == docked,
                    "wrong usage pane presentation at {width}"
                );
                anyhow::ensure!(
                    (app.usage_detail.bounds.get().size.width - px(420.)).abs() <= px(2.),
                    "pane width changed at {width}/{theme}: {:?}; open={} details={} analysis={} scroll={:?} offset={:?} table={:?}",
                    app.usage_detail.bounds.get(), app.usage_detail_open(), app.activity.details, app.analysis.active, app.page_scroll.bounds(), app.page_scroll.offset(), app.app_usage_bounds.get()
                );
                anyhow::ensure!(
                    app.page_scroll.max_offset().width <= px(1.)
                        && app.usage_detail.scroll.max_offset().width <= px(1.),
                    "usage pane overflowed horizontally: page={:?} detail={:?}",
                    app.page_scroll.max_offset(),
                    app.usage_detail.scroll.max_offset()
                );
                anyhow::ensure!(
                    app.usage_detail.scroll.max_offset().height > px(0.),
                    "long detail cannot scroll"
                );
                anyhow::ensure!(
                    app.usage_detail.scroll.max_offset().width <= px(1.),
                    "combined trend overflows the application detail pane: {theme} {width}"
                );
                let left = app.page_scroll.offset();
                app.usage_detail.scroll.set_offset(point(px(0.), px(-90.)));
                let key = app.usage_detail.key.clone().unwrap();
                let id = app.detail_id;
                let previous = app.detail.clone();
                app.act(Action::Application(ApplicationAction::ToggleApplicationPath), window, cx);
                anyhow::ensure!(app.expanded_application_path.as_ref() == Some(&key) && app.detail_id == id, "expanding the path reloaded detail or expanded another application");
                app.act(Action::Application(ApplicationAction::OpenUsageDetail(key.clone())), window, cx);
                anyhow::ensure!(
                    app.detail_id == id && app.usage_detail.scroll.offset().y == px(-90.) && app.expanded_application_path.as_ref() == Some(&key),
                    "repeated click reset the detail"
                );
                app.act(
                    Action::Application(ApplicationAction::OpenUsageDetail(app.report.rows[1].key.clone())),
                    window,
                    cx,
                );
                {
                    let mut shared = app.backend.shared.lock().unwrap();
                    shared.detail = previous.into();
                    shared.detail_id = id;
                }
                app.poll_query_results(cx);
                anyhow::ensure!(
                    app.detail.query.as_ref().is_none_or(
                        |query| Some(query.app.as_str()) == app.usage_detail.key.as_deref()
                    ),
                    "a late result replaced the selected application"
                );
                anyhow::ensure!(
                    app.page_scroll.offset() == left
                        && app.usage_detail.scroll.offset().y.is_zero()
                        && !app.app_usage_anchor_pending
                        && app.expanded_application_path.is_none(),
                    "switching applications changed context: before={left:?}, after={:?}, detail={:?}, pending={}, expanded={:?}", app.page_scroll.offset(), app.usage_detail.scroll.offset(), app.app_usage_anchor_pending, app.expanded_application_path
                );
                seed(app, false, false);
                app.detail_cache.remember(&app.detail);
                let selected = app.usage_detail.key.clone();
                let detail_id = app.detail_id;
                let main_id = app.query_id;
                app.pending_query = None;
                app.refresh_main_query();
                anyhow::ensure!(
                    app.query_id > main_id && app.detail_id == detail_id,
                    "list refresh replaced detail request"
                );
                seed(app, false, false);
                app.poll_query_results(cx);
                anyhow::ensure!(
                    app.received_report_id == app.query_id && app.received_detail_id == detail_id,
                    "the two query results did not complete independently"
                );
                app.act(Action::Application(ApplicationAction::AppSort(TrafficSort::Download)), window, cx);
                app.act(Action::Next(1), window, cx);
                anyhow::ensure!(
                    app.usage_detail.key == selected
                        && app.unsent_detail_query.is_none()
                        && app.pending_detail_query.is_none(),
                    "sorting or paging reloaded/closed the selected detail"
                );
                let old = app.query.from;
                app.query.from += 60_000;
                app.refresh();
                anyhow::ensure!(
                    app.usage_detail.key == selected && app.detail.query.is_none(),
                    "date change reused another range"
                );
                app.query.from = old;
                app.act(Action::Application(ApplicationAction::CloseUsageDetail), window, cx);
                let list_id = app.query_id;
                let offset = app.page_scroll.offset();
                app.poll_query_results(cx);
                anyhow::ensure!(
                    !app.usage_detail_open()
                        && app.query_id == list_id
                        && app.page_scroll.offset() == offset,
                    "closing or a late result disturbed the list"
                );
                app.act(Action::Application(ApplicationAction::OpenUsageDetail(selected.unwrap())), window, cx);
                anyhow::ensure!(
                    app.detail.query.is_some(),
                    "cached usage detail was cleared"
                );
                app.act(Action::ActivityDetails(ActivityTab::Current), window, cx);
                anyhow::ensure!(
                    app.usage_detail.key.is_none(),
                    "leaving the usage page retained the pane"
                );
                app.act(Action::CloseActivityDetails, window, cx);
                app.act(
                    Action::Application(ApplicationAction::OpenUsageDetail(app.report.rows[0].key.clone())),
                    window,
                    cx,
                );
                app.toggle_picker(
                    "retention-time-unit",
                    vec![(
                        "天".into(),
                        Action::Storage(StorageAction::RetentionTimeUnit(RetentionTimeUnit::Days)),
                        true,
                    )],
                    window,
                    cx,
                );
                Ok::<_, anyhow::Error>(())
            })??;
            cx.update(|window, cx| {
                window.dispatch_keystroke(Keystroke::parse("escape").unwrap(), cx);
            })?;
            this.update_in(cx, |app, _, _| {
                anyhow::ensure!(
                    app.picker.is_none() && app.usage_detail_open(),
                    "Escape did not close the menu before the pane"
                );
                Ok::<_, anyhow::Error>(())
            })??;
            cx.update(|window, cx| {
                window.dispatch_keystroke(Keystroke::parse("escape").unwrap(), cx);
            })?;
            this.update_in(cx, |app, window, cx| {
                anyhow::ensure!(
                    !app.usage_detail_open(),
                    "Escape did not close the usage pane"
                );
                let key = app.report.rows[0].key.clone();
                app.act(
                    Action::Application(ApplicationAction::OpenUsageDetail(key.clone())),
                    window,
                    cx,
                );
                app.act(Action::AppConnections, window, cx);
                anyhow::ensure!(
                    app.page == Page::Connections
                        && app.activity.connection_group.as_ref() == Some(&key)
                        && app.usage_detail.key.is_none(),
                    "pane connection action lost the selected application"
                );
                app.act(Action::Nav(Page::Apps), window, cx);
                let list_id = app.query_id;
                app.act(
                    Action::Application(ApplicationAction::OpenUsageDetail("未知进程".into())),
                    window,
                    cx,
                );
                anyhow::ensure!(
                    app.query_id == list_id
                        && app
                            .application_detail_query()
                            .is_some_and(|query| query.app == "未知进程" && query.group_apps),
                    "unknown-process shortcut did not scope the detail query"
                );
                let policy = app.config.process_control;
                app.act(Action::OpenProcessSettings, window, cx);
                anyhow::ensure!(
                    app.page == Page::Settings
                        && app.settings_section == SettingsSection::Connection
                        && app.usage_detail.key.is_none()
                        && app.config.process_control == policy,
                    "process-settings shortcut did not navigate safely"
                );
                app.act(Action::Nav(Page::Apps), window, cx);
                Ok::<_, anyhow::Error>(())
            })??;
        }
    }
    Ok(())
}

pub fn schedule(window: &mut Window, cx: &mut Context<TrafficApp>) {
    cx.spawn_in(window, async move |this, cx| {
        let result: anyhow::Result<()> = async {
            if COMPARISON_ONLY.load(Ordering::Acquire) {
                return comparison_tests::exercise(&this, cx).await;
            }
            if std::env::var_os("MIHOMO_HISTORY_ONLY").is_some(){return history_tests::exercise(&this,cx).await;}
            if std::env::var_os("MIHOMO_HISTORY_YEAR_ONLY").is_some(){return history_tests::exercise_dual_view(&this,cx).await;}
            if std::env::var_os("MIHOMO_SPEED_ONLY").is_some(){return speed_tests::exercise(&this,cx).await;}
            this.update_in(cx, |app, window, cx| {
                anyhow::ensure!(
                    app.tick == 0 && !native::visible(app.float_hwnd),
                    "disabled floating window flashed before the first visibility update"
                );
                app.act(Action::Nav(Page::Apps), window, cx);
                anyhow::ensure!(
                    app.query.page_size == 10,
                    "apps must use ten items per page"
                );
                app.act(Action::Period(TimePeriod::LastThreeMonths), window, cx);
                anyhow::ensure!(
                    Some((app.query.from, app.query.to))
                        == TimePeriod::LastThreeMonths.bounds(chrono::Local::now())?,
                    "three-month preset did not apply"
                );
                app.act(Action::Period(TimePeriod::Custom), window, cx);
                anyhow::ensure!(
                    !app.history_state.time_parts[0].read(cx).value().is_empty(),
                    "custom range was not synchronized"
                );
                app.query.page = 2;
                app.act(Action::Application(ApplicationAction::AppSort(TrafficSort::Upload)), window, cx);
                anyhow::ensure!(
                    app.query.sort == TrafficSort::Upload && app.query.page == 0,
                    "historical sort did not reset pagination"
                );
                app.act(Action::ActivityDetails(ActivityTab::Current), window, cx);
                anyhow::ensure!(app.activity.details, "view-all navigation failed");
                app.act(Action::CloseActivityDetails, window, cx);
                app.query.page = 1;
                app.page_scroll.set_offset(point(px(0.), px(-200.)));
                app.act(Action::Application(ApplicationAction::Select("test.exe".into())), window, cx);
                anyhow::ensure!(
                    app.page_scroll.offset().y == px(0.),
                    "detail did not start at the top"
                );
                app.act(Action::Application(ApplicationAction::ClearAppSelection), window, cx);
                anyhow::ensure!(
                    app.page_scroll.offset().y == px(-200.),
                    "back navigation lost list position"
                );
                anyhow::ensure!(
                    app.selected.is_empty() && app.query.page == 1,
                    "back navigation lost list pagination"
                );
                Ok(())
            })??;
            this.update_in(cx, |app, window, cx| {
                seed(app, false, false);
                if let Some(previous) = cx.read_from_clipboard() {
                    let result = (|| -> anyhow::Result<()> {
                        let key = app.report.rows[0].key.clone();
                        let selected = app.selected.clone();
                        app.data.identities.insert(key.clone(), crate::attribution::Owner { key:key.clone(),label:"示例应用".into(),icon:key.clone() });
                        app.act(Action::Application(ApplicationAction::CopyApplicationName(key.clone())),window,cx);
                        anyhow::ensure!(cx.read_from_clipboard().and_then(|item|item.text()).as_deref()==Some("示例应用"), "copy name did not use the displayed application name");
                        anyhow::ensure!(app.selected==selected && app.page==Page::Apps, "copy navigated away from the list");
                        app.act(Action::Application(ApplicationAction::CopyApplicationPath(key.clone())),window,cx);
                        anyhow::ensure!(cx.read_from_clipboard().and_then(|item|item.text()).as_deref()==Some(key.as_str()), "copy path did not preserve the complete Unicode path");
                        Ok(())
                    })();
                    cx.write_to_clipboard(previous);
                    result?;
                }
                Ok::<_,anyhow::Error>(())
            })??;
            this.update_in(cx, |app, window, cx| {
                seed(app,false,false);
                let key=app.report.rows[0].key.clone();
                app.act(Action::Application(ApplicationAction::Select(key.clone())),window,cx);
                seed(app,false,false);
                app.detail_cache.remember(&app.detail);
                let total=app.detail.total;
                let detail_id=app.detail_id;
                app.act(Action::Application(ApplicationAction::DetailDimension("route")),window,cx);
                anyhow::ensure!(app.detail_id==detail_id && app.detail.routes[0].key=="DIRECT", "switching destinations issued another trend query");
                app.act(Action::Application(ApplicationAction::DetailDimension("source")),window,cx);
                anyhow::ensure!(app.detail_id==detail_id && !app.detail.sources.is_empty(), "source results were not loaded with the detail snapshot");
                app.act(Action::Application(ApplicationAction::ClearAppSelection),window,cx);
                app.act(Action::Application(ApplicationAction::Select(key)),window,cx);
                anyhow::ensure!(app.detail.query.is_some() && app.detail.total==total && app.received_detail_id==app.detail_id, "reentering a cached application cleared its data");
                let original=app.query.from;app.query.from+=60_000;app.refresh();
                anyhow::ensure!(app.detail.query.is_none(), "another time range reused the previous snapshot");
                app.query.from=original;
                app.act(Action::Application(ApplicationAction::DetailDimension("domain")),window,cx);
                app.act(Action::Application(ApplicationAction::ClearAppSelection),window,cx);
                Ok::<_,anyhow::Error>(())
            })??;
            exercise_usage_detail(&this, cx).await?;
            speed_tests::exercise(&this,cx).await?;
            for theme in ["light", "dark"] {
                for width in [1240., 900.] {
                    for state in [0, 1, 2, 3, 4, 5] {
                        this.update_in(cx, |app, window, cx| {
                            window.resize(size(px(width), px(700.)));
                            app.act(Action::Theme(theme), window, cx);
                            app.act(Action::Nav(Page::Apps), window, cx);
                            app.act(
                                Action::Period(if state == 3 {
                                    TimePeriod::Custom
                                } else {
                                    TimePeriod::LastThreeMonths
                                }),
                                window,
                                cx,
                            );
                            seed(app, state == 2, state == 2);
                            if state == 0 {
                                let activity = crate::app_activity::activity(&app.data);
                                use crate::app_activity::ActivityState::{Idle, Transferring};
                                anyhow::ensure!(
                                    activity.len() == 21,
                                    "activity omitted recent or idle apps"
                                );
                                anyhow::ensure!(
                                    activity
                                        .iter()
                                        .take(4)
                                        .map(|app| app.state())
                                        .collect::<Vec<_>>()
                                        == [Transferring, Transferring, Idle, Idle],
                                    "default activity slots did not follow group priority"
                                );
                                anyhow::ensure!(
                                    activity.last().unwrap().state() == Idle,
                                    "idle app was not last"
                                );
                            }
                            app.activity.preview = if state == 5 {
                                ActivityTab::Recent
                            } else {
                                ActivityTab::Current
                            };
                            app.activity.recent_snapshot =
                                Some(std::sync::Arc::new(crate::analysis::Snapshot::fixture(
                                    crate::analysis::AnalysisRequest::new(
                                        1,
                                        0,
                                        crate::analysis::AnalysisRange::FiveMinutes,
                                        chrono::Utc::now().timestamp_millis(),
                                    ),
                                )));
                            if state == 4 {
                                app.report.query = None;
                                std::sync::Arc::make_mut(&mut app.backend.shared.lock().unwrap().report).query = None;
                            }
                            if state == 1 {
                                let key = app.report.rows[0].key.clone();
                                app.act(Action::Application(ApplicationAction::Select(key)), window, cx);
                                seed(app, false, false);
                                let total = app.detail.total;
                                let child = Bytes { up: total.up / 2, down: total.down / 2 };
                                app.detail.members = vec![Row { key: app.selected.clone(), bytes: Bytes { up: total.up - child.up, down: total.down - child.down } }, Row { key: r"C:\Program Files\Microsoft\EdgeWebView\Application\msedgewebview2.exe".into(), bytes: child }];
                                app.act(Action::Application(ApplicationAction::ToggleApplicationProcesses), window, cx);
                                app.data.detail = app.detail.clone().into();
                                app.backend.shared.lock().unwrap().detail = app.data.detail.clone();
                            }
                            app.page_scroll.set_offset(point(px(0.), px(0.)));
                            cx.notify();
                            Ok(())
                        })??;
                        settle_layout(&this, cx).await?;
                        this.update_in(cx, |app, _, _| {
                            anyhow::ensure!(
                                app.p.dark == (theme == "dark"),
                                "appearance did not remain selected: {theme}"
                            );
                            anyhow::ensure!(
                                app.page_scroll.bounds().size.height > px(0.),
                                "page collapsed"
                            );
                            anyhow::ensure!(
                                app.page_scroll.max_offset().width <= px(1.),
                                "apps horizontal overflow: {theme} {width} state={state}: {:?}",
                                app.page_scroll.max_offset()
                            );
                            if state != 1 {
                                let caption = app.activity.preview_caption_bounds.get();
                                let expected = if width == 900. { 275. } else { 310. };
                                anyhow::ensure!(
                                    caption.size.width >= px(expected - 1.),
                                    "preview caption collapsed to ellipsis: {theme} {width} state={state}, bounds={caption:?}"
                                );
                            }
                            if matches!(state, 0 | 3 | 5) {
                                anyhow::ensure!(
                                    app.page_scroll.max_offset().height > px(0.),
                                    "long page must overflow"
                                );
                            }
                            anyhow::ensure!(
                                app.report.rows.len() <= 10,
                                "apps rendered too many rows"
                            );
                            Ok(())
                        })??;
                    }
                    println!("application layouts passed: {theme} {width}x700");
                }
            }
            this.update_in(cx, |app, window, cx| {
                app.act(Action::Nav(Page::History), window, cx);
                anyhow::ensure!(app.query.page_size == 10, "history must keep ten items");
                app.act(Action::Nav(Page::Apps), window, cx);
                app.activity.preview = ActivityTab::Current;
                seed(app, false, false);
                cx.notify();
                Ok(())
            })??;
            settle_layout(&this, cx).await?;
            let height = this.update_in(cx, |app, window, cx| {
                let height = app.app_usage_bounds.get().size.height;
                app.page_scroll
                    .set_offset(point(px(0.), -app.page_scroll.max_offset().height));
                app.act(Action::Next(1), window, cx);
                anyhow::ensure!(
                    app.app_usage_anchor_pending,
                    "pagination should request a usage anchor"
                );
                Ok(height)
            })??;
            settle_layout(&this, cx).await?;
            this.update_in(cx, |app, _, cx| {
                anyhow::ensure!(
                    app.app_usage_bounds.get().size.height >= height - px(2.),
                    "loading collapsed the previous table"
                );
                seed(app, false, false);
                anyhow::ensure!(
                    app.report.rows.len() == 10,
                    "second page should contain ten items"
                );
                cx.notify();
                Ok(())
            })??;
            let anchor_deadline = Instant::now() + Duration::from_secs(3);
            loop {
                Timer::after(Duration::from_millis(50)).await;
                let anchored = this.update_in(cx, |app, _, _| (app.app_usage_bounds.get().top() - app.page_scroll.bounds().top() - px(16.)).abs() <= px(3.))?;
                if anchored || Instant::now() >= anchor_deadline { break; }
            }
            this.update_in(cx, |app, window, cx| {
                let top = app.app_usage_bounds.get().top();
                let expected = app.page_scroll.bounds().top() + px(16.);
                anyhow::ensure!(
                    (top - expected).abs() <= px(3.),
                    "pagination failed to anchor usage: {top:?} expected {expected:?}"
                );
                app.act(Action::Next(1), window, cx);
                seed(app, false, false);
                anyhow::ensure!(
                    app.report.rows.len() == 1 && app.report.row_count == 21,
                    "last page should contain one item"
                );
                app.page_number.update(cx, |input, cx| input.set_value("4", window, cx));
                app.act(Action::JumpToPage, window, cx);
                anyhow::ensure!(app.query.page == 2, "out-of-range jump changed the page");
                app.page_number.update(cx, |input, cx| input.set_value("1", window, cx));
                app.act(Action::JumpToPage, window, cx);
                anyhow::ensure!(
                    app.query.page == 0 && app.displayed_page() == Some(2)
                        && app.report.rows.len() == 1,
                    "jump should keep the previous rows and their page until the result arrives"
                );
                seed(app, false, false);
                anyhow::ensure!(
                    app.displayed_page() == Some(0) && app.report.rows.len() == 10,
                    "jump did not replace the previous rows"
                );
                Ok(())
            })??;
            println!("application pagination and loading anchors passed");
            this.update_in(cx, |app, window, cx| {
                use crate::analysis::{AnalysisRange, Snapshot};
                use std::sync::Arc;
                app.act(Action::Nav(Page::Apps), window, cx);
                seed(app, false, false);
                let request = app.next_snapshot_request(AnalysisRange::FiveMinutes);
                let frozen = Arc::new(Snapshot::fixture(request.clone()));
                app.activity.recent_snapshot = Some(frozen.clone());
                app.act(Action::ActivityPreview(ActivityTab::Recent), window, cx);
                app.page_scroll.set_offset(point(px(0.), px(-180.)));
                app.act(Action::ActivityDetails(ActivityTab::Recent), window, cx);
                anyhow::ensure!(
                    app.activity.details
                        && app.activity.tab == ActivityTab::Recent
                        && !app.analysis.active,
                    "recent preview did not open matching secondary tab"
                );
                anyhow::ensure!(
                    Arc::ptr_eq(app.activity.recent_snapshot.as_ref().unwrap(), &frozen)
                        && app.activity.recent_pending.is_none(),
                    "opening view-all refreshed recent usage"
                );
                let current_count = frozen
                    .rows
                    .iter()
                    .filter(|row| row.metrics.current.total() > 0)
                    .count();
                let bar_maximum = app.recent_usage_maximum();
                anyhow::ensure!(
                    app.recent_usage_rows(false).len() == current_count,
                    "recent list included previous-only apps"
                );
                anyhow::ensure!(
                    app.recent_usage_rows(false).windows(2).all(|pair| pair[0]
                        .metrics
                        .current
                        .total()
                        >= pair[1].metrics.current.total()),
                    "recent rows not ordered by current volume"
                );
                app.act(Action::ActivityPage(1), window, cx);
                app.page_scroll.set_offset(point(px(0.), px(-120.)));
                app.act(Action::ActivityDetails(ActivityTab::Current), window, cx);
                app.act(Action::ActivityPage(1), window, cx);
                app.act(Action::ActivityPage(1), window, cx);
                anyhow::ensure!(
                    app.activity.live_page == 2,
                    "current transfer pagination failed"
                );
                app.activity
                    .live_search
                    .update(cx, |input, cx| input.set_value("HELPER", window, cx));
                app.poll_recent_usage(cx);
                anyhow::ensure!(
                    app.activity.live_page == 0,
                    "live search did not reset pagination"
                );
                app.act(Action::ActivityPage(2), window, cx);
                anyhow::ensure!(
                    app.activity.live_page == 1,
                    "live search did not limit pagination to matching applications"
                );
                app.activity
                    .live_search
                    .update(cx, |input, cx| input.set_value("", window, cx));
                app.poll_recent_usage(cx);
                app.act(Action::ActivityPage(2), window, cx);
                app.act(Action::Application(ApplicationAction::Select("test.exe".into())), window, cx);
                app.act(Action::Application(ApplicationAction::ClearAppSelection), window, cx);
                anyhow::ensure!(
                    app.activity.details
                        && app.activity.tab == ActivityTab::Current
                        && app.activity.live_page == 2,
                    "single-app back navigation lost activity parent"
                );
                app.act(Action::ActivityDetails(ActivityTab::Recent), window, cx);
                anyhow::ensure!(
                    app.activity.recent_page == 1 && app.page_scroll.offset().y == px(-120.),
                    "recent tab context was not restored"
                );
                app.activity
                    .recent_search
                    .update(cx, |input, cx| input.set_value("应用-20", window, cx));
                app.poll_recent_usage(cx);
                anyhow::ensure!(
                    app.activity.recent_page == 0
                        && app.recent_usage_rows(true).len() == 1
                        && app.recent_usage_rows(false).len() == current_count,
                    "recent search changed global snapshot or failed to reset page"
                );
                anyhow::ensure!(
                    app.recent_usage_maximum() == bar_maximum
                        && (app.recent_usage_rows(true)[0].metrics.current.total() as f64)
                            < bar_maximum,
                    "search rescaled usage bars to the filtered application"
                );
                app.activity
                    .recent_search
                    .update(cx, |input, cx| input.set_value("", window, cx));
                app.poll_recent_usage(cx);
                app.act(Action::CloseActivityDetails, window, cx);
                anyhow::ensure!(
                    !app.activity.details
                        && app.activity.preview == ActivityTab::Recent
                        && app.page_scroll.offset().y == px(-180.),
                    "activity back lost dashboard context"
                );
                let analysis_request = app.next_snapshot_request(AnalysisRange::Hour);
                app.analysis.pending = Some(analysis_request.clone());
                app.analysis.range = AnalysisRange::Hour;
                app.analysis.snapshot = Some(frozen.clone());
                app.request_recent_usage();
                anyhow::ensure!(
                    app.analysis.pending.is_none()
                        && app.analysis.range == AnalysisRange::FiveMinutes,
                    "superseded analysis request stayed pending or left inconsistent range"
                );
                let refresh = app.next_snapshot_request(AnalysisRange::FiveMinutes);
                anyhow::ensure!(
                    refresh.id > analysis_request.id && refresh.id > request.id,
                    "snapshot request IDs collided"
                );
                app.activity.recent_pending = Some(refresh.clone());
                *app.backend.analysis_result.lock().unwrap() =
                    Some(crate::backend::AnalysisReply {
                        complete: true,
request: analysis_request,
                        result: Ok(frozen.clone()),
                    });
                app.poll_recent_usage(cx);
                anyhow::ensure!(
                    app.activity.recent_pending.as_ref() == Some(&refresh),
                    "stale analysis reply replaced pending recent snapshot"
                );
                *app.backend.analysis_result.lock().unwrap() =
                    Some(crate::backend::AnalysisReply {
                        complete: true,
request: refresh.clone(),
                        result: Err("test refresh error".into()),
                    });
                app.poll_recent_usage(cx);
                anyhow::ensure!(
                    Arc::ptr_eq(app.activity.recent_snapshot.as_ref().unwrap(), &frozen)
                        && app.activity.recent_pending.is_none()
                        && !app.activity.recent_error.is_empty(),
                    "failed refresh discarded recent snapshot"
                );
                let updated = Arc::new(Snapshot::fixture(refresh.clone()));
                app.activity.recent_pending = Some(refresh.clone());
                *app.backend.analysis_result.lock().unwrap() =
                    Some(crate::backend::AnalysisReply {
                        complete: true,
request: refresh,
                        result: Ok(updated.clone()),
                    });
                app.poll_recent_usage(cx);
                anyhow::ensure!(
                    Arc::ptr_eq(app.activity.recent_snapshot.as_ref().unwrap(), &updated)
                        && app.activity.recent_error.is_empty()
                        && Arc::ptr_eq(app.analysis.snapshot.as_ref().unwrap(), &frozen),
                    "recent refresh changed analysis snapshot"
                );
                app.request_analysis(AnalysisRange::Hour);
                anyhow::ensure!(
                    app.activity.recent_pending.is_none()
                        && Arc::ptr_eq(app.activity.recent_snapshot.as_ref().unwrap(), &updated),
                    "analysis refresh replaced recent snapshot"
                );
                app.analysis.pending = None;
                app.analysis.error.clear();
                *app.backend.analysis_result.lock().unwrap() = None;
                Ok(())
            })??;
            println!("activity navigation, paging, shared snapshot and request isolation passed");
            this.update_in(cx, |app, _, cx| {
                use crate::analysis::{AnalysisRange, Snapshot};
                use std::sync::Arc;
                let recent = app.activity.recent_snapshot.clone();
                let analysis = app.analysis.snapshot.clone();
                let request = app.next_snapshot_request(AnalysisRange::FiveMinutes);
                let preview = Arc::new(Snapshot::fixture(request.clone()));
                app.activity.recent_snapshot = None;
                app.activity.recent_provisional = false;
                app.activity.recent_pending = Some(request.clone());
                *app.backend.analysis_result.lock().unwrap() = Some(crate::backend::AnalysisReply { request: request.clone(), complete: false, result: Ok(preview.clone()) });
                app.poll_recent_usage(cx);
                anyhow::ensure!(app.activity.recent_provisional && app.activity.recent_pending.as_ref() == Some(&request) && Arc::ptr_eq(app.activity.recent_snapshot.as_ref().unwrap(), &preview), "initial recent preview did not render while sampling continued");
                let final_snapshot = Arc::new(Snapshot::fixture(request.clone()));
                *app.backend.analysis_result.lock().unwrap() = Some(crate::backend::AnalysisReply { request: request.clone(), complete: true, result: Ok(final_snapshot.clone()) });
                app.poll_recent_usage(cx);
                anyhow::ensure!(!app.activity.recent_provisional && app.activity.recent_pending.is_none() && Arc::ptr_eq(app.activity.recent_snapshot.as_ref().unwrap(), &final_snapshot), "recent preview was not replaced by the completed snapshot");
                let refresh = app.next_snapshot_request(AnalysisRange::FiveMinutes);
                app.activity.recent_pending = Some(refresh.clone());
                *app.backend.analysis_result.lock().unwrap() = Some(crate::backend::AnalysisReply { request: refresh.clone(), complete: false, result: Ok(Arc::new(Snapshot::fixture(refresh.clone()))) });
                app.poll_recent_usage(cx);
                anyhow::ensure!(!app.activity.recent_provisional && Arc::ptr_eq(app.activity.recent_snapshot.as_ref().unwrap(), &final_snapshot) && app.activity.recent_pending.is_some(), "background preview replaced the existing completed recent snapshot");
                app.activity.recent_pending = None;
                app.activity.recent_snapshot = recent;

                app.analysis.snapshot = None;
                app.analysis.provisional = false;
                app.analysis.pending = Some(request.clone());
                *app.backend.analysis_result.lock().unwrap() = Some(crate::backend::AnalysisReply { request: request.clone(), complete: false, result: Ok(preview.clone()) });
                app.poll_analysis(cx);
                anyhow::ensure!(app.analysis.provisional && app.analysis.pending.as_ref() == Some(&request) && Arc::ptr_eq(app.analysis.snapshot.as_ref().unwrap(), &preview), "initial analysis preview did not render while sampling continued");
                app.analysis.pinned_bucket = Some(request.anchor - 3000);
                app.poll_analysis(cx);
                anyhow::ensure!(app.analysis.pinned_bucket.is_some(), "re-reading the same preliminary reply reset chart interaction");
                *app.backend.analysis_result.lock().unwrap() = Some(crate::backend::AnalysisReply { request: request.clone(), complete: true, result: Ok(final_snapshot.clone()) });
                app.poll_analysis(cx);
                anyhow::ensure!(!app.analysis.provisional && app.analysis.pending.is_none() && Arc::ptr_eq(app.analysis.snapshot.as_ref().unwrap(), &final_snapshot), "analysis preview was not completed");
                let stale = crate::analysis::AnalysisRequest { generation: request.generation + 1, ..request };
                app.analysis.pending = Some(stale.clone());
                *app.backend.analysis_result.lock().unwrap() = Some(crate::backend::AnalysisReply { request: stale.clone(), complete: false, result: Ok(Arc::new(Snapshot::fixture(stale))) });
                app.poll_analysis(cx);
                anyhow::ensure!(app.analysis.pending.is_none() && Arc::ptr_eq(app.analysis.snapshot.as_ref().unwrap(), &final_snapshot) && !app.analysis.error.is_empty(), "stale-source preview replaced current data");
                app.analysis.snapshot = analysis;
                app.analysis.error.clear();
                app.analysis.pinned_bucket = None;
                *app.backend.analysis_result.lock().unwrap() = None;
                Ok::<_, anyhow::Error>(())
            })??;
            println!("progressive snapshots passed: first result, completion, cached refresh, repeated delivery and source isolation");

            this.update_in(cx, |app, window, cx| {
                use crate::analysis::{AnalysisRange, Snapshot};
                use std::sync::Arc;
                for mode in RecentUsageRefreshMode::ALL {
                    app.act(Action::RecentUsageRefreshMode(mode), window, cx);
                    app.backend.writer.flush(Duration::from_secs(5))?;
                    anyhow::ensure!(
                        Config::load(&app.backend.dir)?.recent_usage_refresh_mode == mode,
                        "recent refresh preference was not persisted"
                    );
                    for full in [false, true] {
                        for age in [0, 300_001] {
                            let before = app.next_snapshot_request(AnalysisRange::FiveMinutes);
                            let mut snapshot = Snapshot::fixture(before.clone());
                            snapshot.generated_at = chrono::Utc::now().timestamp_millis() - age;
                            let frozen = Arc::new(snapshot);
                            app.activity.recent_snapshot = Some(frozen.clone());
                            app.activity.recent_pending = None;
                            app.activity.recent_error.clear();
                            app.act(
                                if full {
                                    Action::ActivityDetails(ActivityTab::Recent)
                                } else {
                                    Action::ActivityPreview(ActivityTab::Recent)
                                },
                                window,
                                cx,
                            );
                            let after = app.next_snapshot_request(AnalysisRange::FiveMinutes);
                            let refresh = mode == RecentUsageRefreshMode::Always
                                || (mode == RecentUsageRefreshMode::AfterFiveMinutes && age > 300_000);
                            anyhow::ensure!(
                                after.id - before.id == 1 + u64::from(refresh),
                                "wrong entry refresh: mode={mode:?}, full={full}, age={age}"
                            );
                            anyhow::ensure!(
                                Arc::ptr_eq(app.activity.recent_snapshot.as_ref().unwrap(), &frozen),
                                "entry refresh discarded the visible snapshot"
                            );
                        }
                    }
                    app.activity.recent_snapshot = None;
                    app.activity.recent_pending = None;
                    let before = app.next_snapshot_request(AnalysisRange::FiveMinutes);
                    app.act(Action::ActivityPreview(ActivityTab::Recent), window, cx);
                    let after = app.next_snapshot_request(AnalysisRange::FiveMinutes);
                    anyhow::ensure!(after.id == before.id + 2, "first entry did not request a snapshot");
                }
                app.config.recent_usage_refresh_mode = RecentUsageRefreshMode::Always;
                let pending = app.next_snapshot_request(AnalysisRange::FiveMinutes);
                app.activity.recent_pending = Some(pending.clone());
                app.act(Action::ActivityPreview(ActivityTab::Recent), window, cx);
                app.act(Action::ActivityDetails(ActivityTab::Recent), window, cx);
                let after = app.next_snapshot_request(AnalysisRange::FiveMinutes);
                anyhow::ensure!(
                    after.id == pending.id + 1 && app.activity.recent_pending.as_ref() == Some(&pending),
                    "repeated entry duplicated the pending request"
                );
                app.activity.recent_pending = None;
                app.config.recent_usage_refresh_mode = RecentUsageRefreshMode::Manual;
                let before = app.next_snapshot_request(AnalysisRange::FiveMinutes);
                app.act(Action::RecentRefresh, window, cx);
                let after = app.next_snapshot_request(AnalysisRange::FiveMinutes);
                anyhow::ensure!(after.id == before.id + 2, "manual refresh stopped working");
                app.act(Action::RecentUsageRefreshMode(RecentUsageRefreshMode::default()), window, cx);
                app.activity.recent_pending = None;
                let request = app.next_snapshot_request(AnalysisRange::FiveMinutes);
                let mut old = Snapshot::fixture(request.clone());
                old.generated_at = chrono::Utc::now().timestamp_millis() - 300_001;
                let old = Arc::new(old);
                app.activity.recent_snapshot = Some(old.clone());
                app.activity.recent_pending = Some(request.clone());
                *app.backend.analysis_result.lock().unwrap() = Some(crate::backend::AnalysisReply {
                    complete: true,
request: request.clone(),
                    result: Err("test automatic refresh failure".into()),
                });
                app.poll_recent_usage(cx);
                app.act(Action::ActivityPreview(ActivityTab::Recent), window, cx);
                let after = app.next_snapshot_request(AnalysisRange::FiveMinutes);
                anyhow::ensure!(
                    after.id == request.id + 2
                        && Arc::ptr_eq(app.activity.recent_snapshot.as_ref().unwrap(), &old),
                    "failed refresh reset expiry or discarded the previous snapshot"
                );
                let fresh = Arc::new(Snapshot::fixture(after.clone()));
                app.activity.recent_pending = Some(after.clone());
                *app.backend.analysis_result.lock().unwrap() = Some(crate::backend::AnalysisReply {
                    complete: true,
request: after.clone(),
                    result: Ok(fresh.clone()),
                });
                app.poll_recent_usage(cx);
                app.act(Action::ActivityDetails(ActivityTab::Recent), window, cx);
                let next = app.next_snapshot_request(AnalysisRange::FiveMinutes);
                anyhow::ensure!(
                    next.id == after.id + 1
                        && Arc::ptr_eq(app.activity.recent_snapshot.as_ref().unwrap(), &fresh),
                    "successful refresh did not reset expiry for both views"
                );
                app.activity.recent_pending = None;
                app.activity.recent_error.clear();
                *app.backend.analysis_result.lock().unwrap() = None;
                Ok(())
            })??;
            println!("recent usage refresh modes, shared entry policy and persistence passed");
            for theme in ["light", "dark"] {
                for width in [1240., 900.] {
                    for page in [Page::Apps, Page::Overview] {
                        for state in 0..3 {
                            this.update_in(cx, |app, window, cx| {
                                window.resize(size(px(width), px(700.)));
                                app.act(Action::Theme(theme), window, cx);
                                app.act(Action::Nav(page), window, cx);
                                app.activity.details = false;
                                app.activity.preview = ActivityTab::Current;
                                seed(app, false, false);
                                app.data.live.clear();
                                let now = chrono::Utc::now().timestamp_millis();
                                if state > 0 {
                                    app.data.live.push_back(LiveSample {
                                        from: now - 61_000, to: now - 60_000,
                                        down: 24_000., up: 8_000., valid: true,
                                    });
                                }
                                if state == 2 {
                                    app.data.live.push_back(LiveSample {
                                        from: now - 60_000, to: now - 59_000,
                                        down: 0., up: 0., valid: true,
                                    });
                                    app.data.live.push_back(LiveSample {
                                        from: now - 2_000, to: now - 1_000,
                                        down: 64_000., up: 48_000., valid: true,
                                    });
                                }
                                *app.backend.shared.lock().unwrap() = app.data.clone();
                                app.page_scroll.set_offset(point(px(0.), px(0.)));
                                cx.notify();
                            })?;
                            settle_layout(&this, cx).await?;
                            this.update_in(cx, |app, _, _| {
                                anyhow::ensure!(
                                    app.page_scroll.max_offset().width <= px(1.),
                                    "live chart overflow: {theme}, width={width}, state={state}"
                                );
                                anyhow::ensure!(app.data.down == 2_097_152., "chart smoothing changed raw current speed");
                                Ok(())
                            })??;
                        }
                    }
                }
            }
            println!("live chart empty, single-point and gap layouts passed in both views");
            if native::client_area_animations_enabled() {
                this.update_in(cx, |app, window, cx| {
                    app.act(Action::Nav(Page::Apps), window, cx);
                    seed(app, false, false);
                    app.page_scroll.set_offset(point(px(0.), px(0.)));
                    cx.notify();
                })?;
                settle_layout(&this, cx).await?;
                // Let page navigation and scrollbar fade animations finish first.
                Timer::after(Duration::from_millis(1_000)).await;
                let (before, raw) = this.update_in(cx, |app, _, cx| {
                    (app.live_chart_view.frame(cx), app.data.live.clone())
                })?;
                Timer::after(Duration::from_millis(240)).await;
                this.update_in(cx, |app, _, cx| {
                    let after = app.live_chart_view.frame(cx);
                    anyhow::ensure!(after.0 >= before.0 + 3 && after.1 > before.1 + 100,
                        "live chart did not advance between polling ticks: {before:?} -> {after:?}");
                    anyhow::ensure!(after.2 == before.2 && app.data.live == raw,
                        "animation rebuilt the sample cache or modified raw traffic");
                    println!("live chart advances {} frames between polling ticks", after.0 - before.0);
                    Ok(())
                })??;
                println!("live chart advances independently of polling and preserves the raw sample cache");
            }
            if std::env::var("MIHOMO_LIVE_CHART_QA_ONLY").is_ok_and(|value| value == "1") {
                return Ok(());
            }
            for theme in ["light", "dark"] {
                for width in [1240., 900.] {
                    for tab in [ActivityTab::Current, ActivityTab::Recent] {
                        this.update_in(cx, |app, window, cx| {
                            window.resize(size(px(width), px(700.)));
                            app.act(Action::Theme(theme), window, cx);
                            app.act(Action::Nav(Page::Apps), window, cx);
                            seed(app, false, false);
                            app.act(Action::ActivityDetails(tab), window, cx);
                            app.activity.live_page = 0;
                            app.activity.recent_page = 0;
                            app.page_scroll.set_offset(point(px(0.), px(0.)));
                            cx.notify();
                        })?;
                        settle_layout(&this, cx).await?;
                        this.update_in(cx, |app, _, _| {
                            anyhow::ensure!(
                                app.page_scroll.max_offset().width <= px(1.),
                                "activity list horizontal overflow: {theme} {width} {tab:?}"
                            );
                            anyhow::ensure!(
                                app.page_scroll.max_offset().height > px(0.),
                                "full activity list should use page scrolling"
                            );
                            Ok(())
                        })??;
                    }
                }
            }
            println!("current and recent activity layouts passed: both themes and window sizes");
            for theme in ["light", "dark"] {
                for full in [false, true] {
                    for state in 0..3 {
                        this.update_in(cx, |app, window, cx| {
                            use crate::analysis::{AnalysisRange, AnalysisRequest, SampleWindow};
                            window.resize(size(px(900.),px(700.)));
                            app.act(Action::Theme(theme),window,cx);
                            app.act(Action::Nav(Page::Apps),window,cx);
                            seed(app,false,false);
                            let request = AnalysisRequest::new(900,0,AnalysisRange::FiveMinutes,chrono::Utc::now().timestamp_millis());
                            let mut samples = SampleWindow::default();
                            if state == 0 {
                                let apps = std::collections::HashMap::from([
                                    (format!(r"C:\{}\{}.exe", "long application path ".repeat(8), "长应用名称-name-".repeat(12)),Bytes{down:1<<40,up:0}),
                                    (r"C:\upload-only.exe".into(),Bytes{down:0,up:2<<40}),
                                    (r"C:\large-values.exe".into(),Bytes{down:1_048_471_142,up:1_048_471_142}),
                                    (r"C:\tiny.exe".into(),Bytes{down:1,up:1}),
                                ]);
                                let total = Bytes { down:apps.values().map(|bytes|bytes.down).sum(),up:apps.values().map(|bytes|bytes.up).sum() };
                                samples.record(request.anchor-1000,request.anchor,total,&apps);
                            }
                            app.activity.recent_snapshot = if state == 2 {None} else {Some(std::sync::Arc::new(samples.snapshot(request.clone())))};
                            app.activity.recent_pending = (state == 2).then_some(request);
                            app.activity.recent_error.clear();
                            app.activity.recent_page=0;
                            if full {app.act(Action::ActivityDetails(ActivityTab::Recent),window,cx);} else {app.act(Action::ActivityPreview(ActivityTab::Recent),window,cx);}
                            app.page_scroll.set_offset(point(px(0.),px(0.)));
                            cx.notify();
                        })?;
                        settle_layout(&this, cx).await?;
                        this.update_in(cx, |app,_,_| {
                            anyhow::ensure!(app.page_scroll.max_offset().width<=px(1.), "two-line usage layout overflow: {theme}, full={full}, state={state}");
                            if state == 0 {anyhow::ensure!(app.recent_usage_rows(false).len()==4,"usage fixture lost single-direction or tiny applications");}
                            Ok(())
                        })??;
                    }
                }
            }
            this.update_in(cx, |app,_,_| {app.activity.recent_pending=None;})?;
            println!("two-line usage layouts passed: large values, long names, single directions, tiny, empty and loading");
            for theme in ["light", "dark"] {
                for width in [900., 1240.] {
                    for state in 0..4 {
                        this.update_in(cx, |app, window, cx| {
                            window.resize(size(px(width), px(700.)));
                            app.act(Action::Theme(theme), window, cx);
                            app.act(Action::Nav(Page::Apps), window, cx);
                            app.act(Action::Period(TimePeriod::LastSevenDays), window, cx);
                            let outside = (app.query.from, app.query.to);
                            app.history_state.tab = HistoryTab::Records;
                            app.act(Action::Nav(Page::History), window, cx);
                            app.act(Action::History(HistoryAction::Period(TimePeriod::Yesterday)), window, cx);
                            anyhow::ensure!(app.period == TimePeriod::LastSevenDays, "history changed shared dates");
                            anyhow::ensure!(Some((app.query.from, app.query.to)) == TimePeriod::Yesterday.bounds(chrono::Local::now())?, "history yesterday did not apply");
                            seed(app, state == 1, state == 2);
                            if state == 3 { app.report.query = None; }
                            if state == 1 {
                                let key = app.report.rows[0].key.clone();
                                app.act(Action::Application(ApplicationAction::OpenUsageDetail(key.clone())), window, cx);
                                seed(app, true, false);
                                anyhow::ensure!(app.usage_detail_open(), "history detail did not open");
                                let context = app.active_detail_context().unwrap();
                                anyhow::ensure!(context.key == key && !context.group_apps, "history detail attribution differs from table");
                                let detail = app.application_detail_query().unwrap();
                                anyhow::ensure!((detail.from, detail.to) == (app.query.from, app.query.to), "history detail dates differ");
                            }
                            app.history_state.quality_open = state == 1;
                            app.history_state.record_query = app.query.clone();
                            app.page_scroll.set_offset(point(px(0.), px(0.)));
                            anyhow::ensure!(app.history_state.outside_range == Some(outside), "history lost shared date context");
                            cx.notify();
                            Ok(())
                        })??;
                        settle_layout(&this, cx).await?;
                        this.update_in(cx, |app, window, cx| {
                            anyhow::ensure!(app.page_scroll.max_offset().width <= px(1.), "history records overflow: {theme} {width} state={state}");
                            if state == 1 {
                                anyhow::ensure!(app.usage_detail_docked(window) == (width == 1240.), "history detail breakpoint differs from apps");
                                anyhow::ensure!(app.usage_detail.scroll.max_offset().width <= px(1.), "combined trend overflows the history detail pane: {theme} {width}");
                                app.act(Action::Dimension("domain"), window, cx);
                                anyhow::ensure!(!app.usage_detail_open(), "changing history dimension kept detail open");
                            }
                            app.query.page = 1;
                            app.query.sort = TrafficSort::Upload;
                            app.query.ascending = true;
                            app.query.search = "example".into();
                            app.last_search = "example".into();
                            app.search.update(cx, |s, cx| s.set_value("example", window, cx));
                            let records = app.query.clone();
                            let offset = point(px(0.), -app.page_scroll.max_offset().height.min(px(100.)));
                            app.page_scroll.set_offset(offset);
                            app.act(Action::History(HistoryAction::Tab(HistoryTab::Analysis)), window, cx);
                            let old_analysis_period = app.history_state.analysis_period;
                            app.act(Action::History(HistoryAction::Tab(HistoryTab::Records)), window, cx);
                            anyhow::ensure!(app.query == records, "history tab switch lost record query");
                            anyhow::ensure!(app.page_scroll.offset() == offset, "history tab switch lost scroll position");
                            anyhow::ensure!(app.history_state.analysis_period == old_analysis_period, "record tab changed analysis dates");
                            let outside = app.history_state.outside_range.unwrap();
                            app.act(Action::Nav(Page::Apps), window, cx);
                            anyhow::ensure!((app.query.from, app.query.to) == outside, "history leaked its dates into Apps");
                            app.act(Action::Nav(Page::History), window, cx);
                            anyhow::ensure!(app.query == records, "history navigation lost record query");
                            anyhow::ensure!(app.search.read(cx).value() == "example", "history search text was not restored");
                            app.act(Action::History(HistoryAction::Period(TimePeriod::Custom)), window, cx);
                            app.history_state.time_precision = true;
                            app.set_time_values(["2024-02-20", "09:00", "2024-02-20", "09:00"], window, cx);
                            app.act(Action::History(HistoryAction::ApplyRange), window, cx);
                            anyhow::ensure!(app.query.to - app.query.from == 60_000, "custom history range lost inclusive end minute");
                            app.act(Action::History(HistoryAction::Period(TimePeriod::Today)), window, cx);
                            seed(app, false, false);
                            Ok(())
                        })??;
                    }
                    println!("history records layouts, details and independent context passed: {theme} {width}x700");
                }
            }

            time_editor_tests::exercise(&this, cx).await?;
            history_tests::exercise(&this, cx).await?;
            comparison_tests::exercise(&this, cx).await?;
            for theme in ["light", "dark"] {
                for width in [900., 1240., 1600.] {
                    for period in [TimePeriod::Today, TimePeriod::LastSevenDays, TimePeriod::LastThreeMonths] {
                        this.update_in(cx, |app, window, cx| {
                            window.resize(size(px(width), px(700.)));
                            app.act(Action::Theme(theme), window, cx);
                            app.page = Page::History;
                            app.history_state.tab = HistoryTab::Analysis;
                            app.history_state.analysis_period = Some(period);
                            app.history_state.restore_offset = None;
                            app.activity.details = false;
                            app.analysis.active = true;
                            app.analysis.pending = None;
                            app.analysis.history_bucket = None;
                            app.analysis.focus = None;
                            app.analysis.error.clear();
                            app.analysis.search_value.clear();
                            app.analysis.search.update(cx, |s, cx| s.set_value("", window, cx));
                            let (from, to) = period.bounds(chrono::Local::now())?.unwrap();
                            let request = crate::analysis::AnalysisRequest::historical(300, 0, from, to, chrono::Utc::now().timestamp_millis())?;
                            app.analysis.snapshot = Some(std::sync::Arc::new(crate::analysis::Snapshot::fixture(request)));
                            app.analysis.show_trends = false;
                            app.analysis.pinned_bucket = None;
                            app.analysis.anchor = None;
                            app.analysis.restore_offset = None;
                            app.page_scroll.set_offset(point(px(0.), px(0.)));
                            seed(app, false, false);
                            cx.notify();
                            Ok::<_, anyhow::Error>(())
                        })??;
                        settle_layout(&this, cx).await?;
                        this.update_in(cx, |app, window, cx| {
                            anyhow::ensure!(app.page_scroll.max_offset().width <= px(1.), "historical comparison overflow: {theme} {width} {period:?}");
                            let rect = app.analysis.summary_bounds.get();
                            anyhow::ensure!(rect.size.height > px(60.) && app.analysis.bounds[2].get().top() + px(160.) < app.page_scroll.bounds().bottom(), "comparison summary and first application did not fit first screen: {rect:?}");
                            let picker = app.picker_bounds["history-range"].get();
                            anyhow::ensure!(picker.size.width >= px(58.) && picker.size.width <= px(140.), "history range trigger width is invalid");
                            let snapshot = app.analysis.snapshot.clone().unwrap();
                            let bucket = app.analysis_bucket(&snapshot);
                            anyhow::ensure!(snapshot.histogram(bucket, None).len() <= 60, "historical default is too dense");
                            app.act(Action::Analysis(AnalysisAction::Compare), window, cx);
                            app.act(Action::Analysis(AnalysisAction::Details(true)), window, cx);
                            anyhow::ensure!(std::sync::Arc::ptr_eq(&snapshot, app.analysis.snapshot.as_ref().unwrap()), "historical view action resampled data");
                            Ok(())
                        })??;
                    }
                    println!("historical comparison layouts and frozen aggregates passed: {theme} {width}x700");
                }
            }
            for theme in ["light", "dark"] {
                for width in [1240., 900., 1600.] {
                    for range in crate::analysis::AnalysisRange::ALL {
                        this.update_in(cx, |app, window, cx| {
                            window.resize(size(px(width), px(700.)));
                            app.act(Action::Theme(theme), window, cx);
                            app.activity.details = true;
                            app.page = Page::History;
                            app.history_state.tab = HistoryTab::Analysis;
                            app.history_state.analysis_period = None;
                            app.activity.details = false;
                            app.analysis.active = true;
                            app.analysis.pending = None;
                            app.analysis.range = range;
                            app.analysis.show_trends = false;
                            app.analysis.snapshot =
                                Some(std::sync::Arc::new(crate::analysis::Snapshot::fixture(
                                    crate::analysis::AnalysisRequest::new(
                                        10,
                                        0,
                                        range,
                                        chrono::Utc::now().timestamp_millis(),
                                    ),
                                )));
                            app.analysis.focus = None;
                            app.analysis.app_page = 0;
                            app.analysis.burst_page = 0;
                            app.analysis.anchor = None;
                            app.analysis.restore_offset = None;
                            app.page_scroll.set_offset(point(px(0.),px(0.)));
                            cx.notify();
                        })?;
                        settle_layout(&this, cx).await?;
                        this.update_in(cx, |app, window, cx| {
                            anyhow::ensure!(
                                app.page_scroll.max_offset().width <= px(1.),
                                "analysis overflow: {theme} {width} {}",
                                range.label()
                            );
                            anyhow::ensure!(
                                app.analysis.summary_bounds.get().size.height > px(0.)
                                    && app.analysis.bounds[if app.analysis.show_trends {3} else {2}].get().size.height > px(0.),
                                "analysis sections did not render"
                            );
                            anyhow::ensure!(
                                app.analysis_rows().len() == 22,
                                "analysis lost application union"
                            );
                            let snapshot = app.analysis.snapshot.clone().unwrap();
                            app.act(Action::Analysis(AnalysisAction::Page(1, false)), window, cx);
                            anyhow::ensure!(
                                app.analysis.app_page == 1,
                                "analysis page did not advance"
                            );
                            let key = snapshot.rows[0].key.clone();
                            app.act(Action::Analysis(AnalysisAction::Focus(Some(key.clone()))), window, cx);
                            anyhow::ensure!(
                                app.analysis.focus.as_ref() == Some(&key),
                                "analysis focus failed"
                            );
                            anyhow::ensure!(
                                app.analysis_rows().len() == 22,
                                "focus filtered comparison rows"
                            );
                            anyhow::ensure!(
                                std::sync::Arc::ptr_eq(
                                    app.analysis.snapshot.as_ref().unwrap(),
                                    &snapshot
                                ),
                                "view actions changed frozen data"
                            );
                            app.act(Action::Analysis(AnalysisAction::Focus(None)), window, cx);
                            let burst = &snapshot.bursts[0];
                            app.act(
                                Action::Analysis(AnalysisAction::Burst(burst.app.clone(), burst.time)),
                                window,
                                cx,
                            );
                            anyhow::ensure!(
                                app.analysis.highlight == Some(burst.time),
                                "burst interval did not focus"
                            );
                            app.act(Action::Analysis(AnalysisAction::Focus(None)), window, cx);
                            app.act(Action::History(HistoryAction::Tab(HistoryTab::Records)), window, cx);
                            anyhow::ensure!(
                                !app.analysis.active && app.query.page_size == 10,
                                "usage context changed"
                            );
                            app.act(Action::History(HistoryAction::Tab(HistoryTab::Analysis)), window, cx);
                            anyhow::ensure!((app.analysis.pending.is_some() || !app.analysis.error.is_empty()) && !app.analysis.show_trends
                                && app.analysis.focus.is_none() && app.analysis.app_page == 0
                                && app.history_state.analysis_range == (app.query.from, app.query.to),
                                "reopening comparison did not refresh the current history selection");
                            Ok(())
                        })??;
                    }
                    println!("analysis layouts and frozen context passed: {theme} {width}x700");
                }
            }
            for theme in ["light", "dark"] {
                for width in [900., 1240., 1600.] {
                    for range in crate::analysis::AnalysisRange::ALL {
                        for size in range.bucket_sizes() {
                            for compare in [true, false] {
                                this.update_in(cx, |app, window, cx| {
                                    window.resize(gpui::size(px(width), px(700.)));
                                    app.act(Action::Theme(theme), window, cx);
                                    app.activity.details = true;
                                    app.page = Page::History;
                            app.history_state.tab = HistoryTab::Analysis;
                            app.history_state.analysis_period = None;
                            app.activity.details = false;
                                    app.analysis.active = true;
                                    app.analysis.show_trends = true;
                                    app.analysis.restore_offset = None;
                                    app.analysis.pending = None;
                                    app.analysis.range = range;
                                    app.analysis.snapshot = Some(std::sync::Arc::new(crate::analysis::Snapshot::fixture(crate::analysis::AnalysisRequest::new(44, 0, range, chrono::Utc::now().timestamp_millis()))));
                                    app.analysis.focus = None;
                                    app.analysis.compare_previous = compare;
                                    let frozen = app.analysis.snapshot.clone().unwrap();
                                    app.act(Action::Analysis(AnalysisAction::Bucket(*size)), window, cx);
                                    let bins = frozen.histogram(*size, None);
                                    let time = bins[1].from;
                                    app.act(Action::Analysis(AnalysisAction::Pin(time)), window, cx);
                                    anyhow::ensure!(app.analysis.pinned_bucket == Some(time), "histogram click did not pin");
                                    app.act(Action::Analysis(AnalysisAction::Pin(time)), window, cx);
                                    anyhow::ensure!(app.analysis.pinned_bucket.is_none(), "histogram click did not unpin");
                                    app.act(Action::Analysis(AnalysisAction::Compare), window, cx);
                                    anyhow::ensure!(app.analysis.compare_previous != compare, "previous period toggle failed");
                                    app.act(Action::Analysis(AnalysisAction::Compare), window, cx);
                                    anyhow::ensure!(std::sync::Arc::ptr_eq(app.analysis.snapshot.as_ref().unwrap(), &frozen), "histogram view controls regenerated snapshot");
                                    app.analysis.anchor = Some(1);
                                    cx.notify();
                                    Ok(())
                                })??;
                                Timer::after(Duration::from_millis(100)).await;
                                this.update_in(cx, |app, _, _| {
                                    anyhow::ensure!(app.page_scroll.max_offset().width <= px(1.), "histogram overflow: {theme} {width} {range:?} {size:?} compare={compare}");
                                    anyhow::ensure!(app.analysis.bounds[1].get().size.height > px(200.), "histogram or detail panel collapsed");
                                    Ok(())
                                })??;
                            }
                        }
                    }
                }
            }
            println!("histogram layouts passed: all intervals, previous toggle and pinning without snapshot refresh");
            for theme in ["light", "dark"] {
                for width in [900., 1240., 1600.] {
                    for bursts in [false, true] {
                        this.update_in(cx, |app, window, cx| {
                            window.resize(gpui::size(px(width), px(700.)));
                            app.act(Action::Theme(theme), window, cx);
                            app.activity.details = true;
                            app.page = Page::History;
                            app.history_state.tab = HistoryTab::Analysis;
                            app.history_state.analysis_period = None;
                            app.activity.details = false;
                            app.analysis.active = true;
                            app.analysis.pending = None;
                            app.analysis.range = crate::analysis::AnalysisRange::FiveMinutes;
                            app.analysis.snapshot = Some(std::sync::Arc::new(crate::analysis::Snapshot::fixture(crate::analysis::AnalysisRequest::new(70,0,app.analysis.range,chrono::Utc::now().timestamp_millis()))));
                            app.analysis.focus = None;
                            app.analysis.pinned_bucket = None;
                            app.analysis.highlight = None;
                            app.analysis.app_page = 1;
                            app.analysis.burst_page = 1;
                            let frozen = app.analysis.snapshot.clone().unwrap();
                            app.act(Action::Analysis(AnalysisAction::Details(bursts)), window, cx);
                            anyhow::ensure!(app.analysis.show_trends == bursts && app.analysis.app_page == 1 && app.analysis.burst_page == 1, "analysis detail switch lost context");
                            app.analysis.anchor = None;
                            app.act(Action::Analysis(AnalysisAction::Details(bursts)), window, cx);
                            anyhow::ensure!(app.analysis.anchor.is_none(), "reselecting a tab unexpectedly moved the page");
                            anyhow::ensure!(std::sync::Arc::ptr_eq(app.analysis.snapshot.as_ref().unwrap(), &frozen), "detail switch refreshed snapshot");
                            app.analysis.anchor = None;
                            app.analysis.restore_offset = None;
                            app.page_scroll.set_offset(point(px(0.),px(0.)));
                            cx.notify();
                            Ok(())
                        })??;
                        settle_layout(&this, cx).await?;
                        this.update_in(cx, |app, _, _| {
                            anyhow::ensure!(app.analysis.summary_bounds.get().size.height > px(0.), "global summary did not render: {theme} {width}");
                            anyhow::ensure!(app.analysis.bounds[if bursts {3} else {2}].get().size.height > px(0.) && app.analysis.bounds[if bursts {2} else {3}].get().size.height == px(0.), "analysis details were not shown exclusively");
                            anyhow::ensure!(app.page_scroll.max_offset().width <= px(1.), "analysis detail overflow");
                            Ok(())
                        })??;
                    }
                }
            }
            println!("simplified analysis passed: first-screen hero/chart, exclusive detail tabs and context preservation");
            for theme in ["light", "dark"] {
                for state in 0..6 {
                    this.update_in(cx, |app, window, cx| {
                        window.resize(size(px(900.), px(700.)));
                        app.act(Action::Theme(theme), window, cx);
                        app.activity.details = true;
                        app.page = Page::History;
                            app.history_state.tab = HistoryTab::Analysis;
                            app.history_state.analysis_period = None;
                            app.activity.details = false;
                        app.analysis.active = true;
                        app.analysis.focus = None;
                        app.analysis.error.clear();
                        let request = crate::analysis::AnalysisRequest::new(
                            200,
                            0,
                            crate::analysis::AnalysisRange::FiveMinutes,
                            chrono::Utc::now().timestamp_millis(),
                        );
                        *app.backend.analysis_result.lock().unwrap() = None;
                        app.analysis.snapshot = None;
                        app.analysis.pending = None;
                        match state {
                            0 => app.analysis.pending = Some(request),
                            1 => app.analysis.error = "读取失败，请重试".into(),
                            2 => {
                                app.analysis.snapshot = Some(std::sync::Arc::new(
                                    crate::analysis::SampleWindow::default().snapshot(request),
                                ))
                            }
                            3 => {
                                let mut samples = crate::analysis::SampleWindow::default();
                                for second in 0..60 {
                                    let from = request.anchor - 60_000 + second * 1000;
                                    samples.record(
                                        from,
                                        from + 1000,
                                        Bytes {
                                            down: 1000,
                                            up: 100,
                                        },
                                        &std::collections::HashMap::from([(
                                            "single.exe".into(),
                                            Bytes {
                                                down: 1000,
                                                up: 100,
                                            },
                                        )]),
                                    );
                                }
                                let snapshot = samples.snapshot(request);
                                anyhow::ensure!(
                                    snapshot.series(None).iter().flatten().count() == 1,
                                    "single point fixture invalid"
                                );
                                app.analysis.snapshot = Some(std::sync::Arc::new(snapshot));
                            }
                            5 => {
                                let mut samples = crate::analysis::SampleWindow::default();
                                for second in 0..600 {
                                    if second == 100 {
                                        continue;
                                    }
                                    let from = request.anchor - 600_000 + second * 1000;
                                    samples.record(
                                        from,
                                        from + 1000,
                                        Bytes {
                                            down: 1000,
                                            up: 100,
                                        },
                                        &std::collections::HashMap::from([(
                                            "partial.exe".into(),
                                            Bytes {
                                                down: 1000,
                                                up: 100,
                                            },
                                        )]),
                                    );
                                }
                                let snapshot = samples.snapshot(request);
                                anyhow::ensure!(
                                    snapshot.can_compare_observed() && !snapshot.comparable(),
                                    "partial comparison fixture invalid"
                                );
                                app.analysis.snapshot = Some(std::sync::Arc::new(snapshot));
                                app.analysis.sort = super::analysis_page::Sort::Change;
                                anyhow::ensure!(
                                    app.analysis_effective_sort()
                                        == super::analysis_page::Sort::Change,
                                    "partial coverage mislabeled observed-change sorting"
                                );
                            }
                            _ => {
                                let old = std::sync::Arc::new(crate::analysis::Snapshot::fixture(
                                    request,
                                ));
                                let request = crate::analysis::AnalysisRequest::new(
                                    201,
                                    0,
                                    crate::analysis::AnalysisRange::Hour,
                                    chrono::Utc::now().timestamp_millis(),
                                );
                                app.analysis.snapshot = Some(old.clone());
                                app.analysis.pending = Some(request.clone());
                                *app.backend.analysis_result.lock().unwrap() =
                                    Some(crate::backend::AnalysisReply {
                                        complete: true,
request,
                                        result: Err("查询失败，原快照保留".into()),
                                    });
                                app.poll_analysis(cx);
                                anyhow::ensure!(
                                    std::sync::Arc::ptr_eq(
                                        app.analysis.snapshot.as_ref().unwrap(),
                                        &old
                                    ),
                                    "failure discarded old snapshot"
                                );
                                anyhow::ensure!(
                                    app.analysis.pending.is_none(),
                                    "failed request stayed busy"
                                );
                            }
                        }
                        app.page_scroll.set_offset(point(px(0.), px(0.)));
                        cx.notify();
                        Ok(())
                    })??;
                    settle_layout(&this, cx).await?;
                    this.update_in(cx, |app, _, _| {
                        anyhow::ensure!(
                            app.page_scroll.max_offset().width <= px(1.),
                            "analysis edge state overflow {theme} state={state}"
                        );
                        Ok(())
                    })??;
                }
            }
            println!("analysis loading, empty and failed refresh checks passed");
            for theme in ["light", "dark"] {
                for width in [1240., 900.] {
                    this.update_in(cx, |app, window, cx| {
                        window.resize(size(px(width), px(700.)));
                        app.act(Action::Theme(theme), window, cx);
                        app.act(Action::Nav(Page::Settings), window, cx);
                        app.act(Action::SettingsSection(SettingsSection::General), window, cx);
                        cx.notify();
                    })?;
                    settle_layout(&this, cx).await?;
                    this.update_in(cx, |app, _, cx| {
                        app.connection_test_scroll.set_offset(point(
                            px(0.),
                            -app.connection_test_scroll.max_offset().height,
                        ));
                        cx.notify();
                    })?;
                    settle_layout(&this, cx).await?;
                    this.update_in(cx, |app, window, cx| {
                        let bounds = app.picker_bounds["recent-usage-refresh"].get();
                        anyhow::ensure!(
                            bounds.size.width >= px(176.) && bounds.size.width <= px(182.)
                                && bounds.top() > px(40.)
                                && bounds.bottom() < window.bounds().size.height,
                            "recent usage settings control did not fit: {theme}, {width}, {bounds:?}"
                        );
                        anyhow::ensure!(
                            app.connection_test_scroll.max_offset().width <= px(1.),
                            "recent usage settings overflowed horizontally"
                        );
                        app.toggle_picker(
                            "recent-usage-refresh",
                            RecentUsageRefreshMode::ALL.into_iter().map(|mode| (
                                mode.label().into(),
                                Action::RecentUsageRefreshMode(mode),
                                mode == app.config.recent_usage_refresh_mode,
                            )).collect(),
                            window,
                            cx,
                        );
                        anyhow::ensure!(app.picker.as_ref().unwrap().choices.len() == 3, "missing refresh modes");
                        Ok(())
                    })??;
                    cx.update(|window, cx| {
                        window.dispatch_keystroke(Keystroke::parse("home").unwrap(), cx);
                        window.dispatch_keystroke(Keystroke::parse("enter").unwrap(), cx);
                    })?;
                    this.update_in(cx, |app, window, cx| {
                        anyhow::ensure!(
                            app.picker.is_none() && app.config.recent_usage_refresh_mode == RecentUsageRefreshMode::Always,
                            "refresh mode picker keyboard selection failed"
                        );
                        app.act(Action::RecentUsageRefreshMode(RecentUsageRefreshMode::default()), window, cx);
                        Ok(())
                    })??;
                }
            }
            println!("recent usage settings layout and keyboard picker passed");
            let hold = std::env::var("MIHOMO_APPS_QA_HOLD_MS")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0);
            if hold > 0 {
                let review_width = std::env::var("MIHOMO_APPS_QA_WIDTH")
                    .ok()
                    .and_then(|value| value.parse::<f32>().ok())
                    .unwrap_or(1240.)
                    .max(900.);
                for theme in ["dark", "light"] {
                    this.update_in(cx, |app, window, cx| {
                        window.resize(size(
                            px(review_width),
                            px(if review_width <= 900. { 700. } else { 870. }),
                        ));
                        app.act(Action::Theme(theme), window, cx);
                        app.act(Action::Nav(Page::Apps), window, cx);
                        app.act(Action::Period(TimePeriod::Today), window, cx);

                        seed(app, false, false);
                        app.activity.details = true;
                        app.page = Page::History;
                            app.history_state.tab = HistoryTab::Analysis;
                            app.history_state.analysis_period = None;
                            app.activity.details = false;
                        app.analysis.active = true;
                        app.analysis.range = crate::analysis::AnalysisRange::FiveMinutes;
                        app.analysis.snapshot =
                            Some(std::sync::Arc::new(crate::analysis::Snapshot::fixture(
                                crate::analysis::AnalysisRequest::new(
                                    100,
                                    0,
                                    app.analysis.range,
                                    chrono::Utc::now().timestamp_millis(),
                                ),
                            )));
                        app.analysis.pending = None;
                        app.analysis.app_page = 0;
                        app.analysis.burst_page = 0;
                        app.analysis.focus = None;
                        app.activity.recent_snapshot = app.analysis.snapshot.clone();
                        app.activity.preview = ActivityTab::Recent;
                        app.page = Page::Apps;
                        app.activity.details = false;
                        app.analysis.active = false;
                        app.analysis.anchor = None;
                        app.page_scroll.set_offset(point(px(0.), px(0.)));
                        if std::env::var("MIHOMO_APPS_QA_HISTORY").is_ok_and(|value| value == "1") {
                            app.history_state.tab = HistoryTab::Records;
                            app.act(Action::Nav(Page::History), window, cx);
                            app.act(Action::History(HistoryAction::Period(TimePeriod::Today)), window, cx);
                            let (from, to) = TimePeriod::Today.bounds(chrono::Local::now()).unwrap().unwrap();
                            app.history_state.analysis_period = Some(TimePeriod::Today);
                            let request = crate::analysis::AnalysisRequest::historical(101, 0, from, to, chrono::Utc::now().timestamp_millis()).unwrap();
                            app.analysis.snapshot = Some(std::sync::Arc::new(crate::analysis::Snapshot::fixture(request)));
                            app.analysis.error.clear();
                            seed(app, false, false);
                            if std::env::var("MIHOMO_APPS_QA_CALENDAR_YEAR").is_ok_and(|value| value == "1") {
                                app.act(Action::History(HistoryAction::Calendar(super::history_state::CalendarAction::View(super::history_state::CalendarView::Year))), window, cx);
                                seed(app, false, false);
                            }
                            if std::env::var("MIHOMO_APPS_QA_CALENDAR_SELECTION").is_ok_and(|value| value == "1") {
                                let today = chrono::Local::now().date_naive();
                                app.commit_history_dates(today.checked_sub_days(chrono::Days::new(7)).unwrap(), today, today);
                                seed(app, false, false);
                                if let Some(month) = app.history_state.cache.front_mut() {
                                    for day in &mut std::sync::Arc::make_mut(month).days {
                                        if crate::history::local_date(day.from) == today { day.covered_ms /= 2; }
                                    }
                                }
                            }
                        }
                        if std::env::var("MIHOMO_APPS_QA_ANALYSIS").is_ok_and(|value| value == "1") {
                            app.activity.details = true;
                            app.page = Page::History;
                            app.history_state.tab = HistoryTab::Analysis;
                            if std::env::var("MIHOMO_APPS_QA_HISTORY").is_err() { app.history_state.analysis_period = None; }
                            app.activity.details = false;
                            app.analysis.active = true;
                            app.analysis.show_trends = false;
                            app.analysis.compare_previous = true;
                            app.analysis.bucket_sizes[0] = crate::analysis::BucketSize::Seconds10;
                            app.analysis.pinned_bucket = None;
                            app.analysis.highlight = None;
                            app.analysis.anchor = None;
                            app.analysis.restore_offset = None;
                            app.page_scroll.set_offset(point(px(0.), px(0.)));
                        }
                        cx.notify();
                    })?;
                    println!("application visual review: {theme}");
                    let deadline = Instant::now() + Duration::from_millis(hold);
                    while Instant::now() < deadline {
                        Timer::after(Duration::from_millis(200)).await;
                        this.update_in(cx, |app, _, cx| {
                            if let Some(request) = app.activity.recent_pending.take() {
                                app.activity.recent_snapshot = Some(std::sync::Arc::new(
                                    crate::analysis::Snapshot::fixture(request),
                                ));
                                app.activity.recent_error.clear();
                            }
                            if let Some(request) = app.analysis.pending.take() {
                                app.analysis.snapshot = Some(std::sync::Arc::new(
                                    crate::analysis::Snapshot::fixture(request),
                                ));
                                app.analysis.error.clear();
                            }
                            if app.backend.shared.lock().unwrap().report.query.as_ref()
                                != Some(&app.query)
                            {
                                seed(app, false, false);
                            }
                            if std::env::var("MIHOMO_APPS_QA_CALENDAR_SELECTION").is_ok_and(|value| value == "1") {
                                if let Some(month) = app.history_state.cache.front_mut() {
                                    std::sync::Arc::make_mut(month).request.as_of = chrono::Utc::now().timestamp_millis();
                                }
                                app.history_state.month_error.clear();
                                app.history_state.month_retry = None;
                            }
                            let mut shared = app.backend.shared.lock().unwrap();
                            std::sync::Arc::make_mut(&mut shared.report).query = Some(app.query.clone());
                            shared.report_id = app.query_id;
                            std::sync::Arc::make_mut(&mut shared.detail).query = Some(Query {
                                app: app.selected.clone(),
                                dimension: app.detail_dimension.into(),
                                sort: TrafficSort::Total,
                                ascending: false,
                                page_size: 50,
                                ..app.query.clone()
                            });
                            shared.detail_id = app.detail_id;
                            app.report = shared.report.as_ref().clone();
                            app.detail = shared.detail.as_ref().clone();
                            app.data = shared.clone();
                            cx.notify();
                        })?;
                    }
                }
            }
            Ok(())
        }
        .await;
        *OUTCOME.lock().unwrap() = Some(result.map_err(|error| error.to_string()));
        let _ = cx.update(|_, cx| cx.quit());
    })
    .detach();
}

#[::core::prelude::v1::test]
#[ignore = "opens isolated application dashboard and detail layouts"]
fn gpui_application_dashboard_layout_and_navigation() {
    let dir = tempfile::tempdir().unwrap();
    Config {
        endpoint: "http://127.0.0.1:9".into(),
        floating: false,
        taskbar: false,
        tray_visible: false,
        open_hotkey: String::new(),
        ..Default::default()
    }
    .save(dir.path())
    .unwrap();
    let backend = Backend::start(dir.path().to_path_buf()).unwrap();
    backend.stop.store(true, Ordering::Relaxed);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !backend.done.load(Ordering::Acquire) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(backend.done.load(Ordering::Acquire));
    RUN.store(true, Ordering::Release);
    let result = run(backend.clone());
    RUN.store(false, Ordering::Release);
    backend.writer.close(Duration::from_secs(5)).unwrap();
    result.unwrap();
    OUTCOME
        .lock()
        .unwrap()
        .take()
        .expect("apps checks did not complete")
        .unwrap();
}

#[::core::prelude::v1::test]
#[ignore = "opens isolated comparison secondary page and exercises navigation and snapshot isolation"]
fn gpui_comparison_secondary_page() {
    COMPARISON_ONLY.store(true, Ordering::Release);
    gpui_application_dashboard_layout_and_navigation();
    COMPARISON_ONLY.store(false, Ordering::Release);
}
