use super::*;
use crate::exit_ip;

const EXIT_ROTATION: Duration = Duration::from_secs(8);
const EXIT_TRANSITION: Duration = Duration::from_millis(360);

#[derive(Default)]
pub(super) struct ExitCard {
    ip_page: bool,
    paused: Option<bool>,
    hovered: bool,
    deadline: Option<Instant>,
    elapsed: Duration,
    animation: Option<Instant>,
    source: Option<exit_ip::Source>,
    pub(super) results: [exit_ip::Probe; 2],
    selected: Option<exit_ip::Family>,
    error: Option<&'static str>,
    request: u64,
}

impl ExitCard {
    fn loading(&self) -> bool {
        self.results
            .iter()
            .any(|result| matches!(result, exit_ip::Probe::Loading))
    }

    fn selected_family(&self) -> exit_ip::Family {
        self.selected.unwrap_or_else(|| {
            if self.results[0].detection().is_some() || self.results[1].detection().is_none() {
                exit_ip::Family::V4
            } else {
                exit_ip::Family::V6
            }
        })
    }

    fn elapsed_at(&self, now: Instant) -> Duration {
        self.deadline.map_or(self.elapsed, |deadline| {
            EXIT_ROTATION.saturating_sub(deadline.saturating_duration_since(now))
        })
    }

    fn freeze(&mut self, now: Instant) {
        self.elapsed = self.elapsed_at(now);
        self.deadline = None;
    }

    fn rotation_due(&mut self, now: Instant, running: bool) -> bool {
        if !running {
            self.freeze(now);
            return false;
        }
        now >= *self
            .deadline
            .get_or_insert(now + EXIT_ROTATION.saturating_sub(self.elapsed))
    }
}

#[derive(Default)]
pub(super) struct State {
    pub(super) exit: ExitCard,
    pub show_saved: bool,
    pub subscription_scroll: ScrollHandle,
    pub subscription_scope: String,
    pub usage_source: Option<(u64, u64)>,
    pub hover: Option<super::overview_hover::Hover>,
    pub hover_data: Option<crate::overview_usage::Detail>,
    pub hover_error: String,
    pub pending: Option<crate::overview_usage::Request>,
    pub sequence: u64,
    pub close_sequence: u64,
    pub hover_suppressed: Option<usize>,
    pub cache: HashMap<(i64, i64), (Instant, crate::overview_usage::Detail)>,
    pub bar_bounds: Rc<RefCell<HashMap<usize, Bounds<Pixels>>>>,
    pub hover_bounds: Rc<Cell<Bounds<Pixels>>>,
    #[cfg(test)]
    pub bounds: Rc<RefCell<HashMap<&'static str, Bounds<Pixels>>>>,
}

fn title(text: &'static str, icon: &'static str, p: Palette) -> Div {
    row()
        .gap(px(7.))
        .min_w_0()
        .child(
            svg()
                .path(icon)
                .size(px(15.))
                .flex_none()
                .text_color(rgb(p.muted)),
        )
        .child(
            label(text, p.text)
                .text_size(px(13.))
                .font_weight(FontWeight::MEDIUM)
                .whitespace_nowrap(),
        )
}

fn value(text: String, size: f32, color: u32, p: Palette) -> Div {
    let (number, unit) = text.split_once(' ').unwrap_or((&text, ""));
    row()
        .gap(px(5.))
        .items_end()
        .min_w_0()
        .child(
            label(number.to_string(), color)
                .font_family("Segoe UI")
                .font_weight(FontWeight::MEDIUM)
                .text_size(px(size))
                .line_height(px(size + 4.))
                .min_w_0()
                .truncate(),
        )
        .when(!unit.is_empty(), |d| {
            d.child(
                label(unit.to_string(), p.muted)
                    .text_size(px(12.))
                    .line_height(px(18.))
                    .pb(px(3.))
                    .flex_none(),
            )
        })
}

#[derive(Clone)]
pub(super) enum OverviewAction {
    SubscriptionSource(bool),
    CloseHover,
    RangeHistory(crate::overview_usage::Bin),
    RefreshSubscriptions,
    ExitPage(bool),
    ExitPause,
    ExitDetect,
    ExitFamily(crate::exit_ip::Family),
    Activity,
    Records(&'static str),
}

impl TrafficApp {
    pub(super) fn handle_overview_action(
        &mut self,
        action: OverviewAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            OverviewAction::SubscriptionSource(saved) => {
                self.overview_state.show_saved = saved;
                self.overview_state
                    .subscription_scroll
                    .set_offset(point(px(0.), px(0.)));
            }
            OverviewAction::CloseHover => self.dismiss_overview_hover(window.mouse_position(), cx),
            OverviewAction::RangeHistory(bin) => self.open_overview_range(bin, None, window, cx),
            OverviewAction::RefreshSubscriptions => self.backend.refresh_subscriptions(),
            OverviewAction::ExitPage(ip) => self.show_overview_exit(ip),
            OverviewAction::ExitPause => self.toggle_overview_exit_rotation(),
            OverviewAction::ExitDetect => self.detect_overview_exit(cx),
            OverviewAction::ExitFamily(family) => self.select_overview_exit_family(family),
            OverviewAction::Activity => {
                self.act(Action::Nav(Page::Apps), window, cx);
                self.act(
                    Action::ActivityDetails(activity_page::ActivityTab::Current),
                    window,
                    cx,
                );
            }
            OverviewAction::Records(dimension) => {
                if dimension == "app" {
                    self.act(Action::Nav(Page::Apps), window, cx);
                } else {
                    self.act(Action::Nav(Page::History), window, cx);
                    self.act(
                        Action::History(HistoryAction::Tab(HistoryTab::Records)),
                        window,
                        cx,
                    );
                    self.act(Action::Dimension(dimension), window, cx);
                }
            }
        }
    }

    pub(super) fn show_overview_exit(&mut self, ip_page: bool) {
        let state = &mut self.overview_state.exit;
        if state.ip_page != ip_page {
            let now = Instant::now();
            // Both panes exchange roles. Reverse the same curve when interrupted.
            let reverse = state.animation.map_or(Duration::ZERO, |started| {
                EXIT_TRANSITION.saturating_sub(now.saturating_duration_since(started))
            });
            state.ip_page = ip_page;
            state.animation = native::client_area_animations_enabled().then_some(now - reverse);
        }
        state.deadline = None;
        state.elapsed = Duration::ZERO;
    }

    fn overview_exit_paused(&self) -> bool {
        self.overview_state
            .exit
            .paused
            .unwrap_or_else(|| !native::client_area_animations_enabled())
    }

    pub(super) fn toggle_overview_exit_rotation(&mut self) {
        let paused = !self.overview_exit_paused();
        let state = &mut self.overview_state.exit;
        state.paused = Some(paused);
        state.freeze(Instant::now());
    }

    pub(super) fn reset_overview_exit_hover(&mut self) {
        let state = &mut self.overview_state.exit;
        state.hovered = false;
        state.freeze(Instant::now());
    }

    pub(super) fn select_overview_exit_family(&mut self, family: exit_ip::Family) {
        if self.overview_state.exit.results[family.index()]
            .detection()
            .is_some()
        {
            self.overview_state.exit.selected = Some(family);
        }
    }

    pub(super) fn animate_overview_exit(&mut self, window: &Window) {
        let active = self.page == Page::Overview
            && window.is_window_active()
            && native::visible(self.main_hwnd);
        let paused = self.overview_exit_paused();
        let now = Instant::now();
        let state = &mut self.overview_state.exit;
        let running = active && !paused && !state.hovered && !state.loading();
        if state.rotation_due(now, running) {
            let ip = !state.ip_page;
            self.show_overview_exit(ip);
        }
        let state = &mut self.overview_state.exit;
        if state
            .animation
            .is_some_and(|started| !active || now.duration_since(started) >= EXIT_TRANSITION)
        {
            state.animation = None;
        }
        if running || state.animation.is_some() {
            window.request_animation_frame();
        }
    }

    pub(super) fn tick_overview_exit(&mut self, active: bool, cx: &mut Context<Self>) {
        let state = &mut self.overview_state.exit;
        if state.source.is_some() && state.source != exit_ip::Source::current(&self.backend) {
            state.source = None;
            state.results = Default::default();
            state.selected = None;
            state.error = None;
            state.request += 1;
        }
        if !active {
            state.freeze(Instant::now());
        }
        if active
            && state.ip_page
            && state.source.is_none()
            && state
                .results
                .iter()
                .all(|result| matches!(result, exit_ip::Probe::Idle))
            && let Some(source) = exit_ip::Source::current(&self.backend)
        {
            self.start_overview_exit_detection(source, cx);
        }
    }

    pub(super) fn detect_overview_exit(&mut self, cx: &mut Context<Self>) {
        let state = &mut self.overview_state.exit;
        if state.loading() {
            return;
        }
        let Some(source) = exit_ip::Source::current(&self.backend) else {
            state.results = Default::default();
            state.error = Some("请先连接本机 Mihomo");
            return;
        };
        self.start_overview_exit_detection(source, cx);
    }

    fn start_overview_exit_detection(&mut self, source: exit_ip::Source, cx: &mut Context<Self>) {
        let state = &mut self.overview_state.exit;
        state.source = Some(source.clone());
        state.results = [exit_ip::Probe::Loading, exit_ip::Probe::Loading];
        state.selected = None;
        state.error = None;
        state.freeze(Instant::now());
        state.request += 1;
        let request = state.request;
        let backend = self.backend.clone();
        let (send, mut receive) = tokio::sync::mpsc::unbounded_channel();
        let task = cx.background_executor().spawn(async move {
            exit_ip::detect(&backend, &source, |family, result| {
                let _ = send.send((family, result));
            })
        });
        cx.spawn(async move |app, cx| {
            while let Some((family, result)) = receive.recv().await {
                let _ = app.update(cx, |app, cx| {
                    let state = &mut app.overview_state.exit;
                    if state.request == request
                        && state.source == exit_ip::Source::current(&app.backend)
                    {
                        state.results[family.index()] = result
                            .map_or(exit_ip::Probe::Unavailable, |result| {
                                exit_ip::Probe::Found(Box::new(result))
                            });
                        cx.notify();
                    }
                });
            }
            let result = task.await;
            let _ = app.update(cx, |app, cx| {
                let state = &mut app.overview_state.exit;
                if state.request != request {
                    return;
                }
                if state.source == exit_ip::Source::current(&app.backend)
                    && let Err(error) = result
                {
                    state.results = [exit_ip::Probe::Unavailable, exit_ip::Probe::Unavailable];
                    state.error = Some(error);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn overview_panel(&self, id: &'static str) -> Stateful<Div> {
        let panel = self.card().p(px(14.)).gap(px(10.)).rounded(px(14.));
        self.overview_measure(id, panel)
    }

    fn overview_measure(&self, id: &'static str, panel: Div) -> Stateful<Div> {
        let panel = panel.id(id).relative();
        #[cfg(test)]
        let panel = {
            let measured = self.overview_state.bounds.clone();
            panel.child(
                canvas(
                    move |bounds, _, _| {
                        measured.borrow_mut().insert(id, bounds);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
        };
        panel
    }

    fn overview_period_usage(&self, cx: &Context<Self>) -> Stateful<Div> {
        let p = self.p;
        let loading = self.report.query.as_ref() != Some(&self.query);
        let bins = self.overview_bins();
        let max = bins
            .iter()
            .map(|b| b.bytes.total())
            .max()
            .unwrap_or(0)
            .max(1) as f64;
        let now = chrono::Utc::now().timestamp_millis();
        let from = self.query.from;
        let to = self.query.to.min(now).max(from);
        let end_label = if to == self.query.to {
            to.saturating_sub(1)
        } else {
            to
        };
        let format = if crate::history::local_date(from) == crate::history::local_date(end_label) {
            "%H:%M"
        } else if chrono::Datelike::year(&crate::history::local_date(from))
            == chrono::Datelike::year(&crate::history::local_date(end_label))
        {
            "%m/%d %H:%M"
        } else {
            "%Y/%m/%d %H:%M"
        };
        let time_label = |time| {
            chrono::DateTime::from_timestamp_millis(time)
                .map(|time| {
                    time.with_timezone(&chrono::Local)
                        .format(format)
                        .to_string()
                })
                .unwrap_or_else(|| "—".into())
        };
        let caption = match self.period {
            TimePeriod::Today => "今日用量",
            TimePeriod::Yesterday => "昨日用量",
            TimePeriod::LastSevenDays => "近7天用量",
            TimePeriod::LastThreeMonths => "近3个月用量",
            TimePeriod::Custom => "时段用量",
        };
        self.overview_panel("overview-usage")
            .h(px(216.))
            .gap(px(6.))
            .child(
                row()
                    .justify_between()
                    .gap(px(8.))
                    .child(title(caption, "icons/history-trend.svg", p))
                    .child(
                        label("本机统计", p.muted)
                            .text_size(px(11.))
                            .whitespace_nowrap(),
                    ),
            )
            .child(value(
                if loading {
                    "—".into()
                } else {
                    human_bytes(self.report.total.total())
                },
                39.,
                p.text,
                p,
            ))
            .child(
                row()
                    .gap(px(14.))
                    .child(
                        label(
                            if loading {
                                "↓ —".into()
                            } else {
                                format!("↓ {}", human_bytes(self.report.total.down))
                            },
                            p.download_text,
                        )
                        .text_size(px(12.)),
                    )
                    .child(
                        label(
                            if loading {
                                "↑ —".into()
                            } else {
                                format!("↑ {}", human_bytes(self.report.total.up))
                            },
                            p.upload_text,
                        )
                        .text_size(px(12.)),
                    ),
            )
            .child(
                column()
                    .flex_1()
                    .justify_end()
                    .gap(px(4.))
                    .child(row().items_end().gap(px(3.)).h(px(60.)).children(
                        bins.into_iter().enumerate().map(|(index, bin)| {
                            let bytes = bin.bytes;
                            let down = (bytes.down as f64 / max * 59.) as f32;
                            let up = (bytes.up as f64 / max * 59.) as f32;
                            let measured = self.overview_state.bar_bounds.clone();
                            column()
                                .id(SharedString::from(format!("overview-usage-bar-{index}")))
                                .relative()
                                .cursor_pointer()
                                .flex_1()
                                .min_w_0()
                                .h_full()
                                .justify_end()
                                .gap(px(1.))
                                .when(up > 0., |d| {
                                    d.child(
                                        div()
                                            .h(px(up.max(1.)))
                                            .flex_none()
                                            .rounded(px(2.))
                                            .bg(rgb(p.upload)),
                                    )
                                })
                                .when(down > 0., |d| {
                                    d.child(
                                        div()
                                            .h(px(down.max(1.)))
                                            .flex_none()
                                            .rounded(px(2.))
                                            .bg(rgb(p.download)),
                                    )
                                })
                                .when(bytes.total() == 0, |d| {
                                    d.child(div().h(px(1.)).bg(surface_border(p)))
                                })
                                .child(
                                    canvas(
                                        move |rect, _, _| {
                                            measured.borrow_mut().insert(index, rect);
                                        },
                                        |_, _, _, _| {},
                                    )
                                    .absolute()
                                    .top_0()
                                    .left_0()
                                    .size_full(),
                                )
                                .on_hover(cx.listener(move |app, entered: &bool, _, cx| {
                                    app.hover_overview_usage(index, *entered, cx)
                                }))
                                .on_mouse_move(cx.listener(move |app, _, _, cx| {
                                    app.hover_overview_usage(index, true, cx)
                                }))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |app, _, _, cx| {
                                        app.pin_overview_usage(index, cx);
                                        cx.stop_propagation();
                                    }),
                                )
                                .on_mouse_down(
                                    MouseButton::Right,
                                    cx.listener(move |app, _, window, cx| {
                                        app.open_overview_range(bin, None, window, cx);
                                        cx.stop_propagation();
                                    }),
                                )
                        }),
                    ))
                    .child(
                        row()
                            .justify_between()
                            .gap(px(4.))
                            .child(label(time_label(from), p.muted).text_size(px(11.)))
                            .child(
                                label(
                                    if loading {
                                        "读取中…".into()
                                    } else {
                                        format!("截至 {}", time_label(end_label))
                                    },
                                    p.muted,
                                )
                                .text_size(px(11.)),
                            ),
                    ),
            )
    }

    fn overview_speed(&self, upload: bool) -> Stateful<Div> {
        let p = self.p;
        let color = if upload {
            p.upload_text
        } else {
            p.download_text
        };
        let samples: Vec<_> = self
            .data
            .live
            .iter()
            .rev()
            .take(120)
            .copied()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let connected = self.data.connected;
        self.overview_panel(if upload {
            "overview-upload"
        } else {
            "overview-download"
        })
        .h(px(102.))
        .p(px(12.))
        .gap(px(4.))
        .child(title(
            if upload {
                "当前上传"
            } else {
                "当前下载"
            },
            if upload {
                "icons/arrow-up.svg"
            } else {
                "icons/arrow-down.svg"
            },
            p,
        ))
        .child(value(
            if connected {
                speed(if upload { self.data.up } else { self.data.down })
            } else {
                "—".into()
            },
            26.,
            color,
            p,
        ))
        .child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    if !connected {
                        return;
                    }
                    let valid = |s: &LiveSample| {
                        s.valid && s.from < s.to && (if upload { s.up } else { s.down }).is_finite()
                    };
                    let last = samples.last().map_or(0, |s| s.to);
                    let first = samples.first().map_or(last, |s| s.from);
                    let span = (last - first).max(1) as f32;
                    let max = samples
                        .iter()
                        .filter(|s| valid(s))
                        .map(|s| if upload { s.up } else { s.down })
                        .fold(1., f64::max);
                    let mut points = Vec::new();
                    let draw = |points: &mut Vec<Point<Pixels>>, window: &mut Window| {
                        let mut path = PathBuilder::stroke(px(1.3));
                        for (i, pt) in chart::smooth_points(points).iter().enumerate() {
                            if i == 0 {
                                path.move_to(*pt);
                            } else {
                                path.line_to(*pt);
                            }
                        }
                        if !points.is_empty()
                            && let Ok(path) = path.build()
                        {
                            window.paint_path(path, rgb(color));
                        }
                        points.clear();
                    };
                    for sample in &samples {
                        if !valid(sample) {
                            draw(&mut points, window);
                            continue;
                        }
                        let amount = if upload { sample.up } else { sample.down };
                        points.push(point(
                            bounds.left() + bounds.size.width * ((sample.to - first) as f32 / span),
                            bounds.bottom()
                                - px(2.)
                                - (bounds.size.height - px(4.)) * (amount.max(0.) / max) as f32,
                        ));
                    }
                    draw(&mut points, window);
                },
            )
            .h(px(17.))
            .w_full()
            .flex_none(),
        )
    }

    fn overview_subscription_card(&self, cx: &Context<Self>) -> Stateful<Div> {
        let p = self.p;
        let view = &self.data.subscriptions;
        let saved = self.overview_state.show_saved;
        let (items, stale, message, read_at) = if saved {
            (
                &view.saved,
                view.saved_stale,
                &view.saved_message,
                view.saved_read_at,
            )
        } else {
            (&view.items, view.stale, &view.message, view.read_at)
        };
        let mut card = self
            .overview_panel("overview-subscription")
            .h(px(216.))
            .gap(px(6.))
            .child(
                row()
                    .justify_between()
                    .gap(px(8.))
                    .child(title("订阅用量", "icons/calendar.svg", p))
                    .child(
                        row()
                            .gap(px(2.))
                            .child(
                                self.button(
                                    "overview-subscription-current",
                                    "当前配置",
                                    !saved,
                                    Action::Overview(OverviewAction::SubscriptionSource(false)),
                                    cx,
                                )
                                .h(px(20.))
                                .px(px(4.))
                                .text_size(px(10.5)),
                            )
                            .child(
                                self.button(
                                    "overview-subscription-saved",
                                    "已保存",
                                    saved,
                                    Action::Overview(OverviewAction::SubscriptionSource(true)),
                                    cx,
                                )
                                .h(px(20.))
                                .px(px(4.))
                                .text_size(px(10.5)),
                            ),
                    ),
            );
        let mut list = column()
            .id("overview-subscription-list")
            .flex_1()
            .min_h_0()
            .gap(px(4.))
            .overflow_y_scroll()
            .track_scroll(&self.overview_state.subscription_scroll);
        if items.is_empty() {
            let text = if view.loading {
                "正在读取订阅…"
            } else if !message.is_empty() {
                message
            } else if saved {
                "暂无已保存的远程订阅"
            } else {
                "当前配置未包含远程子订阅"
            };
            list = list.justify_center().child(
                label(text.to_string(), p.muted)
                    .text_size(px(12.))
                    .whitespace_normal(),
            );
        } else {
            for (index, item) in items.iter().enumerate() {
                let (expiry, expired) = overview_expiry(item.expires);
                let remaining = item
                    .remaining()
                    .map(human_bytes)
                    .unwrap_or_else(|| "—".into());
                let total = item.total.map(human_bytes).unwrap_or_else(|| "—".into());
                let fraction = item.fraction().map(|f| 1. - f);
                let color = if expired || fraction.is_some_and(|f| f < 0.1) {
                    p.amber
                } else {
                    p.accent
                };
                let details = format!(
                    "{}{}\n{}\n已用 {} · 剩余 {} · 总量 {}\n{}\n{}{}",
                    item.name,
                    if saved && item.active {
                        " · 客户端当前选择"
                    } else {
                        ""
                    },
                    if saved {
                        "来源：Clash Verge 已保存订阅缓存"
                    } else {
                        "来源：当前内核配置的代理集合"
                    },
                    item.used
                        .map(|b| human_bytes(b.total()))
                        .unwrap_or_else(|| "未提供".into()),
                    remaining,
                    total,
                    expiry,
                    item.updated
                        .and_then(chrono::DateTime::from_timestamp_millis)
                        .map(|v| format!(
                            "订阅更新于 {}",
                            v.with_timezone(&chrono::Local).format("%Y/%m/%d %H:%M")
                        ))
                        .unwrap_or_else(|| "未提供更新时间".into()),
                    if message.is_empty() {
                        String::new()
                    } else {
                        format!("\n{message}")
                    }
                );
                if items.len() == 1 {
                    list = list.justify_center().child(
                        column()
                            .id("overview-single-subscription")
                            .gap(px(7.))
                            .child(
                                label(item.name.clone(), p.text)
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .truncate(),
                            )
                            .child(
                                row()
                                    .justify_between()
                                    .items_end()
                                    .gap(px(8.))
                                    .child(
                                        column()
                                            .gap(px(2.))
                                            .child(label("剩余流量", p.muted).text_size(px(10.5)))
                                            .child(value(remaining.clone(), 28., p.text, p)),
                                    )
                                    .child(
                                        label(format!("总量 {total}"), p.muted)
                                            .text_size(px(11.))
                                            .pb(px(4.))
                                            .flex_none(),
                                    ),
                            )
                            .child(
                                div()
                                    .h(px(5.))
                                    .rounded_full()
                                    .bg(surface_border(p))
                                    .when_some(fraction, |bar, f| {
                                        bar.child(
                                            div()
                                                .h_full()
                                                .w(relative(f))
                                                .rounded_full()
                                                .bg(rgb(color))
                                                .opacity(0.75),
                                        )
                                    }),
                            )
                            .child(
                                row()
                                    .justify_between()
                                    .gap(px(8.))
                                    .child(
                                        label(
                                            format!(
                                                "已用 {}",
                                                item.used
                                                    .map(|b| human_bytes(b.total()))
                                                    .unwrap_or_else(|| "—".into())
                                            ),
                                            p.muted,
                                        )
                                        .text_size(px(10.5))
                                        .flex_none(),
                                    )
                                    .child(label(expiry, p.muted).text_size(px(10.5)).truncate()),
                            )
                            .tooltip(move |window, cx| {
                                Tooltip::new(details.clone()).build(window, cx)
                            }),
                    );
                    continue;
                }
                list = list.child(
                    column()
                        .id(SharedString::from(format!(
                            "overview-subscription-row-{index}"
                        )))
                        .h(px(40.))
                        .flex_none()
                        .gap(px(4.))
                        .child(
                            row()
                                .justify_between()
                                .gap(px(7.))
                                .child(
                                    label(item.name.clone(), p.text)
                                        .text_size(px(12.))
                                        .flex_1()
                                        .min_w_0()
                                        .truncate(),
                                )
                                .child(
                                    label(format!("余 {remaining} / {total}"), p.text)
                                        .font_family("Segoe UI")
                                        .text_size(px(11.))
                                        .flex_none(),
                                ),
                        )
                        .child(
                            row()
                                .gap(px(9.))
                                .child(
                                    div()
                                        .relative()
                                        .h(px(4.))
                                        .flex_1()
                                        .min_w(px(24.))
                                        .rounded_full()
                                        .bg(surface_border(p))
                                        .when_some(fraction, |bar, f| {
                                            bar.child(
                                                div()
                                                    .h_full()
                                                    .w(relative(f))
                                                    .rounded_full()
                                                    .bg(rgb(color))
                                                    .opacity(0.75),
                                            )
                                        }),
                                )
                                .child(
                                    label(expiry, p.muted)
                                        .text_size(px(10.5))
                                        .max_w(px(170.))
                                        .truncate(),
                                ),
                        )
                        .tooltip(move |window, cx| Tooltip::new(details.clone()).build(window, cx)),
                );
            }
        }
        let cache = format!(
            "{}{}{}",
            if stale { "缓存 · " } else { "" },
            items.len(),
            if items.len() == 1 {
                " 个订阅"
            } else {
                " 个 · 剩余 / 总量"
            }
        );
        let source = format!(
            "{}\n{}{}",
            if saved {
                "已保存的独立远程订阅；不代表当前分流的子订阅"
            } else {
                "当前内核配置包含的远程子订阅；用量由订阅服务方提供"
            },
            read_at
                .and_then(chrono::DateTime::from_timestamp_millis)
                .map(|v| format!(
                    "读取于 {}",
                    v.with_timezone(&chrono::Local).format("%H:%M:%S")
                ))
                .unwrap_or_else(|| "尚未成功读取".into()),
            if message.is_empty() {
                String::new()
            } else {
                format!("\n{message}")
            }
        );
        card = card.child(list).child(
            row()
                .justify_between()
                .gap(px(6.))
                .pt(px(4.))
                .border_t_1()
                .border_color(surface_border(p))
                .child(
                    label(cache, if stale { p.amber } else { p.muted })
                        .id("overview-subscription-cache-info")
                        .text_size(px(10.5))
                        .truncate()
                        .tooltip(move |window, cx| Tooltip::new(source.clone()).build(window, cx)),
                )
                .child(self.overview_subscription_refresh(cx)),
        );
        card
    }

    fn overview_subscription_refresh(&self, cx: &Context<Self>) -> Stateful<Div> {
        self.button(
            "overview-subscription-refresh",
            if self.data.subscriptions.loading {
                "读取中"
            } else {
                "刷新"
            },
            false,
            Action::Overview(OverviewAction::RefreshSubscriptions),
            cx,
        )
        .h(px(22.))
        .px(px(4.))
        .text_size(px(11.))
        .flex_none()
        .tooltip(|window, cx| {
            Tooltip::new("重新读取内核订阅信息与客户端缓存；不触发订阅下载").build(window, cx)
        })
    }

    fn overview_connections(&self, cx: &Context<Self>) -> Stateful<Div> {
        let p = self.p;
        let connected = self.data.connected;
        let tcp = self
            .data
            .connections
            .iter()
            .filter(|c| c.metadata.network.eq_ignore_ascii_case("tcp"))
            .count();
        let udp = self
            .data
            .connections
            .iter()
            .filter(|c| c.metadata.network.eq_ignore_ascii_case("udp"))
            .count();
        let applications = crate::app_activity::application_activity(&self.data);
        let transmitting = applications.iter().filter(|a| a.rate.total() > 0).count();
        let other = self.data.connections.len().saturating_sub(tcp + udp);
        let connection_bar = row()
            .id("overview-connection-bar")
            .h(px(5.))
            .flex_none()
            .gap(px(3.))
            .when(connected && !self.data.connections.is_empty(), |bar| {
                bar.children(
                    [(tcp, 1.), (udp, 0.35), (other, 0.15)]
                        .into_iter()
                        .filter(|(count, _)| *count > 0)
                        .map(|(count, opacity)| {
                            let mut segment = div()
                                .h_full()
                                .flex_1()
                                .min_w_0()
                                .rounded_full()
                                .bg(rgb(p.accent))
                                .opacity(opacity);
                            segment.style().flex_grow = Some(count as f32);
                            segment
                        }),
                )
            })
            .when(!connected || self.data.connections.is_empty(), |bar| {
                bar.child(div().size_full().rounded_full().bg(surface_border(p)))
            })
            .tooltip(move |window, cx| {
                Tooltip::new(if connected {
                    format!("连接协议占比 · TCP {tcp} · UDP {udp} · 其他 {other}")
                } else {
                    "实时数据暂停".into()
                })
                .build(window, cx)
            });
        let application_bar = row()
            .id("overview-application-bar")
            .h(px(5.))
            .flex_none()
            .gap(px(4.))
            .children((0..12).map(|index| {
                let filled = (transmitting as f32 / applications.len().max(1) as f32 * 12.
                    - index as f32)
                    .clamp(0., 1.);
                div()
                    .relative()
                    .h_full()
                    .flex_1()
                    .min_w_0()
                    .rounded_full()
                    .bg(surface_border(p))
                    .when(filled > 0., |segment| {
                        segment.child(
                            div()
                                .absolute()
                                .top_0()
                                .left_0()
                                .h_full()
                                .w(relative(filled))
                                .rounded_full()
                                .bg(rgb(p.upload))
                                .opacity(0.75),
                        )
                    })
            }))
            .tooltip({
                let count = applications.len();
                move |window, cx| {
                    Tooltip::new(if connected {
                        format!("正在传输 {transmitting} / 联网应用 {count} · 分段条表示占比")
                    } else {
                        "实时数据暂停".into()
                    })
                    .build(window, cx)
                }
            });
        self.overview_panel("overview-status")
            .h(px(216.))
            .gap(px(12.))
            .child(
                self.overview_measure(
                    "overview-current-connections",
                    column().flex_1().min_h_0().gap(px(4.)).justify_between(),
                )
                .child(
                    row()
                        .justify_between()
                        .gap(px(8.))
                        .child(title("当前连接", "icons/sidebar-connections.svg", p))
                        .child(
                            self.button(
                                "overview-open-connections",
                                "查看 ›",
                                false,
                                Action::Nav(Page::Connections),
                                cx,
                            )
                            .h(px(20.))
                            .px(px(2.))
                            .text_size(px(11.)),
                        ),
                )
                .child(
                    row()
                        .justify_between()
                        .gap(px(8.))
                        .child(value(
                            if connected {
                                format!("{} 条", self.data.connections.len())
                            } else {
                                "—".into()
                            },
                            29.,
                            p.text,
                            p,
                        ))
                        .child(
                            label(
                                if connected {
                                    format!("TCP {tcp} · UDP {udp}")
                                } else {
                                    "等待重连".into()
                                },
                                p.muted,
                            )
                            .text_size(px(11.))
                            .whitespace_nowrap(),
                        ),
                )
                .child(connection_bar),
            )
            .child(div().h(px(1.)).flex_none().bg(surface_border(p)))
            .child(
                self.overview_measure(
                    "overview-current-applications",
                    column().flex_1().min_h_0().gap(px(4.)).justify_between(),
                )
                .child(
                    row()
                        .justify_between()
                        .gap(px(8.))
                        .child(title("联网应用", "icons/sidebar-apps.svg", p))
                        .child(
                            self.button(
                                "overview-open-activity",
                                "查看 ›",
                                false,
                                Action::Overview(OverviewAction::Activity),
                                cx,
                            )
                            .h(px(20.))
                            .px(px(2.))
                            .text_size(px(11.)),
                        ),
                )
                .child(
                    row()
                        .justify_between()
                        .gap(px(8.))
                        .child(value(
                            if connected {
                                format!("{} 个", applications.len())
                            } else {
                                "—".into()
                            },
                            29.,
                            p.text,
                            p,
                        ))
                        .child(
                            row()
                                .gap(px(6.))
                                .when(connected && transmitting > 0, |status| {
                                    status
                                        .child(div().size(px(5.)).rounded_full().bg(rgb(p.upload)))
                                })
                                .child(
                                    label(
                                        if connected {
                                            format!("{transmitting} 个正在传输")
                                        } else {
                                            "实时数据暂停".into()
                                        },
                                        p.muted,
                                    )
                                    .text_size(px(11.))
                                    .whitespace_nowrap(),
                                ),
                        ),
                )
                .child(application_bar),
            )
    }

    fn overview_ranking(&self, apps: bool, cx: &Context<Self>) -> Stateful<Div> {
        let p = self.p;
        let rows = if apps {
            &self.report.apps
        } else {
            &self.report.domains
        };
        let loading = self.report.query.as_ref() != Some(&self.query);
        let max = rows.first().map_or(1, |r| r.bytes.total().max(1)) as f64;
        let dimension = if apps { "app" } else { "domain" };
        self.overview_panel(if apps {
            "overview-apps"
        } else {
            "overview-domains"
        })
        .min_h(px(222.))
        .flex_1()
        .child(
            row()
                .gap(px(8.))
                .justify_between()
                .child(title(
                    if apps { "应用用量" } else { "目标域名" },
                    if apps {
                        "icons/sidebar-apps.svg"
                    } else {
                        "icons/locate.svg"
                    },
                    p,
                ))
                .child(
                    self.button(
                        if apps {
                            "overview-all-apps"
                        } else {
                            "overview-all-domains"
                        },
                        "全部 ›",
                        false,
                        Action::Overview(OverviewAction::Records(dimension)),
                        cx,
                    )
                    .h(px(24.))
                    .px(px(2.))
                    .text_size(px(11.)),
                ),
        )
        .child(
            column().gap(px(11.)).children(
                rows.iter()
                    .take(if loading { 0 } else { 4 })
                    .enumerate()
                    .map(|(index, item)| {
                        let name = if apps {
                            self.application_name(&item.key).to_string()
                        } else {
                            item.key.clone()
                        };
                        let key = item.key.clone();
                        column()
                            .id(SharedString::from(format!(
                                "overview-rank-{dimension}-{index}"
                            )))
                            .gap(px(6.))
                            .child(
                                row()
                                    .gap(px(6.))
                                    .min_w_0()
                                    .when(apps, |d| {
                                        d.child(self.application_icon(&key, true).flex_none())
                                    })
                                    .child(
                                        label(name, p.text)
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .text_size(px(12.)),
                                    )
                                    .child(
                                        label(human_bytes(item.bytes.total()), p.muted)
                                            .text_size(px(12.))
                                            .font_family("Segoe UI")
                                            .w(px(88.))
                                            .flex_none()
                                            .truncate()
                                            .text_right(),
                                    ),
                            )
                            .child(
                                div().h(px(3.)).rounded_full().bg(surface_border(p)).child(
                                    div()
                                        .h_full()
                                        .rounded_full()
                                        .w(relative(
                                            (item.bytes.total() as f64 / max).clamp(0., 1.) as f32,
                                        ))
                                        .bg(rgb(p.accent)),
                                ),
                            )
                            .cursor_pointer()
                            .hover(move |d| d.opacity(0.8))
                            .tooltip({
                                let key = key.clone();
                                move |window, cx| Tooltip::new(key.clone()).build(window, cx)
                            })
                            .on_click(cx.listener(move |app, _, window, cx| {
                                if apps {
                                    app.act(
                                        Action::Application(ApplicationAction::Select(key.clone())),
                                        window,
                                        cx,
                                    );
                                } else {
                                    app.act(
                                        Action::Overview(OverviewAction::Records("domain")),
                                        window,
                                        cx,
                                    );
                                }
                            }))
                    }),
            ),
        )
        .when(loading || rows.is_empty(), |d| {
            d.child(
                column().flex_1().justify_center().items_center().child(
                    label(
                        if loading {
                            "读取中…"
                        } else {
                            "暂无用量"
                        },
                        p.muted,
                    )
                    .text_size(px(12.)),
                ),
            )
        })
    }

    fn overview_routes(&self, cx: &Context<Self>) -> Stateful<Div> {
        let p = self.p;
        let state = &self.overview_state.exit;
        let ip = state.ip_page;
        let paused = self.overview_exit_paused();
        let now = Instant::now();
        let progress = state.elapsed_at(now).as_secs_f32() / EXIT_ROTATION.as_secs_f32();
        let mut viewport = column()
            .gap_0()
            .relative()
            .flex_1()
            .min_w_0()
            .overflow_hidden();
        let incoming = self.overview_egress_pane(ip, true, cx).w_full().flex_1();
        if let Some(started) = state.animation {
            let t = (now.saturating_duration_since(started).as_secs_f32()
                / EXIT_TRANSITION.as_secs_f32())
            .clamp(0., 1.);
            // Symmetric easing comes to rest at both ends and keeps reversals continuous.
            let eased = t * t * t * (t * (6. * t - 15.) + 10.);
            let direction = if ip { 1. } else { -1. };
            viewport = viewport
                .child(
                    self.overview_egress_pane(!ip, false, cx)
                        .absolute()
                        .top_0()
                        .left(relative(-direction * eased))
                        .h_full()
                        .w_full(),
                )
                .child(incoming.relative().left(relative(direction * (1. - eased))));
        } else {
            viewport = viewport.child(incoming);
        }
        self.overview_panel("overview-routes")
            .min_h(px(222.))
            .flex_1()
            .min_w_0()
            .gap(px(3.))
            .on_hover(cx.listener(|app, hovered: &bool, _, cx| {
                let state = &mut app.overview_state.exit;
                state.hovered = *hovered;
                state.freeze(Instant::now());
                cx.notify();
            }))
            .child(viewport)
            .child(
                row()
                    .h(px(24.))
                    .children(
                        [(false, "出站分布"), (true, "出口 IP")].map(|(target, name)| {
                            self.button(
                                name,
                                "",
                                false,
                                Action::Overview(OverviewAction::ExitPage(target)),
                                cx,
                            )
                            .w(px(17.))
                            .h(px(24.))
                            .p_0()
                            .child(
                                div()
                                    .w(px(if ip == target { 10. } else { 5. }))
                                    .h(px(5.))
                                    .rounded(px(3.))
                                    .bg(rgb(if ip == target { p.accent } else { p.muted })),
                            )
                            .tooltip(move |w, cx| Tooltip::new(name).build(w, cx))
                        }),
                    )
                    .child(
                        self.button(
                            "overview-exit-pause",
                            "",
                            false,
                            Action::Overview(OverviewAction::ExitPause),
                            cx,
                        )
                        .size(px(24.))
                        .p_0()
                        .child(
                            svg()
                                .path(if paused {
                                    "icons/carousel-play.svg"
                                } else {
                                    "icons/carousel-pause.svg"
                                })
                                .size(px(13.))
                                .text_color(rgb(p.muted)),
                        )
                        .tooltip(move |w, cx| {
                            Tooltip::new(if paused {
                                "恢复轮播"
                            } else {
                                "暂停轮播"
                            })
                            .build(w, cx)
                        }),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .h(px(4.))
                            .w(px(108.))
                            .flex_none()
                            .rounded_full()
                            .overflow_hidden()
                            .bg(state_surface(p))
                            .child(
                                div()
                                    .h_full()
                                    .w(relative(progress.clamp(0., 1.)))
                                    .rounded_full()
                                    .bg(rgb(p.accent)),
                            ),
                    ),
            )
    }

    fn overview_egress_pane(&self, ip: bool, interactive: bool, cx: &Context<Self>) -> Div {
        let p = self.p;
        let state = &self.overview_state.exit;
        let (name, icon) = if ip {
            ("出口 IP", "icons/globe.svg")
        } else {
            ("出站分布", "icons/outbound-distribution.svg")
        };
        let heading = title(name, icon, p).h(px(24.));
        let heading = if interactive {
            heading
                .id(if ip {
                    "overview-title-ip"
                } else {
                    "overview-title-routes"
                })
                .rounded(px(6.))
                .cursor_pointer()
                .hover(move |style| style.bg(state_surface(p)))
                .on_click(cx.listener(move |app, _, window, cx| {
                    app.act(Action::Overview(OverviewAction::ExitPage(!ip)), window, cx)
                }))
                .tooltip(move |w, cx| {
                    Tooltip::new(if ip {
                        "切换到出站分布"
                    } else {
                        "切换到出口 IP"
                    })
                    .build(w, cx)
                })
                .into_any_element()
        } else {
            heading.into_any_element()
        };
        let (caption, action) = if !ip {
            ("详情 ›", Action::Overview(OverviewAction::Records("route")))
        } else if state.loading() {
            ("检测中…", Action::Overview(OverviewAction::ExitDetect))
        } else if state.error.is_some()
            || state
                .results
                .iter()
                .any(|r| !matches!(r, exit_ip::Probe::Idle))
        {
            ("重新检测", Action::Overview(OverviewAction::ExitDetect))
        } else {
            ("检测", Action::Overview(OverviewAction::ExitDetect))
        };
        let action = if interactive && (!ip || !state.loading()) {
            self.button(
                if ip {
                    "overview-exit-detect"
                } else {
                    "overview-all-routes"
                },
                caption,
                ip,
                action,
                cx,
            )
            .h(px(23.))
            .px(px(5.))
            .text_size(px(11.))
            .tooltip(move |w, cx| {
                Tooltip::new(if ip {
                    "通过 Mihomo 请求 IP.SB，使用 IPQuery 和 Proxycheck 查询资料"
                } else {
                    "查看出站流量详情"
                })
                .build(w, cx)
            })
            .into_any_element()
        } else {
            label(caption, p.muted)
                .px(px(5.))
                .text_size(px(11.))
                .into_any_element()
        };
        column()
            .gap(px(6.))
            .min_w_0()
            .child(
                row()
                    .h(px(24.))
                    .justify_between()
                    .child(heading)
                    .child(action),
            )
            .child(if ip {
                self.overview_exit_body(interactive, cx)
            } else {
                self.overview_routes_body()
            })
    }

    fn overview_exit_body(&self, interactive: bool, cx: &Context<Self>) -> Div {
        use exit_ip::{Family, Probe, Quality};
        let p = self.p;
        let state = &self.overview_state.exit;
        let selected = state.selected_family();
        let selected_result = state.results[selected.index()].detection();
        let badge = |id, text: String, tip: Option<String>, color| {
            let badge = label(text, color)
                .text_size(px(10.))
                .w(px(if id == "ip-quality" { 44. } else { 48. }))
                .text_center()
                .py(px(1.))
                .rounded(px(4.))
                .bg(state_surface(p))
                .whitespace_nowrap()
                .flex_none();
            if interactive && let Some(tip) = tip {
                badge
                    .id(id)
                    .tooltip(move |w, cx| Tooltip::new(tip.clone()).build(w, cx))
                    .into_any_element()
            } else {
                badge.into_any_element()
            }
        };
        let rows = Family::ALL.map(|family| {
            let probe = &state.results[family.index()];
            let result = probe.detection();
            let address = match probe {
                Probe::Idle => "待检测".into(),
                Probe::Loading => "检测中…".into(),
                Probe::Unavailable => "未测到".into(),
                Probe::Found(result) => result.record.ip.to_string(),
            };
            let address_color = if result.is_some() { p.text } else { p.muted };
            let address_view = row().gap_0().flex_wrap().min_w_0().flex_1().children(
                address.split_inclusive(':').map(|part| {
                    label(part.to_owned(), address_color)
                        .text_size(px(if family == Family::V4 { 13. } else { 11. }))
                        .font_weight(FontWeight::MEDIUM)
                        .line_height(px(14.))
                        .whitespace_nowrap()
                        .flex_none()
                }),
            );
            let line = row()
                .min_h(px(25.))
                .gap(px(4.))
                .min_w_0()
                .pr(px(4.))
                .child(
                    div()
                        .w(px(2.))
                        .h(px(12.))
                        .flex_none()
                        .rounded_full()
                        .when(family == selected && result.is_some(), |marker| {
                            marker.bg(rgb(p.accent))
                        }),
                )
                .child(
                    label(family.label(), p.muted)
                        .text_size(px(10.))
                        .w(px(24.))
                        .flex_none(),
                )
                .child(address_view)
                .when_some(result, |line, result| {
                    let (quality, source) = result.quality();
                    let (text, color) = match quality {
                        Quality::Good => ("优", p.upload_text),
                        Quality::Normal => ("普通", p.muted),
                        Quality::Risky => ("风险", p.amber),
                        Quality::Unknown => ("质量未知", p.muted),
                    };
                    let mut tip = source.map_or_else(
                        || "质量未知 · 暂无分档依据".into(),
                        |source| format!("质量 {text} · 依据 {source}"),
                    );
                    if let Some((score, source)) = result.score {
                        tip.push_str(&format!("\n原始风险分 {score}/100 · {source}"));
                    }
                    line.child(badge(
                        "network-type",
                        result.kind.map_or("类型未知", |(kind, _)| kind).into(),
                        result.kind.map(|(_, source)| format!("来源 {source}")),
                        p.accent_text,
                    ))
                    .child(badge("ip-quality", text.into(), Some(tip), color))
                });
            if interactive && result.is_some() {
                line.id(family.label())
                    .cursor_pointer()
                    .on_click(cx.listener(move |app, _, w, cx| {
                        app.act(Action::Overview(OverviewAction::ExitFamily(family)), w, cx)
                    }))
                    .tooltip(move |w, cx| Tooltip::new(address.clone()).build(w, cx))
                    .into_any_element()
            } else {
                line.into_any_element()
            }
        });
        let mut body = column().gap(px(5.)).min_w_0().flex_1().children(rows);
        if let Some(result) = selected_result {
            let record = &result.record;
            let joined = |a: &str, b: &str| match (a.trim(), b.trim()) {
                ("", "") => "未知".into(),
                (a, "") => a.into(),
                ("", b) => b.into(),
                (a, b) => format!("{a} · {b}"),
            };
            let org = if record.isp.org.trim().is_empty() {
                &record.isp.isp
            } else {
                &record.isp.org
            };
            let mode = match result.mode.to_ascii_lowercase().as_str() {
                "rule" => "规则",
                "global" => "全局",
                "direct" => "直连",
                _ => "Mihomo",
            };
            let country_code = &record.location.country_code;
            let location = joined(&record.location.country, &record.location.city);
            let network = joined(org, &record.isp.asn);
            let field = |id, value: String| {
                let text = label(value.clone(), p.text)
                    .text_size(px(11.))
                    .flex_1()
                    .min_w_0()
                    .truncate();
                if interactive {
                    text.id(id)
                        .tooltip(move |w, cx| Tooltip::new(value.clone()).build(w, cx))
                        .into_any_element()
                } else {
                    text.into_any_element()
                }
            };
            let stamp = label(
                format!("{mode} · {}", result.time.format("%m-%d %H:%M")),
                p.muted,
            )
            .text_size(px(10.))
            .whitespace_nowrap();
            let stamp = if interactive {
                stamp
                    .id("overview-ip-stamp")
                    .tooltip(|w, cx| {
                        Tooltip::new("出口由 IP.SB 检测，Mihomo 规则可能为不同目标选择不同线路")
                            .build(w, cx)
                    })
                    .into_any_element()
            } else {
                stamp.into_any_element()
            };
            let detail = column()
                .gap(px(3.))
                .px(px(8.))
                .py(px(5.))
                .rounded(px(8.))
                .bg(state_surface(p))
                .min_w_0()
                .child(
                    row()
                        .gap(px(5.))
                        .justify_between()
                        .child(
                            label(format!("{} 详情", selected.label()), p.text)
                                .text_size(px(10.))
                                .font_weight(FontWeight::MEDIUM)
                                .whitespace_nowrap(),
                        )
                        .child(stamp),
                )
                .child(
                    row()
                        .gap(px(5.))
                        .child(
                            label("地区", p.muted)
                                .text_size(px(11.))
                                .w(px(47.))
                                .flex_none(),
                        )
                        .when(
                            super::assets::country_flag_data(country_code).is_some(),
                            |d| {
                                d.child(
                                    img(ImageSource::Resource(Resource::Embedded(
                                        format!("flags/{}.png", country_code.to_ascii_lowercase())
                                            .into(),
                                    )))
                                    .w(px(16.))
                                    .h(px(12.))
                                    .rounded(px(2.))
                                    .flex_none(),
                                )
                            },
                        )
                        .child(field("overview-ip-location", location)),
                )
                .child(
                    row()
                        .gap(px(5.))
                        .child(
                            label("网络归属", p.muted)
                                .text_size(px(11.))
                                .w(px(47.))
                                .flex_none(),
                        )
                        .child(field("overview-ip-network", network)),
                );
            body = body.child(detail);
        } else if let Some(error) = state.error {
            body = body.child(label(error, p.amber).text_size(px(11.)));
        }
        body
    }

    fn overview_routes_body(&self) -> Div {
        let p = self.p;
        let loading = self.report.query.as_ref() != Some(&self.query);
        let total = self.report.observed.total().max(1) as f64;
        let routes = &self.report.routes;
        let mut colors = [rgb(p.accent); 3];
        colors[1] = rgb(p.muted);
        colors[1].a = 0.35;
        colors[2].a = 0.45;
        column()
            .gap(px(6.))
            .when(!loading && !routes.is_empty(), |d| {
                d.child(
                    row()
                        .justify_between()
                        .gap(px(6.))
                        .child(label("已观测流量", p.muted).text_size(px(12.)))
                        .child(
                            label(human_bytes(self.report.observed.total()), p.text)
                                .text_size(px(12.)),
                        ),
                )
                .child(
                    row()
                        .gap(px(3.))
                        .h(px(9.))
                        .children(routes.iter().enumerate().map(|(index, route)| {
                            div()
                                .h_full()
                                .w(relative(
                                    (route.bytes.total() as f64 / total).clamp(0., 1.) as f32
                                ))
                                .rounded(px(3.))
                                .bg(colors[index.min(2)])
                        })),
                )
                .child(column().gap(px(9.)).pt(px(4.)).children(
                    routes.iter().take(3).enumerate().map(|(index, route)| {
                        let full = route.key.clone();
                        row()
                            .id(SharedString::from(format!("overview-route-{index}")))
                            .gap(px(6.))
                            .child(
                                div()
                                    .size(px(5.))
                                    .rounded(px(2.))
                                    .flex_none()
                                    .bg(colors[index]),
                            )
                            .child(
                                label(full.clone(), p.text)
                                    .text_size(px(12.))
                                    .flex_1()
                                    .min_w_0()
                                    .truncate(),
                            )
                            .child(
                                label(
                                    format!("{:.1}%", route.bytes.total() as f64 / total * 100.),
                                    p.muted,
                                )
                                .text_size(px(12.))
                                .flex_none(),
                            )
                            .tooltip(move |window, cx| Tooltip::new(full.clone()).build(window, cx))
                    }),
                ))
            })
            .when(loading || routes.is_empty(), |d| {
                d.child(
                    column().flex_1().justify_center().items_center().child(
                        label(
                            if loading {
                                "读取中…"
                            } else {
                                "等待观测流量"
                            },
                            p.muted,
                        )
                        .text_size(px(12.)),
                    ),
                )
            })
    }

    fn overview_footer(&self) -> Div {
        let p = self.p;
        let delta = self.report.total.total() as i128 - self.report.observed.total() as i128;
        let text = format!(
            "{} {}",
            if delta < 0 { "待对账" } else { "未归属" },
            human_bytes(delta.unsigned_abs().min(u64::MAX as u128) as u64)
        );
        let help = format!(
            "本机用量来自实际采集；订阅用量来自服务方，由客户端缓存提供，两者分别统计。\n未知进程 {}\n恢复缺口 {} · {} 段\n跨边界缺口 {} 段",
            human_bytes(self.report.unknown.total()),
            human_bytes(self.report.recovered.total()),
            self.report.gaps,
            self.report.crossing_gaps
        );
        row()
            .gap(px(9.))
            .flex_wrap()
            .px(px(2.))
            .child(
                div()
                    .size(px(5.))
                    .flex_none()
                    .rounded_full()
                    .bg(rgb(if self.data.connected {
                        p.upload_text
                    } else {
                        p.amber
                    })),
            )
            .child(
                label(
                    if self.data.connected {
                        "持续记录中"
                    } else {
                        "等待重连"
                    },
                    p.muted,
                )
                .text_size(px(11.)),
            )
            .child(label(text, p.muted).text_size(px(11.)))
            .child(
                Popover::new("overview-quality-popover")
                    .no_style()
                    .trigger(InfoHelpTrigger {
                        id: "overview-quality",
                        palette: p,
                        selected: false,
                    })
                    .content(move |window, cx| {
                        let help = help.clone();
                        cx.new(|cx| {
                            PopoverContent::new(window, cx, move |_, cx| {
                                column()
                                    .w(px(340.))
                                    .p(px(14.))
                                    .gap(px(8.))
                                    .rounded(px(10.))
                                    .bg(popup_surface(p))
                                    .border_1()
                                    .border_color(card_border(p))
                                    .shadow_sm()
                                    .child(heading("统计口径", p))
                                    .child(
                                        label(help.clone(), p.muted)
                                            .text_size(px(12.))
                                            .whitespace_normal(),
                                    )
                                    .on_mouse_down_out(
                                        cx.listener(|_, _, _, cx| cx.emit(DismissEvent)),
                                    )
                                    .into_any_element()
                            })
                            .p_0()
                        })
                    }),
            )
            .child(
                label(
                    if self.data.client_name.is_empty() {
                        "Mihomo".into()
                    } else {
                        format!("Mihomo · {}", self.data.client_name)
                    },
                    p.muted,
                )
                .text_size(px(11.))
                .flex_1()
                .min_w(px(140.))
                .text_right(),
            )
    }

    pub(super) fn overview(&self, window: &Window, cx: &Context<Self>) -> Div {
        let available = f32::from(window.viewport_size().width - self.sidebar_width() - px(48.));
        let wide = available >= 820.;
        let spacing = 12.;
        let speed_width = if wide {
            (available - spacing * 2.) * 0.25
        } else {
            190.
        };
        let usage_width = (available - spacing * 2.) * 5. / 12.;
        let speeds = column()
            .gap(px(spacing))
            .w(px(speed_width))
            .flex_none()
            .child(self.overview_speed(false))
            .child(self.overview_speed(true));
        let legend = |text, color| {
            row()
                .gap(px(5.))
                .flex_none()
                .child(div().w(px(10.)).h(px(4.)).rounded(px(2.)).bg(rgb(color)))
                .child(label(text, self.p.muted).text_size(px(11.)))
        };
        let chart = self
            .overview_panel("overview-chart")
            .h(px(216.))
            .flex_1()
            .child(
                row()
                    .gap(px(8.))
                    .justify_between()
                    .child(title("实时网络", "icons/history-trend.svg", self.p))
                    .child(
                        row()
                            .gap(px(8.))
                            .child(label("近5分钟", self.p.muted).text_size(px(11.)))
                            .child(
                                row()
                                    .gap(px(12.))
                                    .child(legend("下载", self.p.download))
                                    .child(legend("上传", self.p.upload)),
                            ),
                    ),
            )
            .child(self.live_chart(300_000, 116., false).flex_none());
        let top = if wide {
            stretch()
                .gap(px(spacing))
                .child(
                    self.overview_period_usage(cx)
                        .w(px(usage_width))
                        .flex_none(),
                )
                .child(speeds)
                .child(self.overview_subscription_card(cx).flex_1())
        } else {
            stretch()
                .gap(px(spacing))
                .child(self.overview_period_usage(cx).flex_1())
                .child(speeds)
        };
        let mut page = column().gap(px(spacing)).min_w_0().child(top);
        if wide {
            let width = (available - spacing * 2.) / 3.;
            page = page.child(
                stretch()
                    .gap(px(spacing))
                    .child(chart)
                    .child(self.overview_connections(cx).w(px(width)).flex_none()),
            );
        } else {
            page = page
                .child(
                    stretch()
                        .gap(px(spacing))
                        .child(self.overview_subscription_card(cx).flex_1())
                        .child(self.overview_connections(cx).flex_1()),
                )
                .child(chart);
        }
        page.child(
            stretch()
                .gap(px(spacing))
                .child(self.overview_ranking(true, cx))
                .child(self.overview_ranking(false, cx))
                .child(self.overview_routes(cx)),
        )
        .child(self.overview_footer())
    }
}

fn overview_expiry(expiry: Option<i64>) -> (String, bool) {
    let Some(expiry) = expiry.and_then(chrono::DateTime::from_timestamp_millis) else {
        return ("到期时间未提供".into(), false);
    };
    let now = chrono::Local::now();
    let expiry = expiry.with_timezone(&chrono::Local);
    let days = expiry
        .date_naive()
        .signed_duration_since(now.date_naive())
        .num_days();
    if expiry < now {
        (format!("{} · 已到期", expiry.format("%Y/%m/%d")), true)
    } else if days == 0 {
        ("今天到期".into(), true)
    } else {
        (
            format!(
                "{}到期 · {days}天",
                expiry.format(
                    if expiry.format("%Y").to_string() == now.format("%Y").to_string() {
                        "%m/%d"
                    } else {
                        "%Y/%m/%d"
                    }
                )
            ),
            days <= 7,
        )
    }
}

#[cfg(test)]
mod exit_card_tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn rotation_progress_freezes_and_resumes_from_same_offset() {
        let start = Instant::now();
        let mut card = ExitCard::default();
        assert!(!card.rotation_due(start, true));
        let pause = start + Duration::from_secs(3);
        assert!(!card.rotation_due(pause, false));
        let resume = start + Duration::from_secs(20);
        assert_eq!(card.elapsed_at(resume), Duration::from_secs(3));
        assert!(!card.rotation_due(resume, true));
        assert!(!card.rotation_due(resume + Duration::from_millis(4999), true));
        assert!(card.rotation_due(resume + Duration::from_secs(5), true));
    }
}
