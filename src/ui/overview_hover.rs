use super::*;
use crate::overview_usage::{Bin, Request};
use chrono::Datelike;

#[derive(Clone, Copy)]
pub(super) struct Hover {
    pub index: usize,
    pub bin: Bin,
    pub scope: (i64, i64),
    pub source: (u64, u64),
    pub pinned: bool,
}

fn caption(from: i64, to: i64) -> String {
    let Some(start) =
        chrono::DateTime::from_timestamp_millis(from).map(|v| v.with_timezone(&chrono::Local))
    else {
        return String::new();
    };
    let Some(end) = chrono::DateTime::from_timestamp_millis(to.saturating_sub(1))
        .map(|v| v.with_timezone(&chrono::Local))
    else {
        return String::new();
    };
    if to - from <= 60_000 {
        return format!("{} · 1分钟", start.format("%m/%d %H:%M"));
    }
    let (left, right) = if start.date_naive() == end.date_naive() {
        ("%m/%d %H:%M", "%H:%M")
    } else if start.year() == end.year() {
        ("%m/%d %H:%M", "%m/%d %H:%M")
    } else {
        ("%Y/%m/%d %H:%M", "%Y/%m/%d %H:%M")
    };
    format!("{} — {}", start.format(left), end.format(right))
}

impl TrafficApp {
    pub(super) fn overview_bins(&self) -> Vec<Bin> {
        if self.report.query.as_ref() != Some(&self.query) {
            return Vec::new();
        }
        crate::overview_usage::bins(
            &self.report.series,
            self.query.from,
            self.query.to,
            chrono::Utc::now().timestamp_millis(),
        )
    }
    pub(super) fn close_overview_hover(&mut self, cx: &mut Context<Self>) {
        let state = &mut self.overview_state;
        state.hover = None;
        state.pending = None;
        state.hover_data = None;
        state.hover_error.clear();
        state.close_sequence += 1;
        cx.notify();
    }
    pub(super) fn dismiss_overview_hover(
        &mut self,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.overview_state.hover_suppressed = self.overview_state.hover.and_then(|hover| {
            self.overview_state
                .bar_bounds
                .borrow()
                .get(&hover.index)
                .filter(|bounds| bounds.contains(&position))
                .map(|_| hover.index)
        });
        self.close_overview_hover(cx);
    }
    pub(super) fn hover_overview_usage(
        &mut self,
        index: usize,
        entered: bool,
        cx: &mut Context<Self>,
    ) {
        if !entered {
            if self.overview_state.hover_suppressed == Some(index) {
                self.overview_state.hover_suppressed = None;
            }
            self.defer_overview_close(cx);
            return;
        }
        if self.overview_state.hover_suppressed == Some(index) {
            return;
        }
        self.overview_state.hover_suppressed = None;
        if self.overview_state.hover.is_some_and(|h| h.pinned) {
            return;
        }
        self.overview_state.close_sequence += 1;
        let Some(bin) = self.overview_bins().get(index).copied() else {
            return;
        };
        if self
            .overview_state
            .hover
            .is_some_and(|h| h.index == index && h.bin == bin)
        {
            return;
        }
        let source = {
            let state = self.backend.shared.lock().unwrap();
            (state.storage_revision, state.storage_epoch)
        };
        self.overview_state.hover = Some(Hover {
            index,
            bin,
            scope: (self.query.from, self.query.to),
            source,
            pinned: false,
        });
        self.overview_state.hover_data = None;
        self.overview_state.hover_error.clear();
        self.overview_state.sequence += 1;
        let request = Request {
            id: self.overview_state.sequence,
            source,
            from: bin.from,
            to: bin.to,
        };
        let cached = self
            .overview_state
            .cache
            .get(&(bin.from, bin.to))
            .filter(|(at, _)| {
                bin.to < chrono::Utc::now().timestamp_millis() - 120_000
                    || at.elapsed() < Duration::from_secs(5)
            })
            .cloned();
        if let Some((_, detail)) = cached {
            self.overview_state.hover_data = Some(detail);
            self.overview_state.pending = None;
        } else {
            self.overview_state.pending = Some(request.clone());
            self.backend.overview_usage.query(request);
        }
        cx.notify();
    }
    pub(super) fn pin_overview_usage(&mut self, index: usize, cx: &mut Context<Self>) {
        if self
            .overview_state
            .hover
            .is_some_and(|h| h.pinned && h.index == index)
        {
            self.close_overview_hover(cx);
            return;
        }
        if let Some(hover) = &mut self.overview_state.hover
            && hover.index == index
        {
            hover.pinned = true;
            cx.notify();
            return;
        }
        self.overview_state.hover = None;
        self.hover_overview_usage(index, true, cx);
        if let Some(hover) = &mut self.overview_state.hover {
            hover.pinned = true;
        }
        cx.notify();
    }
    fn defer_overview_close(&mut self, cx: &mut Context<Self>) {
        if self.overview_state.hover.is_none_or(|h| h.pinned) {
            return;
        }
        self.overview_state.close_sequence += 1;
        let sequence = self.overview_state.close_sequence;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(220))
                .await;
            let _ = this.update(cx, |app, cx| {
                if app.overview_state.close_sequence == sequence
                    && app.overview_state.hover.is_some_and(|h| !h.pinned)
                {
                    app.close_overview_hover(cx);
                }
            });
        })
        .detach();
    }
    pub(super) fn poll_overview(&mut self, cx: &mut Context<Self>) {
        let (source, subscriptions) = {
            let state = self.backend.shared.lock().unwrap();
            (
                (state.storage_revision, state.storage_epoch),
                state.subscriptions.clone(),
            )
        };
        self.data.subscriptions = subscriptions;
        if self.overview_state.subscription_scope != self.data.subscriptions.scope {
            self.overview_state.subscription_scope = self.data.subscriptions.scope.clone();
            self.overview_state.show_saved = false;
            self.overview_state
                .subscription_scroll
                .set_offset(point(px(0.), px(0.)));
        }
        if self.overview_state.usage_source != Some(source) {
            self.overview_state.hover_suppressed = None;
            self.overview_state.usage_source = Some(source);
            self.overview_state.cache.clear();
            self.close_overview_hover(cx);
        }
        if self.page != Page::Overview {
            self.overview_state.hover_suppressed = None;
        }
        if self.overview_state.hover.is_some_and(|h| {
            self.page != Page::Overview
                || h.scope != (self.query.from, self.query.to)
                || h.source != source
        }) {
            self.close_overview_hover(cx);
        }
        let reply = self.backend.overview_usage.reply.lock().unwrap().take();
        if let Some(reply) = reply
            && self.overview_state.pending.as_ref() == Some(&reply.request)
            && self.overview_state.hover.is_some_and(|h| {
                h.source == source
                    && h.bin.from == reply.request.from
                    && h.bin.to == reply.request.to
            })
        {
            self.overview_state.pending = None;
            match reply.result {
                Ok(detail) => {
                    if self.overview_state.cache.len() >= 24 {
                        self.overview_state.cache.clear();
                    }
                    self.overview_state.cache.insert(
                        (reply.request.from, reply.request.to),
                        (Instant::now(), detail.clone()),
                    );
                    self.overview_state.hover_data = Some(detail);
                }
                Err(error) => self.overview_state.hover_error = error,
            }
            cx.notify();
        }
    }
    pub(super) fn open_overview_range(
        &mut self,
        bin: Bin,
        application: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let source = {
            let state = self.backend.shared.lock().unwrap();
            (state.storage_revision, state.storage_epoch)
        };
        if self.page != Page::Overview
            || self.report.query.as_ref() != Some(&self.query)
            || bin.from < self.query.from
            || bin.to > self.query.to
            || bin.to <= bin.from
            || self.overview_state.usage_source != Some(source)
        {
            return;
        }
        self.close_overview_hover(cx);
        self.act(Action::Nav(Page::History), window, cx);
        self.act(
            Action::History(HistoryAction::Tab(HistoryTab::Records)),
            window,
            cx,
        );
        self.query.dimension = "app".into();
        self.query.group_apps = false;
        self.query.app.clear();
        self.query.search.clear();
        self.selected.clear();
        self.last_search.clear();
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.history_state.record_period = TimePeriod::Custom;
        let date = crate::history::local_date(bin.from);
        self.history_state.month = date.with_day(1).unwrap();
        self.history_state.anchor = date;
        self.history_state.cursor = date;
        self.history_state.time_initialized = false;
        self.page_scroll.set_offset(point(px(0.), px(0.)));
        self.set_history_range((bin.from, bin.to), true);
        self.history_state.record_query = self.query.clone();
        if let Some(application) = application {
            self.open_usage_detail(application);
        }
        cx.notify();
    }
    pub(super) fn overview_usage_overlay(
        &self,
        window: &Window,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let hover = self.overview_state.hover.filter(|h| {
            self.page == Page::Overview
                && h.scope == (self.query.from, self.query.to)
                && self.report.query.as_ref() == Some(&self.query)
        })?;
        let bounds = *self.overview_state.bar_bounds.borrow().get(&hover.index)?;
        let p = self.p;
        let detail = self.overview_state.hover_data.as_ref();
        let text_width = |text: &str, size: f32, weight: FontWeight| {
            let mut style = window.text_style();
            style.font_family = "Microsoft YaHei UI".into();
            style.font_weight = weight;
            window
                .text_system()
                .shape_line(
                    text.to_owned().into(),
                    px(size),
                    &[style.to_run(text.len())],
                    None,
                )
                .width
        };
        let hint = if hover.pinned {
            "应用点击或右键查看历史 · Esc 收起"
        } else {
            "左键固定 · 右键查看时段历史"
        };
        let total = detail.map_or(hover.bin.bytes, |d| d.total);
        let directions = text_width(
            &format!("↓ {}", human_bytes(total.down)),
            11.,
            FontWeight::NORMAL,
        ) + px(12.)
            + text_width(
                &format!("↑ {}", human_bytes(total.up)),
                11.,
                FontWeight::NORMAL,
            );
        let mut needed = (text_width(
            &caption(hover.bin.from, hover.bin.to),
            12.,
            FontWeight::MEDIUM,
        ) + px(26.))
        .max(
            directions
                + px(16.)
                + text_width(
                    &format!("合计 {}", human_bytes(total.total())),
                    12.,
                    FontWeight::MEDIUM,
                ),
        )
        .max(text_width(hint, 10.5, FontWeight::NORMAL));
        if let Some(detail) = detail {
            for item in &detail.applications {
                needed = needed.max(
                    px(24.)
                        + text_width(self.application_name(&item.key), 12., FontWeight::MEDIUM)
                            .min(px(144.))
                        + px(8.)
                        + text_width(&human_bytes(item.bytes.total()), 11., FontWeight::NORMAL),
                );
                needed = needed.max(
                    px(24.)
                        + text_width(
                            &format!("↓ {}", human_bytes(item.bytes.down)),
                            10.5,
                            FontWeight::NORMAL,
                        )
                        + px(10.)
                        + text_width(
                            &format!("↑ {}", human_bytes(item.bytes.up)),
                            10.5,
                            FontWeight::NORMAL,
                        ),
                );
            }
            needed = needed.max(
                text_width(
                    &format!("{} 个应用 · 显示前5项", detail.count),
                    10.5,
                    FontWeight::NORMAL,
                ) + px(8.)
                    + text_width("时段历史 ›", 11., FontWeight::NORMAL)
                    + px(8.),
            );
        }
        let width = px(((f32::from(needed) + 26.) / 8.).ceil() * 8.)
            .clamp(px(256.), px(336.))
            .min(window.viewport_size().width - px(32.));
        let left = bounds
            .left()
            .min(window.viewport_size().width - width - px(16.))
            .max(px(16.));
        let below = window.viewport_size().height - bounds.bottom() - px(23.);
        let above = bounds.top() - px(23.);
        let (top, max_height) = if below >= px(220.) || below >= above {
            (bounds.bottom() + px(7.), below)
        } else {
            (px(16.), above)
        };
        let measured = self.overview_state.hover_bounds.clone();
        let bin = hover.bin;
        let mut panel = column()
            .id("overview-usage-hover")
            .absolute()
            .left(left)
            .top(top)
            .w(width)
            .max_h(max_height.max(px(120.)))
            .overflow_y_scroll()
            .p(px(12.))
            .font_family("Microsoft YaHei UI")
            .gap(px(7.))
            .rounded(px(10.))
            .border_1()
            .border_color(control_border(p))
            .bg(popup_surface(p))
            .occlude()
            .on_hover(cx.listener(|app, entered: &bool, _, cx| {
                if *entered {
                    app.overview_state.close_sequence += 1;
                } else {
                    app.defer_overview_close(cx);
                }
            }))
            .child(
                canvas(move |rect, _, _| measured.set(rect), |_, _, _, _| {})
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
            )
            .child(
                row()
                    .justify_between()
                    .gap(px(6.))
                    .child(
                        label(caption(bin.from, bin.to), p.text)
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM),
                    )
                    .child(
                        self.button(
                            "overview-usage-close",
                            "×",
                            false,
                            Action::Overview(OverviewAction::CloseHover),
                            cx,
                        )
                        .h(px(20.))
                        .px(px(4.)),
                    ),
            );
        if let Some(detail) = detail {
            if detail.observed {
                panel = panel
                    .child(
                        row()
                            .justify_between()
                            .gap(px(16.))
                            .child(
                                row()
                                    .gap(px(12.))
                                    .child(
                                        label(
                                            format!("↓ {}", human_bytes(detail.total.down)),
                                            p.download_text,
                                        )
                                        .text_size(px(11.)),
                                    )
                                    .child(
                                        label(
                                            format!("↑ {}", human_bytes(detail.total.up)),
                                            p.upload_text,
                                        )
                                        .text_size(px(11.)),
                                    ),
                            )
                            .child(
                                label(
                                    format!("合计 {}", human_bytes(detail.total.total())),
                                    p.text,
                                )
                                .text_size(px(12.))
                                .font_weight(FontWeight::MEDIUM)
                                .flex_none(),
                            ),
                    )
                    .child(div().h(px(1.)).bg(surface_border(p)));
                for (index, application) in detail.applications.iter().enumerate() {
                    let key = application.key.clone();
                    let right_key = key.clone();
                    panel = panel.child(
                        column()
                            .id(SharedString::from(format!("overview-hover-app-{index}")))
                            .gap(px(2.))
                            .when(index > 0, |d| {
                                d.pt(px(5.)).border_t_1().border_color(surface_border(p))
                            })
                            .cursor_pointer()
                            .hover(move |d| d.bg(state_surface(p)))
                            .rounded(px(4.))
                            .child(
                                row()
                                    .gap(px(6.))
                                    .child(self.application_icon(&key, true))
                                    .child(
                                        label(self.application_name(&key).to_string(), p.text)
                                            .text_size(px(12.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .flex_1()
                                            .min_w_0()
                                            .truncate(),
                                    )
                                    .child(
                                        label(human_bytes(application.bytes.total()), p.text)
                                            .text_size(px(11.)),
                                    ),
                            )
                            .child(
                                row()
                                    .pl(px(24.))
                                    .gap(px(10.))
                                    .child(
                                        label(
                                            format!("↓ {}", human_bytes(application.bytes.down)),
                                            p.download_text,
                                        )
                                        .text_size(px(10.5)),
                                    )
                                    .child(
                                        label(
                                            format!("↑ {}", human_bytes(application.bytes.up)),
                                            p.upload_text,
                                        )
                                        .text_size(px(10.5)),
                                    ),
                            )
                            .on_click(cx.listener(move |app, _, window, cx| {
                                app.open_overview_range(bin, Some(key.clone()), window, cx)
                            }))
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(move |app, _, window, cx| {
                                    app.open_overview_range(
                                        bin,
                                        Some(right_key.clone()),
                                        window,
                                        cx,
                                    );
                                    cx.stop_propagation();
                                }),
                            ),
                    );
                }
                if detail.applications.is_empty() {
                    panel = panel.child(label("暂无应用明细", p.muted).text_size(px(11.)));
                }
                if detail.unassigned.total() > 0 {
                    panel = panel.child(
                        label(
                            format!("未归属 {}", human_bytes(detail.unassigned.total())),
                            p.muted,
                        )
                        .text_size(px(10.5)),
                    );
                }
                panel = panel.child(
                    row()
                        .justify_between()
                        .gap(px(8.))
                        .child(
                            label(format!("{} 个应用 · 显示前5项", detail.count), p.muted)
                                .text_size(px(10.5)),
                        )
                        .child(
                            self.button(
                                "overview-usage-history",
                                "时段历史 ›",
                                false,
                                Action::Overview(OverviewAction::RangeHistory(bin)),
                                cx,
                            )
                            .h(px(22.))
                            .px(px(4.))
                            .text_size(px(11.)),
                        ),
                );
            } else {
                panel = panel.child(label("此时段未记录流量", p.muted).text_size(px(12.)));
            }
        } else {
            panel = panel.child(
                label(
                    if self.overview_state.hover_error.is_empty() {
                        "正在读取应用用量…".into()
                    } else {
                        self.overview_state.hover_error.clone()
                    },
                    p.muted,
                )
                .text_size(px(12.)),
            );
        }
        panel = panel.child(label(hint, p.muted).text_size(px(10.5)));
        Some(deferred(panel).with_priority(2).into_any_element())
    }
}
