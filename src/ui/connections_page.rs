use super::*;

pub(super) const PAGE_SIZE: usize = 10;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Protocol {
    #[default]
    All,
    Tcp,
    Udp,
}

impl Protocol {
    fn matches(self, connection: &Connection) -> bool {
        match self {
            Self::All => true,
            Self::Tcp => connection.metadata.network.eq_ignore_ascii_case("tcp"),
            Self::Udp => connection.metadata.network.eq_ignore_ascii_case("udp"),
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Sort {
    #[default]
    Total,
    Download,
    Upload,
    Duration,
}

impl Sort {
    fn label(self) -> &'static str {
        match self {
            Self::Total => "累计合计",
            Self::Download => "累计下载",
            Self::Upload => "累计上传",
            Self::Duration => "持续时间",
        }
    }
}

#[derive(Default)]
pub(super) struct State {
    pub raw_process: bool,
    pub protocol: Protocol,
    pub sort: Sort,
    pub ascending: bool,
    pub page: usize,
    pub expanded: Option<String>,
}

// Ports remain unambiguous for IPv6, including when the kernel omitted a host name.
pub(super) fn endpoint(host: &str, port: &str) -> String {
    if host.is_empty() {
        return "—".into();
    }
    if port.is_empty() {
        host.into()
    } else if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

fn duration(connection: &Connection, now: i64) -> String {
    let started = connection.started();
    if started <= 0 {
        return "—".into();
    }
    let seconds = now.saturating_sub(started).max(0) / 1000;
    if seconds < 60 {
        format!("{seconds}秒")
    } else if seconds < 3600 {
        format!("{}分{}秒", seconds / 60, seconds % 60)
    } else if seconds < 86400 {
        format!("{}时{}分", seconds / 3600, seconds % 3600 / 60)
    } else {
        format!("{}天{}时", seconds / 86400, seconds % 86400 / 3600)
    }
}

#[derive(Clone)]
pub(super) enum ConnectionsAction {
    Protocol(connections_page::Protocol),
    Sort(connections_page::Sort),
    Order,
    Expand(String),
    ClearGroup,
}

impl TrafficApp {
    pub(super) fn handle_connections_action(&mut self, action: ConnectionsAction) {
        match action {
            ConnectionsAction::Protocol(protocol) => {
                self.connections_view.protocol = protocol;
                self.connections_view.page = 0;
                self.page_scroll.set_offset(point(px(0.), px(0.)));
            }
            ConnectionsAction::Sort(sort) => {
                self.connections_view.sort = sort;
                self.connections_view.page = 0;
            }
            ConnectionsAction::Order => {
                self.connections_view.ascending = !self.connections_view.ascending;
                self.connections_view.page = 0;
            }
            ConnectionsAction::Expand(id) => {
                self.connections_view.expanded =
                    (self.connections_view.expanded.as_ref() != Some(&id)).then_some(id);
            }
            ConnectionsAction::ClearGroup => {
                self.activity.connection_group = None;
                self.connections_view.page = 0;
            }
        }
    }

    pub(super) fn filtered_connections(&self) -> Vec<&Connection> {
        let search = self.last_search.trim().to_lowercase();
        let mut connections: Vec<_> = self
            .data
            .connections
            .iter()
            .filter(|connection| {
                self.connections_view.protocol.matches(connection)
                    && self.activity.connection_group.as_ref().is_none_or(|key| {
                        if self.connections_view.raw_process {
                            connection.app() == *key
                        } else {
                            connection.application() == *key
                        }
                    })
                    && (search.is_empty()
                        || [
                            connection.app(),
                            connection.application(),
                            self.application_name(&connection.application()).to_string(),
                            connection.domain(),
                            connection.metadata.destination_ip.clone(),
                            connection.metadata.destination_port.clone(),
                            connection.metadata.source_ip.clone(),
                            connection.metadata.source_port.clone(),
                            connection.metadata.network.clone(),
                            connection.chains.join(" "),
                            connection.rule.clone(),
                        ]
                        .join(" ")
                        .to_lowercase()
                        .contains(&search))
            })
            .collect();
        let sort = self.connections_view.sort;
        connections.sort_unstable_by(|a, b| {
            let order = match sort {
                Sort::Total => b.bytes().total().cmp(&a.bytes().total()),
                Sort::Download => b.download.cmp(&a.download),
                Sort::Upload => b.upload.cmp(&a.upload),
                Sort::Duration => {
                    // An invalid timestamp is unknown, rather than an ancient connection.
                    let time = |c: &Connection| {
                        let started = c.started();
                        if started > 0 { started } else { i64::MAX }
                    };
                    time(a).cmp(&time(b))
                }
            };
            let order = if self.connections_view.ascending {
                order.reverse()
            } else {
                order
            };
            order.then_with(|| a.id.cmp(&b.id))
        });
        connections
    }

    pub(super) fn sync_connections_view(&mut self) {
        self.connections_view.page = self
            .connections_view
            .page
            .min(self.filtered_connections().len().saturating_sub(1) / PAGE_SIZE);
        if self.connections_view.expanded.as_ref().is_some_and(|id| {
            !self
                .data
                .connections
                .iter()
                .any(|connection| &connection.id == id)
        }) {
            self.connections_view.expanded = None;
        }
    }

    fn connection_summary(&self) -> Div {
        let p = self.p;
        let all = &self.data.connections;
        let applications = all
            .iter()
            .map(Connection::application)
            .collect::<std::collections::HashSet<_>>()
            .len();
        let tcp = all.iter().filter(|c| Protocol::Tcp.matches(c)).count();
        let udp = all.iter().filter(|c| Protocol::Udp.matches(c)).count();
        self.card().p(px(16.)).child(
            row().gap(px(0.)).children(
                [
                    ("当前连接", all.len()),
                    ("联网应用", applications),
                    ("TCP", tcp),
                    ("UDP", udp),
                ]
                .into_iter()
                .enumerate()
                .map(|(index, (title, value))| {
                    column()
                        .flex_1()
                        .min_w_0()
                        .gap(px(4.))
                        .when(index > 0, |d| {
                            d.pl(px(20.)).border_l_1().border_color(surface_border(p))
                        })
                        .child(label(title, p.muted).text_size(px(12.)))
                        .child(
                            label(value.to_string(), p.text)
                                .text_size(px(24.))
                                .line_height(px(30.))
                                .font_weight(FontWeight::MEDIUM),
                        )
                }),
            ),
        )
    }

    fn connection_details(&self, connection: &Connection, wide: bool) -> Stateful<Div> {
        let p = self.p;
        let field = |title, value: String| {
            row()
                .items_start()
                .gap(px(12.))
                .min_w_0()
                .child(
                    label(title, p.muted)
                        .w(px(64.))
                        .flex_none()
                        .text_size(px(12.)),
                )
                .child(
                    label(
                        if value.is_empty() {
                            "—".into()
                        } else {
                            value
                        },
                        p.text,
                    )
                    .text_size(px(12.))
                    .flex_1()
                    .min_w_0()
                    .whitespace_normal(),
                )
        };
        let owner = connection.application();
        let pid = connection
            .attribution
            .as_ref()
            .map(|value| value.pid)
            .or(connection.metadata.process_id);
        let process = format!(
            "{}{}",
            connection.app(),
            pid.map(|pid| format!(" · PID {pid}")).unwrap_or_default()
        );
        let started = chrono::DateTime::parse_from_rfc3339(&connection.start)
            .map(|time| {
                time.with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M:%S")
                    .to_string()
            })
            .unwrap_or_default();
        column()
            .id(SharedString::from(format!(
                "connection-details-{}",
                connection.id
            )))
            .gap(px(10.))
            .mx(px(10.))
            .mb(px(12.))
            .p(px(12.))
            .rounded(px(8.))
            .border_1()
            .border_color(surface_border(p))
            .child(
                row()
                    .items_start()
                    .gap(px(20.))
                    .flex_wrap()
                    .child(
                        column()
                            .gap(px(8.))
                            .flex_1()
                            .min_w(px(if wide { 280. } else { 220. }))
                            .child(field(
                                "目标地址",
                                endpoint(
                                    &connection.domain(),
                                    &connection.metadata.destination_port,
                                ),
                            ))
                            .child(field(
                                "目标 IP",
                                endpoint(
                                    &connection.metadata.destination_ip,
                                    &connection.metadata.destination_port,
                                ),
                            ))
                            .child(field(
                                "来源地址",
                                endpoint(
                                    &connection.metadata.source_ip,
                                    &connection.metadata.source_port,
                                ),
                            )),
                    )
                    .child(
                        column()
                            .gap(px(8.))
                            .flex_1()
                            .min_w(px(if wide { 280. } else { 220. }))
                            .child(field(
                                "出站链路",
                                connection
                                    .chains
                                    .iter()
                                    .rev()
                                    .cloned()
                                    .collect::<Vec<_>>()
                                    .join(" → "),
                            ))
                            .child(field("匹配规则", connection.rule.clone()))
                            .child(field("开始时间", started)),
                    ),
            )
            .child(div().h(px(1.)).bg(surface_border(p)))
            .child(field("所属应用", self.application_name(&owner).to_string()))
            .child(field("原始进程", process))
            .child(field("连接类型", connection.metadata.r#type.clone()))
            .child(field("累计合计", human_bytes(connection.bytes().total())))
    }

    fn connection_row(
        &self,
        connection: &Connection,
        wide: bool,
        now: i64,
        cx: &Context<Self>,
    ) -> Div {
        let p = self.p;
        let key = connection.application();
        let expanded = self.connections_view.expanded.as_ref() == Some(&connection.id);
        let target = endpoint(&connection.domain(), &connection.metadata.destination_port);
        let route = connection.route();
        let title = self.application_name(&key).to_string();
        let original = connection.app();
        let tooltip = if expanded {
            "点击收起连接详情（Esc）"
        } else {
            "点击展开连接详情"
        };
        let id = connection.id.clone();
        let identity = column()
            .gap(px(4.))
            .min_w_0()
            .flex_1()
            .child(
                row()
                    .gap(px(6.))
                    .min_w_0()
                    .child(self.application_icon(&key, true).flex_none())
                    .child(label(title, p.text).min_w_0().flex_1().truncate()),
            )
            .child(
                label(
                    if wide {
                        if key != original {
                            format!("进程：{}", app_name(&original))
                        } else {
                            original.clone()
                        }
                    } else {
                        target.clone()
                    },
                    p.muted,
                )
                .text_size(px(12.))
                .truncate(),
            );
        let traffic = if wide {
            row()
                .gap(px(8.))
                .w(px(160.))
                .flex_none()
                .child(
                    label(human_bytes(connection.download), p.download_text)
                        .w(px(76.))
                        .text_right(),
                )
                .child(
                    label(human_bytes(connection.upload), p.upload_text)
                        .w(px(76.))
                        .text_right(),
                )
        } else {
            column()
                .gap(px(3.))
                .w(px(96.))
                .flex_none()
                .items_end()
                .child(
                    label(
                        format!("↓ {}", human_bytes(connection.download)),
                        p.download_text,
                    )
                    .text_size(px(12.)),
                )
                .child(
                    label(
                        format!("↑ {}", human_bytes(connection.upload)),
                        p.upload_text,
                    )
                    .text_size(px(12.)),
                )
        };
        column()
            .gap(px(0.))
            .min_w_0()
            .child(
                row()
                    .id(SharedString::from(format!(
                        "connection-row-{}",
                        connection.id
                    )))
                    .gap(px(8.))
                    .h(px(66.))
                    .px(px(10.))
                    .rounded(px(7.))
                    .when(expanded, |d| d.bg(state_surface(p)))
                    .hover(move |d| d.bg(state_surface(p)))
                    .cursor_pointer()
                    .child(if wide {
                        div().w(px(170.)).flex_none().child(identity)
                    } else {
                        div().flex_1().min_w_0().child(identity)
                    })
                    .when(wide, |d| {
                        d.child(label(target, p.text).flex_1().min_w_0().truncate())
                    })
                    .child(
                        label(
                            if connection.metadata.network.is_empty() {
                                "—".into()
                            } else {
                                connection.metadata.network.to_uppercase()
                            },
                            p.muted,
                        )
                        .text_size(px(11.))
                        .w(px(46.))
                        .flex_none(),
                    )
                    .child(
                        label(route, p.muted)
                            .w(px(if wide { 130. } else { 100. }))
                            .flex_none()
                            .truncate(),
                    )
                    .child(traffic)
                    .child(
                        label(duration(connection, now), p.muted)
                            .text_size(px(12.))
                            .w(px(68.))
                            .flex_none()
                            .text_right()
                            .whitespace_nowrap(),
                    )
                    .child(
                        svg()
                            .path(if expanded {
                                "icons/chevron-up.svg"
                            } else {
                                "icons/chevron-down.svg"
                            })
                            .size(px(14.))
                            .flex_none()
                            .text_color(rgb(p.muted)),
                    )
                    .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
                    .on_click(cx.listener(move |app, _, window, cx| {
                        app.act(
                            Action::Connections(ConnectionsAction::Expand(id.clone())),
                            window,
                            cx,
                        )
                    })),
            )
            .when(expanded, |d| {
                d.child(self.connection_details(connection, wide))
            })
    }

    pub(super) fn connections(&self, window: &Window, cx: &Context<Self>) -> Div {
        let p = self.p;
        let wide = window.viewport_size().width - self.sidebar_width() - px(48.) >= px(1000.);
        let connections = self.filtered_connections();
        let count = connections.len();
        let page = self
            .connections_view
            .page
            .min(count.saturating_sub(1) / PAGE_SIZE);
        let now = chrono::Utc::now().timestamp_millis();
        let protocol = row()
            .gap(px(2.))
            .p(px(2.))
            .rounded(px(9.))
            .border_1()
            .border_color(control_border(p))
            .flex_none()
            .children(
                [
                    ("connections-all", "全部", Protocol::All),
                    ("connections-tcp", "TCP", Protocol::Tcp),
                    ("connections-udp", "UDP", Protocol::Udp),
                ]
                .into_iter()
                .map(|(id, title, protocol)| {
                    self.settings_segment(
                        id,
                        title,
                        self.connections_view.protocol == protocol,
                        Action::Connections(ConnectionsAction::Protocol(protocol)),
                        cx,
                    )
                    .min_w(px(48.))
                    .px(px(8.))
                }),
            );
        let header = row()
            .gap(px(8.))
            .h(px(38.))
            .px(px(10.))
            .border_b_1()
            .border_color(surface_border(p))
            .child(
                label(if wide { "应用" } else { "应用 / 目标" }, p.muted)
                    .when(wide, |d| d.w(px(170.)).flex_none())
                    .when(!wide, |d| d.flex_1().min_w_0()),
            )
            .when(wide, |d| {
                d.child(label("目标地址", p.muted).flex_1().min_w_0())
            })
            .child(label("协议", p.muted).w(px(46.)).flex_none())
            .child(
                label("出站", p.muted)
                    .w(px(if wide { 130. } else { 100. }))
                    .flex_none(),
            )
            .child(if wide {
                row()
                    .gap(px(8.))
                    .w(px(160.))
                    .flex_none()
                    .child(label("累计下载", p.muted).w(px(76.)).text_right())
                    .child(label("累计上传", p.muted).w(px(76.)).text_right())
            } else {
                div()
                    .w(px(96.))
                    .flex_none()
                    .child(label("累计 ↓ / ↑", p.muted).text_right())
            })
            .child(label("持续", p.muted).w(px(68.)).flex_none().text_right())
            .child(div().w(px(14.)).flex_none());
        let table = column()
            .gap(px(0.))
            .min_w_0()
            .child(header)
            .children(
                connections
                    .iter()
                    .skip(page * PAGE_SIZE)
                    .take(PAGE_SIZE)
                    .map(|connection| self.connection_row(connection, wide, now, cx)),
            )
            .when(count == 0, |d| {
                d.child(self.empty(
                    if self.data.connections.is_empty() {
                        "当前没有连接"
                    } else {
                        "没有匹配的连接"
                    },
                    if self.data.connections.is_empty() {
                        "连接建立后会显示在这里，已结束连接的观测用量可在历史查看。"
                    } else {
                        "试试其他关键词、协议，或清除应用筛选。"
                    },
                ))
            });
        let filtered = self.connections_view.protocol != Protocol::All
            || !self.last_search.trim().is_empty()
            || self.activity.connection_group.is_some();
        column().gap(px(16.)).min_w_0()
            .child(self.connection_summary())
            .child(self.card().p(px(16.)).gap(px(12.))
                .child(row().gap(px(8.)).justify_between().flex_wrap()
                    .child(row().gap(px(8.)).child(heading("连接列表", p))
                        .child(label(if filtered { format!("{count} 条匹配") } else { format!("共 {count} 条") }, p.muted).text_size(px(12.))))
                    .when(self.activity.connection_group.is_some(), |d| {
                        let key = self.activity.connection_group.as_ref().unwrap();
                        d.child(self.button("clear-connection-group",
                            format!("{}  ×", self.application_name(key)), true, Action::Connections(ConnectionsAction::ClearGroup), cx)
                            .h(px(28.)).max_w(px(280.)).tooltip(|window, cx| Tooltip::new("清除应用筛选，查看全部应用连接").build(window, cx)))
                    }))
                .child(row().gap(px(10.)).flex_wrap()
                    .child(div().flex_1().min_w(px(200.)).child(self.text_input(&self.search)))
                    .child(protocol)
                    .child(row().gap(px(2.)).flex_none()
                        .child(self.taskbar_picker("connections-sort", self.connections_view.sort.label().into(),
                            [Sort::Total, Sort::Download, Sort::Upload, Sort::Duration].into_iter()
                                .map(|sort| (sort.label().into(), Action::Connections(ConnectionsAction::Sort(sort)), self.connections_view.sort == sort)).collect(), cx))
                        .child(self.button("connections-order", if self.connections_view.ascending { "↑" } else { "↓" }, false, Action::Connections(ConnectionsAction::Order), cx)
                            .w(px(28.)).px(px(0.)).tooltip(|window, cx| Tooltip::new("切换升序 / 降序").build(window, cx)))))
                .child(table)
                .child(self.pagination_footer(count, PAGE_SIZE, page, page, cx))
                .child(label("流量为连接建立以来的累计值，可能包含本程序启动前的用量；连接结束后从列表移除。", p.muted)
                    .text_size(px(11.)).whitespace_normal()))
    }
}
