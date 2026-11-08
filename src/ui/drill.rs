use super::*;
use crate::speed_stats::{Scope, SecondsData, SecondsRequest, WINDOW};
use crate::traffic_drill::{HoverRequest, Target, View};
use chrono::Local;
use seconds_page::date;

const NO_MINUTE_SECONDS: &str = "此分钟暂无秒级采样，显示分钟用量";

impl TrafficApp {
    pub(super) fn open_seconds(
        &mut self,
        context: Option<apps_page::DetailContext>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let was_modal = self.seconds.is_modal();
        self.seconds.inline_history = false;
        self.close_picker(window, cx);
        self.history_state.chart_hover = None;
        let mut scope = Scope::from(&self.query);
        scope.app.clear();
        self.seconds.title = context.as_ref().map_or_else(
            || "全部流量".into(),
            |c| self.application_name(&c.key).to_string(),
        );
        if let Some(c) = context {
            scope.app = c.key;
            scope.grouped = c.group_apps;
        }
        if !was_modal {
            self.seconds.previous_focus = window.focused(cx);
        }
        self.seconds.scope = Some(scope);
        self.seconds.view = None;
        self.seconds.path.clear();
        self.seconds.scroll.set_offset(point(px(0.), px(0.)));
        self.seconds.locator.reset();
        self.seconds.focus.focus(window);
        self.request_seconds(None, cx);
    }
    pub(super) fn open_history_seconds(
        &mut self,
        at: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_picker(window, cx);
        window.blur();
        let root = self
            .history_state
            .range_stack
            .first()
            .map(|&(a, b, _)| (a, b))
            .unwrap_or((self.query.from, self.query.to));
        let scope = Scope {
            from: root.0,
            to: root.1,
            app: String::new(),
            grouped: self.query.group_apps,
        };
        let view = View::centered(at, &scope);
        self.navigate_history_seconds(scope, view, true, cx);
    }
    pub(super) fn clear_history_seconds(&mut self) {
        self.history_state.seconds_level = false;
        if self.seconds.inline_history {
            self.seconds.inline_history = false;
            self.seconds.scope = None;
            self.seconds.data = None;
            self.seconds.pending = None;
            self.seconds.loaded_request = None;
            self.seconds.displayed = None;
            self.seconds.hover = None;
            self.seconds.hover_help_open = false;
            self.seconds.hover_data = None;
            self.seconds.hover_pending = None;
        }
    }
    fn navigate_history_seconds(
        &mut self,
        scope: Scope,
        view: View,
        push: bool,
        cx: &mut Context<Self>,
    ) {
        if push {
            let revision = self.backend.storage_revision();
            if self.data.storage_revision == revision {
                self.history_state.navigation_reports.remember(
                    &self.report,
                    &self.query,
                    &self.history_state.range_stack,
                    revision,
                );
            }
            self.history_state.range_stack.push((
                self.query.from,
                self.query.to,
                self.history_state.record_period,
            ));
            self.history_state.record_period = TimePeriod::Custom;
        }
        self.set_history_range((view.from, view.to), false);
        self.history_state.seconds_level = true;
        self.seconds.inline_history = true;
        self.seconds.scope = Some(scope);
        self.seconds.title = "全部流量".into();
        self.seconds.previous_focus = None;
        self.seconds.path = vec![view];
        self.set_drill_view(view, cx);
    }
    fn resume_history_seconds(&mut self, cx: &mut Context<Self>) {
        let root = self
            .history_state
            .range_stack
            .first()
            .map(|&(a, b, _)| (a, b))
            .unwrap_or((self.query.from, self.query.to));
        let scope = Scope {
            from: root.0,
            to: root.1,
            app: String::new(),
            grouped: self.query.group_apps,
        };
        self.seconds.scope = Some(scope);
        self.seconds.inline_history = true;
        let view = View {
            from: self.query.from,
            to: self.query.to,
            seconds: true,
        };
        self.seconds.path = vec![view];
        self.set_drill_view(view, cx);
    }
    pub(super) fn open_application_drill(
        &mut self,
        from: i64,
        to: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(context) = self.active_detail_context() else {
            return;
        };
        self.open_seconds(Some(context), window, cx);
        let view = View {
            from,
            to,
            seconds: false,
        };
        self.seconds.path = vec![view];
        self.set_drill_view(view, cx);
    }
    pub(super) fn close_seconds(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.seconds.inline_history = false;
        self.seconds.locator.reset();
        self.seconds.scope = None;
        self.seconds.pending = None;
        self.seconds.data = None;
        self.seconds.displayed = None;
        self.seconds.loaded_request = None;
        self.seconds.hover = None;
        self.seconds.hover_help_open = false;
        self.seconds.hover_pending = None;
        self.seconds.hover_data = None;
        self.seconds.hover_cache.clear();
        self.seconds.path.clear();
        if let Some(focus) = self.seconds.previous_focus.take() {
            focus.focus(window);
        } else {
            window.blur();
        }
        if self.history_state.seconds_level
            && self.page == Page::History
            && self.history_state.tab == HistoryTab::Records
        {
            self.resume_history_seconds(cx);
        }
        cx.notify();
    }
    fn set_drill_view(&mut self, view: View, cx: &mut Context<Self>) {
        self.seconds.locate_open = false;
        self.seconds.locator.reset();
        self.seconds.view = Some(view);
        self.seconds.data = None;
        self.seconds.loaded_request = None;
        self.seconds.displayed = None;
        self.seconds.hover = None;
        self.seconds.hover_help_open = false;
        self.seconds.pinned = false;
        self.seconds.expanded = false;
        self.seconds.hover_data = None;
        self.seconds.hover_pending = None;
        self.seconds.hover_cache.clear();
        let Some(scope) = self.seconds.scope.clone() else {
            return;
        };
        let state = self.backend.shared.lock().unwrap();
        self.seconds.serial = self.seconds.serial.wrapping_add(1);
        let request = SecondsRequest {
            id: self.seconds.serial,
            generation: state.storage_revision,
            revision: state.seconds_status.revision,
            scope,
            from: Some(view.from),
            view: Some(view),
        };
        drop(state);
        self.seconds.sent = self.backend.query_seconds(request.clone());
        self.seconds.pending = Some(request);
        self.seconds.error.clear();
        cx.notify();
    }
    pub(super) fn request_seconds(&mut self, from: Option<i64>, cx: &mut Context<Self>) {
        let Some(scope) = self.seconds.scope.as_ref() else {
            return;
        };
        let view = if let Some(current) = self
            .seconds
            .view
            .filter(|v| from.is_none() || Some(v.from) == from)
        {
            current
        } else {
            let end = self
                .data
                .seconds_status
                .last
                .unwrap_or_else(|| chrono::Utc::now().timestamp_millis())
                .min(scope.to);
            let from = from
                .unwrap_or_else(|| end.saturating_sub(WINDOW))
                .max(scope.from);
            View {
                from,
                to: (from + WINDOW).min(scope.to),
                seconds: true,
            }
        };
        if self.seconds.path.is_empty() {
            self.seconds.path.push(view);
        }
        self.set_drill_view(view, cx);
    }
    pub(super) fn move_drill(&mut self, offset: i64, cx: &mut Context<Self>) {
        let (Some(current), Some(scope)) = (self.seconds.view, self.seconds.scope.as_ref()) else {
            return;
        };
        if !current.seconds {
            return;
        }
        let (from, to) = if offset > 0 {
            (current.to, (current.to + WINDOW).min(scope.to))
        } else {
            ((current.from - WINDOW).max(scope.from), current.from)
        };
        if to <= from || from < scope.from || to > scope.to {
            return;
        }
        let view = View {
            from,
            to,
            seconds: true,
        };
        if self.seconds.inline_history {
            let scope = self.seconds.scope.clone().unwrap();
            self.navigate_history_seconds(scope, view, false, cx);
            return;
        }
        if let Some(last) = self.seconds.path.last_mut() {
            *last = view;
        }
        self.set_drill_view(view, cx);
    }
    pub(super) fn drill_back(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(view) = self.seconds.path.get(index).copied() {
            self.seconds.path.truncate(index + 1);
            self.set_drill_view(view, cx);
        }
    }
    fn drill_bucket(&mut self, from: i64, to: i64, minute: bool, cx: &mut Context<Self>) {
        let Some(scope) = self.seconds.scope.as_ref() else {
            return;
        };
        let view = if minute {
            View::centered(from, scope)
        } else {
            View {
                from,
                to,
                seconds: false,
            }
        };
        if self.seconds.inline_history {
            let scope = self.seconds.scope.clone().unwrap();
            self.navigate_history_seconds(scope, view, false, cx);
            return;
        }
        self.seconds.path.push(view);
        self.set_drill_view(view, cx);
    }
    pub(super) fn poll_seconds(&mut self, cx: &mut Context<Self>) {
        if self.seconds.inline_history
            && (self.page != Page::History
                || !self.history_state.seconds_level
                || self
                    .seconds
                    .view
                    .is_none_or(|v| (v.from, v.to) != (self.query.from, self.query.to)))
        {
            self.clear_history_seconds();
        }
        if self.seconds.scope.is_none() {
            return;
        }
        let source = self.backend.shared.lock().unwrap();
        let matches = |a, b| a == source.storage_revision && b == source.seconds_status.revision;
        let stale = self.seconds.displayed.is_some_and(|(a, b)| !matches(a, b))
            || self
                .seconds
                .pending
                .as_ref()
                .is_some_and(|r| !matches(r.generation, r.revision));
        drop(source);
        if stale {
            self.seconds.pending = None;
            self.seconds.data = None;
            self.seconds.displayed = None;
            self.seconds.loaded_request = None;
            self.seconds.hover = None;
            self.seconds.hover_help_open = false;
            self.seconds.hover_data = None;
            self.seconds.hover_pending = None;
            self.seconds.hover_cache.clear();
            self.seconds.error = "明细已清理或恢复，请刷新。".into();
        }
        if !self.seconds.sent
            && let Some(request) = self.seconds.pending.clone()
        {
            self.seconds.sent = self.backend.query_seconds(request);
        }
        let reply = self.backend.take_seconds();
        if let Some(reply) = reply
            && self.seconds.pending.as_ref() == Some(&reply.request)
            && self.seconds.scope.as_ref() == Some(&reply.request.scope)
        {
            self.seconds.pending = None;
            match reply.result {
                Ok(data) => {
                    self.seconds.displayed =
                        Some((reply.request.generation, reply.request.revision));
                    self.seconds.loaded_request = Some(reply.request);
                    self.seconds.data = Some(data);
                }
                Err(e) => {
                    self.seconds.data = None;
                    self.seconds.error = e;
                }
            }
            cx.notify();
        }
        if !self.seconds.hover_sent
            && let Some(request) = self.seconds.hover_pending.clone()
        {
            self.seconds.hover_sent = self.backend.query_hover(request);
        }
        let reply = self.backend.take_hover();
        if let Some(reply) = reply
            && self.seconds.hover_pending.as_ref() == Some(&reply.request)
            && self.seconds.loaded_request.as_ref() == Some(&reply.request.window)
            && self.seconds.hover == Some(reply.request.target)
        {
            self.seconds.hover_pending = None;
            match reply.result {
                Ok(data) => {
                    if self.seconds.hover_cache.len() >= 128 {
                        self.seconds.hover_cache.clear();
                    }
                    self.seconds
                        .hover_cache
                        .insert(reply.request.target, data.clone());
                    self.seconds.hover_data = Some(data);
                }
                Err(error) => {
                    self.seconds.hover_data = None;
                    self.seconds.hover_error = error;
                }
            }
            cx.notify();
        }
    }
    pub(super) fn hover_drill(
        &mut self,
        target: Target,
        entered: bool,
        pin: bool,
        cx: &mut Context<Self>,
    ) {
        if self.seconds.locate_open {
            return;
        }
        if !entered {
            if !self.seconds.pinned && self.seconds.hover == Some(target) {
                self.seconds.hover = None;
                self.seconds.hover_help_open = false;
                self.seconds.hover_data = None;
                self.seconds.hover_pending = None;
                cx.notify();
            }
            return;
        }
        if self.seconds.pinned && !pin {
            return;
        }
        if self.seconds.hover == Some(target) {
            if pin {
                self.seconds.pinned = !self.seconds.pinned;
                cx.notify();
            }
            return;
        }
        self.seconds.pinned = pin;
        self.seconds.hover_help_open = false;
        self.seconds.expanded = false;
        self.seconds.hover = Some(target);
        self.seconds.hover_error.clear();
        self.seconds.hover_data = self.seconds.hover_cache.get(&target).cloned();
        self.seconds.hover_pending = None;
        if self.seconds.hover_data.is_none()
            && let Some(window) = self.seconds.loaded_request.clone()
        {
            self.seconds.serial = self.seconds.serial.wrapping_add(1);
            let request = HoverRequest {
                id: self.seconds.serial,
                window,
                target,
            };
            self.seconds.hover_sent = self.backend.query_hover(request.clone());
            self.seconds.hover_pending = Some(request);
        }
        cx.notify();
    }
    pub(super) fn drill_sample_keystroke(
        &mut self,
        event: &KeystrokeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let direction = match event.keystroke.key.as_str() {
            "left" => -1,
            "right" => 1,
            _ => return false,
        };
        if event.keystroke.modifiers != Modifiers::default()
            || !self.seconds.pinned
            || self.seconds.locate_open
            || self.picker.is_some()
            || !self.seconds.focus.is_focused(window)
            || !self.seconds.view.is_some_and(|v| v.seconds)
            || !(self.seconds.inline_history || self.seconds.is_modal())
        {
            return false;
        }
        let Some(current @ Target::Second(_)) = self.seconds.hover else {
            return false;
        };
        let Some(data) = self.seconds.data.as_ref() else {
            return false;
        };
        if let Some(next) = adjacent_sample(data, current, direction) {
            self.hover_drill(next, true, true, cx);
        }
        true
    }
    pub(super) fn locate_seconds(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let parsed = self.drill_time_result(cx);
        if let Some(at) = parsed
            && let Some(scope) = self
                .seconds
                .scope
                .as_ref()
                .filter(|s| at >= s.from && at < s.to)
        {
            let view = View::centered(at, scope);
            self.seconds.focus.focus(window);
            if self.seconds.inline_history {
                let scope = self.seconds.scope.clone().unwrap();
                self.navigate_history_seconds(scope, view, false, cx);
                return;
            }
            self.seconds.path.push(view);
            self.set_drill_view(view, cx);
        } else {
            self.seconds.locator.error = "请输入所选时段内的有效日期时间".into();
            cx.notify();
        }
    }
    pub(super) fn seconds_keystroke(
        &mut self,
        event: &KeystrokeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match event.keystroke.key.as_str() {
            "escape" => {
                if self.seconds.hover_help_open {
                    self.seconds.hover_help_open = false;
                    cx.notify();
                } else {
                    self.close_seconds(window, cx);
                }
                true
            }
            "tab" => {
                let mut handles: Vec<_> = self
                    .seconds
                    .locator
                    .parts
                    .iter()
                    .map(|f| f.focus_handle(cx))
                    .collect();
                handles.extend(self.seconds.controls.iter().cloned());
                let current = handles.iter().position(|h| h.is_focused(window));
                let next = if event.keystroke.modifiers.shift {
                    current.map_or(handles.len() - 1, |i| {
                        (i + handles.len() - 1) % handles.len()
                    })
                } else {
                    current.map_or(0, |i| (i + 1) % handles.len())
                };
                if next < 6 {
                    self.edit_drill_time(window, cx);
                }
                handles[next].focus(window);
                true
            }
            "enter" | "space" => {
                if let Some(i) = self
                    .seconds
                    .controls
                    .iter()
                    .position(|h| h.is_focused(window))
                {
                    self.act(
                        match i {
                            0 => Action::Drill(DrillAction::SecondsLocate),
                            1 => Action::Drill(DrillAction::SecondsMove(-WINDOW)),
                            2 => Action::Drill(DrillAction::SecondsMove(WINDOW)),
                            3 => Action::Drill(DrillAction::SecondsRefresh),
                            4 => Action::Drill(DrillAction::SecondsClose),
                            _ => Action::Drill(DrillAction::DrillFollowTime),
                        },
                        window,
                        cx,
                    );
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }
    pub(super) fn seconds_overlay(&self, window: &Window, cx: &Context<Self>) -> Option<Div> {
        if self.seconds.inline_history {
            return None;
        }
        let scope = self.seconds.scope.as_ref()?;
        let p = self.p;
        let width = (window.viewport_size().width - px(48.)).min(px(1040.));
        let measured = self.seconds.bounds.clone();
        let button = |id, title, action, index| {
            self.button(id, title, false, action, cx)
                .track_focus(&self.seconds.controls[index])
        };
        let mut crumbs = row().gap(px(5.)).flex_wrap();
        for (index, view) in self.seconds.path.iter().enumerate() {
            let exact = format!("{} — {}", sample_date(view.from), sample_date(view.to));
            if index > 0 {
                crumbs = crumbs.child(label("›", p.muted));
            }
            crumbs = crumbs.child(
                self.button(
                    SharedString::from(format!("drill-path-{index}")),
                    drill_time::compact_range(view.from, view.to, false),
                    index + 1 == self.seconds.path.len(),
                    Action::Drill(DrillAction::DrillBack(index)),
                    cx,
                )
                .text_size(px(11.))
                .h(px(26.))
                .tooltip(move |window, cx| Tooltip::new(exact.clone()).build(window, cx)),
            );
        }
        let view = self.seconds.view;
        let mut body = column()
            .gap(px(10.))
            .w_full()
            .child(
                row()
                    .justify_between()
                    .gap(px(12.))
                    .child(heading(self.seconds.title.clone(), p).flex_1().min_w_0())
                    .child(button(
                        "seconds-close",
                        "关闭 Esc",
                        Action::Drill(DrillAction::SecondsClose),
                        4,
                    )),
            )
            .child(crumbs)
            .child(
                row()
                    .gap(px(8.))
                    .flex_wrap()
                    .child(self.drill_locator(cx))
                    .child(
                        row()
                            .gap(px(6.))
                            .flex_none()
                            .child(
                                button(
                                    "seconds-previous",
                                    "前10分钟",
                                    Action::Drill(DrillAction::SecondsMove(-WINDOW)),
                                    1,
                                )
                                .when(
                                    !view.is_some_and(|v| v.seconds && v.from > scope.from),
                                    |d| d.opacity(0.4),
                                ),
                            )
                            .child(
                                button(
                                    "seconds-next",
                                    "后10分钟",
                                    Action::Drill(DrillAction::SecondsMove(WINDOW)),
                                    2,
                                )
                                .when(!view.is_some_and(|v| v.seconds && v.to < scope.to), |d| {
                                    d.opacity(0.4)
                                }),
                            )
                            .child(button(
                                "seconds-refresh",
                                "刷新",
                                Action::Drill(DrillAction::SecondsRefresh),
                                3,
                            )),
                    ),
            )
            .when(self.seconds.pending.is_some(), |d| {
                d.child(label("正在读取流量明细…", p.muted))
            })
            .when(!self.seconds.error.is_empty(), |d| {
                d.child(label(self.seconds.error.clone(), p.amber).whitespace_normal())
            });
        if let Some(data) = self.seconds.data.as_ref() {
            if view.is_none_or(|v| v.seconds) {
                body = body.child(self.seconds_chart(data, false, cx));
            } else if let Some(trend) = data.trend.as_ref() {
                body = body.child(self.drill_volume_chart(trend, false, cx));
            }
            let stats = crate::traffic_drill::window_stats(&data.samples, data.from, data.to);
            let saved = data.first.zip(data.last).map_or_else(
                || "暂无秒级记录".into(),
                |(a, b)| format!("秒级记录  {}", drill_time::compact_range(a, b, true)),
            );
            let exact = format!(
                "秒级记录\n{} — {}",
                data.first.map(sample_date).unwrap_or_else(|| "—".into()),
                data.last.map(sample_date).unwrap_or_else(|| "—".into())
            );
            body = body.child(
                row()
                    .gap(px(12.))
                    .flex_wrap()
                    .pt(px(6.))
                    .border_t_1()
                    .border_color(surface_border(p))
                    .child(
                        label(saved, p.muted)
                            .text_size(px(11.))
                            .id("drill-saved-range")
                            .tooltip(move |window, cx| {
                                Tooltip::new(exact.clone()).build(window, cx)
                            }),
                    )
                    .when(
                        view.is_some_and(|v| v.seconds)
                            && stats.covered_ms > 0.
                            && stats.covered_ms + 2. < (data.to - data.from) as f64,
                        |d| {
                            d.child(
                                label(
                                    format!(
                                        "有效覆盖 {:.1}%",
                                        (stats.covered_ms / (data.to - data.from).max(1) as f64
                                            * 100.)
                                            .min(100.)
                                    ),
                                    p.amber,
                                )
                                .text_size(px(11.))
                                .id("drill-coverage")
                                .tooltip(|window, cx| {
                                    Tooltip::new("峰值与均速仅按有效采样计算，采集空档不计")
                                        .build(window, cx)
                                }),
                            )
                        },
                    ),
            );
        }
        let content = self
            .card()
            .id("seconds-modal")
            .relative()
            .bg(popup_surface(p))
            .shadow_md()
            .track_focus(&self.seconds.focus)
            .w(width)
            .max_h(window.viewport_size().height - px(48.))
            .overflow_y_scroll()
            .track_scroll(&self.seconds.scroll)
            .child(
                canvas(move |bounds, _, _| measured.set(bounds), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .child(body)
            .children(self.drill_hover_card(window, cx));
        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .occlude()
                .bg(rgba(if p.dark { 0x00000099 } else { 0x24324a66 }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_down(MouseButton::Right, |_, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                })
                .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                .child(content),
        )
    }
    pub(super) fn history_seconds_tools(&self, cx: &Context<Self>) -> Div {
        let p = self.p;
        let scope = self.seconds.scope.as_ref();
        let view = self.seconds.view;
        let stats = self
            .seconds
            .data
            .as_ref()
            .map(|d| crate::traffic_drill::window_stats(&d.samples, d.from, d.to));
        let snapshot = self
            .seconds
            .data
            .as_ref()
            .and_then(|d| d.trend.as_ref())
            .map(|t| date(t.generated_at))
            .unwrap_or_else(|| "—".into());
        let refresh_hint = format!(
            "刷新10分钟明细\n快照：{snapshot}\n淡色柱：分钟用量；左轴用量，右轴速度\n曲线按图宽抽稀和平滑，悬浮读取原始采样\n峰值为实线，均速为虚线\n有效采样：{:.1}秒",
            stats.map_or(0., |s| s.covered_ms / 1000.)
        );
        let button = |id, title, action, hint: String| {
            self.button(id, title, false, action, cx)
                .w(px(24.))
                .h(px(24.))
                .px(px(0.))
                .text_size(px(18.))
                .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
        };
        row()
            .gap(px(2.))
            .child(button(
                "history-seconds-locate-open",
                "⌖",
                Action::Drill(DrillAction::DrillLocateToggle),
                self.drill_locator_time().map_or_else(
                    || "定位日期时间".into(),
                    |at| format!("定位日期时间\n{}", date(at)),
                ),
            ))
            .child(
                button(
                    "history-seconds-prev",
                    "‹",
                    Action::Drill(DrillAction::SecondsMove(-WINDOW)),
                    "前10分钟".into(),
                )
                .opacity(
                    if view.zip(scope).is_some_and(|(v, s)| v.from > s.from) {
                        1.
                    } else {
                        0.4
                    },
                ),
            )
            .child(
                button(
                    "history-seconds-next",
                    "›",
                    Action::Drill(DrillAction::SecondsMove(WINDOW)),
                    "后10分钟".into(),
                )
                .opacity(if view.zip(scope).is_some_and(|(v, s)| v.to < s.to) {
                    1.
                } else {
                    0.4
                }),
            )
            .child(button(
                "history-seconds-refresh",
                "↻",
                Action::Drill(DrillAction::SecondsRefresh),
                refresh_hint,
            ))
            .text_color(rgb(p.muted))
    }
    pub(super) fn history_seconds_content(&self, window: &Window, cx: &Context<Self>) -> Div {
        let p = self.p;
        let annual = self.history_state.view == history_state::CalendarView::Year;
        let mut body = column()
            .relative()
            .min_w_0()
            .h(px(if annual { 155. } else { 167. }))
            .gap(px(if annual { 10. } else { 6. }));
        if let Some(data) = self.seconds.data.as_ref() {
            body = body.child(self.seconds_chart(data, true, cx));
        } else {
            body = body.child(
                column().size_full().justify_center().items_center().child(
                    label(
                        if self.seconds.error.is_empty() {
                            "正在读取10分钟明细…".into()
                        } else {
                            self.seconds.error.clone()
                        },
                        p.muted,
                    )
                    .text_size(px(12.))
                    .whitespace_normal(),
                ),
            );
        }
        body = body.children(self.drill_hover_card(window, cx));
        if self.seconds.locate_open {
            body = body.child(
                self.card()
                    .absolute()
                    .top_0()
                    .right_0()
                    .w(px(400.))
                    .p(px(10.))
                    .gap(px(6.))
                    .bg(popup_surface(p))
                    .shadow_md()
                    .occlude()
                    .child(self.drill_locator(cx)),
            );
        }
        body
    }
    fn drill_volume_chart(
        &self,
        trend: &crate::history::Trend,
        compact: bool,
        cx: &Context<Self>,
    ) -> Div {
        let p = self.p;
        let height = if compact {
            if self.history_state.view == history_state::CalendarView::Year {
                48.
            } else {
                56.
            }
        } else {
            128.
        };
        let axis = if compact { 42. } else { 72. };
        let references = if compact { 78. } else { 148. };
        let gap = if compact { 6. } else { 10. };
        let data = std::sync::Arc::new(trend.clone());
        let paint = data.clone();
        let from = data.from;
        let to = data.to;
        let ceiling = chart::ChartScale::for_peak(
            data.buckets
                .iter()
                .map(|b| b.bytes.total() as f64)
                .fold(0., f64::max),
            false,
        );
        let scale = ceiling.ceiling;
        let measured = self.seconds.minute_bounds.clone();
        let mut plot = div().relative().h(px(height)).flex_1().min_w_0().child(
            canvas(
                move |b, _, _| measured.set(b),
                move |bounds, _, window, _| {
                    paint_drill_buckets(bounds, &paint, scale, p, false, window);
                },
            )
            .absolute()
            .size_full(),
        );
        plot = plot.on_mouse_move(cx.listener(move |app, event: &MouseMoveEvent, _, cx| {
            if let Some(at) =
                drill_time::cursor_time(from, to, app.seconds.minute_bounds.get(), event.position)
            {
                app.preview_drill_time(at, cx);
            }
        }));
        for (index, bucket) in data.buckets.iter().enumerate() {
            let (a, b) = (bucket.from, bucket.to);
            let minute = data.grain == crate::history::Grain::Minute;
            let hint = format!(
                "{} — {}\n下载 {}\n上传 {}\n右键{}",
                date(a),
                date(b),
                human_bytes(bucket.bytes.down),
                human_bytes(bucket.bytes.up),
                if minute {
                    "查看前后5分钟"
                } else {
                    "继续下钻"
                }
            );
            plot = plot.child(
                div()
                    .id(("drill-minute", index))
                    .absolute()
                    .top_0()
                    .h_full()
                    .left(relative((a - from) as f32 / (to - from).max(1) as f32))
                    .w(relative((b - a) as f32 / (to - from).max(1) as f32))
                    .hover(|s| s.bg(rgba(0x6688aa12)))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |app, _, window, cx| {
                            window.prevent_default();
                            cx.stop_propagation();
                            app.drill_bucket(a, b, minute, cx);
                        }),
                    )
                    .when(minute, |d| {
                        d.on_hover(cx.listener(move |app, entered: &bool, _, cx| {
                            app.hover_drill(Target::Minute(a, b), *entered, false, cx)
                        }))
                        .on_click(cx.listener(move |app, _, _, cx| {
                            app.hover_drill(Target::Minute(a, b), true, true, cx)
                        }))
                    })
                    .when(!minute, |d| {
                        d.tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
                    }),
            );
        }
        column()
            .gap(px(if compact { 4. } else { 6. }))
            .when(!compact, |d| {
                d.child(
                    label(
                        if data.grain == crate::history::Grain::Minute {
                            "分钟用量 · 右键查看前后5分钟"
                        } else {
                            "用量分布 · 右键继续下钻"
                        },
                        p.muted,
                    )
                    .text_size(px(12.)),
                )
            })
            .child(
                stretch()
                    .gap(px(gap))
                    .child(
                        column()
                            .w(px(axis))
                            .h(px(height))
                            .flex_none()
                            .justify_between()
                            .children(
                                ceiling
                                    .labels
                                    .into_iter()
                                    .enumerate()
                                    .filter(|(i, _)| !compact || *i == 0 || *i == 3)
                                    .map(|(_, s)| {
                                        label(s, p.muted).text_size(px(if compact {
                                            10.5
                                        } else {
                                            11.
                                        }))
                                    }),
                            ),
                    )
                    .child(plot)
                    .child(div().w(px(references)).flex_none()),
            )
            .when(!compact, |d| {
                d.child(self.drill_time_labels(from, to, false))
            })
    }
    fn drill_time_labels(&self, from: i64, to: i64, compact: bool) -> Div {
        let caption = |time| {
            chrono::DateTime::from_timestamp_millis(time)
                .map(|t| {
                    t.with_timezone(&Local)
                        .format(if compact || to - from < crate::history::DAY {
                            "%H:%M"
                        } else {
                            "%m-%d %H:%M"
                        })
                        .to_string()
                })
                .unwrap_or_default()
        };
        row()
            .justify_between()
            .pl(px(if compact { 48. } else { 82. }))
            .pr(px(if compact { 84. } else { 158. }))
            .when(compact, |d| d.h(px(19.)))
            .child(label(caption(from), self.p.muted).text_size(px(10.5)))
            .child(label(caption(to), self.p.muted).text_size(px(10.5)))
    }
    fn drill_hover_layout(
        &self,
        help: Option<&str>,
        missing_seconds: bool,
        cx: &Context<Self>,
    ) -> HoverLayout {
        let system = cx.text_system();
        let font_id = system.resolve_font(&font("Microsoft YaHei UI"));
        let measure = |text: &str, size: f32| {
            text.chars()
                .map(|ch| {
                    system
                        .advance(font_id, px(size), ch)
                        .map_or(px(size), |b| b.width)
                })
                .fold(px(0.), |a, b| a + b)
        };
        let second_target = matches!(self.seconds.hover, Some(Target::Second(_)));
        let status = format!(
            "{} · {}",
            if second_target {
                "秒级速度"
            } else {
                "分钟用量"
            },
            if self.seconds.pinned {
                "已固定"
            } else {
                "点击固定"
            }
        );
        let mut content = measure(&status, 11.);
        if self.seconds.pinned {
            content += px(6. + 20. + 8. + 14.) + measure("取消固定", 11.);
        }
        if let Some(help) = help {
            content = content.max(measure(help, 11.) + px(16.));
        }
        let mut down_width = px(56.);
        let mut up_width = px(56.);
        if let Some(data) = self.seconds.hover_data.as_ref() {
            let seconds = data.duration_ms > 0;
            let value = |v: f64| {
                if seconds {
                    speed(v)
                } else if !data.observed {
                    "—".into()
                } else {
                    human_bytes(v.max(0.) as u64)
                }
            };
            content = content.max(measure(
                &hover_time_caption(data.from, data.to, seconds),
                11.,
            ));
            content = content.max(
                measure(&format!("↓ {}", value(data.down)), 13.)
                    + px(12.)
                    + measure(&format!("↑ {}", value(data.up)), 13.),
            );
            for item in
                data.applications
                    .iter()
                    .take(if self.seconds.expanded { usize::MAX } else { 5 })
            {
                let factor = if seconds {
                    1000. / data.duration_ms as f64
                } else {
                    1.
                };
                down_width = down_width.max(
                    measure(
                        &format!("↓ {}", value(item.bytes.down as f64 * factor)),
                        11.,
                    ) + px(2.),
                );
                up_width = up_width.max(
                    measure(&format!("↑ {}", value(item.bytes.up as f64 * factor)), 11.) + px(2.),
                );
                content = content
                    .max(px(26.) + measure(self.application_name(&item.key), 12.).min(px(228.)));
            }
            content = content.max(px(26. + 10.) + down_width + up_width);
            if missing_seconds {
                content = content.max(measure(NO_MINUTE_SECONDS, 11.));
            }
            if data.applications.is_empty() {
                content = content.max(measure(
                    if data.observed || seconds {
                        "当前对象在此采样中无活动"
                    } else {
                        "此分钟缺少采集记录，不按零流量计算"
                    },
                    11.,
                ));
            }
            if data.unassigned.total() > 0 {
                let factor = if seconds {
                    1000. / data.duration_ms as f64
                } else {
                    1.
                };
                content = content.max(measure(
                    &format!(
                        "未归属 ↓ {}  ↑ {}",
                        value(data.unassigned.down as f64 * factor),
                        value(data.unassigned.up as f64 * factor)
                    ),
                    11.,
                ));
            }
            let width = px(((f32::from(content) + 26.) / 8.).ceil() * 8.).clamp(px(224.), px(328.));
            self.seconds.hover_card_width.set(width);
            HoverLayout {
                width,
                down_width,
                up_width,
            }
        } else {
            HoverLayout {
                width: self
                    .seconds
                    .hover_card_width
                    .get()
                    .max(content + px(26.))
                    .min(px(328.)),
                down_width,
                up_width,
            }
        }
    }
    fn drill_hover_card(&self, window: &Window, cx: &Context<Self>) -> Option<AnyElement> {
        if self.seconds.locate_open {
            return None;
        }
        let target = self.seconds.hover?;
        let data = self.seconds.data.as_ref()?;
        let combined = self.seconds.view.is_none_or(|view| view.seconds);
        let (plot, range) = if combined {
            (self.seconds.second_bounds.get(), (data.from, data.to))
        } else {
            let trend = data.trend.as_ref()?;
            (self.seconds.minute_bounds.get(), (trend.from, trend.to))
        };
        let interval = match target {
            Target::Minute(from, to) => (from, to),
            Target::Second(seq) => {
                let sample = data.samples.iter().find(|sample| sample.seq == seq)?;
                (sample.from, sample.to)
            }
        };
        let p = self.p;
        let minute_seconds = minute_seconds_available(data, target, combined);
        let missing_seconds = minute_seconds == Some(false);
        let help = (self.seconds.pinned && self.seconds.hover_help_open).then_some(match target {
            Target::Second(_) => "← / → 微调，跳过空档",
            Target::Minute(..) => match minute_seconds {
                Some(true) => "此分钟有秒级采样，取消固定后移到采样位置查看",
                Some(false) => NO_MINUTE_SECONDS,
                None => "当前为分钟用量，右键可查看前后 5 分钟",
            },
        });
        let layout = self.drill_hover_layout(help, missing_seconds, cx);
        let viewport = window.viewport_size();
        let width = layout.width.min((viewport.width - px(16.)).max(px(0.)));
        let (corner, position) = drill_hover_anchor(plot, range, interval, viewport.width, width)?;
        let second_target = matches!(target, Target::Second(_));
        let mut body = column().gap(px(8.)).child(
            row()
                .justify_between()
                .gap(px(8.))
                .child(
                    row()
                        .gap(px(6.))
                        .child(
                            label(
                                format!(
                                    "{} · {}",
                                    if second_target {
                                        "秒级速度"
                                    } else {
                                        "分钟用量"
                                    },
                                    if self.seconds.pinned {
                                        "已固定"
                                    } else {
                                        "点击固定"
                                    }
                                ),
                                p.muted,
                            )
                            .text_size(px(11.))
                            .whitespace_nowrap(),
                        )
                        .when(self.seconds.pinned, |d| {
                            d.child(
                                div()
                                    .id("drill-help")
                                    .size(px(20.))
                                    .rounded(px(6.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .text_color(rgb(if self.seconds.hover_help_open {
                                        p.accent_text
                                    } else {
                                        p.muted
                                    }))
                                    .when(self.seconds.hover_help_open, |d| d.bg(state_surface(p)))
                                    .hover(move |d| d.bg(state_surface(p)).text_color(rgb(p.text)))
                                    .child(
                                        Icon::default()
                                            .path("icons/position-info.svg")
                                            .size(px(14.)),
                                    )
                                    .tooltip(|window, cx| Tooltip::new("说明").build(window, cx))
                                    .on_click(cx.listener(|app, _, window, cx| {
                                        app.act(
                                            Action::Drill(DrillAction::DrillHelpToggle),
                                            window,
                                            cx,
                                        )
                                    })),
                            )
                        }),
                )
                .when(self.seconds.pinned, |d| {
                    d.child(
                        self.button(
                            "drill-unpin",
                            "取消固定",
                            false,
                            Action::Drill(DrillAction::DrillUnpin),
                            cx,
                        )
                        .h(px(24.))
                        .px(px(6.))
                        .text_size(px(11.))
                        .whitespace_nowrap(),
                    )
                }),
        );
        if let Some(help) = help {
            let help = column()
                .gap(px(6.))
                .p(px(8.))
                .border_1()
                .border_color(surface_border(p))
                .rounded(px(6.))
                .bg(control_surface(p))
                .child(
                    label(help, p.text)
                        .text_size(px(11.))
                        .whitespace_normal()
                        .font_weight(FontWeight::MEDIUM),
                );
            body = body.child(help);
        } else if let Some(data) = self.seconds.hover_data.as_ref() {
            let seconds = data.duration_ms > 0;
            if missing_seconds {
                body = body.child(label(NO_MINUTE_SECONDS, p.muted).text_size(px(11.)));
            }
            let format = |v: f64| {
                if !seconds && !data.observed {
                    return "—".into();
                }
                if seconds {
                    speed(v)
                } else {
                    human_bytes(v.max(0.) as u64)
                }
            };
            body = body
                .child(
                    label(hover_time_caption(data.from, data.to, seconds), p.text)
                        .text_size(px(11.))
                        .whitespace_normal()
                        .id("drill-hover-time")
                        .tooltip({
                            let exact =
                                format!("{} — {}", sample_date(data.from), sample_date(data.to));
                            move |window, cx| Tooltip::new(exact.clone()).build(window, cx)
                        }),
                )
                .child(
                    row()
                        .gap(px(12.))
                        .child(label(format!("↓ {}", format(data.down)), p.download_text))
                        .child(label(format!("↑ {}", format(data.up)), p.upload_text)),
                )
                .child(
                    label(
                        if data.observed || seconds {
                            format!("活动应用 · {}项", data.applications.len())
                        } else {
                            "应用记录不可用".into()
                        },
                        p.muted,
                    )
                    .text_size(px(11.)),
                );
            let mut list = column()
                .gap(px(0.))
                .max_h(px(230.))
                .id("drill-hover-apps")
                .overflow_y_scroll();
            for (index, item) in data
                .applications
                .iter()
                .take(if self.seconds.expanded { usize::MAX } else { 5 })
                .enumerate()
            {
                let path = self.application_icon_path(&item.key).to_string();
                let factor = if seconds {
                    1000. / data.duration_ms as f64
                } else {
                    1.
                };
                list = list.child(
                    column()
                        .gap(px(3.))
                        .py(px(4.))
                        .when(index > 0, |d| {
                            d.border_t_1().border_color(surface_border(p))
                        })
                        .child(
                            row()
                                .gap(px(6.))
                                .child(self.application_icon(&item.key, true))
                                .child(
                                    label(self.application_name(&item.key).to_string(), p.text)
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(px(12.))
                                        .line_height(px(16.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .id(("drill-hover-app", index))
                                        .tooltip(move |window, cx| {
                                            Tooltip::new(path.clone()).build(window, cx)
                                        }),
                                ),
                        )
                        .child(
                            row()
                                .pl(px(26.))
                                .gap(px(10.))
                                .child(
                                    label(
                                        format!("↓ {}", format(item.bytes.down as f64 * factor)),
                                        p.download_text,
                                    )
                                    .w(layout.down_width)
                                    .flex_none()
                                    .text_size(px(11.))
                                    .line_height(px(14.))
                                    .whitespace_nowrap(),
                                )
                                .child(
                                    label(
                                        format!("↑ {}", format(item.bytes.up as f64 * factor)),
                                        p.upload_text,
                                    )
                                    .w(layout.up_width)
                                    .flex_none()
                                    .text_size(px(11.))
                                    .line_height(px(14.))
                                    .whitespace_nowrap(),
                                ),
                        ),
                );
            }
            body = body.child(list).when(data.applications.is_empty(), |d| {
                d.child(
                    label(
                        if data.observed || seconds {
                            "当前对象在此采样中无活动"
                        } else {
                            "此分钟缺少采集记录，不按零流量计算"
                        },
                        p.muted,
                    )
                    .text_size(px(11.)),
                )
            });
            if data.unassigned.total() > 0 {
                let factor = if seconds {
                    1000. / data.duration_ms as f64
                } else {
                    1.
                };
                body = body.child(
                    label(
                        format!(
                            "未归属 ↓ {}  ↑ {}",
                            format(data.unassigned.down as f64 * factor),
                            format(data.unassigned.up as f64 * factor)
                        ),
                        p.muted,
                    )
                    .text_size(px(11.))
                    .whitespace_normal(),
                );
            }
            if self.seconds.pinned && data.applications.len() > 5 {
                body = body.child(
                    self.button(
                        "drill-expand",
                        if self.seconds.expanded {
                            "收起应用"
                        } else {
                            "展开全部应用"
                        },
                        false,
                        Action::Drill(DrillAction::DrillExpand),
                        cx,
                    )
                    .h(px(26.)),
                );
            }
        } else {
            body = body.child(
                label(
                    if self.seconds.hover_error.is_empty() {
                        "正在读取应用…".into()
                    } else {
                        self.seconds.hover_error.clone()
                    },
                    p.muted,
                )
                .text_size(px(12.))
                .whitespace_normal(),
            );
        }
        Some(
            deferred(
                anchored()
                    .anchor(corner)
                    .position(position)
                    .snap_to_window_with_margin(px(8.))
                    .child(
                        self.card()
                            .w(width)
                            .p(px(12.))
                            .bg(popup_surface(p))
                            .shadow_md()
                            .when(self.seconds.pinned, |card| {
                                card.occlude().on_mouse_down(MouseButton::Left, |_, _, cx| {
                                    cx.stop_propagation()
                                })
                            })
                            .child(body),
                    ),
            )
            .with_priority(2)
            .into_any_element(),
        )
    }
    fn seconds_chart(&self, data: &SecondsData, compact: bool, cx: &Context<Self>) -> Div {
        let p = self.p;
        let height = if compact {
            if self.history_state.view == history_state::CalendarView::Year {
                132.
            } else {
                144.
            }
        } else {
            280.
        };
        let axis = if compact { 42. } else { 72. };
        let speed_axis = if compact { 48. } else { 64. };
        let reference_width = if compact { 82. } else { 148. };
        let gap = if compact { 6. } else { 10. };
        let data = std::sync::Arc::new(data.clone());
        let selected = if self.seconds.pinned {
            match self.seconds.hover {
                Some(Target::Second(seq)) => data
                    .samples
                    .iter()
                    .find(|s| s.seq == seq && s.valid)
                    .map(|s| s.from),
                _ => None,
            }
        } else {
            None
        };
        let speed_scale = chart::ChartScale::for_peak(
            data.samples
                .iter()
                .filter(|s| s.valid)
                .fold(0f64, |m, s| m.max(s.down).max(s.up)),
            true,
        );
        let volume_scale = chart::ChartScale::for_peak(
            data.trend
                .as_ref()
                .map(|t| {
                    t.buckets
                        .iter()
                        .map(|b| b.bytes.total() as f64)
                        .fold(0., f64::max)
                })
                .unwrap_or(0.),
            false,
        );
        let scale = speed_scale.ceiling;
        let volume_ceiling = volume_scale.ceiling;
        let stats = crate::traffic_drill::window_stats(&data.samples, data.from, data.to);
        let references = reference_labels(stats, scale, height);
        let painted_references = references.clone();
        let connectors = references.clone();
        let measured = self.seconds.second_bounds.clone();
        let minutes = self.seconds.minute_bounds.clone();
        let hit_bounds = measured.clone();
        let click_bounds = measured.clone();
        let paint = data.clone();
        let hover = data.clone();
        let click = data.clone();
        let plot = div()
            .id("seconds-combined-plot")
            .when(self.seconds.inline_history, |d| {
                d.track_focus(&self.seconds.focus)
            })
            .relative()
            .flex_1()
            .min_w_0()
            .h(px(height))
            .child(
                canvas(
                    move |bounds, _, _| {
                        measured.set(bounds);
                        minutes.set(bounds);
                    },
                    move |bounds, _, window, _| {
                        for tick in 0..=3 {
                            let mut path = PathBuilder::stroke(px(1.));
                            let y = bounds.top() + bounds.size.height * (tick as f32 / 3.);
                            path.move_to(point(bounds.left(), y));
                            path.line_to(point(bounds.right(), y));
                            if let Ok(path) = path.build() {
                                window.paint_path(path, rgb(p.line));
                            }
                        }
                        window.with_content_mask(Some(ContentMask { bounds }), |window| {
                            if let Some(trend) = paint.trend.as_ref() {
                                paint_drill_buckets(bounds, trend, volume_ceiling, p, true, window);
                            }
                            for reference in &painted_references {
                                let y = bounds.top() + px(reference.actual);
                                let mut path = PathBuilder::stroke(px(1.));
                                if reference.average {
                                    path = path.dash_array(&[px(5.), px(4.)]);
                                }
                                path.move_to(point(bounds.left(), y));
                                path.line_to(point(bounds.right(), y));
                                if let Ok(path) = path.build() {
                                    let mut color =
                                        rgb(if reference.down { p.download } else { p.upload });
                                    color.a = 0.5;
                                    window.paint_path(path, color);
                                }
                            }
                            if let Some(at) = selected {
                                let x = bounds.left()
                                    + bounds.size.width
                                        * ((at - paint.from) as f64
                                            / (paint.to - paint.from).max(1) as f64)
                                            .clamp(0., 1.)
                                            as f32;
                                let mut guide =
                                    PathBuilder::stroke(px(0.75)).dash_array(&[px(2.), px(3.)]);
                                guide.move_to(point(x, bounds.top()));
                                guide.line_to(point(x, bounds.bottom()));
                                if let Ok(path) = guide.build() {
                                    let mut color = rgb(p.muted);
                                    color.a = 0.45;
                                    window.paint_path(path, color);
                                }
                            }
                            for down in [true, false] {
                                let mut segment = Vec::new();
                                let mut previous = None;
                                let draw = |points: &[Point<Pixels>], window: &mut Window| {
                                    let color = rgb(if down { p.download } else { p.upload });
                                    if points.len() == 1 {
                                        window.paint_quad(
                                            fill(
                                                Bounds::new(
                                                    points[0] - point(px(1.5), px(1.5)),
                                                    size(px(3.), px(3.)),
                                                ),
                                                color,
                                            )
                                            .corner_radii(px(1.5)),
                                        );
                                    } else if points.len() > 1 {
                                        let controls = display_controls(points, px(5.));
                                        let smooth = chart::smooth_points(&controls);
                                        let mut path = PathBuilder::stroke(px(1.75));
                                        path.move_to(smooth[0]);
                                        for pos in &smooth[1..] {
                                            path.line_to(*pos);
                                        }
                                        if let Ok(path) = path.build() {
                                            window.paint_path(path, color);
                                        }
                                    }
                                };
                                for sample in &paint.samples {
                                    if !sample.valid
                                        || previous.is_some_and(|end| end != sample.from)
                                    {
                                        draw(&segment, window);
                                        segment.clear();
                                    }
                                    if sample.valid {
                                        let x = bounds.left()
                                            + bounds.size.width
                                                * ((sample.to.clamp(paint.from, paint.to)
                                                    - paint.from)
                                                    as f64
                                                    / (paint.to - paint.from).max(1) as f64)
                                                    as f32;
                                        let value = if down { sample.down } else { sample.up };
                                        let y = bounds.bottom()
                                            - bounds.size.height
                                                * (value / scale).clamp(0., 1.) as f32;
                                        segment.push(point(x, y));
                                        previous = Some(sample.to);
                                    } else {
                                        previous = None;
                                    }
                                }
                                draw(&segment, window);
                            }
                        });
                    },
                )
                .absolute()
                .size_full(),
            )
            .on_mouse_move(cx.listener(move |app, event: &MouseMoveEvent, _, cx| {
                if let Some(at) =
                    drill_time::cursor_time(hover.from, hover.to, hit_bounds.get(), event.position)
                {
                    app.preview_drill_time(at, cx);
                }
                if let Some(target) = combined_target(&hover, hit_bounds.get(), event.position) {
                    app.hover_drill(target, true, false, cx);
                }
            }))
            .on_hover(cx.listener(|app, entered: &bool, _, cx| {
                if !entered && let Some(target) = app.seconds.hover {
                    app.hover_drill(target, false, false, cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |app, event: &MouseDownEvent, window, cx| {
                    window.prevent_default();
                    app.seconds.focus.focus(window);
                    cx.stop_propagation();
                    if let Some(target) =
                        combined_target(&click, click_bounds.get(), event.position)
                    {
                        app.hover_drill(target, true, true, cx);
                    }
                }),
            )
            .when(data.samples.is_empty(), |d| {
                d.child(div().absolute().top(px(8.)).left(px(10.)).child(
                    label("暂无已保存的秒级记录 · 分钟用量仍可查看", p.muted).text_size(px(11.)),
                ))
            });
        let axis_labels = |labels: [String; 4], width: f32| {
            column()
                .w(px(width))
                .h(px(height))
                .flex_none()
                .justify_between()
                .children(
                    labels
                        .into_iter()
                        .enumerate()
                        .filter(|(i, _)| !compact || *i == 0 || *i == 3)
                        .map(|(_, s)| {
                            label(if compact { s.replace(' ', "") } else { s }, p.muted)
                                .text_size(px(if compact { 10.5 } else { 11. }))
                                .whitespace_nowrap()
                        }),
                )
        };
        column()
            .gap(px(if compact { 4. } else { 10. }))
            .when(!compact, |d| {
                d.child(
                    row().gap(px(14.)).flex_wrap()
                        .child(label("▥ 分钟用量", p.muted).text_size(px(11.)))
                        .child(label("— 下载速度", p.download_text).text_size(px(11.)))
                        .child(label("— 上传速度", p.upload_text).text_size(px(11.)))
                        .child(div().id("drill-chart-help").size(px(20.)).flex().items_center().justify_center()
                            .child(Icon::default().path("icons/position-info.svg").size(px(14.)).text_color(rgb(p.muted)))
                            .tooltip(|window,cx| Tooltip::new("淡色柱为分钟用量，左轴用量、右轴速度。\n曲线辅助观察走势，悬浮读取原始采样。\n横实线为峰值，横虚线为均速；仅按有效采样统计。").build(window,cx))),
                )
            })
            .child(
                stretch()
                    .gap(px(gap))
                    .child(axis_labels(volume_scale.labels, axis))
                    .child(plot)
                    .child(axis_labels(
                        if data.samples.is_empty() {
                            ["—".into(), "".into(), "".into(), "".into()]
                        } else {
                            speed_scale.labels
                        },
                        speed_axis,
                    ))
                    .child(
                        div()
                            .relative()
                            .w(px(reference_width))
                            .h(px(height))
                            .flex_none()
                            .child(
                                canvas(
                                    |_, _, _| (),
                                    move |bounds, _, window, _| {
                                        for reference in &connectors {
                                            let mut path = PathBuilder::stroke(px(0.75));
                                            path.move_to(point(
                                                bounds.left() - px(gap),
                                                bounds.top() + px(reference.actual),
                                            ));
                                            path.line_to(point(
                                                bounds.left() + px(2.),
                                                bounds.top() + px(reference.label),
                                            ));
                                            path.line_to(point(
                                                bounds.left() + px(6.),
                                                bounds.top() + px(reference.label),
                                            ));
                                            if let Ok(path) = path.build() {
                                                let mut color = rgb(if reference.down {
                                                    p.download
                                                } else {
                                                    p.upload
                                                });
                                                color.a = 0.5;
                                                window.paint_path(path, color);
                                            }
                                        }
                                    },
                                )
                                .absolute()
                                .size_full(),
                            )
                            .children(references.into_iter().map(|reference| {
                                let text = format!(
                                    "{}{} {}",
                                    if reference.down { "↓" } else { "↑" },
                                    if reference.average { "均" } else { "峰" },
                                    speed(reference.value)
                                );
                                div()
                                    .absolute()
                                    .top(px(reference.label - 8.))
                                    .left(px(8.))
                                    .child(
                                        label(
                                            text,
                                            if reference.down {
                                                p.download_text
                                            } else {
                                                p.upload_text
                                            },
                                        )
                                        .text_size(px(if compact { 10.5 } else { 11. }))
                                        .whitespace_nowrap(),
                                    )
                            })),
                    ),
            )
            .child(
                self.drill_time_labels(data.from, data.to, compact)
                    .pr(px(speed_axis + reference_width + gap * 2.)),
            )
    }
}

// Drawing alone uses spaced local peaks; all statistics and hovers retain the
// original samples. Each continuous valid segment is processed independently.
fn display_controls(points: &[Point<Pixels>], spacing: Pixels) -> Vec<Point<Pixels>> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let first = points[0];
    let last = *points.last().unwrap();
    let mut result = vec![first];
    let mut i = 1;
    while i + 1 < points.len() {
        let start = i;
        let key = (f32::from(points[i].x - first.x) / f32::from(spacing)).floor();
        while i + 1 < points.len()
            && (f32::from(points[i].x - first.x) / f32::from(spacing)).floor() == key
        {
            i += 1;
        }
        let group = &points[start..i];
        let x = (group[0].x + group.last().unwrap().x) / 2.;
        let y = group.iter().map(|p| p.y).min().unwrap();
        if x > result.last().unwrap().x && x < last.x {
            result.push(point(x, y));
        }
    }
    result.push(last);
    result
}
fn combined_target(
    data: &SecondsData,
    bounds: Bounds<Pixels>,
    position: Point<Pixels>,
) -> Option<Target> {
    let at = drill_time::cursor_time(data.from, data.to, bounds, position)?;
    if let Some(sample) = data
        .samples
        .iter()
        .find(|s| s.valid && s.from <= at && at < s.to)
    {
        return Some(Target::Second(sample.seq));
    }
    data.trend
        .as_ref()
        .and_then(|t| t.buckets.iter().find(|b| b.from <= at && at < b.to))
        .map(|b| Target::Minute(b.from, b.to))
}

fn minute_seconds_available(data: &SecondsData, target: Target, combined: bool) -> Option<bool> {
    let Target::Minute(from, to) = target else {
        return None;
    };
    combined.then(|| {
        let from = from.max(data.from);
        let to = to.min(data.to);
        from < to
            && data
                .samples
                .iter()
                .any(|s| s.valid && s.to > from && s.from < to)
    })
}

fn paint_drill_buckets(
    bounds: Bounds<Pixels>,
    trend: &crate::history::Trend,
    scale: f64,
    p: Palette,
    faint: bool,
    window: &mut Window,
) {
    let from = trend.from;
    let to = trend.to;
    for bucket in &trend.buckets {
        let x = bounds.left()
            + bounds.size.width * ((bucket.from - from) as f64 / (to - from).max(1) as f64) as f32;
        let width = bounds.size.width
            * ((bucket.to - bucket.from) as f64 / (to - from).max(1) as f64) as f32;
        if !bucket.observed {
            window.paint_quad(
                outline(
                    Bounds::new(
                        point(x + width / 2. - px(2.), bounds.bottom() - px(4.)),
                        size(px(4.), px(4.)),
                    ),
                    rgb(p.muted),
                    BorderStyle::Solid,
                )
                .corner_radii(px(2.))
                .border_widths(px(0.75)),
            );
            continue;
        }
        if bucket.bytes.total() == 0 {
            let mut color = rgb(p.muted);
            color.a = 0.45;
            window.paint_quad(fill(
                Bounds::new(
                    point(x + width / 2. - px(2.), bounds.bottom() - px(1.)),
                    size(px(4.), px(1.)),
                ),
                color,
            ));
        }
        let mut bottom = bounds.bottom();
        for (bytes, color) in [(bucket.bytes.down, p.download), (bucket.bytes.up, p.upload)] {
            let height = bounds.size.height * (bytes as f64 / scale).clamp(0., 1.) as f32;
            let mut color = rgb(color);
            color.a = if faint {
                if p.dark { 0.28 } else { 0.20 }
            } else if p.dark {
                0.6
            } else {
                0.48
            };
            window.paint_quad(
                fill(
                    Bounds::new(
                        point(x + width * 0.15, bottom - height),
                        size(width * 0.7, height),
                    ),
                    color,
                )
                .corner_radii(px(2.)),
            );
            bottom -= height;
        }
    }
}

#[derive(Clone, Debug)]
struct Reference {
    value: f64,
    down: bool,
    average: bool,
    actual: f32,
    label: f32,
}
fn reference_labels(
    stats: crate::traffic_drill::WindowStats,
    ceiling: f64,
    height: f32,
) -> Vec<Reference> {
    if stats.covered_ms <= 0. {
        return vec![];
    }
    let mut result = [
        (stats.down_peak, true, false),
        (stats.up_peak, false, false),
        (stats.down_average, true, true),
        (stats.up_average, false, true),
    ]
    .into_iter()
    .map(|(value, down, average)| {
        let actual = height * (1. - (value / ceiling).clamp(0., 1.) as f32);
        Reference {
            value,
            down,
            average,
            actual,
            label: actual.clamp(8., height - 8.),
        }
    })
    .collect::<Vec<_>>();
    result.sort_by(|a, b| a.actual.total_cmp(&b.actual));
    for i in 1..result.len() {
        result[i].label = result[i].label.max(result[i - 1].label + 18.);
    }
    if let Some(last) = result.last_mut() {
        last.label = last.label.min(height - 8.);
    }
    for i in (0..result.len() - 1).rev() {
        result[i].label = result[i].label.min(result[i + 1].label - 18.);
    }
    result
}

struct HoverLayout {
    width: Pixels,
    down_width: Pixels,
    up_width: Pixels,
}

fn drill_hover_anchor(
    plot: Bounds<Pixels>,
    range: (i64, i64),
    interval: (i64, i64),
    viewport_width: Pixels,
    width: Pixels,
) -> Option<(Corner, Point<Pixels>)> {
    let (from, to) = range;
    if plot.size.width <= px(0.) || to <= from || interval.1 <= from || interval.0 >= to {
        return None;
    }
    let x = |at: i64| {
        plot.left()
            + plot.size.width * ((at.clamp(from, to) - from) as f64 / (to - from) as f64) as f32
    };
    let left = x(interval.0) - px(8.);
    let right = x(interval.1) + px(8.);
    let y = (plot.top() + px(4.)).max(px(8.));
    if right + width <= viewport_width - px(8.) || left - width < px(8.) {
        Some((Corner::TopLeft, point(right, y)))
    } else {
        Some((Corner::TopRight, point(left, y)))
    }
}

fn adjacent_sample(data: &SecondsData, current: Target, direction: i8) -> Option<Target> {
    let Target::Second(seq) = current else {
        return None;
    };
    let index = data.samples.iter().position(|s| s.seq == seq)?;
    let visible =
        |s: &&crate::speed_stats::SecondSample| s.valid && s.to > data.from && s.from < data.to;
    if direction < 0 {
        data.samples[..index].iter().rev().find(visible)
    } else {
        data.samples[index + 1..].iter().find(visible)
    }
    .map(|s| Target::Second(s.seq))
}
fn hover_time_caption(from: i64, to: i64, seconds: bool) -> String {
    if seconds {
        return sample_date(from);
    }
    chrono::DateTime::from_timestamp_millis(to)
        .map(|d| d.with_timezone(&Local).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "—".into())
}

fn sample_date(time: i64) -> String {
    if time.rem_euclid(1000) == 0 {
        return date(time);
    }
    chrono::DateTime::from_timestamp_millis(time)
        .map(|d| {
            d.with_timezone(&Local)
                .format("%Y-%m-%d %H:%M:%S%.3f")
                .to_string()
        })
        .unwrap_or_else(|| "—".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    #[::core::prelude::v1::test]
    fn drill_hover_follows_interval_and_flips_at_right_edge() {
        let plot = Bounds::new(point(px(100.), px(120.)), size(px(700.), px(200.)));
        let anchor = |interval| drill_hover_anchor(plot, (0, 100), interval, px(900.), px(300.));
        assert_eq!(
            anchor((0, 10)),
            Some((Corner::TopLeft, point(px(178.), px(124.))))
        );
        assert_eq!(
            anchor((30, 40)),
            Some((Corner::TopLeft, point(px(388.), px(124.))))
        );
        assert_eq!(
            anchor((90, 100)),
            Some((Corner::TopRight, point(px(722.), px(124.))))
        );
        assert_eq!(anchor((100, 110)), None);
    }

    #[::core::prelude::v1::test]
    fn hover_time_uses_sample_start_for_seconds_and_bucket_end_for_minutes() {
        let at = |y, m, d, h, n, s| {
            Local
                .with_ymd_and_hms(y, m, d, h, n, s)
                .single()
                .unwrap()
                .timestamp_millis()
        };
        let from = at(2026, 11, 2, 22, 26, 30);
        assert_eq!(
            hover_time_caption(from, from + 1000, true),
            "2026-11-02 22:26:30"
        );
        assert_eq!(
            hover_time_caption(from + 98, from + 1094, true),
            "2026-11-02 22:26:30.098"
        );
        assert_eq!(
            hover_time_caption(
                at(2026, 11, 2, 22, 26, 0),
                at(2026, 11, 2, 22, 27, 0),
                false
            ),
            "2026-11-02 22:27"
        );
        assert_eq!(
            hover_time_caption(at(2026, 11, 2, 23, 59, 0), at(2026, 11, 3, 0, 0, 0), false),
            "2026-11-03 00:00"
        );
        assert_eq!(
            hover_time_caption(at(2026, 12, 31, 23, 59, 0), at(2027, 1, 1, 0, 0, 0), false),
            "2027-01-01 00:00"
        );
    }
    #[::core::prelude::v1::test]
    fn adjacent_samples_skip_gaps_keep_idle_and_stop_at_window_edges() {
        let sample = |seq, from, valid| crate::speed_stats::SecondSample {
            seq,
            from,
            to: from + 1000,
            duration_ms: 1000,
            down: 0.,
            up: 0.,
            valid,
        };
        let data = SecondsData {
            from: 0,
            to: 10_000,
            first: Some(0),
            last: Some(6000),
            trend: None,
            samples: vec![
                sample(10, 0, true),
                sample(11, 1000, false),
                sample(12, 5000, true),
            ],
        };
        assert_eq!(
            adjacent_sample(&data, Target::Second(10), 1),
            Some(Target::Second(12))
        );
        assert_eq!(
            adjacent_sample(&data, Target::Second(12), -1),
            Some(Target::Second(10))
        );
        assert_eq!(adjacent_sample(&data, Target::Second(10), -1), None);
        assert_eq!(adjacent_sample(&data, Target::Second(12), 1), None);
        assert_eq!(adjacent_sample(&data, Target::Minute(0, 10_000), 1), None);
    }
    #[::core::prelude::v1::test]
    fn combined_display_retains_local_peaks_and_rounds_dense_controls() {
        let raw: Vec<_> = (0..600)
            .map(|i| point(px(i as f32), px(if i == 303 { 0. } else { 100. })))
            .collect();
        let controls = display_controls(&raw, px(5.));
        assert!(controls.len() < raw.len() / 3);
        assert_eq!(controls.first(), raw.first());
        assert_eq!(controls.last(), raw.last());
        assert!(controls.iter().any(|p| p.y == px(0.)));
        let outline = chart::smooth_points(&controls);
        assert!(outline.windows(2).all(|p| p[1].x > p[0].x));
        assert!(outline.iter().all(|p| p.y >= px(0.) && p.y <= px(100.)));
        assert!(display_controls(&[], px(5.)).is_empty());
    }
    #[::core::prelude::v1::test]
    fn combined_hover_uses_raw_seconds_at_every_height_and_marks_gaps() {
        let mut data = SecondsData {
            from: 0,
            to: 600_000,
            first: Some(30_000),
            last: Some(31_000),
            samples: vec![crate::speed_stats::SecondSample {
                seq: 7,
                from: 30_000,
                to: 31_000,
                duration_ms: 1000,
                down: 100.,
                up: 0.,
                valid: true,
            }],
            trend: Some(crate::history::Trend {
                from: 0,
                to: 600_000,
                generated_at: 600_000,
                grain: crate::history::Grain::Minute,
                buckets: vec![crate::history::Bucket {
                    from: 0,
                    to: 60_000,
                    bytes: crate::model::Bytes { down: 500, up: 0 },
                    covered_ms: 60_000,
                    expected_ms: 60_000,
                    observed: true,
                }],
            }),
        };
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(600.), px(100.)));
        for y in [1., 30., 60., 90., 99.] {
            assert_eq!(
                combined_target(&data, bounds, point(px(30.5), px(y))),
                Some(Target::Second(7))
            );
        }
        data.samples[0].to = 31_500;
        data.samples[0].duration_ms = 1500;
        assert_eq!(
            combined_target(&data, bounds, point(px(31.2), px(60.))),
            Some(Target::Second(7))
        );
        assert_eq!(
            combined_target(&data, bounds, point(px(31.6), px(60.))),
            Some(Target::Minute(0, 60_000))
        );
        assert_eq!(
            minute_seconds_available(&data, Target::Minute(0, 60_000), true),
            Some(true)
        );
        assert_eq!(
            minute_seconds_available(&data, Target::Minute(60_000, 120_000), true),
            Some(false)
        );
        assert_eq!(
            minute_seconds_available(&data, Target::Minute(0, 60_000), false),
            None
        );
        data.samples[0].valid = false;
        assert_eq!(
            minute_seconds_available(&data, Target::Minute(0, 60_000), true),
            Some(false)
        );
        assert_eq!(
            combined_target(&data, bounds, point(px(30.5), px(90.))),
            Some(Target::Minute(0, 60_000))
        );
        assert_eq!(
            combined_target(&data, bounds, point(px(-1.), px(60.))),
            None
        );
    }
    #[::core::prelude::v1::test]
    fn four_reference_labels_stay_separate_at_zero_and_near_identical_peaks() {
        for value in [0., 999.] {
            let stats = crate::traffic_drill::WindowStats {
                covered_ms: 1000.,
                down_peak: value,
                up_peak: value,
                down_average: value,
                up_average: value,
            };
            let labels = reference_labels(stats, 1000., 220.);
            assert_eq!(labels.len(), 4);
            assert_eq!(labels.iter().filter(|r| r.average).count(), 2);
            assert!(
                labels
                    .windows(2)
                    .all(|pair| pair[1].label - pair[0].label >= 17.9)
            );
            assert!(labels.iter().all(|r| r.label >= 8. && r.label <= 212.));
        }
        assert!(reference_labels(Default::default(), 1., 220.).is_empty());
    }
}
