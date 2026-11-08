use super::*;
use crate::analysis::{
    AnalysisRange, AnalysisRequest, AppComparison, Snapshot, change_text, signed_bytes,
};
use chrono::{Datelike, Timelike};
use std::sync::Arc;
#[path = "histogram.rs"]
mod histogram;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Sort {
    #[default]
    Change,
    Total,
    Peak,
    Name,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AnalysisScope {
    Historical(i64, i64),
    Recent(AnalysisRange),
}
pub(super) struct AnalysisState {
    pub active: bool,
    pub range: AnalysisRange,
    pub snapshot: Option<Arc<Snapshot>>,
    pub pending: Option<AnalysisRequest>,
    pub provisional: bool,
    next_id: u64,
    pub error: String,
    pub focus: Option<String>,
    pub highlight: Option<i64>,
    pub bucket_sizes: [crate::analysis::BucketSize; 3],
    pub history_bucket: Option<crate::analysis::BucketSize>,
    pub compare_previous: bool,
    pub pinned_bucket: Option<i64>,
    pub show_trends: bool,
    pub offsets: [Point<Pixels>; 2],
    pub restore_offset: Option<Point<Pixels>>,
    scope: Option<(u64, AnalysisScope)>,
    pub sort: Sort,
    pub ascending: bool,
    pub app_page: usize,
    pub burst_page: usize,
    pub search: Entity<InputState>,
    pub search_value: String,
    pub anchor: Option<usize>,
    pub bounds: [Rc<Cell<Bounds<Pixels>>>; 4],
    pub summary_bounds: Rc<Cell<Bounds<Pixels>>>,
}
impl AnalysisState {
    pub fn new(window: &mut Window, cx: &mut Context<TrafficApp>) -> Self {
        Self {
            active: false,
            range: AnalysisRange::FiveMinutes,
            snapshot: None,
            pending: None,
            provisional: false,
            next_id: 0,
            error: String::new(),
            focus: None,
            highlight: None,
            bucket_sizes: AnalysisRange::ALL.map(AnalysisRange::default_bucket),
            history_bucket: None,
            compare_previous: true,
            pinned_bucket: None,
            show_trends: false,
            offsets: [point(px(0.), px(0.)); 2],
            restore_offset: None,
            scope: None,
            sort: Sort::Change,
            ascending: false,
            app_page: 0,
            burst_page: 0,
            search: input("", "搜索应用名称或完整路径…", false, window, cx),
            search_value: String::new(),
            anchor: None,
            bounds: std::array::from_fn(|_| Rc::new(Cell::new(Bounds::default()))),
            summary_bounds: Rc::new(Cell::new(Bounds::default())),
        }
    }
}

fn stamp(time: i64) -> String {
    chrono::DateTime::from_timestamp_millis(time)
        .map(|date| {
            date.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_default()
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
fn comparison_datetime(time: i64, seconds: bool, p: Palette) -> Div {
    let Some(date) = chrono::DateTime::from_timestamp_millis(time) else {
        return label("—", p.muted);
    };
    let date = date.with_timezone(&chrono::Local);
    let part = |number: String, unit: &str| {
        row()
            .items_baseline()
            .gap(px(2.))
            .child(
                label(number, p.text)
                    .font_family("Segoe UI")
                    .text_size(px(13.)),
            )
            .child(label(unit.to_owned(), p.muted).text_size(px(11.)))
    };
    row()
        .gap(px(12.))
        .flex_none()
        .child(
            row()
                .items_baseline()
                .gap(px(5.))
                .child(part(date.year().to_string(), "年"))
                .child(part(format!("{:02}", date.month()), "月"))
                .child(part(format!("{:02}", date.day()), "日")),
        )
        .child(
            row()
                .items_baseline()
                .gap(px(5.))
                .child(part(format!("{:02}", date.hour()), "时"))
                .child(part(format!("{:02}", date.minute()), "分"))
                .when(seconds, |d| {
                    d.child(part(format!("{:02}", date.second()), "秒"))
                }),
        )
}

fn comparison_period(
    title: &str,
    from: i64,
    to: i64,
    seconds: bool,
    p: Palette,
    current: bool,
) -> Div {
    row()
        .gap(px(12.))
        .items_start()
        .child(
            label(
                title.to_owned(),
                if current { p.accent_text } else { p.muted },
            )
            .w(px(30.))
            .flex_none()
            .text_size(px(13.))
            .font_weight(FontWeight::MEDIUM),
        )
        .child(
            row()
                .gap(px(10.))
                .flex_wrap()
                .min_w_0()
                .child(comparison_datetime(from, seconds, p))
                .child(label("→", p.muted).text_size(px(12.)).flex_none())
                .child(comparison_datetime(to, seconds, p)),
        )
}
fn measured(bounds: Rc<Cell<Bounds<Pixels>>>) -> impl IntoElement {
    canvas(move |rect, _, _| bounds.set(rect), |_, _, _, _| {})
        .absolute()
        .top_0()
        .left_0()
        .size_full()
}
fn coverage_text(snapshot: &Snapshot) -> String {
    let duration = snapshot.request.duration() / 1000;
    let render = |previous| {
        let seconds = snapshot.coverage(previous) / 1000;
        format!(
            "{}分{:02}秒 / {}分",
            seconds / 60,
            seconds % 60,
            duration / 60
        )
    };
    if snapshot.comparable() {
        "两期采样充分 · 峰值为1分钟平均速度".into()
    } else {
        format!(
            "{}：本期 {}，前期 {} · 缺失区间不按零计算",
            if snapshot.request.range == AnalysisRange::FiveMinutes {
                "采样覆盖"
            } else {
                "完整分钟覆盖"
            },
            render(false),
            render(true)
        )
    }
}
fn volume_change(snapshot: &Snapshot, current: u64, previous: u64) -> String {
    if snapshot.comparable() {
        change_text(current, previous, true)
    } else if snapshot.can_compare_observed() {
        "仅观测变化".into()
    } else {
        "数据不足".into()
    }
}
fn comparison_headline(snapshot: &Snapshot, metrics: &crate::analysis::Metrics) -> String {
    if !snapshot.can_compare_observed() {
        return if snapshot.coverage(false) == 0 {
            "本期尚无采样，暂无法对比"
        } else {
            "前期尚无采样，暂无法对比"
        }
        .into();
    }
    let delta = metrics.delta();
    let change = if delta == 0 {
        "与前期持平".into()
    } else {
        format!(
            "比前期{} {}",
            if delta > 0 { "增加" } else { "减少" },
            human_bytes(delta.unsigned_abs().min(u64::MAX as u128) as u64)
        )
    };
    if !snapshot.comparable() {
        format!("{change} · 仅观测变化")
    } else if metrics.previous.total() == 0 {
        if metrics.current.total() > 0 {
            format!("{change} · 本期新增")
        } else {
            change
        }
    } else {
        format!(
            "{change} · {}",
            change_text(metrics.current.total(), metrics.previous.total(), true)
        )
    }
}
fn peak_change(snapshot: &Snapshot, metrics: &crate::analysis::Metrics) -> String {
    if snapshot.comparable() {
        return change_text(
            metrics.peak.map(|p| p.bytes.total()).unwrap_or(0),
            metrics.previous_peak.map(|p| p.bytes.total()).unwrap_or(0),
            true,
        );
    }
    match (metrics.peak, metrics.previous_peak) {
        (Some(current), Some(previous)) => {
            let delta = current.rate() - previous.rate();
            format!(
                "观测 {}{}",
                if delta > 0. {
                    "+"
                } else if delta < 0. {
                    "−"
                } else {
                    ""
                },
                speed(delta.abs())
            )
        }
        _ => "数据不足".into(),
    }
}

#[derive(Clone)]
pub(super) enum AnalysisAction {
    Range(crate::analysis::AnalysisRange),
    Refresh,
    Sort(analysis_page::Sort),
    Page(i32, bool),
    Focus(Option<String>),
    Burst(String, i64),
    Bucket(crate::analysis::BucketSize),
    Compare,
    Pin(i64),
    Details(bool),
    Events,
}

impl TrafficApp {
    pub(super) fn handle_analysis_action(
        &mut self,
        action: AnalysisAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            AnalysisAction::Range(range) => {
                self.history_state.analysis_period = None;
                self.history_state.error.clear();
                self.close_picker(window, cx);
                self.analysis.app_page = 0;
                self.request_analysis(range);
            }
            AnalysisAction::Refresh => {
                self.close_picker(window, cx);
                if self.analysis.pending.is_none() {
                    if let Some(period) = self.history_state.analysis_period
                        && let Ok(Some(bounds)) = period.bounds(chrono::Local::now())
                    {
                        self.history_state.analysis_range = bounds;
                    }
                    self.request_analysis(self.analysis.range);
                }
            }
            AnalysisAction::Sort(sort) => {
                self.analysis.ascending = if self.analysis.sort == sort {
                    !self.analysis.ascending
                } else {
                    sort == analysis_page::Sort::Name
                };
                self.analysis.sort = sort;
                self.analysis.app_page = 0;
            }
            AnalysisAction::Page(step, bursts) => {
                self.switch_analysis_details(bursts);
                let maximum = if bursts {
                    self.analysis_bursts().len().saturating_sub(1) / 5
                } else {
                    self.analysis_rows().len().saturating_sub(1) / 10
                };
                let page = if bursts {
                    &mut self.analysis.burst_page
                } else {
                    &mut self.analysis.app_page
                };
                *page = page.saturating_add_signed(step as isize).min(maximum);
                self.analysis.anchor = Some(if bursts { 3 } else { 2 });
                self.analysis.restore_offset = None;
            }
            AnalysisAction::Focus(key) => {
                self.switch_analysis_details(true);
                self.analysis.focus = key;
                self.analysis.burst_page = 0;
                self.analysis.highlight = None;
                self.analysis.pinned_bucket = None;
                self.analysis.anchor = Some(1);
                self.analysis.restore_offset = None;
            }
            AnalysisAction::Burst(key, time) => {
                self.switch_analysis_details(true);
                self.analysis.focus = Some(key);
                self.analysis.burst_page = 0;
                self.analysis.highlight = Some(time);
                self.analysis.pinned_bucket = Some(time);
                self.analysis.anchor = Some(1);
                self.analysis.restore_offset = None;
            }
            AnalysisAction::Bucket(size) => {
                if let Some(snapshot) = &self.analysis.snapshot
                    && snapshot.request.bucket_sizes().contains(&size)
                {
                    if snapshot.request.windows.is_some() {
                        self.analysis.history_bucket = Some(size);
                    } else {
                        self.analysis.bucket_sizes[snapshot.request.range.index()] = size;
                    }
                }
            }
            AnalysisAction::Compare => {
                self.analysis.compare_previous = !self.analysis.compare_previous
            }
            AnalysisAction::Pin(time) => {
                let duration = self
                    .analysis
                    .snapshot
                    .as_ref()
                    .map(|snapshot| self.analysis_bucket(snapshot).millis())
                    .unwrap_or(0);
                self.analysis.pinned_bucket = if self
                    .analysis
                    .pinned_bucket
                    .is_some_and(|pinned| pinned >= time && pinned < time + duration)
                {
                    None
                } else {
                    Some(time)
                };
                self.analysis.highlight = None;
            }
            AnalysisAction::Details(bursts) => {
                self.switch_analysis_details(bursts);
            }
            AnalysisAction::Events => {
                self.switch_analysis_details(true);
                self.analysis.restore_offset = None;
                self.analysis.anchor = Some(3);
            }
        }
    }

    pub(super) fn analysis_bucket(&self, snapshot: &Snapshot) -> crate::analysis::BucketSize {
        if snapshot.request.windows.is_some() {
            self.analysis
                .history_bucket
                .filter(|s| snapshot.request.bucket_sizes().contains(s))
                .unwrap_or_else(|| snapshot.request.default_bucket())
        } else {
            self.analysis.bucket_sizes[snapshot.request.range.index()]
        }
    }
    pub(super) fn next_snapshot_request(&mut self, range: AnalysisRange) -> AnalysisRequest {
        self.analysis.next_id += 1;
        AnalysisRequest::new(
            self.analysis.next_id,
            self.backend.analysis_generation(),
            range,
            chrono::Utc::now().timestamp_millis(),
        )
    }
    pub(super) fn request_analysis(&mut self, range: AnalysisRange) {
        self.activity.recent_pending = None;
        self.analysis.range = range;
        let short = self.next_snapshot_request(range);
        let scope = (
            short.generation,
            if self.history_state.analysis_period.is_some() {
                let (from, to) = self.history_state.analysis_range;
                AnalysisScope::Historical(from, to)
            } else {
                AnalysisScope::Recent(range)
            },
        );
        // Changing the requested scope must hide old values even when the new
        // request fails validation or cannot be queued.
        if self.analysis.scope != Some(scope) {
            self.analysis.snapshot = None;
            self.analysis.provisional = false;
            self.analysis.focus = None;
            self.analysis.highlight = None;
            self.analysis.pinned_bucket = None;
            self.analysis.app_page = 0;
            self.analysis.burst_page = 0;
        }
        self.analysis.scope = Some(scope);
        self.analysis.pending = None;
        let request = if let AnalysisScope::Historical(from, to) = scope.1 {
            match AnalysisRequest::historical(
                short.id,
                short.generation,
                from,
                to,
                chrono::Utc::now().timestamp_millis(),
            ) {
                Ok(request) => request,
                Err(error) => {
                    self.analysis.error = error.to_string();
                    self.analysis.pending = None;
                    return;
                }
            }
        } else {
            short
        };
        self.analysis.error.clear();
        if self.backend.query_analysis(request.clone()) {
            self.analysis.pending = Some(request);
        } else {
            self.analysis.pending = None;
            self.analysis.error = "查询队列繁忙，请重试".into();
            if let Some(snapshot) = &self.analysis.snapshot
                && snapshot.request.windows.is_none()
            {
                self.analysis.range = snapshot.request.range;
            }
        }
    }
    pub(super) fn poll_analysis(&mut self, cx: &mut Context<Self>) {
        let search = self.analysis.search.read(cx).value().to_string();
        if search != self.analysis.search_value {
            self.analysis.search_value = search;
            self.analysis.app_page = 0;
        }
        let reply = self.backend.analysis_result();
        if let Some(reply) = reply
            && self.analysis.pending.as_ref() == Some(&reply.request)
        {
            if reply.complete {
                self.analysis.pending = None;
            }
            match reply.result {
                Ok(snapshot)
                    if snapshot.request.generation != self.backend.analysis_generation() =>
                {
                    self.analysis.pending = None;
                    self.analysis.error = "数据源已变化，请刷新快照".into();
                }
                Ok(snapshot) => {
                    if !reply.complete
                        && self.analysis.snapshot.as_ref().is_some_and(|old| {
                            Arc::ptr_eq(old, &snapshot)
                                || (!self.analysis.provisional
                                    && old.request.generation == snapshot.request.generation)
                        })
                    {
                        return;
                    }
                    self.analysis.provisional = !reply.complete;
                    self.analysis.highlight = None;
                    self.analysis.pinned_bucket = None;
                    if self
                        .analysis
                        .focus
                        .as_ref()
                        .is_some_and(|key| !snapshot.apps.contains_key(key))
                    {
                        self.analysis.focus = None;
                    }
                    self.analysis.snapshot = Some(snapshot);
                    self.analysis.error.clear();
                    let count = self.analysis_rows().len();
                    self.analysis.app_page =
                        self.analysis.app_page.min(count.saturating_sub(1) / 10);
                    let count = self.analysis_bursts().len();
                    self.analysis.burst_page =
                        self.analysis.burst_page.min(count.saturating_sub(1) / 5);
                }
                Err(error) => {
                    self.analysis.pending = None;
                    self.analysis.error = error;
                    if let Some(snapshot) = &self.analysis.snapshot
                        && snapshot.request.windows.is_none()
                    {
                        self.analysis.range = snapshot.request.range;
                    }
                }
            }
        }
        if self.analysis.snapshot.as_ref().is_some_and(|snapshot| {
            snapshot.request.generation != self.backend.analysis_generation()
        }) {
            self.analysis.error = "数据源已变化，当前仍为旧快照，请刷新".into();
        }
    }
    pub(super) fn schedule_analysis_anchor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.page != Page::History
            || !self.selected.is_empty()
            || !self.analysis.active
            || self.analysis.pending.is_some()
        {
            return;
        }
        let Some(target) = self.analysis.anchor.take() else {
            return;
        };
        let request = self
            .analysis
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.request.clone());
        let trends = self.analysis.show_trends;
        let weak = cx.weak_entity();
        window.on_next_frame(move |_, cx| {
            let _ = weak.update(cx, |app, cx| {
                if app.page != Page::History
                    || !app.analysis.active
                    || !app.selected.is_empty()
                    || app.analysis.show_trends != trends
                    || app.analysis.snapshot.as_ref().map(|s| &s.request) != request.as_ref()
                {
                    return;
                }
                let bounds = app.analysis.bounds[target].get();
                if bounds.size.height <= px(0.) {
                    return;
                }
                let scroll = &app.page_scroll;
                let y = scroll.offset().y + scroll.bounds().top() + px(16.) - bounds.top();
                scroll.set_offset(point(
                    scroll.offset().x,
                    y.clamp(-scroll.max_offset().height, px(0.)),
                ));
                cx.notify();
            });
        });
    }
    pub(super) fn analysis_effective_sort(&self) -> Sort {
        if self.analysis.sort == Sort::Change
            && self
                .analysis
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| !snapshot.can_compare_observed())
        {
            Sort::Total
        } else {
            self.analysis.sort
        }
    }
    pub(super) fn analysis_rows(&self) -> Vec<&AppComparison> {
        let Some(snapshot) = &self.analysis.snapshot else {
            return vec![];
        };
        let search = self.analysis.search_value.trim().to_lowercase();
        let mut rows: Vec<_> = snapshot
            .rows
            .iter()
            .filter(|row| {
                row.key.to_lowercase().contains(&search)
                    || self
                        .application_name(&row.key)
                        .to_lowercase()
                        .contains(&search)
            })
            .collect();
        let sort = self.analysis_effective_sort();
        rows.sort_by(|a, b| {
            let order = match sort {
                Sort::Name => a.key.cmp(&b.key),
                Sort::Change => a
                    .metrics
                    .delta()
                    .unsigned_abs()
                    .cmp(&b.metrics.delta().unsigned_abs()),
                Sort::Total => a.metrics.current.total().cmp(&b.metrics.current.total()),
                Sort::Peak => a
                    .metrics
                    .peak
                    .map(|peak| peak.bytes.total())
                    .unwrap_or(0)
                    .cmp(&b.metrics.peak.map(|peak| peak.bytes.total()).unwrap_or(0)),
            };
            let order = if self.analysis.ascending {
                order
            } else {
                order.reverse()
            };
            order.then_with(|| a.key.cmp(&b.key))
        });
        rows
    }
    pub(super) fn analysis_bursts(&self) -> Vec<&crate::analysis::Burst> {
        self.analysis
            .snapshot
            .as_ref()
            .map(|snapshot| {
                snapshot
                    .bursts
                    .iter()
                    .filter(|burst| {
                        self.analysis
                            .focus
                            .as_ref()
                            .is_none_or(|key| key == &burst.app)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
    pub(super) fn analysis_refresh_button(&self, cx: &Context<Self>) -> Stateful<Div> {
        self.button(
            "analysis-refresh",
            if self.analysis.pending.is_some() {
                "更新中…"
            } else {
                "刷新"
            },
            false,
            Action::Analysis(AnalysisAction::Refresh),
            cx,
        )
        .h(px(32.))
        .when(self.analysis.pending.is_some(), |d| d.opacity(0.45))
    }
    pub(super) fn switch_analysis_details(&mut self, trends: bool) {
        if self.analysis.show_trends == trends {
            return;
        }
        self.analysis.offsets[usize::from(self.analysis.show_trends)] = self.page_scroll.offset();
        self.analysis.show_trends = trends;
        self.analysis.restore_offset = Some(self.analysis.offsets[usize::from(trends)]);
        self.analysis.anchor = None;
    }
    pub(super) fn analysis_panel(&self, compact: bool, cx: &Context<Self>) -> Div {
        let p = self.p;
        let mut panel = column().gap(px(12.));
        if let Some(snapshot) = &self.analysis.snapshot {
            let metadata = format!(
                "{} · 生成于 {}\n{}\n进入对比、切换时段或手动刷新时生成快照；浏览列表与切换标签复用快照。",
                snapshot.request.range.precision(),
                stamp(snapshot.generated_at),
                coverage_text(snapshot)
            );
            let seconds = snapshot.request.windows.is_none()
                && snapshot.request.range == AnalysisRange::FiveMinutes;
            let period_stamp = |time| {
                chrono::DateTime::from_timestamp_millis(time)
                    .map(|date| {
                        date.with_timezone(&chrono::Local)
                            .format(if seconds {
                                "%Y年%m月%d日 %H时%M分%S秒"
                            } else {
                                "%Y年%m月%d日 %H时%M分"
                            })
                            .to_string()
                    })
                    .unwrap_or_default()
            };
            panel = panel.child(
                column()
                    .gap(px(6.))
                    .child(comparison_period(
                        "本期",
                        snapshot.current_from(),
                        snapshot.request.anchor,
                        seconds,
                        p,
                        true,
                    ))
                    .child(comparison_period(
                        "前期",
                        snapshot.previous_from(),
                        snapshot.previous_to(),
                        seconds,
                        p,
                        false,
                    ))
                    .child(
                        row()
                            .gap(px(12.))
                            .flex_wrap()
                            .child(
                                label(
                                    if snapshot.request.windows.is_some() {
                                        "与前一完整周期的同期部分比较"
                                    } else {
                                        "与前一个等长时段比较"
                                    },
                                    p.muted,
                                )
                                .text_size(px(11.)),
                            )
                            .child(
                                label(
                                    format!(
                                        "{}截止 {}",
                                        if self.analysis.pending.is_some() {
                                            "上次快照 · "
                                        } else if self.analysis.provisional {
                                            "采样未补齐 · "
                                        } else {
                                            ""
                                        },
                                        period_stamp(snapshot.request.anchor)
                                    ),
                                    p.muted,
                                )
                                .text_size(px(11.))
                                .id("analysis-snapshot-info")
                                .tooltip(move |window, cx| {
                                    Tooltip::new(metadata.clone()).build(window, cx)
                                }),
                            ),
                    ),
            );
        }
        if !self.analysis.error.is_empty() {
            panel = panel.child(
                label(self.analysis.error.clone(), p.amber)
                    .text_size(px(12.))
                    .whitespace_normal(),
            );
        }
        if let Some(snapshot) = &self.analysis.snapshot {
            panel = panel
                .child(self.analysis_summary(snapshot, compact, cx))
                .child(self.analysis_detail_tabs(cx));
            if self.analysis.show_trends {
                self.analysis.bounds[2].set(Bounds::default());
                panel = panel
                    .child(
                        column()
                            .relative()
                            .gap(px(12.))
                            .child(measured(self.analysis.bounds[1].clone()))
                            .child(self.analysis_trend_summary(snapshot, cx))
                            .child(self.analysis_chart(snapshot, compact, cx)),
                    )
                    .child(self.analysis_events(compact, cx));
            } else {
                self.analysis.bounds[1].set(Bounds::default());
                self.analysis.bounds[3].set(Bounds::default());
                panel = panel.child(self.analysis_table(compact, cx));
            }
        } else {
            for bounds in &self.analysis.bounds[1..] {
                bounds.set(Bounds::default());
            }
            self.analysis.summary_bounds.set(Bounds::default());
            panel = panel.child(
                self.empty(
                    if self.analysis.pending.is_some() {
                        "正在生成对比"
                    } else {
                        "暂无对比数据"
                    },
                    "选择时段，查看用量变化和应用排行",
                )
                .min_h(px(200.)),
            );
        }
        panel
    }
    fn analysis_summary(&self, snapshot: &Snapshot, compact: bool, _: &Context<Self>) -> Div {
        let p = self.p;
        let metrics = snapshot.metrics(None);
        let value = |previous| {
            let bytes = if previous {
                metrics.previous
            } else {
                metrics.current
            };
            if snapshot.coverage(previous) > 0 || bytes.total() > 0 {
                human_bytes(bytes.total())
            } else {
                "—".into()
            }
        };
        let direction = |previous| {
            let bytes = if previous {
                metrics.previous
            } else {
                metrics.current
            };
            let observed = snapshot.coverage(previous) > 0 || bytes.total() > 0;
            row()
                .gap(px(10.))
                .flex_wrap()
                .child(
                    label(
                        format!(
                            "↓ {}",
                            if observed {
                                human_bytes(bytes.down)
                            } else {
                                "—".into()
                            }
                        ),
                        p.download_text,
                    )
                    .text_size(px(11.)),
                )
                .child(
                    label(
                        format!(
                            "↑ {}",
                            if observed {
                                human_bytes(bytes.up)
                            } else {
                                "—".into()
                            }
                        ),
                        p.upload_text,
                    )
                    .text_size(px(11.)),
                )
        };
        let mut summary = column()
            .relative()
            .gap(px(8.))
            .py(px(8.))
            .child(measured(self.analysis.summary_bounds.clone()))
            .child(
                label(comparison_headline(snapshot, &metrics), p.text)
                    .text_size(px(if compact { 24. } else { 28. }))
                    .font_weight(FontWeight::MEDIUM)
                    .whitespace_normal(),
            )
            .child(
                row()
                    .gap(px(28.))
                    .items_start()
                    .child(
                        column()
                            .flex_1()
                            .min_w_0()
                            .gap(px(3.))
                            .child(
                                label(format!("本期总用量  {}", value(false)), p.text)
                                    .text_size(px(14.)),
                            )
                            .child(direction(false)),
                    )
                    .child(
                        column()
                            .flex_1()
                            .min_w_0()
                            .gap(px(3.))
                            .child(
                                label(format!("前期总用量  {}", value(true)), p.muted)
                                    .text_size(px(14.)),
                            )
                            .child(direction(true)),
                    ),
            );
        if !snapshot.comparable() {
            summary = summary.child(
                label(coverage_text(snapshot), p.muted)
                    .text_size(px(11.))
                    .whitespace_normal(),
            );
        }
        summary
    }
    fn analysis_trend_summary(&self, snapshot: &Snapshot, cx: &Context<Self>) -> Div {
        let p = self.p;
        let metrics = snapshot.metrics(self.analysis.focus.as_deref());
        let mut scope = row().gap(px(8.)).min_w_0();
        if let Some(key) = &self.analysis.focus {
            scope = scope
                .child(self.application_icon(key, true))
                .child(
                    label(self.application_name(key).to_string(), p.text)
                        .flex_1()
                        .min_w_0()
                        .truncate(),
                )
                .child(self.button(
                    "analysis-clear-focus",
                    "全部应用",
                    false,
                    Action::Analysis(AnalysisAction::Focus(None)),
                    cx,
                ));
        } else {
            scope = scope.child(label("全部应用", p.text).font_weight(FontWeight::MEDIUM));
        }
        let peak_hint = format!(
            "峰值为1分钟平均速度\n{}\n较前期 {}",
            metrics
                .peak
                .map(|peak| format!("{} — {}", stamp(peak.time), stamp(peak.time + 60_000)))
                .unwrap_or_else(|| "暂无完整分钟数据".into()),
            peak_change(snapshot, &metrics)
        );
        let mut summary = column().gap(px(6.)).child(scope);
        if self.analysis.focus.is_some() {
            summary = summary.child(
                label(
                    format!(
                        "本期 {} · 前期 {} · {}",
                        if snapshot.coverage(false) > 0 || metrics.current.total() > 0 {
                            human_bytes(metrics.current.total())
                        } else {
                            "—".into()
                        },
                        if snapshot.coverage(true) > 0 || metrics.previous.total() > 0 {
                            human_bytes(metrics.previous.total())
                        } else {
                            "—".into()
                        },
                        comparison_headline(snapshot, &metrics)
                    ),
                    p.muted,
                )
                .text_size(px(12.))
                .whitespace_normal(),
            );
        }
        summary.child(
            row()
                .gap(px(16.))
                .flex_wrap()
                .child(
                    label(
                        format!(
                            "分钟峰值  {}",
                            metrics
                                .peak
                                .map(|peak| speed(peak.rate()))
                                .unwrap_or_else(|| "—".into())
                        ),
                        p.muted,
                    )
                    .text_size(px(12.))
                    .id("analysis-peak-info")
                    .tooltip(move |window, cx| Tooltip::new(peak_hint.clone()).build(window, cx)),
                )
                .child(
                    self.button(
                        "analysis-show-bursts",
                        format!("暴增记录 {} 次  ↓", self.analysis_bursts().len()),
                        false,
                        Action::Analysis(AnalysisAction::Events),
                        cx,
                    )
                    .h(px(24.))
                    .px(px(0.))
                    .text_size(px(12.)),
                ),
        )
    }
    fn analysis_detail_tabs(&self, cx: &Context<Self>) -> Div {
        row()
            .gap(px(16.))
            .border_b_1()
            .border_color(surface_border(self.p))
            .child(
                self.button(
                    "analysis-details-apps",
                    "用量对比",
                    !self.analysis.show_trends,
                    Action::Analysis(AnalysisAction::Details(false)),
                    cx,
                )
                .h(px(32.)),
            )
            .child(
                self.button(
                    "analysis-details-bursts",
                    "走势与异常",
                    self.analysis.show_trends,
                    Action::Analysis(AnalysisAction::Details(true)),
                    cx,
                )
                .h(px(32.)),
            )
    }
    fn analysis_pager(
        &self,
        count: usize,
        size: usize,
        page: usize,
        bursts: bool,
        cx: &Context<Self>,
    ) -> Div {
        let pages = count.div_ceil(size).max(1);
        row()
            .justify_between()
            .flex_wrap()
            .gap(px(6.))
            .child(
                label(
                    format!("共 {count} 项 · 第 {} / {pages} 页", page + 1),
                    self.p.muted,
                )
                .text_size(px(12.)),
            )
            .child(
                row()
                    .gap(px(4.))
                    .child(
                        self.button(
                            if bursts {
                                "burst-prev"
                            } else {
                                "analysis-prev"
                            },
                            "上一页",
                            false,
                            Action::Analysis(AnalysisAction::Page(-1, bursts)),
                            cx,
                        )
                        .when(page == 0, |d| d.opacity(0.45)),
                    )
                    .child(
                        self.button(
                            if bursts {
                                "burst-next"
                            } else {
                                "analysis-next"
                            },
                            "下一页",
                            false,
                            Action::Analysis(AnalysisAction::Page(1, bursts)),
                            cx,
                        )
                        .when(page + 1 >= pages, |d| d.opacity(0.45)),
                    ),
            )
    }
    fn analysis_table(&self, compact: bool, cx: &Context<Self>) -> Div {
        let p = self.p;
        let snapshot = self.analysis.snapshot.as_ref().unwrap();
        let rows = self.analysis_rows();
        let effective_sort = self.analysis_effective_sort();
        let page = self
            .analysis
            .app_page
            .min(rows.len().saturating_sub(1) / 10);
        let widths = if compact {
            [96., 96., 112.]
        } else {
            [120., 120., 132.]
        };
        let sort_name = |sort| match sort {
            Sort::Change => "变化量",
            Sort::Total => "本期用量",
            Sort::Peak => "分钟峰值",
            Sort::Name => "应用名称",
        };
        let mut table = column()
            .relative()
            .gap(px(8.))
            .child(measured(self.analysis.bounds[2].clone()))
            .child(
                row()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(self.text_input(&self.analysis.search)),
                    )
                    .child(
                        self.taskbar_picker(
                            "analysis-sort",
                            format!(
                                "{} {}",
                                sort_name(effective_sort),
                                if self.analysis.ascending {
                                    "↑"
                                } else {
                                    "↓"
                                }
                            ),
                            [Sort::Change, Sort::Total, Sort::Peak, Sort::Name]
                                .into_iter()
                                .map(|sort| {
                                    (
                                        sort_name(sort).into(),
                                        Action::Analysis(AnalysisAction::Sort(sort)),
                                        effective_sort == sort,
                                    )
                                })
                                .collect(),
                            cx,
                        ),
                    ),
            );
        let sort_header = |id: &str, name: &str, sort| {
            self.button(
                id.to_string(),
                format!(
                    "{name}{}",
                    if effective_sort == sort {
                        if self.analysis.ascending {
                            " ↑"
                        } else {
                            " ↓"
                        }
                    } else {
                        ""
                    }
                ),
                effective_sort == sort,
                Action::Analysis(AnalysisAction::Sort(sort)),
                cx,
            )
            .h(px(30.))
        };
        table = table.child(
            row()
                .gap(px(8.))
                .px(px(4.))
                .border_b_1()
                .border_color(surface_border(p))
                .child(
                    sort_header("analysis-sort-name", "应用", Sort::Name)
                        .flex_1()
                        .min_w_0()
                        .justify_start(),
                )
                .child(
                    sort_header("analysis-sort-0", "本期用量", Sort::Total)
                        .w(px(widths[0]))
                        .flex_none()
                        .justify_end(),
                )
                .child(
                    label("前期用量", p.muted)
                        .w(px(widths[1]))
                        .flex_none()
                        .text_right()
                        .text_size(px(12.)),
                )
                .child(
                    sort_header("analysis-sort-1", "变化", Sort::Change)
                        .w(px(widths[2]))
                        .flex_none()
                        .justify_end(),
                ),
        );
        let mut body = column().gap(px(0.));
        for app in rows.iter().skip(page * 10).take(10) {
            let key = app.key.clone();
            let selected = key.clone();
            let m = &app.metrics;
            let value = |previous| {
                let bytes = if previous { m.previous } else { m.current };
                if snapshot.coverage(previous) > 0 || bytes.total() > 0 {
                    human_bytes(bytes.total())
                } else {
                    "—".into()
                }
            };
            let peak = m
                .peak
                .map(|peak| speed(peak.rate()))
                .unwrap_or_else(|| "—".into());
            let tooltip = format!(
                "{}\n本期：下载 {} · 上传 {} · 合计 {}\n前期：下载 {} · 上传 {} · 合计 {}\n分钟平均峰值 {} · {}\n峰值较前期 {}\n{}\n点击查看此应用的走势与异常",
                key,
                human_bytes(m.current.down),
                human_bytes(m.current.up),
                value(false),
                human_bytes(m.previous.down),
                human_bytes(m.previous.up),
                value(true),
                peak,
                m.peak
                    .map(|peak| format!("{} — {}", stamp(peak.time), stamp(peak.time + 60_000)))
                    .unwrap_or_else(|| "暂无完整分钟".into()),
                peak_change(snapshot, m),
                coverage_text(snapshot)
            );
            body = body.child(
                row()
                    .id(SharedString::from(format!("analysis-app-{key}")))
                    .h(px(60.))
                    .gap(px(8.))
                    .px(px(4.))
                    .border_b_1()
                    .border_color(surface_border(p))
                    .cursor_pointer()
                    .hover(move |s| s.bg(state_surface(p)))
                    .child(self.app_badge(&key))
                    .child(
                        column()
                            .flex_1()
                            .min_w_0()
                            .gap(px(3.))
                            .child(
                                label(self.application_name(&key).to_string(), p.text).truncate(),
                            )
                            .when(
                                m.current.total() == 0
                                    || (m.previous.total() == 0 && snapshot.comparable()),
                                |d| {
                                    d.child(
                                        label(
                                            if m.current.total() == 0 {
                                                "本期未观测到用量"
                                            } else {
                                                "本期新增"
                                            },
                                            p.muted,
                                        )
                                        .text_size(px(11.)),
                                    )
                                },
                            ),
                    )
                    .child(
                        label(value(false), p.text)
                            .w(px(widths[0]))
                            .flex_none()
                            .text_right(),
                    )
                    .child(
                        label(value(true), p.muted)
                            .w(px(widths[1]))
                            .flex_none()
                            .text_right(),
                    )
                    .child(
                        column()
                            .w(px(widths[2]))
                            .flex_none()
                            .items_end()
                            .gap(px(3.))
                            .child(label(
                                if snapshot.can_compare_observed() {
                                    signed_bytes(m.delta())
                                } else {
                                    "—".into()
                                },
                                p.text,
                            ))
                            .child(
                                label(
                                    volume_change(snapshot, m.current.total(), m.previous.total()),
                                    p.muted,
                                )
                                .text_size(px(11.)),
                            ),
                    )
                    .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
                    .on_click(cx.listener(move |app, _, window, cx| {
                        app.act(
                            Action::Analysis(AnalysisAction::Focus(Some(selected.clone()))),
                            window,
                            cx,
                        )
                    })),
            );
        }
        table = table.child(body);
        if rows.is_empty() {
            table = table.child(self.empty(
                if snapshot.rows.is_empty() {
                    "本时段没有应用用量记录"
                } else {
                    "没有匹配的应用"
                },
                "调整搜索或选择其他时段",
            ));
        }
        table.child(self.analysis_pager(rows.len(), 10, page, false, cx))
    }
    fn analysis_events(&self, _: bool, cx: &Context<Self>) -> Div {
        let p = self.p;
        let events = self.analysis_bursts();
        let page = self
            .analysis
            .burst_page
            .min(events.len().saturating_sub(1) / 5);
        let mut card = self
            .card()
            .relative()
            .child(measured(self.analysis.bounds[3].clone()))
            .child(row().justify_between().flex_wrap().gap(px(8.))
                .child(label("暴增记录", p.text).font_weight(FontWeight::MEDIUM))
                .child(label("判定规则 ⓘ", p.muted).text_size(px(11.)).id("analysis-burst-rule")
                    .tooltip(|window, cx| Tooltip::new("按应用、按分钟判断：至少为前5个连续有效分钟平均值的3倍，且增加至少5 MiB。缺口分钟不参与判断。").build(window, cx))));
        for (index, event) in events.iter().skip(page * 5).take(5).enumerate() {
            let key = event.app.clone();
            let time = event.time;
            card = card.child(
                row()
                    .gap(px(12.))
                    .flex_wrap()
                    .py(px(10.))
                    .border_b_1()
                    .border_color(surface_border(p))
                    .child(self.application_icon(&key, true))
                    .child(
                        column()
                            .gap(px(3.))
                            .flex_1()
                            .min_w(px(130.))
                            .child(
                                label(self.application_name(&key).to_string(), p.text).truncate(),
                            )
                            .child(label(stamp(time), p.muted).text_size(px(11.))),
                    )
                    .child(
                        column()
                            .gap(px(3.))
                            .child(label(speed(event.bytes.total() as f64 / 60.), p.amber))
                            .child(
                                label(
                                    if event.baseline == 0 {
                                        "从静默开始".into()
                                    } else {
                                        format!(
                                            "基线 {:.1} 倍",
                                            event.bytes.total() as f64 / event.baseline as f64
                                        )
                                    },
                                    p.muted,
                                )
                                .text_size(px(11.)),
                            ),
                    )
                    .child(label(
                        format!("增加 {}", human_bytes(event.increase())),
                        p.text,
                    ))
                    .child(self.button(
                        format!("analysis-burst-{index}"),
                        "定位",
                        false,
                        Action::Analysis(AnalysisAction::Burst(key, time)),
                        cx,
                    )),
            );
        }
        if events.is_empty() {
            card = card.child(self.empty(
                "没有检测到明显暴增",
                "需有连续有效的分钟数据；采集缺口及轻微波动不标记",
            ));
        }
        card.child(self.analysis_pager(events.len(), 5, page, true, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot(partial: bool) -> Snapshot {
        let request = AnalysisRequest::new(1, 0, AnalysisRange::FiveMinutes, 1_800_000);
        let mut samples = crate::analysis::SampleWindow::default();
        for second in 0..600 {
            if partial && second == 100 {
                continue;
            }
            let from = request.anchor - 600_000 + second * 1000;
            samples.record(
                from,
                from + 1000,
                Bytes::default(),
                &std::collections::HashMap::new(),
            );
        }
        samples.snapshot(request)
    }
    #[::core::prelude::v1::test]
    fn headlines_distinguish_changes_new_usage_zero_and_missing_coverage() {
        let full = snapshot(false);
        assert!(full.comparable());
        let mut metrics = crate::analysis::Metrics::default();
        assert_eq!(comparison_headline(&full, &metrics), "与前期持平");
        metrics.current.down = 2048;
        assert!(comparison_headline(&full, &metrics).contains("本期新增"));
        assert!(!comparison_headline(&full, &metrics).contains('%'));
        metrics.previous.down = 1024;
        assert!(comparison_headline(&full, &metrics).contains("增加"));
        assert!(comparison_headline(&full, &metrics).contains("+100.0%"));
        metrics.current.down = 512;
        assert!(comparison_headline(&full, &metrics).contains("减少"));
        assert!(comparison_headline(&full, &metrics).contains("-50.0%"));
        let partial = snapshot(true);
        assert!(partial.can_compare_observed() && !partial.comparable());
        let text = comparison_headline(&partial, &metrics);
        assert!(text.contains("仅观测变化") && !text.contains('%'));
        metrics.previous.down = 0;
        assert!(!comparison_headline(&partial, &metrics).contains("新增"));
        let empty = crate::analysis::SampleWindow::default().snapshot(full.request.clone());
        assert_eq!(
            comparison_headline(&empty, &metrics),
            "本期尚无采样，暂无法对比"
        );
    }
}
