use super::*;
use crate::speed_stats::{Scope, SecondsData, SecondsRequest, SpeedStats};
use chrono::Local;

pub(super) struct State {
    pub inline_history: bool,
    pub locate_open: bool,
    pub hover_help_open: bool,
    pub hover_card_width: Cell<Pixels>,
    pub scope: Option<Scope>,
    pub data: Option<SecondsData>,
    pub displayed: Option<(u64, u64)>,
    pub pending: Option<SecondsRequest>,
    pub sent: bool,
    pub serial: u64,
    pub error: String,
    pub setting_error: String,
    pub days: Entity<InputState>,
    pub locator: drill_time::Locator,
    pub(super) title: String,
    pub(super) focus: FocusHandle,
    pub(super) previous_focus: Option<FocusHandle>,
    pub(super) controls: Vec<FocusHandle>,
    pub(super) scroll: ScrollHandle,
    pub bounds: Rc<Cell<Bounds<Pixels>>>,
    pub view: Option<crate::traffic_drill::View>,
    pub path: Vec<crate::traffic_drill::View>,
    pub loaded_request: Option<SecondsRequest>,
    pub hover: Option<crate::traffic_drill::Target>,
    pub pinned: bool,
    pub expanded: bool,
    pub hover_data: Option<crate::traffic_drill::HoverData>,
    pub hover_error: String,
    pub hover_pending: Option<crate::traffic_drill::HoverRequest>,
    pub hover_sent: bool,
    pub hover_cache:
        std::collections::HashMap<crate::traffic_drill::Target, crate::traffic_drill::HoverData>,
    pub minute_bounds: Rc<Cell<Bounds<Pixels>>>,
    pub second_bounds: Rc<Cell<Bounds<Pixels>>>,
}
impl State {
    pub fn is_modal(&self) -> bool {
        self.scope.is_some() && !self.inline_history
    }
    pub fn new(c: &Config, window: &mut Window, cx: &mut Context<TrafficApp>) -> Self {
        Self {
            inline_history: false,
            locate_open: false,
            hover_help_open: false,
            hover_card_width: Cell::new(px(240.)),
            view: None,
            path: vec![],
            loaded_request: None,
            hover: None,
            pinned: false,
            expanded: false,
            hover_data: None,
            hover_error: String::new(),
            hover_pending: None,
            hover_sent: false,
            hover_cache: Default::default(),
            minute_bounds: Default::default(),
            second_bounds: Default::default(),
            scope: None,
            data: None,
            displayed: None,
            pending: None,
            sent: false,
            serial: 0,
            error: String::new(),
            setting_error: String::new(),
            days: input(
                &storage_settings::format_amount(
                    u64::from(c.seconds_days),
                    c.seconds_time_unit.multiplier(),
                ),
                "1",
                false,
                window,
                cx,
            ),
            locator: drill_time::Locator::new(window, cx),
            title: String::new(),
            focus: cx.focus_handle(),
            previous_focus: None,
            controls: (0..6).map(|_| cx.focus_handle()).collect(),
            scroll: ScrollHandle::new(),
            bounds: Rc::new(Cell::new(Bounds::default())),
        }
    }
}
pub(super) fn date(time: i64) -> String {
    chrono::DateTime::from_timestamp_millis(time)
        .map(|d| {
            d.with_timezone(&Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|| "—".into())
}
fn duration(ms: u64) -> String {
    if ms < 60_000 {
        format!("{:.1}秒", ms as f64 / 1000.)
    } else {
        format!(
            "{}小时{:02}分{:02}秒",
            ms / 3_600_000,
            (ms / 60_000) % 60,
            (ms / 1000) % 60
        )
    }
}
fn peak_hint(value: Option<crate::speed_stats::Peak>) -> String {
    value.map_or_else(
        || "此时段暂无有效速度摘要；旧历史不补录。".into(),
        |p| {
            format!(
                "峰值发生于 {}\n按实际采样间隔计算，上下行分别统计。",
                date(p.at)
            )
        },
    )
}
pub(super) fn coverage_note(stats: Option<&SpeedStats>) -> String {
    match stats {
        Some(s) if s.covered_ms > 0 && s.partial() => {
            let percent = (s.covered_ms as f64 / s.expected_ms.max(1) as f64 * 100.).min(100.);
            let value = if percent < 0.1 {
                "<0.1%".into()
            } else if percent > 99.9 {
                ">99.9%".into()
            } else {
                format!("{percent:.1}%")
            };
            format!("统计覆盖不足 · 有效覆盖 {value}（旧历史及空档不计）")
        }
        Some(s) if s.covered_ms > 0 => "正常闲时计入均速，采集空档不计".into(),
        _ => "暂无有效速度摘要，旧历史不补录".into(),
    }
}

#[derive(Clone)]
pub(super) enum DrillAction {
    SecondsOpen(Option<apps_page::DetailContext>),
    SecondsClose,
    DrillBack(usize),
    DrillExpand,
    DrillUnpin,
    DrillHelpToggle,
    DrillLocateToggle,
    DrillFollowTime,
    SecondsMove(i64),
    SecondsLocate,
    SecondsRefresh,
    SecondsToggle,
    SecondsTimeUnit(RetentionTimeUnit),
    SecondsLongTerm,
}

impl TrafficApp {
    pub(super) fn handle_drill_action(
        &mut self,
        action: DrillAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            DrillAction::SecondsOpen(context) => self.open_seconds(context, window, cx),
            DrillAction::SecondsClose => self.close_seconds(window, cx),
            DrillAction::DrillBack(index) => self.drill_back(index, cx),
            DrillAction::DrillExpand => {
                self.seconds.expanded = !self.seconds.expanded;
            }
            DrillAction::DrillUnpin => {
                self.seconds.hover_help_open = false;
                self.seconds.pinned = false;
                self.seconds.hover = None;
                self.seconds.hover_data = None;
                self.seconds.hover_pending = None;
            }
            DrillAction::DrillHelpToggle => {
                self.seconds.hover_help_open = !self.seconds.hover_help_open;
            }
            DrillAction::DrillLocateToggle => {
                self.seconds.locate_open = !self.seconds.locate_open;
                if self.seconds.locate_open {
                    self.edit_drill_time(window, cx);
                    self.seconds.locator.parts[0].focus_handle(cx).focus(window);
                } else {
                    window.blur();
                }
            }
            DrillAction::DrillFollowTime => self.follow_drill_time(window, cx),
            DrillAction::SecondsMove(offset) => self.move_drill(offset, cx),
            DrillAction::SecondsLocate => self.locate_seconds(window, cx),
            DrillAction::SecondsRefresh => self.request_seconds(
                self.seconds
                    .pending
                    .as_ref()
                    .and_then(|r| r.from)
                    .or_else(|| self.seconds.data.as_ref().map(|d| d.from)),
                cx,
            ),
            DrillAction::SecondsToggle => {
                if self.config.seconds_enabled {
                    self.config.seconds_enabled = false;
                    self.seconds.setting_error.clear();
                    self.save_config();
                } else {
                    self.save_seconds_setting(window, cx);
                    if self.seconds.setting_error.is_empty() {
                        self.config.seconds_enabled = true;
                        self.save_config();
                    }
                }
            }
            DrillAction::SecondsTimeUnit(unit) => {
                self.save_seconds_setting(window, cx);
                if self.seconds.setting_error.is_empty() {
                    self.config.seconds_time_unit = unit;
                    self.config.seconds_days_enabled = true;
                    self.sync_seconds_inputs(window, cx);
                    self.save_config();
                }
            }
            DrillAction::SecondsLongTerm => {
                self.config.seconds_days_enabled = false;
                self.seconds.setting_error.clear();
                self.sync_seconds_inputs(window, cx);
                self.save_config();
            }
        }
    }

    pub(super) fn history_speed_cell(
        &self,
        id: usize,
        stats: Option<&SpeedStats>,
        average: bool,
        width: f32,
    ) -> Stateful<Div> {
        let p = self.p;
        let (down, up, hint) = if average {
            let avg = stats.and_then(SpeedStats::average);
            (
                avg.map(|v| v.0),
                avg.map(|v| v.1),
                stats.filter(|s| s.covered_ms > 0).map_or_else(
                    || coverage_note(stats),
                    |s| {
                        format!(
                            "有效采集 {}\n统计截至 {}\n{}",
                            duration(s.covered_ms),
                            s.as_of.map(date).unwrap_or_default(),
                            coverage_note(stats)
                        )
                    },
                ),
            )
        } else {
            (
                stats.and_then(|s| s.down_peak).map(|p| p.rate),
                stats.and_then(|s| s.up_peak).map(|p| p.rate),
                format!(
                    "下载：{}\n上传：{}",
                    peak_hint(stats.and_then(|s| s.down_peak)),
                    peak_hint(stats.and_then(|s| s.up_peak))
                ),
            )
        };
        let hint = if average {
            format!(
                "下载：{}\n上传：{}\n{hint}",
                down.map(speed).unwrap_or_else(|| "—".into()),
                up.map(speed).unwrap_or_else(|| "—".into())
            )
        } else {
            hint
        };
        column()
            .w(px(width))
            .flex_none()
            .items_end()
            .gap(px(2.))
            .id(("history-row-speed", id * 2 + usize::from(average)))
            .child(
                label(down.map(speed).unwrap_or_else(|| "—".into()), p.text)
                    .text_size(px(12.))
                    .whitespace_nowrap(),
            )
            .child(
                label(up.map(speed).unwrap_or_else(|| "—".into()), p.muted)
                    .text_size(px(12.))
                    .whitespace_nowrap(),
            )
            .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
    }
    pub(super) fn detail_speed_summary(
        &self,
        id: &'static str,
        stats: Option<&SpeedStats>,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        self.speed_summary_content(id, stats, false, false, cx)
    }
    fn speed_summary_content(
        &self,
        id: &'static str,
        stats: Option<&SpeedStats>,
        show_seconds: bool,
        show_note: bool,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let p = self.p;
        let peak = stats.and_then(|s| s.down_peak);
        let up = stats.and_then(|s| s.up_peak);
        let avg = stats.and_then(SpeedStats::average);
        let average_hint = stats.filter(|s| s.covered_ms > 0).map_or_else(
            || "此时段暂无有效速度摘要；旧历史不补录。".into(),
            |s| {
                format!(
                    "有效采集 {}\n统计截至 {}\n有效字节 ÷ 有效时长；正常闲时计为零，采集空档不计。\n{}",
                    duration(s.covered_ms),
                    s.as_of.map(date).unwrap_or_default(),
                    coverage_note(stats)
                )
            },
        );
        let values = |a: Option<f64>, b: Option<f64>| {
            row()
                .gap(px(10.))
                .flex_none()
                .whitespace_nowrap()
                .child(
                    label(
                        format!("↓ {}", a.map(speed).unwrap_or_else(|| "—".into())),
                        p.download_text,
                    )
                    .text_size(px(13.)),
                )
                .child(
                    label(
                        format!("↑ {}", b.map(speed).unwrap_or_else(|| "—".into())),
                        p.upload_text,
                    )
                    .text_size(px(13.)),
                )
        };
        let peak_group = column()
            .gap(px(5.))
            .flex_none()
            .child(label("时段峰值", p.muted).text_size(px(12.)))
            .child(
                row()
                    .gap(px(10.))
                    .flex_none()
                    .whitespace_nowrap()
                    .child(
                        label(
                            format!(
                                "↓ {}",
                                peak.map(|v| speed(v.rate)).unwrap_or_else(|| "—".into())
                            ),
                            p.download_text,
                        )
                        .text_size(px(13.))
                        .id((id, 0usize))
                        .tooltip(move |window, cx| Tooltip::new(peak_hint(peak)).build(window, cx)),
                    )
                    .child(
                        label(
                            format!(
                                "↑ {}",
                                up.map(|v| speed(v.rate)).unwrap_or_else(|| "—".into())
                            ),
                            p.upload_text,
                        )
                        .text_size(px(13.))
                        .id((id, 1usize))
                        .tooltip(move |window, cx| Tooltip::new(peak_hint(up)).build(window, cx)),
                    ),
            );
        let avg_group = column()
            .gap(px(5.))
            .flex_none()
            .id((id, 2usize))
            .tooltip(move |window, cx| Tooltip::new(average_hint.clone()).build(window, cx))
            .child(label("时段均速", p.muted).text_size(px(12.)))
            .child(values(avg.map(|v| v.0), avg.map(|v| v.1)));
        let note = coverage_note(stats);
        column()
            .gap(px(6.))
            .min_w_0()
            .id(id)
            .child(
                row()
                    .items_end()
                    .gap(px(20.))
                    .flex_wrap()
                    .child(peak_group)
                    .child(avg_group)
                    .when(show_seconds, |d| {
                        d.child(
                            self.button(
                                "open-second-details",
                                "秒级明细",
                                false,
                                Action::Drill(DrillAction::SecondsOpen(None)),
                                cx,
                            )
                            .h(px(28.))
                            .px(px(8.))
                            .text_size(px(12.)),
                        )
                    }),
            )
            .when(show_note, |d| {
                d.child(
                    label(
                        note,
                        if stats.is_some_and(|s| s.covered_ms > 0 && s.partial()) {
                            p.amber
                        } else {
                            p.muted
                        },
                    )
                    .text_size(px(11.))
                    .whitespace_normal(),
                )
            })
    }
    pub(super) fn subscribe_seconds(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.seconds.days.clone();
        let sub = cx.subscribe_in(&input, window, |app, _, event, window, cx| {
            match event {
                InputEvent::Blur | InputEvent::PressEnter { .. } => {
                    app.save_seconds_setting(window, cx)
                }
                InputEvent::Change => app.seconds.setting_error.clear(),
                _ => {}
            }
            cx.notify();
        });
        self._subscriptions.push(sub);
        self.subscribe_drill_locator(window, cx);
    }
    pub(super) fn save_seconds_setting(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.migration_busy {
            return;
        }
        if !self.config.seconds_days_enabled {
            return;
        }
        let input = &self.seconds.days;
        let multiplier = self.config.seconds_time_unit.multiplier();
        let maximum = 3650;
        let saved = u64::from(self.config.seconds_days);
        let current = storage_settings::format_amount(saved, multiplier);
        if input.read(cx).value().trim() == current {
            if input.read(cx).value().as_ref() != current {
                input.update(cx, |s, cx| s.set_value(current, window, cx));
            }
            self.seconds.setting_error.clear();
            return;
        }
        let Some(value) =
            storage_settings::parse_amount(input.read(cx).value().as_ref(), multiplier, maximum)
        else {
            self.seconds.setting_error = "保留时间须为1–3650天；1个月按30天换算".into();
            return;
        };
        self.seconds.setting_error.clear();
        let changed = u64::from(self.config.seconds_days) != value;
        self.config.seconds_days = value as u32;
        if changed {
            self.save_config();
        }
        self.sync_seconds_inputs(window, cx);
    }
    pub(super) fn sync_seconds_inputs(&self, window: &mut Window, cx: &mut Context<Self>) {
        let (input, value) = (
            &self.seconds.days,
            storage_settings::format_amount(
                u64::from(self.config.seconds_days),
                self.config.seconds_time_unit.multiplier(),
            ),
        );
        if self.seconds.setting_error.is_empty()
            && !input.read(cx).focus_handle(cx).is_focused(window)
            && input.read(cx).value().as_ref() != value
        {
            input.update(cx, |s, cx| s.set_value(value, window, cx));
        }
    }
    pub(super) fn seconds_settings(&self, cx: &Context<Self>) -> Stateful<Div> {
        let p = self.p;
        let status = &self.data.seconds_status;
        let mut times: Vec<_> = RetentionTimeUnit::ALL
            .into_iter()
            .map(|u| {
                (
                    u.label().into(),
                    Action::Drill(DrillAction::SecondsTimeUnit(u)),
                    self.config.seconds_days_enabled && u == self.config.seconds_time_unit,
                )
            })
            .collect();
        times.push((
            "长期".into(),
            Action::Drill(DrillAction::SecondsLongTerm),
            !self.config.seconds_days_enabled,
        ));
        let time_label = if self.config.seconds_days_enabled {
            self.config.seconds_time_unit.label()
        } else {
            "长期"
        };
        column()
            .gap(px(8.))
            .id("seconds-storage-settings")
            .child(storage_settings::option_row(
                "秒级明细",
                if self.config.seconds_enabled {
                    "已开启，保留每秒原始采样"
                } else {
                    "按需保存每秒原始采样"
                },
                row()
                    .gap(px(6.))
                    .flex_none()
                    .child(self.storage_policy_part(
                        1,
                        0,
                        div().w_full().when(self.config.seconds_days_enabled, |d| {
                            d.child(self.text_input(&self.seconds.days))
                        }),
                        cx,
                    ))
                    .child(self.storage_policy_part(
                        1,
                        1,
                        self.taskbar_picker("seconds-time-unit", time_label.into(), times, cx),
                        cx,
                    ))
                    .child(self.storage_policy_part(
                        1,
                        2,
                        self.switch_control(
                            "seconds-save-enabled",
                            "保存秒级明细",
                            self.config.seconds_enabled,
                            Action::Drill(DrillAction::SecondsToggle),
                            cx,
                        ),
                        cx,
                    )),
                p,
            ))
            .when(!self.seconds.setting_error.is_empty(), |d| {
                d.child(
                    label(self.seconds.setting_error.clone(), p.amber)
                        .text_size(px(12.))
                        .whitespace_normal(),
                )
            })
            .when(!status.error.is_empty(), |d| {
                d.child(
                    label(status.error.clone(), p.amber)
                        .text_size(px(12.))
                        .whitespace_normal(),
                )
            })
    }
}
