use super::*;
use crate::app_activity::{ActivityState, share_label};

#[derive(Default)]
pub(super) struct UsageDetailState {
    pub key: Option<String>,
    pub scroll: ScrollHandle,
    pub scrollbar: ScrollbarState,
    pub processes_expanded: bool,
    pub bounds: Rc<Cell<Bounds<Pixels>>>,
}

#[derive(Clone)]
pub(super) struct DetailContext {
    pub key: String,
    pub group_apps: bool,
}

fn application_description(context: &DetailContext) -> Option<&'static str> {
    match context.key.as_str() {
        "service:dosvc" => Some(
            "Windows 传递优化，负责下载系统和 Microsoft Store 应用的更新包。这里汇总下载服务本身的流量。",
        ),
        "service:bits" => Some("后台智能传输服务，负责后台上传和下载文件，网络恢复后可继续传输。"),
        "service:wuauserv" => Some("Windows Update 服务，负责检查、下载和安装 Windows 更新。"),
        "service:diagtrack" => Some(
            "Windows 诊断与遥测服务，负责收集和上传设备状态、性能、错误及使用情况。收集范围取决于系统诊断数据设置。",
        ),
        key if app_name(key).eq_ignore_ascii_case("svchost.exe") => Some(if context.group_apps {
            "Windows 服务宿主。这里汇总尚未细分到具体服务的流量。"
        } else {
            "Windows 服务宿主。当前按原始进程统计，可能包含多个后台服务的流量。"
        }),
        key if app_name(key).eq_ignore_ascii_case("sdxhelper.exe") => Some(
            "Microsoft Office 后台更新助手，负责检查和下载 Office 联网功能的更新。可由计划任务启动。",
        ),
        _ => None,
    }
}

fn range_caption(query: &Query) -> String {
    let format = |time| {
        chrono::DateTime::from_timestamp_millis(time)
            .map(|date| {
                date.with_timezone(&chrono::Local)
                    .format("%Y/%m/%d")
                    .to_string()
            })
            .unwrap_or_default()
    };
    let start = format(query.from);
    let end = format(query.to.saturating_sub(1));
    if start == end {
        start
    } else {
        format!("{start} — {end}")
    }
}

#[derive(Clone)]
pub(super) enum ApplicationAction {
    CopyApplicationName(String),
    CopyApplicationPath(String),
    OpenApplicationLocation(String),
    RefreshApplicationDetail,
    ToggleApplicationProcesses,
    ToggleApplicationPath,
    DetailDimension(&'static str),
    Select(String),
    OpenUsageDetail(String),
    CloseUsageDetail,
    ClearAppSelection,
    AppSort(TrafficSort),
}

impl TrafficApp {
    pub(super) fn handle_application_action(
        &mut self,
        action: ApplicationAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            ApplicationAction::CopyApplicationName(key) => {
                cx.write_to_clipboard(ClipboardItem::new_string(
                    self.application_name(&key).to_string(),
                ));
                self.activity.copy_feedback = Some((key, false, Instant::now()));
            }
            ApplicationAction::CopyApplicationPath(key) => {
                let path = self.application_icon_path(&key);
                if std::path::Path::new(path).is_absolute() {
                    cx.write_to_clipboard(ClipboardItem::new_string(path.to_string()));
                    self.activity.copy_feedback = Some((key, true, Instant::now()));
                }
            }
            ApplicationAction::OpenApplicationLocation(key) => {
                let path = std::path::PathBuf::from(self.application_icon_path(&key));
                if path.is_absolute()
                    && let Some(parent) = path.parent()
                {
                    self.open_directory(parent.to_path_buf());
                }
            }
            ApplicationAction::RefreshApplicationDetail => {
                self.pending_detail_query = None;
                self.refresh_detail_query();
            }
            ApplicationAction::ToggleApplicationProcesses => {
                if self.usage_detail_open() {
                    self.usage_detail.processes_expanded = !self.usage_detail.processes_expanded;
                } else {
                    self.activity.processes_expanded = !self.activity.processes_expanded;
                }
            }
            ApplicationAction::ToggleApplicationPath => {
                if let Some(context) = self.active_detail_context() {
                    self.expanded_application_path = if self.expanded_application_path.as_deref()
                        == Some(context.key.as_str())
                    {
                        None
                    } else {
                        Some(context.key)
                    };
                }
            }
            ApplicationAction::DetailDimension(d) => {
                self.detail_dimension = d;
            }
            ApplicationAction::Select(app) => {
                self.expanded_application_path = None;
                self.close_usage_detail();
                self.activity.selected_process = self.page != Page::Apps;
                self.activity.processes_expanded = false;
                self.app_usage_anchor_pending = false;
                self.activity.anchor_table = false;
                self.analysis.anchor = None;
                if self.page == Page::Apps && self.selected.is_empty() {
                    self.app_list_offset = self.page_scroll.offset();
                } else if self.page != Page::Apps {
                    self.app_list_offset = point(px(0.), px(0.));
                    self.activity.details = false;
                    self.analysis.active = false;
                }
                self.page_scroll.set_offset(point(px(0.), px(0.)));
                self.selected = app;
                if self.page != Page::Apps {
                    self.query.page = 0;
                    self.query.search.clear();
                    self.search.update(cx, |s, cx| s.set_value("", window, cx));
                }
                self.page = Page::Apps;
                self.query.group_apps = true;
                self.query.page_size = 10;
                self.search.update(cx, |input, cx| {
                    input.set_placeholder("搜索应用名称或完整路径…", window, cx)
                });
                self.query.dimension = "app".into();
                self.refresh();
            }
            ApplicationAction::OpenUsageDetail(key) => self.open_usage_detail(key),
            ApplicationAction::CloseUsageDetail => self.close_usage_detail(),
            ApplicationAction::ClearAppSelection => {
                self.activity.selected_process = false;
                self.selected.clear();
                self.pending_detail_query = None;
                self.unsent_detail_query = None;
                self.detail = Report::default();
                self.page_scroll.set_offset(self.app_list_offset);
                self.refresh();
            }
            ApplicationAction::AppSort(sort) => {
                self.app_usage_anchor_pending = false;
                self.query.sort = sort;
                self.query.page = 0;
                self.refresh();
            }
        }
    }

    fn application_detail_actions(
        &self,
        context: &DetailContext,
        pane: bool,
        cx: &Context<Self>,
    ) -> Div {
        let has_path = std::path::Path::new(self.application_icon_path(&context.key)).is_absolute();
        row()
            .gap(px(2.))
            .flex_none()
            .child(
                self.button(
                    "app-copy-name",
                    if self.application_copied(&context.key, false) {
                        "已复制"
                    } else {
                        "复制名称"
                    },
                    false,
                    Action::Application(ApplicationAction::CopyApplicationName(
                        context.key.clone(),
                    )),
                    cx,
                )
                .h(px(28.))
                .px(px(if pane { 5. } else { 8. }))
                .w(px(if pane { 62. } else { 74. })),
            )
            .child(
                self.button(
                    "app-copy-path",
                    if self.application_copied(&context.key, true) {
                        "已复制"
                    } else {
                        "复制地址"
                    },
                    false,
                    Action::Application(ApplicationAction::CopyApplicationPath(
                        context.key.clone(),
                    )),
                    cx,
                )
                .h(px(28.))
                .px(px(if pane { 5. } else { 8. }))
                .w(px(if pane { 62. } else { 74. }))
                .when(!has_path, |button| button.opacity(0.4).cursor_default()),
            )
            .child(
                self.button(
                    "app-open-location",
                    "打开位置",
                    false,
                    Action::Application(ApplicationAction::OpenApplicationLocation(
                        context.key.clone(),
                    )),
                    cx,
                )
                .h(px(28.))
                .px(px(if pane { 5. } else { 8. }))
                .when(pane, |button| button.w(px(62.)))
                .when(!has_path, |button| button.opacity(0.4).cursor_default()),
            )
            .when(pane, |row| {
                row.child(
                    self.button(
                        "usage-detail-close",
                        "",
                        false,
                        Action::Application(ApplicationAction::CloseUsageDetail),
                        cx,
                    )
                    .w(px(24.))
                    .h(px(28.))
                    .px(px(0.))
                    .child(
                        svg()
                            .path("icons/window-close.svg")
                            .size(px(13.))
                            .text_color(rgb(self.p.muted)),
                    )
                    .tooltip(|window, cx| Tooltip::new("关闭详情（Esc）").build(window, cx)),
                )
            })
    }
    fn application_detail_path(&self, context: &DetailContext, cx: &Context<Self>) -> Div {
        let path = self.application_icon_path(&context.key).to_string();
        let has_path = std::path::Path::new(&path).is_absolute();
        let expanded = self.expanded_application_path.as_deref() == Some(context.key.as_str());
        row()
            .gap(px(6.))
            .items_start()
            .min_w_0()
            .child(
                label(
                    if has_path {
                        path.clone()
                    } else {
                        "程序地址未知".into()
                    },
                    self.p.muted,
                )
                .id("application-detail-path")
                .text_size(px(12.))
                .line_height(px(20.))
                .flex_1()
                .min_w_0()
                .when(expanded, |label| label.whitespace_normal())
                .when(!expanded, |label| label.truncate())
                .tooltip(move |window, cx| Tooltip::new(path.clone()).build(window, cx)),
            )
            .when(has_path, |row| {
                row.child(
                    self.button(
                        "application-path-toggle",
                        "",
                        false,
                        Action::Application(ApplicationAction::ToggleApplicationPath),
                        cx,
                    )
                    .flex_none()
                    .size(px(20.))
                    .px(px(0.))
                    .child(
                        svg()
                            .path(if expanded {
                                "icons/chevron-up.svg"
                            } else {
                                "icons/chevron-down.svg"
                            })
                            .size(px(14.))
                            .text_color(rgb(self.p.muted)),
                    )
                    .tooltip(move |window, cx| {
                        Tooltip::new(if expanded {
                            "收起完整路径"
                        } else {
                            "展开完整路径"
                        })
                        .build(window, cx)
                    }),
                )
            })
    }

    fn application_copied(&self, key: &str, path: bool) -> bool {
        self.activity
            .copy_feedback
            .as_ref()
            .is_some_and(|(copied, is_path, at)| {
                copied == key && *is_path == path && at.elapsed() < Duration::from_secs(2)
            })
    }
    pub(super) fn schedule_app_usage_anchor(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.app_usage_anchor_pending
            || self.activity.details
            || !matches!(self.page, Page::Apps | Page::History)
            || (self.page == Page::History && self.history_state.tab == HistoryTab::Analysis)
            || !self.selected.is_empty()
            || self.report.query.as_ref() != Some(&self.query)
        {
            return;
        }
        self.app_usage_anchor_pending = false;
        let expected = self.query.clone();
        let weak = cx.entity().downgrade();
        window.on_next_frame(move |_, cx| {
            let _ = weak.update(cx, |app, cx| {
                if !matches!(app.page, Page::Apps | Page::History)
                    || (app.page == Page::History && app.history_state.tab == HistoryTab::Analysis)
                    || app.activity.details
                    || !app.selected.is_empty()
                    || app.query != expected
                {
                    return;
                }
                let scroll = &app.page_scroll;
                let target = scroll.offset().y + scroll.bounds().top() + px(16.)
                    - app.app_usage_bounds.get().top();
                scroll.set_offset(point(
                    scroll.offset().x,
                    target.clamp(-scroll.max_offset().height, px(0.)),
                ));
                cx.notify();
            });
        });
    }

    pub(super) fn app_badge(&self, key: &str) -> Div {
        self.application_icon(key, false)
    }
    pub(super) fn application_name<'a>(&'a self, key: &'a str) -> &'a str {
        self.data
            .identities
            .get(key)
            .map(|owner| owner.label.as_str())
            .unwrap_or_else(|| app_name(key))
    }
    pub(super) fn application_icon_path<'a>(&'a self, key: &'a str) -> &'a str {
        self.data
            .identities
            .get(key)
            .map(|owner| owner.icon.as_str())
            .unwrap_or(key)
    }
    pub(super) fn application_icon(&self, key: &str, compact: bool) -> Div {
        let image_size = if compact { 16. } else { 24. };
        let image = self.app_icons.image(
            self.application_icon_path(key),
            (image_size * self.icon_scale).ceil() as u32,
        );
        div()
            .size(px(if compact { 20. } else { 30. }))
            .flex_none()
            .rounded(px(if compact { 5. } else { 8. }))
            .flex()
            .items_center()
            .justify_center()
            .when(!compact, |icon| icon.bg(control_surface(self.p)))
            .child(if let Some(image) = image {
                img(ImageSource::Render(image))
                    .size(px(image_size))
                    .object_fit(ObjectFit::Contain)
                    .into_any_element()
            } else {
                Icon::default()
                    .path("icons/application.svg")
                    .size(px(image_size))
                    .text_color(rgb(self.p.muted))
                    .into_any_element()
            })
    }

    fn app_stat(&self, title: &str, value: String, color: u32) -> Div {
        column()
            .flex_1()
            .min_w(px(130.))
            .gap(px(8.))
            .child(label(title.to_string(), self.p.muted))
            .child(
                label(value, color)
                    .text_size(px(24.))
                    .font_family("Segoe UI")
                    .font_weight(FontWeight::SEMIBOLD),
            )
    }
    pub(super) fn network_speed_stat(&self, icon: &'static str, value: String, color: u32) -> Div {
        let (number, unit) = value.split_once(' ').unwrap_or((&value, ""));
        let number = number.to_owned();
        let unit = unit.to_owned();
        let p = self.p;
        // Reserve each direction's full slot so changing digits or units cannot
        // shift the other icon. Center against visible digit ink, not font leading.
        div().w(px(112.)).h(px(26.)).flex_none().child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, cx| {
                    let mut style = window.text_style();
                    style.font_family = "Segoe UI".into();
                    style.font_weight = FontWeight::NORMAL;
                    style.color = rgb(p.muted).into();
                    let unit = window.text_system().shape_line(
                        unit.clone().into(),
                        px(12.),
                        &[style.to_run(unit.len())],
                        None,
                    );
                    style.font_weight = FontWeight::MEDIUM;
                    style.font_features =
                        FontFeatures(std::sync::Arc::new(vec![("tnum".into(), 1)]));
                    style.color = rgb(color).into();
                    let run = style.to_run(number.len());
                    let mut number = window.text_system().shape_line(
                        number.clone().into(),
                        px(20.),
                        std::slice::from_ref(&run),
                        None,
                    );
                    let available = bounds.size.width - px(25.) - unit.width;
                    if number.width > available {
                        number = window.text_system().shape_line(
                            number.text.clone(),
                            (px(20.) * (available / number.width)).max(px(12.)),
                            &[run],
                            None,
                        );
                    }
                    let icon_top = bounds.top() + (bounds.size.height - px(18.)) / 2.;
                    window.paint_quad(
                        fill(
                            Bounds::new(point(bounds.left(), icon_top), size(px(18.), px(18.))),
                            tint_surface(p, color),
                        )
                        .corner_radii(px(5.)),
                    );
                    if let Err(error) = window.paint_svg(
                        Bounds::new(
                            point(bounds.left() + px(3.), icon_top + px(3.)),
                            size(px(12.), px(12.)),
                        ),
                        icon.into(),
                        TransformationMatrix::unit(),
                        rgb(color).into(),
                        cx,
                    ) {
                        tracing::warn!(%error, "Network direction icon could not be painted");
                    }
                    let top = bounds.top() - display_editor::prefix_center_offset(&number, window);
                    let unit_top =
                        top + (number.ascent - number.descent - unit.ascent + unit.descent) / 2.;
                    let number_x = bounds.left() + px(22.);
                    for (line, origin) in [
                        (&number, point(number_x, top)),
                        (&unit, point(number_x + number.width + px(3.), unit_top)),
                    ] {
                        if let Err(error) = line.paint(origin, bounds.size.height, window, cx) {
                            tracing::warn!(%error, "Network speed could not be painted");
                        }
                    }
                },
            )
            .size_full(),
        )
    }

    pub(super) fn apps(&self, window: &Window, cx: &Context<Self>) -> Div {
        let compact = window.viewport_size().width - self.sidebar_width() < px(850.);
        if !self.selected.is_empty() {
            return self.application_detail(compact, cx);
        }
        if self.activity.details {
            return self.activity_details_page(compact, cx);
        }
        column()
            .gap(px(16.))
            .child(
                self.current_app_activity(window.viewport_size().width - self.sidebar_width(), cx),
            )
            .child(if self.usage_detail_docked(window) {
                row()
                    .items_start()
                    .gap(px(16.))
                    .child(self.app_usage_table(true, true, cx).flex_1().min_w_0())
                    .child(self.usage_detail_pane(window, cx))
            } else {
                self.app_usage_table(compact, false, cx)
            })
    }

    fn current_app_activity(&self, available_width: Pixels, cx: &Context<Self>) -> Div {
        let p = self.p;
        let compact = available_width < px(850.);
        let apps = self.ordered_activity();
        let active = apps
            .iter()
            .filter(|app| app.state() == ActivityState::Transferring)
            .count();
        let preview_width = if compact { 295. } else { 330. };
        let column_gap = if compact { 18. } else { 28. };
        // Account for page/card padding and the fixed preview column. Keep four
        // statistics in one row only when each has its existing 130px minimum.
        let compact_stats = available_width < px(88. + preview_width + column_gap + 568.);
        let transfers = self.activity_preview(&apps, preview_width, cx);
        let trend = column()
            .flex_1()
            .min_w_0()
            .min_h(px(304.))
            .gap(px(12.))
            .child(self.app_period_summary(compact_stats))
            .child(if self.data.connected {
                self.live_chart(crate::app_activity::ACTIVITY_WINDOW_MS, 190., false)
                    .flex_none()
            } else {
                self.empty("实时数据已暂停", "正在尝试重新连接 Mihomo")
                    .h(px(228.))
            });
        let charts = stretch().gap(px(column_gap)).child(trend).child(transfers);
        self.card()
            .gap(px(16.))
            .child(
                row()
                    .items_start()
                    .justify_between()
                    .gap(px(12.))
                    .child(
                        row()
                            .flex_1()
                            .min_w_0()
                            .gap(px(if compact { 10. } else { 16. }))
                            .flex_wrap()
                            .child(
                                row()
                                    .h(px(26.))
                                    .gap(px(8.))
                                    .child(heading("流量概览", p).id("apps-realtime-help").tooltip(
                                        |window, cx| {
                                            Tooltip::new(
                                                "最近5分钟的平滑趋势，基于10秒均值；速度数字保留当前采样",
                                            )
                                            .build(window, cx)
                                        },
                                    ))
                                    .when(!self.data.connected, |row| {
                                        row.child(
                                            label("已断开", p.amber)
                                                .text_size(px(11.))
                                                .px(px(7.))
                                                .py(px(3.))
                                                .rounded(px(5.))
                                                .bg(state_surface(p)),
                                        )
                                    }),
                            )
                            .child(
                                row()
                                    .h(px(26.))
                                    .flex_none()
                                    .gap(px(8.))
                                    .whitespace_nowrap()
                                    .child(
                                        label("时段统计", p.muted)
                                            .text_size(px(12.))
                                            .font_weight(FontWeight::MEDIUM),
                                    )
                                    .child(
                                        label(range_caption(&self.query), p.muted)
                                            .text_size(px(11.)),
                                    ),
                            )
                            .child(
                                row()
                                    .flex_none()
                                    .gap(px(4.))
                                    .child(self.network_speed_stat(
                                        "icons/arrow-down.svg",
                                        if self.data.connected {
                                            speed(self.data.down)
                                        } else {
                                            "—".into()
                                        },
                                        p.download_text,
                                    ))
                                    .child(self.network_speed_stat(
                                        "icons/arrow-up.svg",
                                        if self.data.connected {
                                            speed(self.data.up)
                                        } else {
                                            "—".into()
                                        },
                                        p.upload_text,
                                    )),
                            ),
                    )
                    .child(
                        row()
                            .w(px(176.))
                            .h(px(26.))
                            .flex_none()
                            .justify_end()
                            .child(
                                label(
                                    format!(
                                        "{active} 项正在传输 · {} 条连接",
                                        self.data.connections.len()
                                    ),
                                    p.muted,
                                )
                                .text_size(px(12.)),
                            ),
                    ),
            )
            .child(settings_separator(p))
            .child(charts)
    }

    fn app_period_summary(&self, compact: bool) -> Div {
        let p = self.p;
        let loading = self.displayed_page().is_none() || !self.report.error.is_empty();
        let bytes = |value| {
            if loading {
                "—".into()
            } else {
                human_bytes(value)
            }
        };
        let mut stats: Vec<_> = [
            (
                if loading {
                    "总流量 · 更新中"
                } else {
                    "总流量"
                },
                bytes(self.report.total.total()),
                p.text,
            ),
            ("下载", bytes(self.report.total.down), p.download_text),
            ("上传", bytes(self.report.total.up), p.upload_text),
            (
                "已识别应用",
                if loading {
                    "—".into()
                } else {
                    self.report.app_count.to_string()
                },
                p.text,
            ),
        ]
        .into_iter()
        .map(|(title, value, color)| self.app_stat(title, value, color).min_w_0().gap(px(4.)))
        .collect();
        if compact {
            let second_row = stats.split_off(2);
            column()
                .gap(px(12.))
                .child(row().gap(px(16.)).children(stats))
                .child(row().gap(px(16.)).children(second_row))
        } else {
            row().gap(px(16.)).children(stats)
        }
        .flex_none()
    }

    fn app_usage_table(&self, compact: bool, preview: bool, cx: &Context<Self>) -> Div {
        let p = self.p;
        let page = self.displayed_page();
        let loading = page.is_none() || !self.report.error.is_empty();
        let offset = page.unwrap_or_default() * self.query.page_size;
        let maximum = self
            .report
            .apps
            .iter()
            .map(|item| item.bytes.total() as f64)
            .fold(1., f64::max);
        let number_width = if preview { 88. } else { 105. };
        let show_share = !compact || self.usage_detail_open();
        let share_width = if compact { 64. } else { 88. };
        let share_bar_width = if compact { 48. } else { 72. };
        let column_gap = if preview { 8. } else { 10. };
        let bar_width = if compact { 88. } else { 112. };
        let header = row().h(px(34.)).gap(px(column_gap)).px(px(8.))
            .child(label("#", p.muted).text_size(px(12.)).w(px(24.)).flex_none().text_center())
            .child(label("应用", p.muted).flex_1())
            .when(!preview, |row| row.child(label("用量对比", p.muted).w(px(bar_width)).flex_none()))
            .when(!compact, |row| row.child(label("下载", p.muted).w(px(90.)).text_right()).child(label("上传", p.muted).w(px(90.)).text_right()))
            .when(compact, |row| row.child(label("下载 / 上传", p.muted).w(px(number_width)).text_right()))
            .child(label("总流量", p.muted).w(px(number_width)).text_right())
            .when(show_share, |row| row.child(label("应用占比", p.muted).w(px(share_width)).text_right()
                .id("application-share-help").tooltip(|window, cx| Tooltip::new("占已记录应用流量的比例，含未知进程；与 Mihomo 总流量可能存在未归属差额。").build(window, cx))));
        let mut table = column().gap(px(0.));
        for (index, item) in self.report.rows.iter().enumerate().filter(|_| !loading) {
            let key = item.key.clone();
            let group = SharedString::from(format!("application-usage-{index}"));
            let connected = self.data.connected
                && self
                    .data
                    .connections
                    .iter()
                    .any(|connection| connection.application() == key);
            let rate = if self.data.connected {
                let rates = if self.data.application_rates.is_empty() {
                    &self.data.app_rates
                } else {
                    &self.data.application_rates
                };
                rates.get(&key).copied().unwrap_or_default()
            } else {
                Bytes::default()
            };
            let status = if rate.total() > 0 {
                "传输中"
            } else if connected {
                "已连接"
            } else {
                "空闲"
            };
            let path = self.application_icon_path(&key);
            let address = if std::path::Path::new(path).is_absolute() {
                path.to_string()
            } else {
                "程序地址未知".to_string()
            };
            let share = (item.bytes.total() as f64 / self.report.observed.total().max(1) as f64)
                .clamp(0., 1.);
            table = table.child(
                row()
                    .id(group.clone())
                    .group(group.clone())
                    .gap(px(column_gap))
                    .h(px(59.))
                    .px(px(8.))
                    .rounded(px(7.))
                    .cursor_pointer()
                    .border_b_1()
                    .border_color(surface_border(p))
                    .when(
                        self.usage_detail.key.as_deref() == Some(key.as_str()),
                        |row| row.bg(state_surface(p)),
                    )
                    .hover(move |row| row.bg(state_surface(p)))
                    .child(
                        label((offset + index + 1).to_string(), p.muted)
                            .id(SharedString::from(format!(
                                "application-ordinal-{}",
                                offset + index + 1
                            )))
                            .text_size(px(11.))
                            .w(px(24.))
                            .flex_none()
                            .text_center(),
                    )
                    .child(
                        row()
                            .flex_1()
                            .min_w_0()
                            .gap(px(10.))
                            .child(self.app_badge(&key))
                            .child(
                                column()
                                    .flex_1()
                                    .min_w_0()
                                    .gap(px(3.))
                                    .child(
                                        row()
                                            .min_w_0()
                                            .gap(px(8.))
                                            .child(
                                                label(
                                                    self.application_name(&key).to_string(),
                                                    p.text,
                                                )
                                                .min_w_0()
                                                .truncate(),
                                            )
                                            .child(
                                                label(
                                                    status,
                                                    if rate.total() > 0 {
                                                        p.upload
                                                    } else {
                                                        p.muted
                                                    },
                                                )
                                                .text_size(px(11.))
                                                .flex_none(),
                                            ),
                                    )
                                    .child(
                                        label(address, p.muted)
                                            .text_size(px(11.))
                                            .opacity(0.8)
                                            .truncate(),
                                    ),
                            ),
                    )
                    .when(!preview, |row| {
                        row.child(
                            div()
                                .w(px(bar_width))
                                .h(px(5.))
                                .flex_none()
                                .child(self.usage_split_bar(item.bytes, maximum)),
                        )
                    })
                    .when(!compact, |row| {
                        row.child(
                            label(human_bytes(item.bytes.down), p.download)
                                .w(px(90.))
                                .text_right(),
                        )
                        .child(
                            label(human_bytes(item.bytes.up), p.upload)
                                .w(px(90.))
                                .text_right(),
                        )
                    })
                    .when(compact, |row| {
                        row.child(
                            column()
                                .w(px(number_width))
                                .items_end()
                                .gap(px(2.))
                                .child(
                                    label(
                                        format!("↓ {}", human_bytes(item.bytes.down)),
                                        p.download,
                                    )
                                    .text_size(px(12.)),
                                )
                                .child(
                                    label(format!("↑ {}", human_bytes(item.bytes.up)), p.upload)
                                        .text_size(px(12.)),
                                ),
                        )
                    })
                    .child(
                        label(human_bytes(item.bytes.total()), p.text)
                            .w(px(number_width))
                            .text_right()
                            .font_weight(FontWeight::MEDIUM),
                    )
                    .when(show_share, |row| {
                        row.child(
                            column()
                                .w(px(share_width))
                                .gap(px(6.))
                                .items_end()
                                .child(
                                    label(
                                        share_label(
                                            item.bytes.total(),
                                            self.report.observed.total(),
                                        ),
                                        p.muted,
                                    )
                                    .text_size(px(12.)),
                                )
                                .child(
                                    div()
                                        .w(px(share_bar_width))
                                        .h(px(if compact { 3. } else { 4. }))
                                        .rounded_full()
                                        .bg(surface_border(p))
                                        .child(
                                            div()
                                                .h_full()
                                                .w(relative(share as f32))
                                                .rounded_full()
                                                .bg(rgb(p.accent)),
                                        ),
                                ),
                        )
                    })
                    .on_click(cx.listener(move |app, _, window, cx| {
                        app.act(
                            Action::Application(ApplicationAction::OpenUsageDetail(key.clone())),
                            window,
                            cx,
                        )
                    })),
            );
        }
        if self.report.rows.is_empty() || loading {
            table = table.child(self.empty(
                if loading {
                    "正在读取应用记录"
                } else {
                    "没有匹配的应用"
                },
                if loading {
                    "时段统计正在更新"
                } else {
                    "调整时间范围，或清空应用搜索"
                },
            ));
        }
        let delta = self.report.total.total() as i128 - self.report.observed.total() as i128;
        let bounds = self.app_usage_bounds.clone();
        let height = self.app_usage_height.clone();
        self.card()
            .relative()
            .min_h(if loading { height.get() } else { px(0.) })
            .child(
                canvas(
                    move |rect, _, _| {
                        bounds.set(rect);
                        if !loading {
                            height.set(rect.size.height + px(2.));
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .gap(px(12.))
            .child(
                row()
                    .justify_between()
                    .gap(px(10.))
                    .flex_wrap()
                    .child(heading("应用用量", p))
                    .child(
                        row()
                            .gap(px(2.))
                            .children(TrafficSort::ALL.into_iter().map(|sort| {
                                self.button(
                                    format!("app-sort-{sort:?}"),
                                    sort.label(),
                                    self.query.sort == sort,
                                    Action::Application(ApplicationAction::AppSort(sort)),
                                    cx,
                                )
                                .h(px(30.))
                                .px(px(8.))
                            }))
                            .child(
                                self.button(
                                    "app-sort-direction",
                                    if self.query.ascending {
                                        "升序 ↑"
                                    } else {
                                        "降序 ↓"
                                    },
                                    false,
                                    Action::Sort,
                                    cx,
                                )
                                .h(px(30.))
                                .px(px(8.)),
                            ),
                    ),
            )
            .child(div().w_full().child(self.text_input(&self.search)))
            .child(header)
            .child(div().id("application-usage-rows").child(table))
            .when(!loading, |card| {
                card.child(self.pagination(cx)).child(
                    row().gap(px(8.)).flex_wrap()
                        .child(label(format!("{} {} ⓘ", if delta >= 0 { "未归属" } else { "待对账" }, human_bytes(delta.unsigned_abs().min(u64::MAX as u128) as u64)), p.muted)
                            .text_size(px(11.)).id("application-unattributed-help")
                            .tooltip(|window, cx| Tooltip::new("总流量与应用汇总的差额，可能受采样或归属缺失影响；没有可追溯的程序名单，开启进程识别也不会补算历史。").build(window, cx)))
                        .when(self.report.unknown.total() > 0, |row| row.child(
                            self.button("application-unknown-details", format!("未识别应用 {} ↗", human_bytes(self.report.unknown.total())), false, Action::Application(ApplicationAction::OpenUsageDetail("未知进程".into())), cx)
                                .h(px(24.)).px(px(4.)).text_size(px(11.))
                                .tooltip(|window, cx| Tooltip::new("查看记录在未知进程下的流量明细；这些记录没有具体程序名称。").build(window, cx))))
                        .when(self.report.unknown.total() == 0, |row| row.child(label("未识别应用 0 B", p.muted).text_size(px(11.))))
                        .when(!self.data.connected || crate::config::ProcessMode::parse(&self.data.process_mode) != Some(crate::config::ProcessMode::Always), |row| row.child(
                            self.button("application-process-settings", if self.data.connected && crate::config::ProcessMode::parse(&self.data.process_mode).is_some() { "完善进程识别 ↗" } else { "检查进程识别 ↗" }, false, Action::OpenProcessSettings, cx)
                                .h(px(24.)).px(px(4.)).text_size(px(11.))
                                .tooltip(|window, cx| Tooltip::new("打开设置中的进程识别选项，可选择 always（始终）。仅影响后续连接，不补算历史流量。").build(window, cx))))
                )
            })
    }

    fn application_detail(&self, compact: bool, cx: &Context<Self>) -> Div {
        let context = self.active_detail_context().expect("selected application");
        self.application_detail_content(&context, compact, false, cx)
    }
    pub(super) fn application_detail_content(
        &self,
        context: &DetailContext,
        compact: bool,
        pane: bool,
        cx: &Context<Self>,
    ) -> Div {
        let p = self.p;
        let processes_expanded = if pane {
            self.usage_detail.processes_expanded
        } else {
            self.activity.processes_expanded
        };
        let loading = !crate::detail_cache::matches(&self.detail, &self.detail_query_for(context));
        let failed = !loading && !self.detail.error.is_empty() && self.detail.series.is_empty();
        let defer_loading = loading
            && self
                .detail_requested_at
                .is_some_and(|at| at.elapsed() < Duration::from_millis(150));
        let breakdown_rows = match self.detail_dimension {
            "route" => &self.detail.routes,
            "source" => &self.detail.sources,
            _ => &self.detail.domains,
        };
        let rate = if !context.group_apps {
            &self.data.app_rates
        } else {
            &self.data.application_rates
        }
        .get(&context.key)
        .copied()
        .unwrap_or_default();
        let connections = self
            .data
            .connections
            .iter()
            .filter(|connection| if !context.group_apps { connection.app() } else { connection.application() } == context.key)
            .count();
        let mut breakdown = self
            .card()
            .flex_1()
            .min_w_0()
            .gap(px(14.))
            .child(
                row()
                    .justify_between()
                    .gap(px(8.))
                    .flex_wrap()
                    .child(heading("流量去向", p))
                    .child(
                        row().gap(px(2.)).children(
                            [("域名", "domain"), ("出站", "route"), ("来源 IP", "source")]
                                .into_iter()
                                .map(|(title, dimension)| {
                                    self.button(
                                        format!("app-detail-{dimension}"),
                                        title,
                                        self.detail_dimension == dimension,
                                        Action::Application(ApplicationAction::DetailDimension(
                                            dimension,
                                        )),
                                        cx,
                                    )
                                    .h(px(28.))
                                    .px(px(8.))
                                }),
                        ),
                    ),
            )
            .when(self.detail.archive_daily && !loading, |card| {
                card.child(label("早期明细按整日汇总", p.amber).text_size(px(12.)))
            })
            .when(self.detail_dimension == "source", |card| {
                card.child(
                    label("连接发起地址；TUN 模式下可能显示虚拟网卡 IP。", p.muted)
                        .text_size(px(12.))
                        .whitespace_normal(),
                )
            });
        for (index, item) in breakdown_rows
            .iter()
            .take(8)
            .enumerate()
            .filter(|_| !loading && !failed)
        {
            let key = item.key.clone();
            breakdown = breakdown.child(
                row()
                    .gap(px(10.))
                    .py(px(5.))
                    .child(
                        label(format!("{:02}", index + 1), p.muted)
                            .text_size(px(11.))
                            .w(px(20.)),
                    )
                    .child(
                        label(item.key.clone(), p.text)
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .id(SharedString::from(format!("app-detail-key-{index}")))
                            .tooltip(move |window, cx| Tooltip::new(key.clone()).build(window, cx)),
                    )
                    .child(label(human_bytes(item.bytes.total()), p.text).flex_none()),
            );
        }
        if defer_loading {
            breakdown = breakdown.child(div().h(px(190.)));
        } else if failed {
            breakdown = breakdown.child(self.empty("读取明细失败", &self.detail.error).child(
                self.button(
                    "app-detail-retry-flow",
                    "重试",
                    false,
                    Action::Application(ApplicationAction::RefreshApplicationDetail),
                    cx,
                ),
            ));
        } else if breakdown_rows.is_empty() || loading {
            breakdown = breakdown.child(self.empty(
                if loading {
                    "正在读取明细"
                } else {
                    "暂无明细记录"
                },
                "可切换时间范围或查看其他统计维度",
            ));
        }
        let trend = self
            .card()
            .flex_1()
            .min_w_0()
            .gap(px(12.))
            .child(heading("时段流量趋势", p))
            .child(
                label(
                    format!(
                        "{} · {}",
                        range_caption(&self.query),
                        crate::app_trend::BucketSize::for_range(self.query.from, self.query.to)
                            .label()
                    ),
                    p.muted,
                )
                .text_size(px(12.)),
            )
            .child(self.detail_speed_summary(
                "application-speed-summary",
                (!loading && !failed).then_some(&self.detail.speeds),
                cx,
            ))
            .when(
                !loading && !failed && !self.detail.error.is_empty(),
                |card| card.child(label("更新失败，显示上次记录", p.muted).text_size(px(11.))),
            )
            .child(if defer_loading {
                div().h(px(248.))
            } else if loading {
                self.empty("正在读取流量趋势", "请稍候")
            } else if failed {
                self.empty("读取流量趋势失败", &self.detail.error)
                    .child(self.button(
                        "app-detail-retry-trend",
                        "重试",
                        false,
                        Action::Application(ApplicationAction::RefreshApplicationDetail),
                        cx,
                    ))
            } else {
                self.application_trend_chart(self.detail.histogram.clone(), 190., cx)
            });
        let trend = if pane { trend.flex_initial() } else { trend };
        let breakdown = if pane {
            breakdown.flex_initial()
        } else {
            breakdown
        };
        let analysis = if compact {
            column().gap(px(16.)).child(trend).child(breakdown)
        } else {
            row()
                .items_start()
                .gap(px(16.))
                .child(trend)
                .child(breakdown)
        };
        column()
            .gap(px(16.))
            .when(!pane, |view| {
                view.child(
                    row()
                        .justify_between()
                        .gap(px(8.))
                        .flex_wrap()
                        .child(
                            self.button(
                                "back-to-applications",
                                "← 返回",
                                false,
                                Action::Application(ApplicationAction::ClearAppSelection),
                                cx,
                            )
                            .px(px(0.)),
                        )
                        .child(label("应用详情", p.muted)),
                )
            })
            .child(
                self.card()
                    .gap(px(16.))
                    .child(
                        row()
                            .items_start()
                            .gap(px(12.))
                            .child(self.app_badge(&context.key))
                            .child(
                                column().flex_1().min_w_0().gap(px(5.)).child(
                                    row()
                                        .items_center()
                                        .gap(px(8.))
                                        .when(!pane, |row| row.flex_wrap())
                                        .child(
                                            heading(
                                                self.application_name(&context.key).to_string(),
                                                p,
                                            )
                                            .when(pane, |name| name.flex_1())
                                            .text_size(px(20.))
                                            .max_w(px(if compact { 200. } else { 340. }))
                                            .min_w_0()
                                            .truncate()
                                            .id("application-detail-name")
                                            .when(
                                                pane,
                                                |name| {
                                                    let full_name = self
                                                        .application_name(&context.key)
                                                        .to_string();
                                                    name.tooltip(move |window, cx| {
                                                        Tooltip::new(full_name.clone())
                                                            .build(window, cx)
                                                    })
                                                },
                                            ),
                                        )
                                        .child(self.application_detail_actions(context, pane, cx)),
                                ),
                            )
                            .when(!pane, |row| {
                                row.child(self.button(
                                    "app-connections",
                                    format!("查看连接 ({connections}) ↗"),
                                    true,
                                    Action::AppConnections,
                                    cx,
                                ))
                            }),
                    )
                    .when_some(application_description(context), |card, description| {
                        card.child(
                            row()
                                .w_full()
                                .items_start()
                                .gap(px(8.))
                                .px(px(10.))
                                .py(px(8.))
                                .rounded(px(8.))
                                .bg(control_surface(p))
                                .border_1()
                                .border_color(surface_border(p))
                                .text_color(rgb(p.muted))
                                .child(
                                    div().mt(px(2.)).flex_none().child(
                                        Icon::default()
                                            .path("icons/position-info.svg")
                                            .size(px(14.)),
                                    ),
                                )
                                .child(
                                    column()
                                        .flex_1()
                                        .min_w_0()
                                        .gap(px(3.))
                                        .child(
                                            label(
                                                if context.key.starts_with("service:") {
                                                    "服务说明"
                                                } else {
                                                    "进程说明"
                                                },
                                                p.muted,
                                            )
                                            .text_size(px(11.))
                                            .font_weight(FontWeight::MEDIUM),
                                        )
                                        .child(
                                            label(description, p.muted)
                                                .w_full()
                                                .min_w_0()
                                                .text_size(px(12.))
                                                .whitespace_normal(),
                                        ),
                                ),
                        )
                    })
                    .child(self.application_detail_path(context, cx))
                    .when(pane, |card| {
                        card.child(
                            row()
                                .gap(px(8.))
                                .justify_between()
                                .flex_wrap()
                                .child(
                                    row()
                                        .gap(px(12.))
                                        .child(
                                            label(
                                                format!(
                                                    "↓ {}",
                                                    if self.data.connected {
                                                        speed(rate.down as f64)
                                                    } else {
                                                        "—".into()
                                                    }
                                                ),
                                                p.download,
                                            )
                                            .text_size(px(13.)),
                                        )
                                        .child(
                                            label(
                                                format!(
                                                    "↑ {}",
                                                    if self.data.connected {
                                                        speed(rate.up as f64)
                                                    } else {
                                                        "—".into()
                                                    }
                                                ),
                                                p.upload,
                                            )
                                            .text_size(px(13.)),
                                        ),
                                )
                                .child(
                                    self.button(
                                        "app-connections",
                                        format!("查看连接 ({connections}) ↗"),
                                        false,
                                        Action::AppConnections,
                                        cx,
                                    )
                                    .h(px(26.))
                                    .px(px(6.))
                                    .text_size(px(12.)),
                                ),
                        )
                    })
                    .when(!pane, |card| {
                        card.child(settings_separator(p))
                            .child(
                                row()
                                    .gap(px(16.))
                                    .flex_wrap()
                                    .child(self.app_stat(
                                        "当前下载",
                                        if self.data.connected {
                                            speed(rate.down as f64)
                                        } else {
                                            "—".into()
                                        },
                                        p.download,
                                    ))
                                    .child(self.app_stat(
                                        "当前上传",
                                        if self.data.connected {
                                            speed(rate.up as f64)
                                        } else {
                                            "—".into()
                                        },
                                        p.upload,
                                    ))
                                    .child(self.app_stat(
                                        "本时段用量",
                                        if loading || failed {
                                            "—".into()
                                        } else {
                                            human_bytes(self.detail.total.total())
                                        },
                                        p.text,
                                    )),
                            )
                            .child(
                                row()
                                    .gap(px(16.))
                                    .flex_wrap()
                                    .child(label(
                                        format!(
                                            "时段下载 {}",
                                            if loading || failed {
                                                "—".into()
                                            } else {
                                                human_bytes(self.detail.total.down)
                                            }
                                        ),
                                        p.download,
                                    ))
                                    .child(label(
                                        format!(
                                            "时段上传 {}",
                                            if loading || failed {
                                                "—".into()
                                            } else {
                                                human_bytes(self.detail.total.up)
                                            }
                                        ),
                                        p.upload,
                                    ))
                                    .child(label(range_caption(&self.query), p.muted)),
                            )
                    }),
            )
            .when(
                context.group_apps
                    && !loading
                    && (self.detail.members.len() > 1
                        || self
                            .detail
                            .members
                            .iter()
                            .any(|member| member.key != context.key)),
                |view| {
                    let mut processes = self.card().gap(px(10.)).child(
                        row()
                            .justify_between()
                            .gap(px(8.))
                            .child(heading(
                                format!("原始进程 · {} 项", self.detail.members.len()),
                                p,
                            ))
                            .child(self.button(
                                "toggle-application-processes",
                                if processes_expanded {
                                    "收起"
                                } else {
                                    "展开"
                                },
                                false,
                                Action::Application(ApplicationAction::ToggleApplicationProcesses),
                                cx,
                            )),
                    );
                    if processes_expanded {
                        for member in &self.detail.members {
                            processes = processes.child(
                                row()
                                    .gap(px(12.))
                                    .items_start()
                                    .child(self.application_icon(&member.key, true))
                                    .child(
                                        column()
                                            .flex_1()
                                            .min_w_0()
                                            .gap(px(3.))
                                            .child(label(app_name(&member.key).to_string(), p.text))
                                            .child(
                                                label(member.key.clone(), p.muted)
                                                    .text_size(px(12.))
                                                    .whitespace_normal(),
                                            ),
                                    )
                                    .child(
                                        column()
                                            .flex_none()
                                            .items_end()
                                            .gap(px(3.))
                                            .child(label(
                                                format!("↓ {}", human_bytes(member.bytes.down)),
                                                p.download,
                                            ))
                                            .child(label(
                                                format!("↑ {}", human_bytes(member.bytes.up)),
                                                p.upload,
                                            )),
                                    ),
                            );
                        }
                    }
                    view.child(processes)
                },
            )
            .child(analysis)
    }
}
