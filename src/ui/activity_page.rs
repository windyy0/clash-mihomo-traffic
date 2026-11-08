use super::*;
use crate::{
    analysis::{AnalysisRange, AnalysisRequest, AppComparison, Snapshot},
    app_activity::{ActivityState, AppActivity, application_activity as activity},
};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum ActivityTab {
    #[default]
    Current,
    Recent,
}
impl ActivityTab {
    pub const ALL: [Self; 2] = [Self::Current, Self::Recent];
    fn index(self) -> usize {
        match self {
            Self::Current => 0,
            Self::Recent => 1,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Current => "当前传输",
            Self::Recent => "近期用量",
        }
    }
}
pub(super) struct ActivityPageState {
    pub copy_feedback: Option<(String, bool, Instant)>,
    pub selected_process: bool,
    pub processes_expanded: bool,
    pub connection_group: Option<String>,
    pub details: bool,
    pub preview: ActivityTab,
    pub tab: ActivityTab,
    pub recent_snapshot: Option<Arc<Snapshot>>,
    pub recent_pending: Option<AnalysisRequest>,
    pub recent_provisional: bool,
    pub recent_error: String,
    pub live_search: Entity<InputState>,
    pub recent_search: Entity<InputState>,
    live_filter: String,
    recent_filter: String,
    pub live_page: usize,
    pub recent_page: usize,
    offsets: [Point<Pixels>; 2],
    dashboard_offset: Point<Pixels>,
    table_bounds: Rc<Cell<Bounds<Pixels>>>,
    pub anchor_table: bool,
    #[cfg(test)]
    pub preview_caption_bounds: Rc<Cell<Bounds<Pixels>>>,
}
impl ActivityPageState {
    pub fn new(window: &mut Window, cx: &mut Context<TrafficApp>) -> Self {
        Self {
            copy_feedback: None,
            selected_process: false,
            processes_expanded: false,
            connection_group: None,
            details: false,
            preview: ActivityTab::Current,
            tab: ActivityTab::Current,
            recent_snapshot: None,
            recent_pending: None,
            recent_provisional: false,
            recent_error: String::new(),
            live_search: input("", "搜索应用名称或完整路径…", false, window, cx),
            recent_search: input("", "搜索应用名称或完整路径…", false, window, cx),
            live_filter: String::new(),
            recent_filter: String::new(),
            live_page: 0,
            recent_page: 0,
            offsets: [point(px(0.), px(0.)); 2],
            dashboard_offset: point(px(0.), px(0.)),
            table_bounds: Rc::new(Cell::new(Bounds::default())),
            anchor_table: false,
            #[cfg(test)]
            preview_caption_bounds: Rc::new(Cell::new(Bounds::default())),
        }
    }
}
fn clock(time: i64) -> String {
    chrono::DateTime::from_timestamp_millis(time)
        .map(|date| {
            date.with_timezone(&chrono::Local)
                .format("%H:%M:%S")
                .to_string()
        })
        .unwrap_or_default()
}
fn stamp(time: i64) -> String {
    chrono::DateTime::from_timestamp_millis(time)
        .map(|date| {
            date.with_timezone(&chrono::Local)
                .format("%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_default()
}

impl TrafficApp {
    pub(super) fn ordered_activity(&self) -> Vec<AppActivity> {
        let mut apps = activity(&self.data);
        self.live_app_order
            .borrow_mut()
            .arrange(&mut apps, Instant::now());
        apps
    }
    pub(super) fn request_recent_usage(&mut self) {
        // The worker coalesces requests, so supersede the other pending view explicitly.
        if self.analysis.pending.take().is_some()
            && let Some(snapshot) = &self.analysis.snapshot
            && snapshot.request.windows.is_none()
        {
            self.analysis.range = snapshot.request.range;
        }
        let request = self.next_snapshot_request(AnalysisRange::FiveMinutes);
        self.activity.recent_error.clear();
        if self.backend.query_analysis(request.clone()) {
            self.activity.recent_pending = Some(request);
        } else {
            self.activity.recent_pending = None;
            self.activity.recent_error = "读取失败，请刷新重试".into();
        }
    }
    pub(super) fn refresh_recent_usage_on_entry(&mut self) {
        if self.activity.recent_pending.is_none()
            && self.config.recent_usage_refresh_mode.should_refresh(
                self.activity
                    .recent_snapshot
                    .as_ref()
                    .filter(|snapshot| {
                        !self.activity.recent_provisional
                            && snapshot.request.generation == self.backend.analysis_generation()
                    })
                    .map(|snapshot| snapshot.generated_at),
                chrono::Utc::now().timestamp_millis(),
            )
        {
            self.request_recent_usage();
        }
    }
    pub(super) fn poll_recent_usage(&mut self, cx: &mut Context<Self>) {
        let live = self.activity.live_search.read(cx).value().to_string();
        let recent = self.activity.recent_search.read(cx).value().to_string();
        if live != self.activity.live_filter {
            self.activity.live_filter = live;
            self.activity.live_page = 0;
        }
        if recent != self.activity.recent_filter {
            self.activity.recent_filter = recent;
            self.activity.recent_page = 0;
        }
        let reply = self.backend.analysis_result();
        if let Some(reply) = reply
            && self.activity.recent_pending.as_ref() == Some(&reply.request)
        {
            if reply.complete {
                self.activity.recent_pending = None;
            }
            match reply.result {
                Ok(snapshot)
                    if snapshot.request.generation != self.backend.analysis_generation() =>
                {
                    self.activity.recent_pending = None;
                    self.activity.recent_error = "数据源已变化，请刷新快照".into();
                }
                Ok(snapshot) => {
                    if reply.complete
                        || self.activity.recent_snapshot.is_none()
                        || self.activity.recent_provisional
                        || self.activity.recent_snapshot.as_ref().is_some_and(|old| {
                            old.request.generation != snapshot.request.generation
                        })
                    {
                        self.activity.recent_snapshot = Some(snapshot);
                        self.activity.recent_provisional = !reply.complete;
                        self.activity.recent_error.clear();
                    }
                }
                Err(error) => {
                    self.activity.recent_pending = None;
                    self.activity.recent_error = error;
                }
            }
        }

        if self
            .activity
            .recent_snapshot
            .as_ref()
            .is_some_and(|snapshot| {
                snapshot.request.generation != self.backend.analysis_generation()
            })
        {
            self.activity.recent_error = "数据源已变化，当前为旧快照，请刷新".into();
        }
        self.activity.live_page = self
            .activity
            .live_page
            .min(self.filtered_live_activity().len().saturating_sub(1) / 10);
        self.activity.recent_page = self
            .activity
            .recent_page
            .min(self.recent_usage_rows(true).len().saturating_sub(1) / 10);
    }
    pub(super) fn show_activity_details(
        &mut self,
        tab: ActivityTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_usage_detail();
        window.blur();
        if self.activity.details {
            self.activity.offsets[self.activity.tab.index()] = self.page_scroll.offset();
        } else {
            self.activity.dashboard_offset = self.page_scroll.offset();
        }
        self.activity.details = true;
        self.activity.tab = tab;
        self.analysis.active = false;
        self.analysis.anchor = None;
        self.activity.anchor_table = false;
        self.app_usage_anchor_pending = false;
        self.page_scroll
            .set_offset(self.activity.offsets[tab.index()]);
        if tab == ActivityTab::Recent {
            self.refresh_recent_usage_on_entry();
        }
        cx.notify();
    }
    pub(super) fn close_activity_details(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.blur();
        self.activity.offsets[self.activity.tab.index()] = self.page_scroll.offset();
        self.activity.details = false;
        self.analysis.active = false;
        self.analysis.anchor = None;
        self.activity.anchor_table = false;
        self.page_scroll.set_offset(self.activity.dashboard_offset);
        self.refresh();
        cx.notify();
    }
    pub(super) fn activity_page(&mut self, step: i32) {
        let (count, recent) = if self.activity.tab == ActivityTab::Recent {
            (self.recent_usage_rows(true).len(), true)
        } else {
            (self.filtered_live_activity().len(), false)
        };
        let page = if recent {
            &mut self.activity.recent_page
        } else {
            &mut self.activity.live_page
        };
        *page = page
            .saturating_add_signed(step as isize)
            .min(count.saturating_sub(1) / 10);
        self.activity.anchor_table = true;
    }
    pub(super) fn schedule_activity_anchor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.activity.anchor_table
            || !self.activity.details
            || self.page != Page::Apps
            || !self.selected.is_empty()
        {
            return;
        }
        self.activity.anchor_table = false;
        let tab = self.activity.tab;
        let weak = cx.weak_entity();
        window.on_next_frame(move |_, cx| {
            let _ = weak.update(cx, |app, cx| {
                if !app.activity.details
                    || app.activity.tab != tab
                    || app.page != Page::Apps
                    || !app.selected.is_empty()
                {
                    return;
                }
                let rect = app.activity.table_bounds.get();
                if rect.size.height <= px(0.) {
                    return;
                }
                let scroll = &app.page_scroll;
                let y = scroll.offset().y + scroll.bounds().top() + px(16.) - rect.top();
                scroll.set_offset(point(
                    scroll.offset().x,
                    y.clamp(-scroll.max_offset().height, px(0.)),
                ));
                cx.notify();
            });
        });
    }
    pub(super) fn recent_usage_rows(&self, filtered: bool) -> Vec<&AppComparison> {
        let filter = if filtered {
            self.activity.recent_filter.trim().to_lowercase()
        } else {
            String::new()
        };
        let mut rows: Vec<_> = self
            .activity
            .recent_snapshot
            .as_ref()
            .map(|snapshot| {
                snapshot
                    .rows
                    .iter()
                    .filter(|row| {
                        row.metrics.current.total() > 0
                            && (row.key.to_lowercase().contains(&filter)
                                || self
                                    .application_name(&row.key)
                                    .to_lowercase()
                                    .contains(&filter))
                    })
                    .collect()
            })
            .unwrap_or_default();
        rows.sort_by(|a, b| {
            b.metrics
                .current
                .total()
                .cmp(&a.metrics.current.total())
                .then_with(|| a.key.cmp(&b.key))
        });
        rows
    }
    pub(super) fn recent_usage_maximum(&self) -> f64 {
        self.activity
            .recent_snapshot
            .as_ref()
            .map(|snapshot| {
                snapshot
                    .rows
                    .iter()
                    .map(|row| row.metrics.current.down as f64 + row.metrics.current.up as f64)
                    .fold(0f64, f64::max)
            })
            .unwrap_or(0.)
            .max(1.)
    }
    fn filtered_live_activity(&self) -> Vec<AppActivity> {
        let filter = self.activity.live_filter.trim().to_lowercase();
        self.ordered_activity()
            .into_iter()
            .filter(|app| {
                app.key.to_lowercase().contains(&filter)
                    || self
                        .application_name(&app.key)
                        .to_lowercase()
                        .contains(&filter)
            })
            .collect()
    }
    fn recent_refresh_button(&self, id: &'static str, cx: &Context<Self>) -> Stateful<Div> {
        self.button(
            id,
            if self.activity.recent_pending.is_some() {
                "读取中…"
            } else {
                "刷新快照"
            },
            false,
            Action::RecentRefresh,
            cx,
        )
        .h(px(28.))
        .px(px(8.))
        .when(self.activity.recent_pending.is_some(), |button| {
            button.opacity(0.45)
        })
    }
    pub(super) fn activity_preview(
        &self,
        apps: &[AppActivity],
        width: f32,
        cx: &Context<Self>,
    ) -> Div {
        let p = self.p;
        let recent = self.activity.preview == ActivityTab::Recent;
        let mut header = row().justify_between().gap(px(4.)).child(
            row()
                .gap(px(2.))
                .p(px(2.))
                .rounded(px(9.))
                .bg(control_surface(p))
                .children(
                    [ActivityTab::Current, ActivityTab::Recent]
                        .into_iter()
                        .map(|tab| {
                            self.button(
                                format!("activity-preview-{tab:?}"),
                                tab.label(),
                                self.activity.preview == tab,
                                Action::ActivityPreview(tab),
                                cx,
                            )
                            .h(px(28.))
                            .px(px(8.))
                        }),
                ),
        );
        if recent {
            header = header.child(self.recent_refresh_button("recent-preview-refresh", cx));
        }
        let caption = if !recent {
            "当前连接 · 自动更新".to_string()
        } else if self.activity.recent_snapshot.is_none() {
            "近5分钟快照".to_string()
        } else if !self.activity.recent_error.is_empty() {
            self.activity.recent_error.clone()
        } else {
            let snapshot = self.activity.recent_snapshot.as_ref().unwrap();
            format!(
                "近5分钟 · 截至 {}{}",
                clock(snapshot.request.anchor),
                if self.activity.recent_pending.is_some() {
                    " · 更新中".to_string()
                } else if snapshot.coverage(false) < AnalysisRange::FiveMinutes.duration() {
                    format!(" · 已采集{}秒", snapshot.coverage(false) / 1000)
                } else {
                    String::new()
                }
            )
        };
        let caption_label = label(
            caption,
            if recent && !self.activity.recent_error.is_empty() {
                p.amber
            } else {
                p.muted
            },
        )
        .text_size(px(11.))
        .flex_1()
        .min_w_0()
        .truncate();
        #[cfg(test)]
        let caption_label = {
            let bounds = self.activity.preview_caption_bounds.clone();
            caption_label.relative().child(
                canvas(move |rect, _, _| bounds.set(rect), |_, _, _, _| {})
                    .absolute()
                    .inset_0(),
            )
        };
        let panel = column()
            .w(px(width))
            .flex_none()
            .min_h(px(304.))
            .min_w_0()
            .gap(px(4.))
            .child(header)
            .child(
                row()
                    .h(px(16.))
                    .px(px(10.))
                    .w_full()
                    .min_w_0()
                    .flex_none()
                    .child(caption_label),
            );
        let mut list = column().gap(px(0.)).flex_1().min_w_0();
        if recent {
            let rows = self.recent_usage_rows(false);
            let maximum = self.recent_usage_maximum();
            if self.activity.recent_snapshot.is_some() {
                for row in rows.iter().take(4) {
                    list = list.child(self.recent_usage_row(row, false, maximum, cx));
                }
                if rows.is_empty() {
                    list = list.child(self.empty("暂无近期用量记录", "采集不足时仅统计已有采样"));
                }
            } else {
                list = list.child(self.empty(
                    if self.activity.recent_pending.is_some() {
                        "正在生成快照"
                    } else {
                        "暂无近期快照"
                    },
                    if self.activity.recent_error.is_empty() {
                        "按点击时刻固定近5分钟用量"
                    } else {
                        &self.activity.recent_error
                    },
                ));
            }
        } else {
            for app in apps.iter().take(4) {
                list = list.child(self.live_preview_row(app, cx));
            }
            if apps.is_empty() {
                list = list.child(self.empty(
                    if self.data.connected {
                        "当前暂无应用连接"
                    } else {
                        "等待连接"
                    },
                    "连接及实时速度会自动更新",
                ));
            }
        }
        panel.child(list).child(
            row().justify_end().child(
                self.button(
                    "activity-see-all",
                    "查看全部",
                    false,
                    Action::ActivityDetails(self.activity.preview),
                    cx,
                )
                .h(px(28.))
                .px(px(0.)),
            ),
        )
    }
    fn live_preview_row(&self, app: &AppActivity, cx: &Context<Self>) -> Stateful<Div> {
        let p = self.p;
        let key = app.key.clone();
        let tooltip = key.clone();
        let active = app.state() == ActivityState::Transferring;
        row()
            .id(SharedString::from(format!("live-preview-{key}")))
            .h(px(52.))
            .gap(px(10.))
            .px(px(4.))
            .rounded(px(7.))
            .cursor_pointer()
            .hover(move |style| style.bg(state_surface(p)))
            .child(self.app_badge(&key))
            .child(
                column()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .child(label(self.application_name(&key).to_string(), p.text).truncate())
                    .child(
                        label(app.state().label(), if active { p.upload } else { p.muted })
                            .text_size(px(11.)),
                    ),
            )
            .child(
                column()
                    .items_end()
                    .gap(px(2.))
                    .flex_none()
                    .child(label(
                        format!("↓ {}", speed(app.rate.down as f64)),
                        if active { p.download } else { p.muted },
                    ))
                    .child(
                        label(
                            format!("↑ {}", speed(app.rate.up as f64)),
                            if active { p.upload } else { p.muted },
                        )
                        .text_size(px(11.)),
                    ),
            )
            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            .on_click(cx.listener(move |app, _, window, cx| {
                app.act(
                    Action::Application(ApplicationAction::Select(key.clone())),
                    window,
                    cx,
                )
            }))
    }
    pub(super) fn usage_split_bar(&self, bytes: Bytes, maximum: f64) -> impl IntoElement {
        let total = bytes.down as f64 + bytes.up as f64;
        let share = if total == 0. {
            0.
        } else {
            bytes.down as f64 / total
        } as f32;
        let p = self.p;
        canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                let span = bounds.size.width * (total / maximum).clamp(0., 1.) as f32;
                let gap = if bytes.down > 0 && bytes.up > 0 {
                    px(1.).min(span * 0.05)
                } else {
                    px(0.)
                };
                let down = (span - gap) * share;
                let up = span - gap - down;
                for (offset, width, color, round_right) in [
                    (px(0.), down, p.download, bytes.up == 0),
                    (down + gap, up, p.upload, true),
                ] {
                    if width > px(0.) {
                        let segment = Bounds::new(
                            point(bounds.left() + offset, bounds.top()),
                            size(width, bounds.size.height),
                        );
                        let radius = (bounds.size.height / 2.).min(width / 2.);
                        let right = if round_right { radius } else { px(0.) };
                        window.paint_quad(fill(segment, rgb(color)).corner_radii(Corners {
                            top_left: px(0.),
                            bottom_left: px(0.),
                            top_right: right,
                            bottom_right: right,
                        }));
                    }
                }
            },
        )
        .h(px(5.))
        .w_full()
    }
    fn recent_usage_row(
        &self,
        app: &AppComparison,
        full: bool,
        maximum: f64,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let p = self.p;
        let key = app.key.clone();
        let bytes = app.metrics.current;
        let tooltip = format!(
            "{}\n下载 {} · 上传 {} · 合计 {}",
            key,
            human_bytes(bytes.down),
            human_bytes(bytes.up),
            human_bytes(bytes.total())
        );
        let content = column()
            .flex_1()
            .min_w_0()
            .gap(px(5.))
            .child(
                row()
                    .justify_between()
                    .gap(px(8.))
                    .child(
                        label(self.application_name(&key).to_string(), p.text)
                            .flex_1()
                            .min_w_0()
                            .truncate(),
                    )
                    .child(
                        label(human_bytes(bytes.total()), p.text)
                            .text_size(px(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .pr(px(5.))
                            .flex_none(),
                    ),
            )
            .child(
                row()
                    .w_full()
                    .h(px(22.))
                    .gap(px(8.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h(px(5.))
                            .child(self.usage_split_bar(bytes, maximum)),
                    )
                    .child(row().gap(px(5.)).flex_none().children(
                        [(false, bytes.down), (true, bytes.up)].into_iter().map(
                            |(upload, value)| {
                                let color = if upload { p.upload } else { p.download };
                                row()
                                    .w(px(72.))
                                    .h(px(22.))
                                    .px(px(5.))
                                    .gap(px(4.))
                                    .rounded(px(4.))
                                    .bg(tint_surface(p, color))
                                    .justify_between()
                                    .child(
                                        label(if upload { "↑" } else { "↓" }, color)
                                            .text_size(px(13.))
                                            .flex_none(),
                                    )
                                    .child(
                                        label(human_bytes(value), p.text)
                                            .text_size(px(11.))
                                            .text_right()
                                            .flex_none(),
                                    )
                            },
                        ),
                    )),
            );
        row()
            .id(SharedString::from(format!("recent-usage-{full}-{key}")))
            .h(px(if full { 56. } else { 52. }))
            .gap(px(10.))
            .px(px(4.))
            .rounded(px(7.))
            .cursor_pointer()
            .hover(move |style| style.bg(state_surface(p)))
            .child(self.app_badge(&key))
            .child(content)
            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            .on_click(cx.listener(move |app, _, window, cx| {
                app.act(
                    Action::Application(ApplicationAction::Select(key.clone())),
                    window,
                    cx,
                )
            }))
    }
    pub(super) fn activity_details_page(&self, compact: bool, cx: &Context<Self>) -> Div {
        let p = self.p;
        let title = row()
            .gap(px(12.))
            .child(self.button(
                "activity-back",
                "← 返回",
                false,
                Action::CloseActivityDetails,
                cx,
            ))
            .child(heading("应用活动", p));
        let bounds = self.analysis.bounds[0].clone();
        let tabs = self
            .card()
            .relative()
            .p(px(6.))
            .flex_row()
            .gap(px(4.))
            .child(
                canvas(move |rect, _, _| bounds.set(rect), |_, _, _, _| {})
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
            )
            .children(ActivityTab::ALL.into_iter().map(|tab| {
                self.button(
                    format!("activity-tab-{tab:?}"),
                    tab.label(),
                    self.activity.tab == tab,
                    Action::ActivityDetails(tab),
                    cx,
                )
                .flex_1()
                .h(px(36.))
                .justify_center()
            }));
        let mut page = column().gap(px(16.)).child(title).child(tabs);
        match self.activity.tab {
            ActivityTab::Current => {
                page = page
                    .child(self.live_activity_summary(compact))
                    .child(self.full_activity_table(false, cx));
            }
            ActivityTab::Recent => {
                let mut controls = self.card().gap(px(8.)).child(
                    row()
                        .justify_between()
                        .child(heading("近5分钟用量", p))
                        .child(self.recent_refresh_button("recent-full-refresh", cx)),
                );
                if let Some(snapshot) = &self.activity.recent_snapshot {
                    controls = controls
                        .child(
                            label(
                                format!(
                                    "{} — {} · 秒级采样",
                                    stamp(snapshot.current_from()),
                                    stamp(snapshot.request.anchor)
                                ),
                                p.text,
                            )
                            .whitespace_normal(),
                        )
                        .child(
                            label(
                                format!(
                                    "快照生成于 {} · 覆盖 {}分{:02}秒 / 5分钟 · {}",
                                    stamp(snapshot.generated_at),
                                    snapshot.coverage(false) / 60_000,
                                    snapshot.coverage(false) / 1000 % 60,
                                    if self.activity.recent_provisional {
                                        if self.activity.recent_pending.is_some() {
                                            "正在补齐最新采样"
                                        } else {
                                            "采样尚未补齐，可刷新"
                                        }
                                    } else if self.activity.recent_pending.is_some() {
                                        "正在更新，保留原快照"
                                    } else {
                                        self.config.recent_usage_refresh_mode.label()
                                    }
                                ),
                                p.muted,
                            )
                            .text_size(px(11.))
                            .whitespace_normal(),
                        );
                    let bytes = snapshot.metrics(None).current;
                    page = page.child(controls).child(self.activity_summary_cards(
                        compact,
                        [
                            ("总用量", human_bytes(bytes.total()), p.text),
                            ("下载", human_bytes(bytes.down), p.download),
                            ("上传", human_bytes(bytes.up), p.upload),
                            (
                                "有用量的应用",
                                format!("{} 项", self.recent_usage_rows(false).len()),
                                p.text,
                            ),
                        ],
                    ));
                } else {
                    page = page.child(controls);
                }
                if !self.activity.recent_error.is_empty() {
                    page = page.child(
                        label(self.activity.recent_error.clone(), p.amber).whitespace_normal(),
                    );
                }
                page = page.child(self.full_activity_table(true, cx));
            }
        }
        page
    }
    fn activity_summary_cards(&self, compact: bool, values: [(&str, String, u32); 4]) -> Div {
        let p = self.p;
        let mut cards: Vec<_> = values
            .into_iter()
            .map(|(title, value, color)| {
                self.card()
                    .flex_1()
                    .min_w_0()
                    .min_h(px(104.))
                    .gap(px(8.))
                    .child(label(title.to_string(), p.muted))
                    .child(label(value, color).text_size(px(23.)).truncate())
            })
            .collect();
        if compact {
            let rest = cards.split_off(2);
            column()
                .gap(px(12.))
                .child(row().gap(px(12.)).children(cards))
                .child(row().gap(px(12.)).children(rest))
        } else {
            row().gap(px(12.)).children(cards)
        }
    }
    fn live_activity_summary(&self, compact: bool) -> Div {
        let p = self.p;
        self.activity_summary_cards(
            compact,
            [
                (
                    "当前下载",
                    if self.data.connected {
                        speed(self.data.down)
                    } else {
                        "—".into()
                    },
                    p.download,
                ),
                (
                    "当前上传",
                    if self.data.connected {
                        speed(self.data.up)
                    } else {
                        "—".into()
                    },
                    p.upload,
                ),
                (
                    "正在传输",
                    format!(
                        "{} 项",
                        activity(&self.data)
                            .iter()
                            .filter(|app| app.state() == ActivityState::Transferring)
                            .count()
                    ),
                    p.text,
                ),
                (
                    "当前连接",
                    format!("{} 条", self.data.connections.len()),
                    p.text,
                ),
            ],
        )
    }
    fn full_activity_table(&self, recent: bool, cx: &Context<Self>) -> Div {
        let p = self.p;
        let bounds = self.activity.table_bounds.clone();
        let mut table = self
            .card()
            .relative()
            .child(
                canvas(move |rect, _, _| bounds.set(rect), |_, _, _, _| {})
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
            )
            .child(heading(
                if recent {
                    "应用用量"
                } else {
                    "实时应用"
                },
                p,
            ))
            .child(self.text_input(if recent {
                &self.activity.recent_search
            } else {
                &self.activity.live_search
            }));
        let live = self.filtered_live_activity();
        let recent_rows = self.recent_usage_rows(true);
        let maximum = self.recent_usage_maximum();
        let count = if recent {
            recent_rows.len()
        } else {
            live.len()
        };
        let page = (if recent {
            self.activity.recent_page
        } else {
            self.activity.live_page
        })
        .min(count.saturating_sub(1) / 10);
        if !recent {
            table = table.child(
                row()
                    .gap(px(8.))
                    .child(label("应用", p.muted).flex_1().min_w_0())
                    .child(label("下载速度", p.muted).w(px(100.)).text_right())
                    .child(label("上传速度", p.muted).w(px(100.)).text_right())
                    .child(label("连接 / 状态", p.muted).w(px(110.)).text_right()),
            );
        }
        let mut recent_list = column().gap(px(0.));
        for index in page * 10..(page * 10 + 10).min(count) {
            if recent {
                recent_list =
                    recent_list.child(self.recent_usage_row(recent_rows[index], true, maximum, cx));
                continue;
            }
            let app = &live[index];
            let (key, down, up, last) = (
                app.key.clone(),
                speed(app.rate.down as f64),
                speed(app.rate.up as f64),
                format!("{} 条 · {}", app.connections, app.state().label()),
            );
            let tooltip = key.clone();
            let selected = key.clone();
            table = table.child(
                row()
                    .id(SharedString::from(format!("activity-row-{recent}-{key}")))
                    .h(px(64.))
                    .gap(px(8.))
                    .px(px(4.))
                    .border_b_1()
                    .border_color(surface_border(p))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .hover(move |style| style.bg(state_surface(p)))
                    .child(self.app_badge(&key))
                    .child(
                        column()
                            .flex_1()
                            .min_w_0()
                            .gap(px(3.))
                            .child(
                                label(self.application_name(&key).to_string(), p.text).truncate(),
                            )
                            .child(label(key, p.muted).text_size(px(11.)).truncate()),
                    )
                    .child(label(down, p.download).w(px(100.)).text_right().flex_none())
                    .child(label(up, p.upload).w(px(100.)).text_right().flex_none())
                    .child(
                        label(last, p.text)
                            .w(px(110.))
                            .text_right()
                            .text_size(px(11.))
                            .flex_none(),
                    )
                    .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
                    .on_click(cx.listener(move |app, _, window, cx| {
                        app.act(
                            Action::Application(ApplicationAction::Select(selected.clone())),
                            window,
                            cx,
                        )
                    })),
            );
        }
        if recent && count > 0 {
            table = table.child(recent_list);
        }
        if count == 0 {
            table = table.child(self.empty(
                if recent && self.activity.recent_pending.is_some() {
                    "正在生成快照"
                } else {
                    "没有匹配的应用"
                },
                if recent {
                    "快照仅包含本期已观测用量，可刷新或调整搜索"
                } else {
                    "实时列表包含正在传输及仍有连接的应用"
                },
            ));
        }
        let pages = count.div_ceil(10).max(1);
        table.child(
            row()
                .justify_between()
                .flex_wrap()
                .gap(px(8.))
                .child(
                    label(
                        format!("共 {count} 项 · 第 {} / {pages} 页", page + 1),
                        p.muted,
                    )
                    .text_size(px(12.)),
                )
                .child(
                    row()
                        .gap(px(4.))
                        .child(
                            self.button(
                                "activity-prev",
                                "上一页",
                                false,
                                Action::ActivityPage(-1),
                                cx,
                            )
                            .when(page == 0, |button| button.opacity(0.45)),
                        )
                        .child(
                            self.button(
                                "activity-next",
                                "下一页",
                                false,
                                Action::ActivityPage(1),
                                cx,
                            )
                            .when(page + 1 >= pages, |button| button.opacity(0.45)),
                        ),
                ),
        )
    }
}
