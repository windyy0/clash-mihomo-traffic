use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Page {
    Overview,
    Apps,
    History,
    Connections,
    Display,
    Settings,
}
impl Page {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Overview => "概览",
            Self::Apps => "应用",
            Self::History => "历史",
            Self::Connections => "连接",
            Self::Display => "显示",
            Self::Settings => "设置",
        }
    }
    pub(super) fn has_time_range(self) -> bool {
        matches!(self, Self::Overview | Self::Apps)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum DisplayTab {
    Taskbar,
    Floating,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SettingsSection {
    General,
    Connection,
    Storage,
    Backup,
    About,
}

impl SettingsSection {
    pub(super) const ALL: [Self; 5] = [
        Self::General,
        Self::Connection,
        Self::Storage,
        Self::Backup,
        Self::About,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::General => "通用",
            Self::Connection => "mihomo连接",
            Self::Storage => "数据存储",
            Self::Backup => "备份与恢复",
            Self::About => "关于",
        }
    }

    pub(super) fn id(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Connection => "connection",
            Self::Storage => "storage",
            Self::Backup => "backup",
            Self::About => "about",
        }
    }
}

impl TrafficApp {
    pub(super) fn navigate(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        if self.page != page {
            self.dismiss_update_notes();
            self.page_number
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
        if page != Page::Overview {
            self.reset_overview_exit_hover();
        }
        self.history_state.seconds_level = false;
        if self.seconds.scope.is_some() {
            self.close_seconds(window, cx);
        }
        if self.page == Page::History {
            self.save_history_context(cx);
            if page != Page::History
                && let Some((from, to)) = self.history_state.outside_range.take()
            {
                self.query.from = from;
                self.query.to = to;
            }
        } else if page == Page::History {
            self.history_state.outside_range = Some((self.query.from, self.query.to));
        }
        if page == Page::Overview && self.period == TimePeriod::Custom {
            self.period = TimePeriod::Today;
            if let Ok(Some((from, to))) = self.period.bounds(chrono::Local::now()) {
                self.query.from = from;
                self.query.to = to;
            }
        } else if page == Page::Apps
            && let Some((from, to)) = self.app_custom_range
        {
            self.period = TimePeriod::Custom;
            self.query.from = from;
            self.query.to = to;
        }
        self.expanded_application_path = None;
        self.close_usage_detail();
        self.activity.selected_process = false;
        self.activity.connection_group = None;
        self.connections_view.expanded = None;
        self.connections_view.page = 0;
        self.query.group_apps = page == Page::Apps;
        self.query.history = page == Page::History;
        self.query.history_context = None;
        self.history_state.drag = None;
        self.history_state.month_open = false;
        self.history_state.time_open = false;
        self.history_state.time_expanded = false;
        self.history_state.time_initialized = false;
        self.history_state.time_quick = None;
        self.history_state.time_drag = None;
        self.history_state.time_dragged = false;
        self.activity.details = false;
        self.analysis.active = false;
        self.analysis.anchor = None;
        self.app_usage_anchor_pending = false;
        if self.page != page {
            self.restore_armed = false;
        }
        self.page = page;
        if page == Page::Settings && self.settings_section == SettingsSection::Storage {
            self.refresh_storage(cx);
        }
        self.search.update(cx, |input, cx| {
            input.set_placeholder(
                if page == Page::Apps {
                    "搜索应用名称或完整路径…"
                } else if page == Page::Connections {
                    "搜索应用、域名、IP、出站或规则…"
                } else {
                    "搜索应用、域名或目标…"
                },
                window,
                cx,
            )
        });
        self.page_scroll.set_offset(point(px(0.), px(0.)));
        self.selected.clear();
        self.detail = Report::default();
        self.query.page = 0;
        self.query.page_size = if page == Page::Apps { 10 } else { 50 };
        self.query.search.clear();
        self.query.dimension = "app".into();
        if page != Page::Apps {
            self.query.sort = TrafficSort::Total;
        }
        self.search.update(cx, |s, cx| s.set_value("", window, cx));
        self.last_search.clear();
        if page == Page::History {
            self.enter_history(window, cx);
        } else if page == Page::Apps && self.period == TimePeriod::Custom {
            self.open_history_time(window, cx);
        }
        self.refresh();
        if page == Page::Display
            && self.display_tab == DisplayTab::Taskbar
            && self.config.taskbar_position_mode == TaskbarPositionMode::Manual
        {
            self.taskbar_position.focus.focus(window);
        }
    }

    pub(super) fn select_display_tab(
        &mut self,
        tab: DisplayTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.display_tab != tab {
            self.update_taskbar_font_size_input(true, cx);
            self.update_taskbar_weight_input(true, cx);
            for target in TaskbarColorTarget::ALL {
                self.update_taskbar_color_input(target, true, cx);
            }
            window.blur();
            std::mem::swap(&mut self.editor, &mut self.inactive_editor);
            self.display_tab = tab;
        }
        if tab == DisplayTab::Taskbar
            && self.config.taskbar_position_mode == TaskbarPositionMode::Manual
        {
            self.taskbar_position.focus.focus(window);
        }
    }

    pub(super) fn select_settings_section(
        &mut self,
        section: SettingsSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.settings_section != section {
            self.dismiss_update_notes();
            if self.settings_section == SettingsSection::Storage {
                self.save_retention(window, cx);
                self.save_retention_size(window, cx);
            }
            self.restore_armed = false;
            self.settings_section = section;
            if section == SettingsSection::Storage {
                self.refresh_storage(cx);
            }
            window.blur();
        }
    }

    pub(super) fn sidebar_width(&self) -> Pixels {
        px(if self.config.collapsed { 68. } else { 184. })
    }

    pub(super) fn sidebar(&self, cx: &Context<Self>) -> Div {
        let p = self.p;
        let collapsed = self.config.collapsed;
        let mut nav = column().gap(px(6.)).flex_1();
        for (page, icon) in [
            (Page::Overview, "\u{e80f}"),
            (Page::Apps, "\u{e8a5}"),
            (Page::History, "\u{e81c}"),
            (Page::Connections, "\u{e968}"),
            (Page::Display, "\u{e7f4}"),
            (Page::Settings, "\u{e713}"),
        ] {
            let active = self.page == page;
            let color = if active { p.accent_text } else { p.text };
            let nav_icon = match page {
                Page::Apps | Page::Connections => svg()
                    .path(if page == Page::Apps {
                        "icons/sidebar-apps.svg"
                    } else {
                        "icons/sidebar-connections.svg"
                    })
                    .size(px(18.))
                    .text_color(rgb(color))
                    .into_any_element(),
                _ => div()
                    .font_family("Segoe MDL2 Assets")
                    .text_size(px(16.))
                    .child(icon)
                    .into_any_element(),
            };
            nav = nav.child(
                div()
                    .id(SharedString::from(format!("nav-{}", page.label())))
                    .relative()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .h(px(42.))
                    .px(px(12.))
                    .rounded(px(11.))
                    .cursor_pointer()
                    .bg(if active { state_surface(p) } else { rgba(0) })
                    .text_color(rgb(color))
                    .hover(move |s| s.bg(state_surface(p)))
                    .child(
                        div()
                            .size(px(18.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(nav_icon),
                    )
                    .when(!collapsed, |d| {
                        d.child(div().font_weight(FontWeight::NORMAL).child(page.label()))
                    })
                    .when(page == Page::Settings && self.updates.has_unread(), |d| {
                        d.child(
                            div()
                                .absolute()
                                .top(px(8.))
                                .right(px(7.))
                                .size(px(6.))
                                .rounded_full()
                                .bg(rgb(p.accent_text)),
                        )
                    })
                    .on_click(cx.listener(move |s, _, w, cx| s.act(Action::Nav(page), w, cx))),
            );
        }
        column()
            .w(self.sidebar_width())
            .flex_shrink_0()
            .h_full()
            .bg(if self.mica_active {
                rgba(0)
            } else {
                rgb(p.side)
            })
            .p(px(12.))
            .gap(px(24.))
            .border_r_1()
            .border_color(card_border(p))
            .child(nav)
            .child(
                row()
                    .h(px(32.))
                    .flex_none()
                    .gap(px(if collapsed { 5. } else { 8. }))
                    .child(
                        row()
                            .flex_1()
                            .min_w_0()
                            .gap(px(8.))
                            .child(
                                div()
                                    .id("connection-status")
                                    .size(px(7.))
                                    .flex_none()
                                    .rounded_full()
                                    .bg(rgb(if self.data.connected {
                                        p.upload
                                    } else {
                                        p.amber
                                    }))
                                    .tooltip({
                                        let connected = self.data.connected;
                                        move |window, cx| {
                                            Tooltip::new(if connected {
                                                "mihomo 已连接"
                                            } else {
                                                "等待连接"
                                            })
                                            .build(window, cx)
                                        }
                                    }),
                            )
                            .when(!collapsed, |d| {
                                d.child(label(
                                    if self.data.connected {
                                        "mihomo 已连接"
                                    } else {
                                        "等待连接"
                                    },
                                    p.muted,
                                ))
                            }),
                    )
                    .child(
                        div()
                            .id("collapse")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(28.))
                            .flex_none()
                            .rounded(px(6.))
                            .text_color(rgb(p.muted))
                            .cursor_pointer()
                            .hover(move |d| d.bg(state_surface(p)).text_color(rgb(p.accent_text)))
                            .tooltip(move |window, cx| {
                                Tooltip::new(if collapsed {
                                    "展开侧栏"
                                } else {
                                    "收起侧栏"
                                })
                                .build(window, cx)
                            })
                            .child(
                                Icon::default()
                                    .path(if collapsed {
                                        "icons/sidebar-expand.svg"
                                    } else {
                                        "icons/sidebar-collapse.svg"
                                    })
                                    .size(px(16.)),
                            )
                            .on_click(
                                cx.listener(|s, _, window, cx| s.act(Action::Collapse, window, cx)),
                            ),
                    ),
            )
    }
}
