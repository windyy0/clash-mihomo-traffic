use super::*;

pub(super) async fn exercise(
    this: &WeakEntity<TrafficApp>,
    cx: &mut AsyncWindowContext,
) -> anyhow::Result<()> {
    let (_requests, original_backend) = this.update_in(cx, |app, _, _| {
        let original = app.backend.clone();
        (app.backend.analysis_fixture_queue(), original)
    })?;
    for theme in ["light", "dark"] {
        for width in [900., 1240., 1600.] {
            this.update_in(cx, |app, window, cx| {
                window.resize(size(px(width), px(700.)));
                app.act(Action::Theme(theme), window, cx);
                app.act(Action::Nav(Page::History), window, cx);
                app.act(
                    Action::History(HistoryAction::Tab(HistoryTab::Records)),
                    window,
                    cx,
                );
                app.act(
                    Action::History(HistoryAction::Period(TimePeriod::Yesterday)),
                    window,
                    cx,
                );
                seed(app, false, false);
                app.history_state.restore_offset = None;
                app.analysis.restore_offset = None;
                app.page_scroll.set_offset(point(px(0.), px(0.)));
                cx.notify();
            })?;
            settle_layout(this, cx).await?;
            let (query, view, month, stack, record_offset, request) =
                this.update_in(cx, |app, window, cx| {
                    let offset = point(px(0.), -app.page_scroll.max_offset().height.min(px(90.)));
                    app.page_scroll.set_offset(offset);
                    let context = (
                        app.query.clone(),
                        app.history_state.view,
                        app.history_state.month,
                        app.history_state.range_stack.clone(),
                        offset,
                    );
                    app.analysis.search_value = "stale search".into();
                    app.analysis.focus = Some("stale.exe".into());
                    app.analysis.app_page = 2;
                    app.analysis.burst_page = 1;
                    app.analysis.show_trends = true;
                    app.act(
                        Action::History(HistoryAction::Tab(HistoryTab::Analysis)),
                        window,
                        cx,
                    );
                    let request = app.analysis.pending.clone().unwrap();
                    anyhow::ensure!(
                        app.history_state.analysis_range == (context.0.from, context.0.to)
                            && !app.analysis.show_trends
                            && app.analysis.focus.is_none()
                            && app.analysis.search_value.is_empty()
                            && app.analysis.search.read(cx).value().is_empty()
                            && app.analysis.app_page == 0
                            && app.analysis.burst_page == 0,
                        "comparison entry did not follow selection and reset browsing state"
                    );
                    app.analysis.snapshot = Some(std::sync::Arc::new(
                        crate::analysis::Snapshot::fixture(request.clone()),
                    ));
                    app.analysis.pending = None;
                    app.analysis.error.clear();
                    seed(app, false, false);
                    cx.notify();
                    Ok((
                        context.0, context.1, context.2, context.3, context.4, request,
                    ))
                })??;
            settle_layout(this, cx).await?;
            let usage_offset = this.update_in(cx, |app, window, cx| {
                anyhow::ensure!(
                    app.query == query
                        && app.analysis.bounds[1].get().size.height == px(0.)
                        && app.analysis.summary_bounds.get().size.height > px(60.)
                        && app.analysis.bounds[2].get().top() + px(160.)
                            < window.viewport_size().height
                        && app.page_scroll.max_offset().width <= px(1.),
                    "comparison first screen missed conclusion/application rows: {theme} {width}"
                );
                let offset = point(px(0.), -app.page_scroll.max_offset().height.min(px(100.)));
                app.page_scroll.set_offset(offset);
                app.act(Action::Analysis(AnalysisAction::Details(true)), window, cx);
                cx.notify();
                Ok(offset)
            })??;
            settle_layout(this, cx).await?;
            let trend_offset = this.update_in(cx, |app, window, cx| {
                anyhow::ensure!(
                    app.analysis.bounds[1].get().size.height > px(150.)
                        && app.analysis.bounds[3].get().size.height > px(0.)
                        && app.analysis.bounds[2].get().size.height == px(0.)
                        && app.analysis.pending.is_none(),
                    "trend tab missing chart/events or resampled"
                );
                let offset = point(px(0.), -app.page_scroll.max_offset().height.min(px(120.)));
                app.page_scroll.set_offset(offset);
                app.act(Action::Analysis(AnalysisAction::Details(false)), window, cx);
                cx.notify();
                Ok(offset)
            })??;
            settle_layout(this, cx).await?;
            this.update_in(cx, |app, window, cx| {
                anyhow::ensure!(
                    app.page_scroll.offset() == usage_offset,
                    "usage tab lost scroll position"
                );
                app.act(Action::Analysis(AnalysisAction::Details(true)), window, cx);
                cx.notify();
                Ok(())
            })??;
            settle_layout(this, cx).await?;
            this.update_in(cx, |app, window, cx| {
                anyhow::ensure!(
                    app.page_scroll.offset() == trend_offset,
                    "trend tab lost scroll position"
                );
                let snapshot = app.analysis.snapshot.clone().unwrap();
                app.act(Action::Analysis(AnalysisAction::Details(false)), window, cx);
                app.act(
                    Action::Analysis(AnalysisAction::Focus(Some(snapshot.rows[0].key.clone()))),
                    window,
                    cx,
                );
                anyhow::ensure!(
                    app.analysis.show_trends
                        && app.analysis.focus.as_ref() == Some(&snapshot.rows[0].key)
                        && app.analysis_rows().len() == snapshot.rows.len()
                        && std::sync::Arc::ptr_eq(
                            &snapshot,
                            app.analysis.snapshot.as_ref().unwrap()
                        ),
                    "application focus did not open scoped trends on the shared snapshot"
                );
                cx.notify();
                Ok(())
            })??;
            settle_layout(this, cx).await?;
            this.update_in(cx, |app, window, cx| {
                let scope = app.analysis.bounds[1].get();
                anyhow::ensure!(
                    scope.top() >= app.page_scroll.bounds().top()
                        && scope.top() <= app.page_scroll.bounds().top() + px(24.),
                    "application focus scrolled its scope title out of view: {scope:?}"
                );
                let time = app.analysis.snapshot.as_ref().unwrap().current_from();
                app.act(Action::Analysis(AnalysisAction::Pin(time)), window, cx);
                let escape = KeystrokeEvent {
                    keystroke: Keystroke::parse("escape").unwrap(),
                    action: None,
                    context_stack: Default::default(),
                };
                app.history_keystroke(&escape, window, cx);
                anyhow::ensure!(
                    app.analysis.pinned_bucket.is_none()
                        && app.history_state.tab == HistoryTab::Analysis,
                    "Escape navigated back before clearing pinned details"
                );
                app.history_keystroke(&escape, window, cx);
                anyhow::ensure!(
                    app.history_state.tab == HistoryTab::Records
                        && app.query == query
                        && app.history_state.view == view
                        && app.history_state.month == month
                        && app.history_state.range_stack == stack,
                    "comparison return lost history context"
                );
                cx.notify();
                Ok(())
            })??;
            settle_layout(this, cx).await?;
            this.update_in(cx, |app, window, cx| {
                anyhow::ensure!(
                    app.page_scroll.offset() == record_offset,
                    "history scroll was not restored"
                );
                app.act(
                    Action::History(HistoryAction::Tab(HistoryTab::Analysis)),
                    window,
                    cx,
                );
                let current = app.analysis.pending.clone().unwrap();
                anyhow::ensure!(
                    current.id > request.id && !app.analysis.show_trends,
                    "reopening comparison did not request a new snapshot"
                );
                let snapshot = std::sync::Arc::new(crate::analysis::Snapshot::fixture(current));
                app.analysis.snapshot = Some(snapshot.clone());
                app.analysis.pending = None;
                app.act(Action::Analysis(AnalysisAction::Refresh), window, cx);
                let refresh = app.analysis.pending.clone().unwrap();
                *app.backend.analysis_result.lock().unwrap() =
                    Some(crate::backend::AnalysisReply {
                        request: refresh.clone(),
                        result: Err("refresh failed".into()),
                        complete: true,
                    });
                app.poll_analysis(cx);
                anyhow::ensure!(
                    std::sync::Arc::ptr_eq(&snapshot, app.analysis.snapshot.as_ref().unwrap())
                        && app.analysis.pending.is_none()
                        && !app.analysis.error.is_empty(),
                    "same-range refresh failure discarded the existing snapshot"
                );
                app.act(
                    Action::History(HistoryAction::Period(TimePeriod::LastSevenDays)),
                    window,
                    cx,
                );
                let next = app.analysis.pending.clone().unwrap();
                anyhow::ensure!(
                    app.analysis.snapshot.is_none(),
                    "changed range displayed old totals"
                );
                *app.backend.analysis_result.lock().unwrap() =
                    Some(crate::backend::AnalysisReply {
                        request: refresh,
                        result: Ok(snapshot),
                        complete: true,
                    });
                app.poll_analysis(cx);
                anyhow::ensure!(
                    app.analysis.snapshot.is_none() && app.analysis.pending.as_ref() == Some(&next),
                    "stale result replaced the new range"
                );
                *app.backend.analysis_result.lock().unwrap() =
                    Some(crate::backend::AnalysisReply {
                        request: next,
                        result: Err("range failed".into()),
                        complete: true,
                    });
                app.poll_analysis(cx);
                anyhow::ensure!(
                    app.analysis.snapshot.is_none() && app.analysis.pending.is_none(),
                    "failed range restored unrelated old totals"
                );
                *app.backend.analysis_result.lock().unwrap() = None;
                app.act(
                    Action::History(HistoryAction::Tab(HistoryTab::Records)),
                    window,
                    cx,
                );
                Ok(())
            })??;
            println!(
                "comparison secondary page navigation, tabs and snapshot isolation passed: {theme} {width}"
            );
        }
    }
    this.update_in(cx, |app, _, _| {
        app.backend = original_backend;
    })?;
    Ok(())
}
