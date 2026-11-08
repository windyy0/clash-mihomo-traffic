use super::*;
use crate::config::ProcessControl;

fn process_title_separator(p: Palette) -> Div {
    let mut line = div()
        .w(px(1.))
        .h(px(10.))
        .flex_none()
        .mt(px(4.))
        .bg(surface_border(p));
    line.style().align_self = Some(AlignItems::FlexStart);
    line
}

pub(super) struct ConnectionForm {
    pub(super) endpoint: Entity<InputState>,
    pub(super) secret: Entity<InputState>,
    pub(super) automatic: bool,
    pub(super) diagnostics_open: bool,
    process_draft: ProcessMode,
    process_maintain_draft: bool,
    process_draft_dirty: bool,
    pub(super) permanent_draft: Option<ProcessMode>,
    pub(super) permanent_draft_dirty: bool,
    pub(super) process_restore_notice_dismissed: bool,
    process_draft_scope: String,
    pub(super) error: String,
}

impl ConnectionForm {
    pub(super) fn new(config: &Config, window: &mut Window, cx: &mut Context<TrafficApp>) -> Self {
        let endpoint = input(
            if config.endpoint == "auto" {
                ""
            } else {
                &config.endpoint
            },
            "例如 http://127.0.0.1:9090",
            false,
            window,
            cx,
        );
        let secret = input(
            &config.secret,
            "客户端设置的连接密码，可留空",
            true,
            window,
            cx,
        );
        Self {
            endpoint,
            secret,
            automatic: config.endpoint == "auto",
            diagnostics_open: false,
            process_draft: ProcessMode::Strict,
            process_maintain_draft: false,
            process_draft_dirty: false,
            permanent_draft: None,
            permanent_draft_dirty: false,
            process_restore_notice_dismissed: false,
            process_draft_scope: String::new(),
            error: String::new(),
        }
    }
}

#[derive(Clone)]
pub(super) enum ConnectionAction {
    Save,
    Mode(bool),
    #[cfg(test)]
    ProcessControl(bool),
    ProcessDraft(ProcessMode),
    ProcessMaintainDraft(bool),
    ApplyProcessDraft,
    PermanentDraft(ProcessMode),
    CancelPermanentDraft,
    DismissRestoreNotice,
    Permanent(crate::permanent_control::Request),
    ChooseClientDirectory,
    OpenProcessLocation(bool),
    RetryProcessControl,
}

impl TrafficApp {
    pub(super) fn sync_connection_form(&mut self) {
        if self.connection_form.process_draft_scope != self.config.endpoint {
            self.connection_form.process_draft_scope = self.config.endpoint.clone();
            self.connection_form.process_draft_dirty = false;
            self.connection_form.permanent_draft_dirty = false;
            self.connection_form.process_restore_notice_dismissed = false;
        }
        if !self.connection_form.process_draft_dirty {
            if let Some(mode) = self
                .config
                .process_control
                .target()
                .and_then(ProcessMode::parse)
                .or_else(|| ProcessMode::parse(&self.data.process_mode))
            {
                self.connection_form.process_draft = mode;
            }
            self.connection_form.process_maintain_draft = self.config.process_control.maintaining();
        }
        self.sync_permanent_draft();
    }

    pub(super) fn reset_permanent_draft(&mut self) {
        self.connection_form.permanent_draft = match self.data.permanent.mode {
            crate::client_config::SavedMode::Explicit(mode) => Some(mode),
            _ => None,
        };
        self.connection_form.permanent_draft_dirty = false;
    }
    pub(super) fn sync_permanent_draft(&mut self) {
        if !self.connection_form.permanent_draft_dirty {
            self.reset_permanent_draft();
        } else {
            self.connection_form.permanent_draft_dirty =
                self.connection_form.permanent_draft.is_some_and(|mode| {
                    self.data.permanent.mode != crate::client_config::SavedMode::Explicit(mode)
                });
        }
    }

    pub(super) fn handle_connection_action(
        &mut self,
        action: ConnectionAction,
        cx: &mut Context<Self>,
    ) {
        match action {
            ConnectionAction::Save => {
                let mut config = self.config.clone();
                config.endpoint = if self.connection_form.automatic {
                    "auto".into()
                } else {
                    self.connection_form.endpoint.read(cx).value().trim().into()
                };
                config.secret = self.connection_form.secret.read(cx).value().to_string();
                config.locked_secret = None;
                if let Err(error) = config.validate() {
                    self.connection_form.error = if config.endpoint.is_empty() {
                        "请填写客户端的控制地址".into()
                    } else {
                        format!("控制地址无效：{error}")
                    };
                } else {
                    self.connection_form.error.clear();
                    if config.endpoint == self.config.endpoint
                        && config.secret == self.config.secret
                        && config.locked_secret == self.config.locked_secret
                    {
                        self.backend.send(Command::Reconnect);
                    } else {
                        self.config = config;
                        self.save_config();
                    }
                }
            }
            ConnectionAction::Mode(automatic) => {
                self.connection_form.automatic = automatic;
                self.connection_form.error.clear();
                if automatic && self.config.endpoint != "auto" {
                    self.config.endpoint = "auto".into();
                    self.save_config();
                    self.connection_form.automatic = self.config.endpoint == "auto";
                }
            }
            #[cfg(test)]
            ConnectionAction::ProcessControl(enabled) => {
                if let Err(error) = self.backend.set_process_control(enabled) {
                    self.backend
                        .notice(format!("进程识别设置保存失败：{error}"));
                }
                self.config = self.backend.config_snapshot();
            }
            ConnectionAction::ProcessDraft(mode) => {
                self.connection_form.process_draft = mode;
                self.connection_form.process_draft_dirty = true;
            }
            ConnectionAction::ProcessMaintainDraft(maintain) => {
                self.connection_form.process_maintain_draft = maintain;
                self.connection_form.process_draft_dirty = true;
            }
            ConnectionAction::ApplyProcessDraft => {
                self.backend
                    .permanent_control
                    .post(&self.backend, crate::permanent_control::Request::Cancel);
                let policy = if self.connection_form.process_maintain_draft {
                    ProcessControl::Maintain(self.connection_form.process_draft)
                } else {
                    ProcessControl::ApplyOnce(self.connection_form.process_draft)
                };
                if let Err(error) = self.backend.apply_process_mode(policy) {
                    let mut shared = self.backend.shared.lock().unwrap();
                    shared.process_error = format!("设置保存失败：{error:#}");
                    shared.process_status = ProcessControlStatus::Failed;
                } else {
                    self.backend
                        .permanent_control
                        .paused
                        .store(false, std::sync::atomic::Ordering::Release);
                    self.connection_form.process_draft_dirty = false;
                }
            }
            ConnectionAction::PermanentDraft(mode) => {
                self.connection_form.permanent_draft = Some(mode);
                self.connection_form.permanent_draft_dirty = true;
                self.sync_permanent_draft();
            }
            ConnectionAction::CancelPermanentDraft => self.reset_permanent_draft(),
            ConnectionAction::DismissRestoreNotice => {
                self.connection_form.process_restore_notice_dismissed = true
            }
            ConnectionAction::Permanent(request) => {
                if matches!(request, crate::permanent_control::Request::Restore) {
                    self.reset_permanent_draft();
                    self.connection_form.process_restore_notice_dismissed = false;
                }
                self.backend.permanent_control.post(&self.backend, request);
            }
            ConnectionAction::ChooseClientDirectory => {
                let receiver = cx.prompt_for_paths(PathPromptOptions {
                    files: false,
                    directories: true,
                    multiple: false,
                    prompt: Some("选择当前客户端的配置目录".into()),
                });
                let expected_endpoint = self.config.endpoint.clone();
                cx.spawn(async move |this, cx| {
                    if let Ok(Ok(Some(paths))) = receiver.await
                        && let Some(path) = paths.into_iter().next()
                    {
                        let _ = this.update(cx, |app, _| {
                            if app.config.endpoint == expected_endpoint {
                                app.backend.permanent_control.post(
                                    &app.backend,
                                    crate::permanent_control::Request::SelectDirectory(path),
                                );
                            }
                        });
                    }
                })
                .detach();
            }
            ConnectionAction::OpenProcessLocation(backup) => {
                let location = if backup {
                    &self.data.permanent.backup
                } else {
                    &self.data.permanent.source
                };
                if !location.is_empty() {
                    let path = std::path::Path::new(location);
                    let directory = if backup { Some(path) } else { path.parent() };
                    if let Some(directory) = directory {
                        self.open_directory(directory.to_path_buf());
                    } else {
                        self.backend.notice("保存目录不存在，请刷新状态后重试");
                    }
                }
            }
            ConnectionAction::RetryProcessControl => self.backend.retry_process_control(),
        }
    }

    fn process_help(&self) -> impl IntoElement {
        let p = self.p;
        let mode_row = move |mode: &'static str, explanation: &'static str| {
            row()
                .gap(px(10.))
                .items_start()
                .child(
                    div()
                        .w(px(44.))
                        .flex_none()
                        .py(px(3.))
                        .rounded(px(6.))
                        .bg(state_surface(p))
                        .text_center()
                        .child(
                            label(mode, p.accent_text)
                                .text_size(px(12.))
                                .line_height(px(18.))
                                .font_weight(FontWeight::MEDIUM),
                        ),
                )
                .child(
                    label(explanation, p.muted)
                        .text_size(px(12.))
                        .line_height(px(18.))
                        .py(px(3.))
                        .flex_1()
                        .min_w_0()
                        .whitespace_normal(),
                )
        };
        Popover::new("process-help-popover")
            .no_style()
            .trigger(InfoHelpTrigger {
                id: "process-help",
                palette: p,
                selected: false,
            })
            .content(move |window, cx| {
                cx.new(|cx| {
                    PopoverContent::new(window, cx, move |_, cx| {
                        column()
                            .id("process-help-card")
                            .gap(px(12.))
                            .p(px(14.))
                            .w(px(360.))
                            .bg(popup_surface(p))
                            .border_1()
                            .border_color(card_border(p))
                            .rounded(px(10.))
                            .shadow_sm()
                            .child(
                                column()
                                    .gap(px(4.))
                                    .child(
                                        label("让流量更准确地归属应用", p.text)
                                            .font_weight(FontWeight::SEMIBOLD),
                                    )
                                    .child(
                                        label(
                                            "帮助减少“未知进程”，仍可能有连接无法识别。",
                                            p.muted,
                                        )
                                        .text_size(px(12.))
                                        .whitespace_normal(),
                                    ),
                            )
                            .child(
                                column()
                                    .gap(px(6.))
                                    .child(mode_row("按需", "匹配进程规则时查询。"))
                                    .child(mode_row("始终", "主动查询，会增加进程查询开销。"))
                                    .child(mode_row("关闭", "停止查询，影响进程规则与应用统计。")),
                            )
                            .child(settings_separator(p))
                            .child(
                                column()
                                    .gap(px(10.))
                                    .child(
                                        column()
                                            .gap(px(3.))
                                            .child(
                                                label("运行设置 · 当前内核", p.text)
                                                    .text_size(px(12.))
                                                    .font_weight(FontWeight::SEMIBOLD),
                                            )
                                            .child(
                                                label(
                                                    "客户端重载可能覆盖；持续保持每分钟检查。",
                                                    p.muted,
                                                )
                                                .text_size(px(12.))
                                                .whitespace_normal(),
                                            )
                                            .child(
                                                label(
                                                    "退出本程序后停止保持，内核保留最后状态。",
                                                    p.muted,
                                                )
                                                .text_size(px(12.))
                                                .whitespace_normal(),
                                            ),
                                    )
                                    .child(
                                        column()
                                            .gap(px(3.))
                                            .child(
                                                label("永久设置 · 所有订阅", p.text)
                                                    .text_size(px(12.))
                                                    .font_weight(FontWeight::SEMIBOLD),
                                            )
                                            .child(
                                                label(
                                                    "重启保留；保存前备份，保存后停止持续保持。",
                                                    p.muted,
                                                )
                                                .text_size(px(12.))
                                                .whitespace_normal(),
                                            )
                                            .child(
                                                label("仍可能被扩展或脚本覆盖。", p.muted)
                                                    .text_size(px(12.))
                                                    .whitespace_normal(),
                                            ),
                                    ),
                            )
                            .child(
                                div().p(px(8.)).rounded(px(6.)).bg(state_surface(p)).child(
                                    label("仅影响后续连接，不补算历史流量。", p.muted)
                                        .text_size(px(12.))
                                        .whitespace_normal(),
                                ),
                            )
                            .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(DismissEvent)))
                            .into_any_element()
                    })
                    .p_0()
                })
            })
    }

    fn process_mode_picker(&self, permanent: bool, cx: &Context<Self>) -> Div {
        let selected = if permanent {
            self.connection_form.permanent_draft
        } else {
            Some(self.connection_form.process_draft)
        };
        row()
            .gap(px(2.))
            .p(px(2.))
            .rounded(px(9.))
            .border_1()
            .border_color(control_border(self.p))
            .flex_none()
            .children(ProcessMode::ALL.into_iter().map(|mode| {
                let id = match (permanent, mode) {
                    (false, ProcessMode::Strict) => "runtime-mode-strict",
                    (false, ProcessMode::Always) => "runtime-mode-always",
                    (false, ProcessMode::Off) => "runtime-mode-off",
                    (true, ProcessMode::Strict) => "permanent-mode-strict",
                    (true, ProcessMode::Always) => "permanent-mode-always",
                    (true, ProcessMode::Off) => "permanent-mode-off",
                };
                self.settings_segment(
                    id,
                    mode.label(),
                    selected == Some(mode),
                    if permanent {
                        Action::Connection(ConnectionAction::PermanentDraft(mode))
                    } else {
                        Action::Connection(ConnectionAction::ProcessDraft(mode))
                    },
                    cx,
                )
                .min_w(px(60.))
            }))
    }

    fn process_settings(&self, cx: &Context<Self>) -> Div {
        use crate::{
            client_config::SavedMode,
            permanent_control::{Request, Status},
        };
        let p = self.p;
        let maintain_help = "点击应用后，每分钟检查并恢复所选模式；关闭后点击应用则仅应用一次。退出本程序后停止保持";
        let state = &self.data.permanent;
        let actual_mode = ProcessMode::parse(&self.data.process_mode);
        let actual =
            actual_mode
                .map(ProcessMode::label)
                .unwrap_or(if self.data.process_mode.is_empty() {
                    "未获取"
                } else {
                    "未知"
                });
        let actual_color = if actual_mode.is_some() {
            p.accent_text
        } else {
            p.amber
        };
        let runtime_status = match self.data.process_status {
            ProcessControlStatus::Idle => None,
            ProcessControlStatus::Checking => Some("检查中"),
            ProcessControlStatus::Applying => Some("应用中"),
            ProcessControlStatus::Applied => Some("已生效"),
            ProcessControlStatus::Waiting
                if self.config.process_control != ProcessControl::Unmanaged =>
            {
                Some("待连接后应用")
            }
            ProcessControlStatus::Waiting => None,
            ProcessControlStatus::Failed => None,
        };
        let busy = matches!(state.status, Status::Saving | Status::WaitingExit);
        let saved = match &state.mode {
            SavedMode::Explicit(mode) => format!("已保存：{}", mode.label()),
            SavedMode::Inherited => "继承客户端配置".into(),
            SavedMode::Unknown(_) => "保存值未知".into(),
            SavedMode::Unavailable => "未获取保存值".into(),
        };
        let different = matches!(state.mode, SavedMode::Explicit(mode) if actual_mode.is_some_and(|actual| actual != mode));
        let permanent_status = if state.error.is_empty() {
            match state.status {
                Status::Idle => None,
                Status::WaitingExit => Some("等待退出"),
                Status::Saving if state.restoring => Some("恢复中…"),
                Status::Saving => Some("保存中…"),
                Status::SavedPending if !self.data.connected => Some("已保存 · 待连接"),
                Status::SavedPending => Some("等待内核确认"),
                Status::Saved => Some("已保存"),
                Status::Applied if different || !self.data.connected || actual_mode.is_none() => {
                    Some("已保存")
                }
                Status::Applied => Some("已生效"),
                Status::Restored if matches!(state.mode, SavedMode::Inherited) => {
                    Some("已恢复继承")
                }
                Status::Restored => None,
                Status::Failed => Some("保存失败"),
            }
        } else {
            None
        };
        let runtime = column()
            .gap(px(8.))
            .min_w_0()
            .child(
                row()
                    .justify_between()
                    .items_start()
                    .gap(px(8.))
                    .flex_wrap()
                    .child(
                        row()
                            .gap(px(8.))
                            .items_start()
                            .pt(px(6.))
                            .flex_1()
                            .min_w_0()
                            .child(
                                label("运行设置", p.text)
                                    .line_height(px(18.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .flex_none()
                                    .whitespace_nowrap(),
                            )
                            .child(process_title_separator(p))
                            .child(
                                label("修改当前 mihomo 内核状态，可能被客户端重载覆盖。", p.muted)
                                    .text_size(px(12.))
                                    .line_height(px(18.))
                                    .flex_1()
                                    .min_w_0()
                                    .whitespace_normal(),
                            ),
                    )
                    .child(
                        row()
                            .gap(px(8.))
                            .flex_none()
                            .child(label("持续保持", p.muted).text_size(px(12.)))
                            .child(self.switch_control_with_tooltip(
                                "process-maintain",
                                maintain_help,
                                self.connection_form.process_maintain_draft,
                                Action::Connection(ConnectionAction::ProcessMaintainDraft(
                                    !self.connection_form.process_maintain_draft,
                                )),
                                cx,
                            )),
                    ),
            )
            .child(
                row()
                    .justify_between()
                    .gap(px(8.))
                    .flex_wrap()
                    .child(self.process_mode_picker(false, cx))
                    .child(
                        row()
                            .gap(px(8.))
                            .flex_wrap()
                            .when(self.connection_form.process_draft_dirty, |d| {
                                d.child(label("待应用", p.amber).text_size(px(12.)))
                            })
                            .when(!self.connection_form.process_draft_dirty, |d| {
                                d.children(runtime_status.map(|text| {
                                    label(
                                        text,
                                        if self.data.process_status == ProcessControlStatus::Applied
                                        {
                                            p.upload_text
                                        } else {
                                            p.muted
                                        },
                                    )
                                    .text_size(px(12.))
                                }))
                            })
                            .when(
                                self.data.process_status == ProcessControlStatus::Failed,
                                |d| {
                                    d.child(self.button(
                                        "process-retry",
                                        "重试",
                                        false,
                                        Action::Connection(ConnectionAction::RetryProcessControl),
                                        cx,
                                    ))
                                },
                            )
                            .child(self.button(
                                "runtime-process-apply",
                                "应用到内核",
                                true,
                                Action::Connection(ConnectionAction::ApplyProcessDraft),
                                cx,
                            )),
                    ),
            )
            .when(!self.data.process_error.is_empty(), |d| {
                d.child(
                    label(self.data.process_error.clone(), p.amber)
                        .min_w_0()
                        .whitespace_normal(),
                )
            });
        let permanent = column()
            .gap(px(8.))
            .min_w_0()
            .child(
                row()
                    .justify_between()
                    .items_start()
                    .gap(px(8.))
                    .flex_wrap()
                    .child(
                        row()
                            .gap(px(8.))
                            .items_start()
                            .flex_1()
                            .min_w_0()
                            .child(
                                label("永久设置", p.text)
                                    .line_height(px(18.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .flex_none()
                                    .whitespace_nowrap(),
                            )
                            .child(process_title_separator(p))
                            .child(
                                label(
                                    "修改客户端全局进程识别配置，作用于所有订阅，重启后保留。",
                                    p.muted,
                                )
                                .text_size(px(12.))
                                .line_height(px(18.))
                                .flex_1()
                                .min_w_0()
                                .whitespace_normal(),
                            ),
                    )
                    .child(
                        label(
                            if state.client.is_empty() {
                                "尚未识别客户端".into()
                            } else {
                                state.client.clone()
                            },
                            p.muted,
                        )
                        .text_size(px(12.))
                        .flex_none(),
                    ),
            )
            .child(
                row()
                    .justify_between()
                    .gap(px(8.))
                    .flex_wrap()
                    .child(self.process_mode_picker(true, cx))
                    .child(
                        row()
                            .gap(px(8.))
                            .flex_wrap()
                            .when(self.connection_form.permanent_draft_dirty && !busy, |d| {
                                d.child(label("待保存", p.amber).text_size(px(12.))).child(
                                    self.button(
                                        "permanent-process-discard",
                                        "取消修改",
                                        false,
                                        Action::Connection(ConnectionAction::CancelPermanentDraft),
                                        cx,
                                    ),
                                )
                            })
                            .children(
                                permanent_status
                                    .filter(|_| !self.connection_form.permanent_draft_dirty || busy)
                                    .map(|text| {
                                        label(
                                            text,
                                            if state.status == Status::Applied {
                                                p.upload_text
                                            } else {
                                                p.muted
                                            },
                                        )
                                        .text_size(px(12.))
                                    }),
                            )
                            .child(
                                label(
                                    saved,
                                    if matches!(state.mode, SavedMode::Unknown(_)) {
                                        p.amber
                                    } else {
                                        p.muted
                                    },
                                )
                                .text_size(px(12.)),
                            )
                            .when(state.available && !busy, |d| {
                                if let Some(mode) = self.connection_form.permanent_draft {
                                    d.child(self.button(
                                        "permanent-process-save",
                                        if state.error.is_empty() {
                                            "保存并应用"
                                        } else {
                                            "重试保存"
                                        },
                                        true,
                                        Action::Connection(ConnectionAction::Permanent(Request::Save(mode))),
                                        cx,
                                    ))
                                } else {
                                    d.child(
                                        div()
                                            .id("permanent-process-save-disabled")
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .px(px(12.))
                                            .h(px(34.))
                                            .rounded(px(9.))
                                            .text_size(px(13.))
                                            .text_color(rgb(p.muted))
                                            .bg(state_surface(p))
                                            .opacity(0.5)
                                            .child("保存并应用")
                                            .tooltip(|window, cx| {
                                                Tooltip::new("请选择要保存的模式").build(window, cx)
                                            }),
                                    )
                                }
                            })
                            .when(!state.available && !busy, |d| {
                                d.child(label("暂不可用", p.muted).text_size(px(12.)))
                            })
                            .when(state.status == Status::WaitingExit, |d| {
                                d.child(self.button(
                                    "permanent-process-cancel",
                                    "取消等待",
                                    false,
                                    Action::Connection(ConnectionAction::Permanent(Request::Cancel)),
                                    cx,
                                ))
                            })
                            .when(state.launch_available, |d| {
                                d.child(self.button(
                                    "permanent-process-launch",
                                    "启动客户端",
                                    false,
                                    Action::Connection(ConnectionAction::Permanent(Request::Launch)),
                                    cx,
                                ))
                            }),
                    ),
            )
            .when(!state.source.is_empty(), |d| {
                let file = std::path::Path::new(&state.source)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| state.source.clone());
                let location = if state.note.is_empty() {
                    state.source.clone()
                } else {
                    format!("{}\n{}", state.source, state.note)
                };
                d.child(
                    row()
                        .gap(px(8.))
                        .min_w_0()
                        .child(label("保存位置", p.muted).text_size(px(12.)).flex_none())
                        .child(
                            div()
                                .id("process-config-source")
                                .flex_1()
                                .min_w_0()
                                .child(label(file, p.muted).text_size(px(12.)).truncate())
                                .tooltip(move |window, cx| {
                                    let location = location.clone();
                                    Tooltip::element(move |_, _| {
                                        label(location.clone(), p.text)
                                            .w(px(420.))
                                            .whitespace_normal()
                                    })
                                    .bg(popup_surface(p))
                                    .border_color(card_border(p))
                                    .build(window, cx)
                                }),
                        )
                        .child(
                            self.button(
                                "open-process-config-dir",
                                "打开目录",
                                false,
                                Action::Connection(ConnectionAction::OpenProcessLocation(false)),
                                cx,
                            )
                            .h(px(28.))
                            .flex_none(),
                        )
                        .when(!busy, |d| {
                            d.child(
                                self.button(
                                    "permanent-process-refresh",
                                    if state.error.is_empty() {
                                        "刷新"
                                    } else {
                                        "重试读取"
                                    },
                                    false,
                                    Action::Connection(ConnectionAction::Permanent(Request::Refresh)),
                                    cx,
                                )
                                .h(px(28.))
                                .flex_none(),
                            )
                        }),
                )
            })
            .when(
                !busy && (state.source.is_empty() || !state.available || !state.backup.is_empty() || matches!(state.mode, SavedMode::Explicit(_)) || state.restoring && !state.error.is_empty()),
                |d| {
                    d.child(
                        row()
                            .gap(px(8.))
                            .flex_wrap()
                            .when(state.source.is_empty(), |d| {
                                d.child(
                                    self.button(
                                        "permanent-process-refresh",
                                        if state.error.is_empty() {
                                            "刷新状态"
                                        } else {
                                            "重试读取"
                                        },
                                        false,
                                        Action::Connection(ConnectionAction::Permanent(Request::Refresh)),
                                        cx,
                                    )
                                    .h(px(28.)),
                                )
                            })
                            .when(
                                !state.available
                                    && self.data.connected
                                    && crate::client_config::Kind::from_name(
                                        &self.data.client_name,
                                    )
                                    .is_some(),
                                |d| {
                                    d.child(
                                        self.button(
                                            "choose-client-config-dir",
                                            "选择配置目录",
                                            false,
                                            Action::Connection(ConnectionAction::ChooseClientDirectory),
                                            cx,
                                        )
                                        .h(px(28.)),
                                    )
                                },
                            )
                            .when(!state.backup.is_empty(), |d| {
                                let backup = state.backup.clone();
                                d.child(
                                    self.button(
                                        "open-process-config-backup",
                                        "查看本次备份",
                                        false,
                                        Action::Connection(ConnectionAction::OpenProcessLocation(true)),
                                        cx,
                                    )
                                    .h(px(28.))
                                    .tooltip(
                                        move |window, cx| {
                                            let backup = backup.clone();
                                            Tooltip::element(move |_, _| {
                                                label(backup.clone(), p.text)
                                                    .w(px(420.))
                                                    .whitespace_normal()
                                            })
                                            .bg(popup_surface(p))
                                            .border_color(card_border(p))
                                            .build(window, cx)
                                        },
                                    ),
                                )
                            })
                            .when(state.available && (matches!(state.mode, SavedMode::Explicit(_)) || state.restoring && !state.error.is_empty()), |d| {
                                d.child(self.button("restore-client-process-mode", if state.restoring && !state.error.is_empty() { "重试恢复" } else { "恢复继承配置" }, false, Action::Connection(ConnectionAction::Permanent(Request::Restore)), cx)
                                    .h(px(28.))
                                    .tooltip(|window, cx| Tooltip::new("备份并清除全局进程模式覆写，恢复由客户端决定模式；客户端重载后生效。").build(window, cx)))
                            }),
                    )
                },
            )
            .when(different, |d| {
                d.child(
                    label(
                        "保存值与当前模式不同，可能尚未加载，或被其他设置覆盖。",
                        p.amber,
                    )
                    .text_size(px(12.))
                    .whitespace_normal(),
                )
            })
            .when(matches!(state.mode, SavedMode::Unknown(_)), |d| {
                d.child(
                    label(state.mode.label(), p.amber)
                        .text_size(px(12.))
                        .whitespace_normal(),
                )
            })
            .when(!state.available && state.error.is_empty(), |d| {
                d.child(
                    label("连接受支持的客户端后，自动读取永久设置。", p.muted)
                        .text_size(px(12.))
                        .whitespace_normal(),
                )
            })
            .when(state.status == Status::WaitingExit, |d| {
                d.child(
                    label(
                        "客户端会缓存设置，请先从客户端菜单正常退出，退出后自动写入设置。",
                        p.amber,
                    )
                    .whitespace_normal(),
                )
            })
            .when(state.status == Status::Restored && matches!(state.mode, SavedMode::Inherited) && state.error.is_empty() && !self.connection_form.process_restore_notice_dismissed, |d| {
                d.child(row().gap(px(8.)).min_w_0()
                    .child(label("已恢复继承。客户端重新加载配置或重启后按其配置生效。", p.muted).text_size(px(12.)).flex_1().min_w_0().whitespace_normal())
                    .child(Button::new("dismiss-restore-notice")
                        .icon(Icon::default().path("icons/window-close.svg").size(px(12.)).text_color(rgb(p.muted)))
                        .w(px(24.)).h(px(24.)).p_0().flex_none()
                        .custom(ButtonCustomVariant::new(cx).color(rgba(0).into()).foreground(rgb(p.muted).into()).border(rgba(0).into()).hover(state_surface(p).into()).active(state_surface(p).into()))
                        .on_click(cx.listener(|app, _, window, cx| app.act(Action::Connection(ConnectionAction::DismissRestoreNotice), window, cx)))))
            })
            .when(!state.error.is_empty(), |d| {
                d.child(
                    label(state.error.clone(), p.amber)
                        .min_w_0()
                        .whitespace_normal(),
                )
            });
        let actual_value = if self.data.process_mode.is_empty() {
            "未获取".into()
        } else {
            self.data.process_mode.clone()
        };
        column()
            .gap(px(12.))
            .min_w_0()
            .child(
                row()
                    .justify_between()
                    .gap(px(8.))
                    .flex_wrap()
                    .child(
                        row()
                            .gap(px(6.))
                            .child(heading("进程识别", p))
                            .child(self.process_help()),
                    )
                    .child(
                        div()
                            .id("process-actual-mode")
                            .child(
                                label(format!("当前模式：{actual}"), actual_color)
                                    .font_weight(FontWeight::MEDIUM)
                                    .px(px(10.))
                                    .py(px(5.))
                                    .rounded(px(8.))
                                    .bg(state_surface(p)),
                            )
                            .tooltip(move |window, cx| {
                                Tooltip::new(format!("内核实际值：{actual_value}"))
                                    .build(window, cx)
                            }),
                    ),
            )
            .child(runtime)
            .child(settings_separator(p))
            .child(permanent)
    }

    pub(super) fn connection_settings(&self, cx: &Context<Self>) -> Div {
        let p = self.p;
        let connected = self.data.connected;
        let client = if self.data.client_name.is_empty() {
            if connected {
                "未识别客户端"
            } else {
                "等待连接"
            }
        } else {
            &self.data.client_name
        };
        let method = if self.data.endpoint.starts_with(r"\\.\pipe\") {
            "本机直连"
        } else {
            "本地网络连接"
        };
        let version = if self.data.core_version.is_empty() {
            "版本未获取"
        } else {
            &self.data.core_version
        };
        let mode_picker = row()
            .gap(px(2.))
            .p(px(3.))
            .rounded(px(10.))
            .border_1()
            .border_color(surface_border(p))
            .child(self.settings_segment(
                "connection-auto",
                "自动连接",
                self.connection_form.automatic,
                Action::Connection(ConnectionAction::Mode(true)),
                cx,
            ))
            .child(self.settings_segment(
                "connection-manual",
                "手动填写",
                !self.connection_form.automatic,
                Action::Connection(ConnectionAction::Mode(false)),
                cx,
            ));

        self.card()
            .gap(px(12.))
            .child(
                row()
                    .gap(px(8.))
                    .child(div().flex_1().min_w_0().child(heading("mihomo连接", p)))
                    .child(
                        self.button("reconnect", "重新连接", false, Action::Reconnect, cx)
                            .flex_none(),
                    )
                    .child(
                        label(
                            if connected {
                                "● 已连接"
                            } else {
                                "● 未连接"
                            },
                            if connected { p.upload_text } else { p.amber },
                        )
                        .flex_none(),
                    ),
            )
            .child(
                label(
                    if connected {
                        format!("{client}  ·  {method}  ·  {version}")
                    } else {
                        client.into()
                    },
                    p.muted,
                )
                .whitespace_normal(),
            )
            .when(!connected, |d| {
                d.child(label(self.data.status.clone(), p.amber).whitespace_normal())
            })
            .child(settings_separator(p))
            .child(
                row()
                    .justify_between()
                    .gap(px(8.))
                    .child(heading("连接设置", p))
                    .child(mode_picker.flex_none()),
            )
            .when(!self.connection_form.automatic, |d| {
                d.child(
                    row()
                        .gap(px(12.))
                        .child(label("控制地址", p.text).w(px(72.)).flex_none())
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(self.text_input(&self.connection_form.endpoint)),
                        ),
                )
                .child(
                    row()
                        .gap(px(12.))
                        .child(label("连接密码", p.text).w(px(72.)).flex_none())
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(self.text_input(&self.connection_form.secret)),
                        ),
                )
            })
            .when(!self.connection_form.error.is_empty(), |d| {
                d.child(label(self.connection_form.error.clone(), p.amber).whitespace_normal())
            })
            .when(self.config.locked_secret.is_some(), |d| {
                d.child(
                    label(
                        "无法读取已保存的连接密码，请在手动连接中重新输入并应用。",
                        p.amber,
                    )
                    .whitespace_normal(),
                )
            })
            .when(!self.connection_form.automatic, |d| {
                d.child(
                    row()
                        .justify_between()
                        .gap(px(8.))
                        .child(
                            label("密码为 API Secret，可留空", p.muted)
                                .flex_1()
                                .min_w_0()
                                .whitespace_normal(),
                        )
                        .child(
                            self.button(
                                "apply-connection",
                                "应用并连接",
                                true,
                                Action::Connection(ConnectionAction::Save),
                                cx,
                            )
                            .flex_none(),
                        ),
                )
            })
            .child(settings_separator(p))
            .child(self.process_settings(cx))
            .child(settings_separator(p))
            .child(self.settings_disclosure(
                "connection-diagnostics",
                "诊断详情",
                self.connection_form.diagnostics_open,
                SettingsFold::Diagnostics,
                cx,
            ))
            .when(self.connection_form.diagnostics_open, |d| {
                d.child(
                    column()
                        .gap(px(10.))
                        .child(settings_info(
                            "实际连接地址",
                            if self.data.endpoint.is_empty() {
                                "未连接".into()
                            } else {
                                self.data.endpoint.clone()
                            },
                            p,
                        ))
                        .child(
                            label(
                                if self.data.core_verified {
                                    "已确认内核进程，可在连接中断后恢复期间总流量。"
                                } else {
                                    "暂未确认内核进程，重新连接后会从新的起点继续统计。"
                                },
                                p.muted,
                            )
                            .whitespace_normal(),
                        )
                        .child(
                            label(format!("采集状态：{}", self.data.status), p.muted)
                                .whitespace_normal(),
                        ),
                )
            })
    }
}
