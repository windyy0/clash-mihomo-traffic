use super::*;

fn about_fact(title: &str, summary: &str, detail: &str, p: Palette) -> Div {
    row()
        .items_start()
        .gap(px(16.))
        .py(px(14.))
        .child(label(title.to_string(), p.muted).w(px(72.)).flex_none())
        .child(
            column()
                .flex_1()
                .min_w_0()
                .gap(px(5.))
                .child(
                    label(summary.to_string(), p.text)
                        .font_weight(FontWeight::SEMIBOLD)
                        .whitespace_normal(),
                )
                .child(
                    label(detail.to_string(), p.muted)
                        .text_size(px(12.))
                        .whitespace_normal(),
                ),
        )
}

#[derive(Clone)]
pub(super) enum BackupAction {
    Backup,
    ChooseBackupDirectory,
    ResetBackupDirectory,
    OpenBackupDirectory,
    RestoreArm,
    ChooseRestoreFile,
    Restore,
}

impl TrafficApp {
    pub(super) fn handle_backup_action(
        &mut self,
        action: BackupAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            BackupAction::Backup => {
                if !self.backup_directory_busy {
                    self.backend.send(Command::Backup);
                }
            }
            BackupAction::ChooseBackupDirectory => self.choose_backup_directory(cx),
            BackupAction::ResetBackupDirectory => self.set_backup_directory(None, cx),
            BackupAction::OpenBackupDirectory => {
                if !self.backup_directory_busy {
                    let config = self.config.clone();
                    self.open_directory_with(move || crate::backup::ensure_directory(&config));
                }
            }
            BackupAction::RestoreArm => self.restore_armed = !self.restore_armed,
            BackupAction::ChooseRestoreFile => self.choose_restore_file(window, cx),
            BackupAction::Restore => {
                let path = self.restore_path.read(cx).value().trim().to_string();
                if self.can_restore_backup(cx) {
                    self.backend.send(Command::Restore(path.into()));
                    self.restore_armed = false;
                }
            }
        }
    }

    pub(super) fn can_restore_backup(&self, cx: &App) -> bool {
        self.page == Page::Settings
            && self.settings_section == SettingsSection::Backup
            && self.restore_armed
            && !self.restore_path.read(cx).value().trim().is_empty()
            && !self.migration_busy
            && !self.backup_directory_busy
            && self.config.backup_directory().is_ok()
    }

    pub(super) fn choose_backup_directory(&mut self, cx: &mut Context<Self>) {
        if self.backup_directory_busy {
            return;
        }
        self.backup_directory_error.clear();
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("选择备份保存目录".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = receiver.await
                && let Some(path) = paths.into_iter().next()
            {
                let _ = this.update(cx, |app, cx| app.set_backup_directory(Some(path), cx));
            }
        })
        .detach();
    }

    pub(super) fn set_backup_directory(
        &mut self,
        selected: Option<std::path::PathBuf>,
        cx: &mut Context<Self>,
    ) {
        if self.backup_directory_busy {
            return;
        }
        self.backup_directory_error.clear();
        if selected.is_none() && self.config.backup_dir.is_none() {
            return;
        }
        self.backup_directory_busy = true;
        self.restore_armed = false;
        let backend = self.backend.clone();
        let task = cx
            .background_executor()
            .spawn(async move { backend.set_backup_directory(selected) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |app, cx| {
                app.backup_directory_busy = false;
                app.config = app.backend.config_snapshot();
                if let Err(error) = result {
                    app.backup_directory_error = format!("无法更改备份目录：{error:#}");
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn choose_restore_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("选择流量账本备份文件".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = receiver.await
                && let Some(path) = paths.into_iter().next()
            {
                let _ = this.update_in(cx, |app, window, cx| {
                    app.restore_armed = false;
                    app.restore_path.update(cx, |input, cx| {
                        input.set_value(path.display().to_string(), window, cx)
                    });
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub(super) fn backup_settings(&self, cx: &Context<Self>) -> Div {
        let p = self.p;
        let has_path = !self.restore_path.read(cx).value().trim().is_empty();
        let directory = self.config.backup_directory();
        let directory_available = directory.is_ok();
        let directory = match directory {
            Ok(path) => path.display().to_string(),
            Err(error) => format!("无法获取备份目录：{error:#}"),
        };
        let busy = self.backup_directory_busy;
        column()
            .gap(px(16.))
            .child(
                self.card()
                    .gap(px(16.))
                    .child(
                        row()
                            .items_start()
                            .justify_between()
                            .gap(px(16.))
                            .child(
                                column()
                                    .flex_1()
                                    .min_w_0()
                                    .gap(px(5.))
                                    .child(heading("创建备份", p))
                                    .child(
                                        label("保存当前流量账本，不包含设置和导出文件。", p.muted)
                                            .whitespace_normal(),
                                    ),
                            )
                            .child(self.button("backup", "立即备份", true, Action::Backup(BackupAction::Backup), cx).opacity(if busy || !directory_available { 0.45 } else { 1. })),
                    )
                    .child(
                        column()
                            .gap(px(8.))
                            .p(px(12.))
                            .rounded(px(8.))
                            .border_1()
                            .border_color(surface_border(p))
                            .child(
                                row()
                                    .justify_between()
                                    .gap(px(12.))
                                    .child(label("备份目录", p.muted))
                                    .child(row().gap(px(6.)).flex_wrap().justify_end()
                                        .child(self.button("open-backups", "打开目录", false, Action::Backup(BackupAction::OpenBackupDirectory), cx).opacity(if busy || !directory_available { 0.45 } else { 1. }))
                                        .child(self.button("choose-backup-directory", "更改目录", false, Action::Backup(BackupAction::ChooseBackupDirectory), cx)
                                            .opacity(if busy { 0.45 } else { 1. })
                                            .tooltip(|window, cx| Tooltip::new("建议选择其他磁盘，将备份与原始数据分开保存。").build(window, cx)))
                                        .child(self.button("reset-backup-directory", "恢复默认", false, Action::Backup(BackupAction::ResetBackupDirectory), cx)
                                            .opacity(if busy || self.config.backup_dir.is_none() { 0.45 } else { 1. }))),
                            )
                            .child(
                                label(directory, if directory_available { p.text } else { p.amber })
                                    .text_size(px(12.))
                                    .whitespace_normal(),
                            )
                            .child(label("更改仅影响新备份，已有文件保留在原目录。", p.muted).text_size(px(12.)).whitespace_normal())
                            .when(busy, |d| d.child(label("正在验证并保存备份目录…", p.muted).text_size(px(12.))))
                            .when(!self.backup_directory_error.is_empty(), |d| d.child(label(self.backup_directory_error.clone(), p.amber).text_size(px(12.)).whitespace_normal())),
                    ),
            )
            .child(
                self.card()
                    .gap(px(14.))
                    .child(column().gap(px(5.)).child(heading("恢复数据", p)).child(
                        label("选择备份文件，替换当前流量账本。", p.muted).whitespace_normal(),
                    ))
                    .child(
                        column().gap(px(7.)).child(label("备份文件", p.text)).child(
                            row()
                                .gap(px(8.))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .child(self.text_input(&self.restore_path)),
                                )
                                .child(self.button(
                                    "choose-restore-file",
                                    "选择文件",
                                    false,
                                    Action::Backup(BackupAction::ChooseRestoreFile),
                                    cx,
                                )),
                        ),
                    )
                    .child(
                        column()
                            .gap(px(4.))
                            .p(px(12.))
                            .rounded(px(8.))
                            .border_1()
                            .border_color(surface_border(p))
                            .child(
                                label("恢复前自动备份当前账本", p.text)
                                    .font_weight(FontWeight::MEDIUM),
                            )
                            .child(
                                label(
                                    "先校验备份，再恢复数据；恢复前的账本保存在备份目录。",
                                    p.muted,
                                )
                                .text_size(px(12.))
                                .whitespace_normal(),
                            ),
                    )
                    .child(settings_separator(p))
                    .child(
                        row()
                            .gap(px(12.))
                            .flex_wrap()
                            .justify_between()
                            .child(
                                div().flex_1().min_w_0().child(
                                    Checkbox::new("restore-confirm")
                                        .label("确认用此备份替换当前账本")
                                        .checked(self.restore_armed)
                                        .disabled(!has_path || busy)
                                        .on_click(cx.listener(|app, _, window, cx| {
                                            app.act(Action::Backup(BackupAction::RestoreArm), window, cx)
                                        })),
                                ),
                            )
                            .child(
                                Button::new("restore")
                                    .label("恢复备份")
                                    .danger()
                                    .disabled(!self.can_restore_backup(cx))
                                    .on_click(cx.listener(|app, _, window, cx| {
                                        app.act(Action::Backup(BackupAction::Restore), window, cx)
                                    })),
                            ),
                    ),
            )
    }

    pub(super) fn about_settings(&self, cx: &Context<Self>) -> Div {
        let p = self.p;
        column().gap(px(16.))
            .child(self.card().gap(px(12.))
                .child(row().items_start().justify_between().flex_wrap()
                    .child(row().gap(px(14.)).flex_none()
                        .child(img("images/traffic.png").size(px(44.)).flex_none())
                        .child(column().gap(px(1.)).items_start().whitespace_nowrap()
                            .child(heading("流量监控", p).text_size(px(20.)).line_height(px(24.)))
                            .child(label(concat!("v", env!("CARGO_PKG_VERSION")), p.muted)
                                .text_size(px(12.))
                                .line_height(px(16.)))))
                    .child(self.update_header_actions(cx)))
                .child(settings_separator(p))
                .child(self.update_frequency_row(cx)))
            .child(self.card().gap(px(0.))
                .child(heading("统计与说明", p).pb(px(2.)))
                .child(about_fact("统计范围", "仅记录 Mihomo 接管的流量", "与系统网卡的全部流量不同。", p))
                .child(settings_separator(p))
                .child(about_fact("应用归属", "总量与应用合计可能不同", "短连接和采样遗漏会产生差额，不能用来判断丢包。", p))
                .child(settings_separator(p))
                .child(about_fact("断线处理", "可核验的流量单列为“恢复缺口”", "首次连接不追溯已有用量，缺口不估算应用归属。", p))
                .child(settings_separator(p))
                .child(self.settings_disclosure("about-statistics-details", "统计细节与数据保留", self.about_details_open, SettingsFold::AboutDetails, cx).mt(px(6.)))
                .when(self.about_details_open, |d| d.child(
                    column().gap(px(14.)).pt(px(12.)).pb(px(4.))
                        .child(column().gap(px(4.))
                            .child(label("计算口径", p.text).font_weight(FontWeight::MEDIUM))
                            .child(label("总量取 Mihomo 累计计数的增量，应用取已观测连接的增量。首次连接、内核重启或身份无法确认时建立新基线。", p.muted).text_size(px(12.)).whitespace_normal()))
                        .child(column().gap(px(4.))
                            .child(label("时间精度", p.text).font_weight(FontWeight::MEDIUM))
                            .child(label("按本地时间展示；跨分钟采样按观测区间分摊。同一内核的断线或睡眠恢复量单列为缺口，不推算逐分钟用量。", p.muted).text_size(px(12.)).whitespace_normal()))
                        .child(column().gap(px(4.))
                            .child(label("数据保留", p.text).font_weight(FontWeight::MEDIUM))
                            .child(label("总量和应用分钟账本长期保留；域名、出站和来源的过期分钟明细仅保留每日汇总。时间与空间限制均关闭时无限保留；空间为软上限，保留当天明细。", p.muted).text_size(px(12.)).whitespace_normal())))))
    }
}
