use super::*;
use crate::speed_stats::{Peak, SecondsData, SecondsReply, SpeedStats, WINDOW};

pub(super) async fn exercise(
    this: &WeakEntity<TrafficApp>,
    cx: &mut AsyncWindowContext,
) -> anyhow::Result<()> {
    this.update_in(cx, |app, window, cx| {
        app.act(Action::Nav(Page::Settings), window, cx);
        app.act(
            Action::SettingsSection(SettingsSection::Storage),
            window,
            cx,
        );
        anyhow::ensure!(
            app.config.seconds_enabled
                && app.config.seconds_days_enabled
                && app.config.seconds_days == 3
                && !app.config.storage_limit_enabled,
            "seconds defaults changed"
        );
        anyhow::ensure!(
            app.seconds.days.read(cx).value().as_ref() == "3"
                && app.retention_size.read(cx).value().as_ref() == "2",
            "default day/GB display wrong"
        );
        let legacy = app.config.retention_policy();
        app.act(
            Action::Drill(DrillAction::SecondsTimeUnit(RetentionTimeUnit::Days)),
            window,
            cx,
        );
        anyhow::ensure!(
            app.seconds.days.read(cx).value().as_ref() == "3",
            "days conversion wrong"
        );
        app.seconds
            .days
            .update(cx, |s, cx| s.set_value("60", window, cx));
        app.save_seconds_setting(window, cx);
        app.act(
            Action::Drill(DrillAction::SecondsTimeUnit(RetentionTimeUnit::Months)),
            window,
            cx,
        );
        anyhow::ensure!(
            app.config.seconds_days == 60 && app.seconds.days.read(cx).value().as_ref() == "2",
            "month conversion changed duration"
        );
        app.seconds
            .days
            .update(cx, |s, cx| s.set_value("0", window, cx));
        app.save_seconds_setting(window, cx);
        anyhow::ensure!(
            !app.seconds.setting_error.is_empty() && app.config.seconds_days == 60,
            "invalid seconds duration saved"
        );
        app.act(Action::Drill(DrillAction::SecondsLongTerm), window, cx);
        anyhow::ensure!(
            !app.config.seconds_days_enabled
                && app.config.seconds_days == 60
                && app.seconds.setting_error.is_empty(),
            "long term rejected invalid inactive draft or lost cached duration"
        );
        anyhow::ensure!(
            crate::speed_stats::SecondsPolicy::from(&app.config)
                .days
                .is_none(),
            "long term still expires"
        );
        app.act(
            Action::Drill(DrillAction::SecondsTimeUnit(RetentionTimeUnit::Months)),
            window,
            cx,
        );
        anyhow::ensure!(
            app.config.seconds_days_enabled
                && app.config.seconds_days == 60
                && app.seconds.days.read(cx).value().as_ref() == "2",
            "returning to months lost duration"
        );
        app.seconds
            .days
            .update(cx, |s, cx| s.set_value("1", window, cx));
        app.save_seconds_setting(window, cx);
        app.act(
            Action::Drill(DrillAction::SecondsTimeUnit(RetentionTimeUnit::Days)),
            window,
            cx,
        );
        app.seconds
            .days
            .update(cx, |s, cx| s.set_value("31", window, cx));
        app.save_seconds_setting(window, cx);
        app.act(
            Action::Drill(DrillAction::SecondsTimeUnit(RetentionTimeUnit::Months)),
            window,
            cx,
        );
        app.save_seconds_setting(window, cx);
        anyhow::ensure!(
            app.config.seconds_days == 31 && app.seconds.setting_error.is_empty(),
            "rounded month display could not preserve 31 days"
        );
        app.seconds
            .days
            .update(cx, |s, cx| s.set_value("1", window, cx));
        app.save_seconds_setting(window, cx);
        app.act(
            Action::Storage(StorageAction::RetentionSizeUnit(RetentionSizeUnit::Mb)),
            window,
            cx,
        );
        anyhow::ensure!(
            app.retention_size.read(cx).value().as_ref() == "2048",
            "MB conversion wrong"
        );
        if !app.config.storage_limit_enabled {
            app.act(
                Action::Storage(StorageAction::RetentionToggle(true)),
                window,
                cx,
            );
        }
        app.retention_size
            .update(cx, |s, cx| s.set_value("4096", window, cx));
        app.save_retention_size(window, cx);
        app.act(
            Action::Storage(StorageAction::RetentionSizeUnit(RetentionSizeUnit::Gb)),
            window,
            cx,
        );
        anyhow::ensure!(
            app.retention_size.read(cx).value().as_ref() == "4",
            "GB conversion wrong"
        );
        if app.config.seconds_enabled {
            app.act(Action::Drill(DrillAction::SecondsToggle), window, cx);
        }
        app.act(Action::Drill(DrillAction::SecondsToggle), window, cx);
        anyhow::ensure!(
            app.config.seconds_enabled && app.config.storage_limit_enabled,
            "seconds switches failed"
        );
        app.act(Action::Drill(DrillAction::SecondsToggle), window, cx);
        anyhow::ensure!(
            !app.config.seconds_enabled && app.config.retention_policy() == legacy,
            "seconds changed ledger policy"
        );
        app.act(Action::Drill(DrillAction::SecondsLongTerm), window, cx);
        Ok::<_, anyhow::Error>(())
    })??;
    Timer::after(Duration::from_millis(500)).await;
    this.update_in(cx, |app, window, cx| {
        let saved = Config::load(&app.backend.dir)?;
        anyhow::ensure!(
            !saved.seconds_enabled
                && !saved.seconds_days_enabled
                && saved.seconds_days == 30
                && saved.storage_limit_mb == 4096
                && saved.storage_limit_enabled,
            "seconds preferences not persisted"
        );
        anyhow::ensure!(
            app.backend.seconds_policy.lock().unwrap().days.is_none()
                && app
                    .backend
                    .storage_policy
                    .lock()
                    .unwrap()
                    .seconds_days
                    .is_none(),
            "long term was not applied to background cleanup"
        );
        app.act(
            Action::Storage(StorageAction::ResetStorageSettings),
            window,
            cx,
        );
        anyhow::ensure!(
            app.config.seconds_days_enabled
                && app.config.seconds_days == 3
                && app.config.seconds_enabled
                && !app.config.storage_limit_enabled,
            "reset did not restore three day collection defaults"
        );
        app.act(
            Action::Storage(StorageAction::CancelDataDirectory),
            window,
            cx,
        );
        Ok::<_, anyhow::Error>(())
    })??;
    for theme in ["light", "dark"] {
        for width in [900., 1240.] {
            for page in [Page::Apps, Page::History] {
                this.update_in(cx, |app, window, cx| {
                    window.resize(size(px(width), px(700.)));
                    app.act(Action::Theme(theme), window, cx);
                    app.act(Action::Nav(page), window, cx);
                    seed(app, false, false);
                    app.act(
                        Action::Application(ApplicationAction::OpenUsageDetail(
                            app.report.rows[0].key.clone(),
                        )),
                        window,
                        cx,
                    );
                    seed(app, false, false);
                    let stats = SpeedStats {
                        bytes: Bytes {
                            down: 80_000_000_000,
                            up: 40_000_000_000,
                        },
                        covered_ms: 120_000,
                        expected_ms: 600_000,
                        first: Some(app.query.from),
                        as_of: Some(app.query.from + 600_000),
                        down_peak: Some(Peak {
                            rate: 80_000_000_000.,
                            at: app.query.from,
                        }),
                        up_peak: Some(Peak {
                            rate: 40_000_000_000.,
                            at: app.query.from + 1000,
                        }),
                    };
                    app.report.speeds = stats.clone();
                    app.detail.speeds = stats;
                    let context = app.active_detail_context();
                    app.open_seconds(context, window, cx);
                    let old = app.seconds.pending.clone().unwrap();
                    app.open_seconds(
                        Some(apps_page::DetailContext {
                            key: "different.exe".into(),
                            group_apps: false,
                        }),
                        window,
                        cx,
                    );
                    *app.backend.speeds.reply.lock().unwrap() = Some(SecondsReply {
                        request: old,
                        result: Err("stale object result".into()),
                    });
                    app.poll_seconds(cx);
                    anyhow::ensure!(
                        app.seconds.error != "stale object result"
                            && app.seconds.scope.as_ref().unwrap().app == "different.exe",
                        "old object result accepted"
                    );
                    app.open_seconds(app.active_detail_context(), window, cx);
                    let mut shown_request = app.seconds.pending.take().unwrap();
                    let from = app.query.from + WINDOW;
                    let view = crate::traffic_drill::View {
                        from,
                        to: from + WINDOW,
                        seconds: true,
                    };
                    app.seconds.view = Some(view);
                    app.seconds.path = vec![view];
                    shown_request.view = Some(view);
                    shown_request.from = Some(from);
                    app.seconds.loaded_request = Some(shown_request);
                    app.seconds.data = Some(SecondsData {
                        trend: Some(crate::history::fixture(from, from + WINDOW, from + WINDOW)),
                        from,
                        to: from + WINDOW,
                        first: Some(from),
                        last: Some(from + WINDOW),
                        samples: (0..250)
                            .filter(|i| !(100..125).contains(i))
                            .map(|i| crate::speed_stats::SecondSample {
                                seq: i,
                                from: from + i * 1000,
                                to: from + (i + 1) * 1000,
                                duration_ms: 1000,
                                down: ((i % 19) + 1) as f64 * 1_000_000_000.,
                                up: ((i % 7) + 1) as f64 * 500_000_000.,
                                valid: true,
                            })
                            .collect(),
                    });
                    let shared = app.backend.shared.lock().unwrap();
                    app.seconds.displayed =
                        Some((shared.storage_revision, shared.seconds_status.revision));
                    drop(shared);
                    cx.notify();
                    Ok::<_, anyhow::Error>(())
                })??;
                settle_layout(this, cx).await?;
                super::drill_time_tests::exercise(this, cx).await?;
                if std::env::var_os("MIHOMO_DRILL_PREVIEW").is_some()
                    && theme == "light"
                    && width == 1240.
                    && page == Page::Apps
                {
                    this.update_in(cx, |app, _, cx| {
                        app.seconds.hover = Some(crate::traffic_drill::Target::Second(2));
                        app.seconds.pinned = true;
                        app.seconds.hover_data = Some(crate::traffic_drill::HoverData {
                            from: app.seconds.data.as_ref().unwrap().from + 2000,
                            to: app.seconds.data.as_ref().unwrap().from + 3000,
                            duration_ms: 1000,
                            down: 5000.,
                            up: 1000.,
                            applications: (0..8)
                                .map(|i| Row {
                                    key: format!("应用{i}.exe"),
                                    bytes: Bytes { down: 500, up: 100 },
                                })
                                .collect(),
                            ..Default::default()
                        });
                        cx.notify();
                    })?;
                    println!("drill visual preview ready");
                    Timer::after(Duration::from_secs(45)).await;
                    this.update_in(cx, |app, _, _| {
                        app.seconds.hover = None;
                        app.seconds.pinned = false;
                        app.seconds.hover_data = None;
                    })?;
                }
                this.update_in(cx, |app, window, cx| {
                    let bounds = app.seconds.bounds.get();
                    anyhow::ensure!(
                        bounds.size.width > px(700.)
                            && bounds.left() >= px(0.)
                            && bounds.right() <= window.viewport_size().width,
                        "seconds modal width overflow: {theme} {width} {bounds:?}"
                    );
                    anyhow::ensure!(
                        bounds.top() >= px(0.) && bounds.bottom() <= window.viewport_size().height,
                        "seconds modal height overflow: {bounds:?}"
                    );
                    let minutes = app.seconds.minute_bounds.get();
                    let seconds = app.seconds.second_bounds.get();
                    anyhow::ensure!(
                        minutes.size.width > px(200.)
                            && (minutes.left() - seconds.left()).abs() < px(1.)
                            && (minutes.right() - seconds.right()).abs() < px(1.),
                        "minute and second plots are not aligned: {minutes:?} {seconds:?}"
                    );
                    let target = crate::traffic_drill::Target::Second(1);
                    app.hover_drill(target, true, false, cx);
                    let old_hover = app.seconds.hover_pending.clone().unwrap();
                    app.hover_drill(crate::traffic_drill::Target::Second(2), true, false, cx);
                    *app.backend.speeds.hover_reply.lock().unwrap() =
                        Some(crate::traffic_drill::HoverReply {
                            request: old_hover,
                            result: Err("stale hover".into()),
                        });
                    app.poll_seconds(cx);
                    anyhow::ensure!(
                        app.seconds.hover_error != "stale hover"
                            && app.seconds.hover == Some(crate::traffic_drill::Target::Second(2)),
                        "stale hover replaced current app list"
                    );
                    let current = app.seconds.hover_pending.clone().unwrap();
                    *app.backend.speeds.hover_reply.lock().unwrap() =
                        Some(crate::traffic_drill::HoverReply {
                            request: current,
                            result: Ok(crate::traffic_drill::HoverData {
                                from: app.seconds.data.as_ref().unwrap().from + 2000,
                                to: app.seconds.data.as_ref().unwrap().from + 3000,
                                duration_ms: 1000,
                                down: 5000.,
                                up: 1000.,
                                applications: (0..8)
                                    .map(|i| Row {
                                        key: format!("应用{i}.exe"),
                                        bytes: Bytes { down: 500, up: 100 },
                                    })
                                    .collect(),
                                ..Default::default()
                            }),
                        });
                    app.poll_seconds(cx);
                    app.hover_drill(crate::traffic_drill::Target::Second(2), true, true, cx);
                    app.act(Action::Drill(DrillAction::DrillExpand), window, cx);
                    anyhow::ensure!(
                        app.seconds.pinned
                            && app.seconds.expanded
                            && app.seconds.hover_data.as_ref().unwrap().applications.len() == 8,
                        "pinned app expansion lost rows"
                    );
                    let parent = (
                        app.query.clone(),
                        app.usage_detail.key.clone(),
                        app.selected.clone(),
                        app.page_scroll.offset(),
                        app.usage_detail.scroll.offset(),
                    );
                    app.act(Action::Drill(DrillAction::SecondsMove(WINDOW)), window, cx);
                    let next = app.seconds.pending.as_ref().unwrap().from.unwrap();
                    app.act(Action::Drill(DrillAction::SecondsMove(WINDOW)), window, cx);
                    anyhow::ensure!(
                        app.seconds.pending.as_ref().unwrap().from == Some(next + WINDOW),
                        "rapid navigation reused old anchor"
                    );
                    app.close_seconds(window, cx);
                    anyhow::ensure!(
                        parent
                            == (
                                app.query.clone(),
                                app.usage_detail.key.clone(),
                                app.selected.clone(),
                                app.page_scroll.offset(),
                                app.usage_detail.scroll.offset()
                            ),
                        "closing modal changed parent context"
                    );
                    app.open_seconds(None, window, cx);
                    anyhow::ensure!(
                        app.seconds.scope.as_ref().unwrap().app.is_empty(),
                        "global entry kept application filter"
                    );
                    Ok::<_, anyhow::Error>(())
                })??;
                cx.update(|window, cx| {
                    window.dispatch_keystroke(Keystroke::parse("escape").unwrap(), cx)
                })?;
                this.update_in(cx, |app, _, _| {
                    anyhow::ensure!(
                        app.seconds.scope.is_none() && app.usage_detail_open(),
                        "Esc closed parent detail instead of modal"
                    );
                    Ok::<_, anyhow::Error>(())
                })??;
            }
        }
    }
    this.update_in(cx, |app, window, cx| {
        app.act(Action::Nav(Page::Apps), window, cx);
        seed(app, false, false);
        app.act(
            Action::Application(ApplicationAction::Select(app.report.rows[0].key.clone())),
            window,
            cx,
        );
        seed(app, false, false);
        let selected = app.selected.clone();
        app.open_seconds(app.active_detail_context(), window, cx);
        anyhow::ensure!(
            app.seconds.scope.as_ref().unwrap().app == selected,
            "full detail seconds source wrong"
        );
        app.close_seconds(window, cx);
        anyhow::ensure!(app.selected == selected, "modal left full detail");
        app.act(Action::Nav(Page::Apps), window, cx);
        Ok::<_, anyhow::Error>(())
    })??;
    println!(
        "speed/seconds UI passed: defaults, persistence, large values, both themes and widths, 10-minute navigation, stale replies, modal Esc and parent context"
    );
    Ok(())
}
