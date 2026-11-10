use super::*;

pub(super) fn format_amount(value: u64, multiplier: u64) -> String {
    if multiplier == 1 {
        return value.to_string();
    }
    let precision = if multiplier == 1024 { 10 } else { 6 };
    format!("{:.*}", precision, value as f64 / multiplier as f64)
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

pub(super) fn parse_amount(text: &str, multiplier: u64, maximum: u64) -> Option<u64> {
    let value = text.trim().parse::<f64>().ok()?;
    if !value.is_finite() || value <= 0. || (multiplier == 1 && value.fract() != 0.) {
        return None;
    }
    let scaled = value * multiplier as f64;
    let rounded = scaled.round();
    (scaled.is_finite()
        && rounded >= 1.
        && rounded <= maximum as f64
        && (scaled - rounded).abs() <= 0.000001)
        .then_some(rounded as u64)
}

#[derive(Clone)]
pub(super) enum StorageAction {
    RetentionToggle(bool),
    StorageExpand,
    StorageHelp,
    StorageRefresh,
    StorageOpen(storage_management_page::Dialog),
    StorageClose,
    StoragePreset(usize),
    StorageRangePicker,
    StoragePreview,
    StorageConfirm,
    RetentionTimeUnit(RetentionTimeUnit),
    RetentionSizeUnit(RetentionSizeUnit),
    ResetStorageSettings,
    ChooseDataDirectory,
    MigrateDataDirectory,
    CancelDataDirectory,
}

impl TrafficApp {
    pub(super) fn save_retention(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.config.retention_days_enabled || self.migration_busy {
            return;
        }
        let value = self.retention.read(cx).value().to_string();
        let multiplier = self.config.retention_time_unit.multiplier();
        let current = format_amount(u64::from(self.config.retention_days), multiplier);
        if value.trim() == current {
            if value != current {
                self.retention
                    .update(cx, |s, cx| s.set_value(current, window, cx));
            }
            self.retention_error.clear();
            return;
        }
        let Some(days) = parse_amount(&value, multiplier, 3650).map(|days| days as u32) else {
            self.retention_error = "有效范围 1–3650 天；月按 30 天换算，最小精度为 1 天".into();
            return;
        };
        self.retention_error.clear();
        let display = format_amount(u64::from(days), multiplier);
        if value != display {
            self.retention
                .update(cx, |s, cx| s.set_value(display, window, cx));
        }
        if days != self.config.retention_days {
            self.config.retention_days = days;
            self.save_config();
        }
    }

    pub(super) fn handle_storage_action(
        &mut self,
        action: StorageAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            StorageAction::RetentionToggle(size) => self.toggle_retention(size, window, cx),
            StorageAction::StorageExpand => {
                self.storage_manager.expanded = !self.storage_manager.expanded;
                if self.storage_manager.expanded {
                    self.refresh_storage(cx);
                }
            }
            StorageAction::StorageHelp => {
                self.storage_manager.help_expanded = !self.storage_manager.help_expanded
            }
            StorageAction::StorageRefresh => self.refresh_storage(cx),
            StorageAction::StorageOpen(dialog) => self.open_storage_dialog(dialog, window, cx),
            StorageAction::StorageClose => self.close_storage_dialog(window, cx),
            StorageAction::StoragePreset(i) => {
                self.storage_manager.preset = i.min(3);
                self.preview_storage(cx);
            }
            StorageAction::StorageRangePicker => {
                let titles = ["7天前", "30天前", "自定义日期之前", "全部可清理明细"];
                self.toggle_picker(
                    "storage-clean-range",
                    titles
                        .into_iter()
                        .enumerate()
                        .map(|(i, t)| {
                            (
                                t.into(),
                                Action::Storage(StorageAction::StoragePreset(i)),
                                self.storage_manager.preset == i,
                            )
                        })
                        .collect(),
                    window,
                    cx,
                );
            }
            StorageAction::StoragePreview => self.preview_storage(cx),
            StorageAction::StorageConfirm => self.confirm_storage(cx),
            StorageAction::RetentionTimeUnit(unit) => {
                self.change_retention_time_unit(unit, window, cx)
            }
            StorageAction::RetentionSizeUnit(unit) => {
                self.change_retention_size_unit(unit, window, cx)
            }
            StorageAction::ResetStorageSettings => self.reset_storage_settings(window, cx),
            StorageAction::ChooseDataDirectory => self.choose_data_directory(cx),
            StorageAction::MigrateDataDirectory => self.migrate_data_directory(window, cx),
            StorageAction::CancelDataDirectory => {
                self.data_directory_draft = None;
                self.data_directory_error.clear();
            }
        }
    }

    pub(super) fn save_retention_size(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.config.storage_limit_enabled || self.migration_busy {
            return;
        }
        let value = self.retention_size.read(cx).value().to_string();
        let multiplier = self.config.storage_limit_unit.multiplier();
        let current = format_amount(self.config.storage_limit_mb, multiplier);
        if value.trim() == current {
            if value != current {
                self.retention_size
                    .update(cx, |s, cx| s.set_value(current, window, cx));
            }
            self.retention_size_error.clear();
            return;
        }
        let Some(mb) = parse_amount(&value, multiplier, u64::MAX / (1024 * 1024)) else {
            self.retention_size_error = "请输入有效容量，最小精度为 1 MB（1 GB = 1024 MB）".into();
            return;
        };
        self.retention_size_error.clear();
        let display = format_amount(mb, multiplier);
        if value != display {
            self.retention_size
                .update(cx, |s, cx| s.set_value(display, window, cx));
        }
        if mb != self.config.storage_limit_mb {
            self.config.storage_limit_mb = mb;
            self.save_config();
        }
    }

    pub(super) fn toggle_retention(
        &mut self,
        size: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if size {
            self.config.storage_limit_enabled = !self.config.storage_limit_enabled;
            self.config.storage_limit_migration_notice = false;
            self.retention_size_error.clear();
            self.retention_size.update(cx, |s, cx| {
                s.set_value(
                    format_amount(
                        self.config.storage_limit_mb,
                        self.config.storage_limit_unit.multiplier(),
                    ),
                    window,
                    cx,
                )
            });
        } else {
            self.config.retention_days_enabled = !self.config.retention_days_enabled;
            self.retention_error.clear();
            self.retention.update(cx, |s, cx| {
                s.set_value(
                    format_amount(
                        u64::from(self.config.retention_days),
                        self.config.retention_time_unit.multiplier(),
                    ),
                    window,
                    cx,
                )
            });
        }
        self.save_config();
    }

    pub(super) fn change_retention_time_unit(
        &mut self,
        unit: RetentionTimeUnit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.save_retention(window, cx);
        if !self.retention_error.is_empty() {
            return;
        }
        self.config.retention_time_unit = unit;
        self.retention.update(cx, |s, cx| {
            s.set_value(
                format_amount(u64::from(self.config.retention_days), unit.multiplier()),
                window,
                cx,
            )
        });
        self.save_config();
    }

    pub(super) fn change_retention_size_unit(
        &mut self,
        unit: RetentionSizeUnit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.save_retention_size(window, cx);
        if !self.retention_size_error.is_empty() {
            return;
        }
        self.config.storage_limit_unit = unit;
        self.retention_size.update(cx, |s, cx| {
            s.set_value(
                format_amount(self.config.storage_limit_mb, unit.multiplier()),
                window,
                cx,
            )
        });
        self.save_config();
    }

    pub(super) fn reset_storage_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let defaults = Config::default();
        self.config.retention_days = defaults.retention_days;
        self.config.retention_time_unit = defaults.retention_time_unit;
        self.config.retention_days_enabled = defaults.retention_days_enabled;
        self.config.storage_limit_mb = defaults.storage_limit_mb;
        self.config.storage_limit_unit = defaults.storage_limit_unit;
        self.config.storage_limit_enabled = defaults.storage_limit_enabled;
        self.config.storage_limit_migration_notice = false;
        self.config.seconds_enabled = defaults.seconds_enabled;
        self.config.seconds_days = defaults.seconds_days;
        self.config.seconds_days_enabled = defaults.seconds_days_enabled;
        self.config.seconds_time_unit = defaults.seconds_time_unit;
        self.config.seconds_size_enabled = defaults.seconds_size_enabled;
        self.config.seconds_size_mb = defaults.seconds_size_mb;
        self.config.seconds_size_unit = defaults.seconds_size_unit;
        self.seconds.setting_error.clear();
        self.sync_seconds_inputs(window, cx);
        self.retention_error.clear();
        self.retention_size_error.clear();
        self.retention.update(cx, |s, cx| {
            s.set_value(
                format_amount(
                    u64::from(defaults.retention_days),
                    defaults.retention_time_unit.multiplier(),
                ),
                window,
                cx,
            )
        });
        self.retention_size
            .update(cx, |s, cx| s.set_value("2", window, cx));
        let default = crate::config::default_data_dir();
        self.data_directory_draft =
            (!crate::data_location::same_directory(&self.backend.dir, &default)).then_some(default);
        self.data_directory_error.clear();
        self.save_config();
    }

    pub(super) fn sync_retention_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for (input, value, error) in [
            (
                &self.retention,
                format_amount(
                    u64::from(self.config.retention_days),
                    self.config.retention_time_unit.multiplier(),
                ),
                &self.retention_error,
            ),
            (
                &self.retention_size,
                format_amount(
                    self.config.storage_limit_mb,
                    self.config.storage_limit_unit.multiplier(),
                ),
                &self.retention_size_error,
            ),
        ] {
            if error.is_empty()
                && !input.read(cx).focus_handle(cx).is_focused(window)
                && input.read(cx).value().as_ref() != value
            {
                input.update(cx, |s, cx| s.set_value(value, window, cx));
            }
        }
    }

    pub(super) fn choose_data_directory(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("选择数据存储目录".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = receiver.await
                && let Some(path) = paths.into_iter().next()
            {
                let _ = this.update(cx, |app, cx| {
                    match crate::data_location::validate(&app.backend.dir, &path) {
                        Ok(_) => {
                            app.data_directory_draft = Some(path);
                            app.data_directory_error.clear();
                        }
                        Err(error) => {
                            app.data_directory_draft = None;
                            app.data_directory_error = error.to_string();
                        }
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub(super) fn migrate_data_directory(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.updates.installing() {
            self.data_directory_error = "应用正在更新，请完成后再迁移".into();
            return;
        }
        if self.backend.shared.lock().unwrap().storage_management.busy {
            self.data_directory_error = "存储任务正在执行，请完成后再迁移".into();
            return;
        }
        if self.backup_directory_busy {
            self.data_directory_error = "备份目录正在保存，请稍候再迁移".into();
            return;
        }
        self.save_retention(window, cx);
        self.save_retention_size(window, cx);
        if !self.retention_error.is_empty() || !self.retention_size_error.is_empty() {
            return;
        }
        let Some(target) = self.data_directory_draft.clone() else {
            return;
        };
        self.migration_busy = true;
        window.blur();
        let backend = self.backend.clone();
        let task = cx
            .background_executor()
            .spawn(async move { backend.prepare_migration(&target) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |app, cx| match result {
                Ok(()) => cx.quit(),
                Err(error) => {
                    app.migration_busy = false;
                    app.data_directory_error = format!("无法迁移：{error:#}");
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(super) fn storage_policy_part(
        &self,
        _row: usize,
        _column: usize,
        control: impl IntoElement,
        cx: &Context<Self>,
    ) -> Div {
        let part = div()
            .relative()
            .w(if _column == 1 {
                controls::storage_unit_width(cx)
            } else {
                px([72., 0., 52.][_column])
            })
            .flex_none()
            .child(control);
        #[cfg(test)]
        let part = {
            let bounds = self.storage_manager.policy_bounds[_row][_column].clone();
            part.child(
                canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
        };
        part
    }
    pub(super) fn storage_settings(&self, cx: &Context<Self>) -> Div {
        let p = self.p;
        let mut days = self.text_input(&self.retention);
        days.disabled = !self.config.retention_days_enabled;
        days.invalid = !self.retention_error.is_empty();
        let mut size = self.text_input(&self.retention_size);
        size.disabled = !self.config.storage_limit_enabled;
        size.invalid = !self.retention_size_error.is_empty();
        let time_choices = RetentionTimeUnit::ALL
            .into_iter()
            .map(|unit| {
                (
                    unit.label().into(),
                    Action::Storage(StorageAction::RetentionTimeUnit(unit)),
                    self.config.retention_time_unit == unit,
                )
            })
            .collect();
        let size_choices = RetentionSizeUnit::ALL
            .into_iter()
            .map(|unit| {
                (
                    unit.label().into(),
                    Action::Storage(StorageAction::RetentionSizeUnit(unit)),
                    self.config.storage_limit_unit == unit,
                )
            })
            .collect();
        let state = &self.data.storage_management;
        let show_details = self.storage_manager.expanded
            || state.refreshing
            || state.over_limit
            || !state.job.is_empty()
            || !state.error.is_empty()
            || !state.job_error.is_empty();
        let space = self
            .card()
            .id("storage-space-card")
            .gap(px(12.))
            .child(self.storage_usage(cx))
            .child(settings_separator(p))
            .child(
                row()
                    .min_h(px(34.))
                    .gap(px(12.))
                    .flex_wrap()
                    .child(
                        row()
                            .gap(px(8.))
                            .flex_1()
                            .min_w(px(210.))
                            .child(label("存储容量限制", p.text).font_weight(FontWeight::MEDIUM))
                            .child(self.button(
                                "storage-expand",
                                if self.storage_manager.expanded {
                                    "收起存储明细 ▴"
                                } else {
                                    "查看存储明细 ▾"
                                },
                                false,
                                Action::Storage(StorageAction::StorageExpand),
                                cx,
                            )),
                    )
                    .child(
                        row()
                            .gap(px(6.))
                            .flex_none()
                            .child(div().w(px(72.)).flex_none().child(size))
                            .child(self.taskbar_picker(
                                "retention-size-unit",
                                self.config.storage_limit_unit.label().into(),
                                size_choices,
                                cx,
                            ))
                            .child(self.switch_control(
                                "retention-size-enabled",
                                "存储容量限制",
                                self.config.storage_limit_enabled,
                                Action::Storage(StorageAction::RetentionToggle(true)),
                                cx,
                            )),
                    ),
            )
            .when(!self.retention_size_error.is_empty(), |d| {
                d.child(
                    label(self.retention_size_error.clone(), p.amber)
                        .text_size(px(12.))
                        .whitespace_normal(),
                )
            })
            .when(self.config.storage_limit_migration_notice, |d| {
                d.child(
                    label("旧容量限制已合并，请确认容量后重新开启。", p.amber)
                        .text_size(px(12.))
                        .whitespace_normal(),
                )
            })
            .when(show_details, |d| d.child(self.storage_breakdown(cx)));
        let retention = self
            .card()
            .id("storage-retention-card")
            .gap(px(12.))
            .child(
                row()
                    .justify_between()
                    .gap(px(12.))
                    .child(heading("采集与保留", p))
                    .child(self.button(
                        "storage-help",
                        "说明",
                        self.storage_manager.help_expanded,
                        Action::Storage(StorageAction::StorageHelp),
                        cx,
                    )),
            )
            .when(self.storage_manager.help_expanded, |d| {
                d.child(self.storage_help())
            })
            .child(option_row(
                "历史明细",
                if self.config.retention_days_enabled {
                    "按时间清理域名、出站、来源明细"
                } else {
                    "不按时间清理历史明细"
                },
                column()
                    .items_end()
                    .gap(px(6.))
                    .child(
                        row()
                            .gap(px(6.))
                            .child(self.storage_policy_part(0, 0, days, cx))
                            .child(self.storage_policy_part(
                                0,
                                1,
                                self.taskbar_picker(
                                    "retention-time-unit",
                                    self.config.retention_time_unit.label().into(),
                                    time_choices,
                                    cx,
                                ),
                                cx,
                            ))
                            .child(self.storage_policy_part(
                                0,
                                2,
                                self.switch_control(
                                    "retention-days-enabled",
                                    "按时间清理历史明细",
                                    self.config.retention_days_enabled,
                                    Action::Storage(StorageAction::RetentionToggle(false)),
                                    cx,
                                ),
                                cx,
                            )),
                    )
                    .when(!self.retention_error.is_empty(), |d| {
                        d.child(
                            label(self.retention_error.clone(), p.amber)
                                .text_size(px(12.))
                                .max_w(px(240.))
                                .whitespace_normal(),
                        )
                    }),
                p,
            ))
            .child(settings_separator(p))
            .child(self.seconds_settings(cx));
        #[cfg(test)]
        let space = measured_card(space, self.storage_manager.cards[0].clone());
        #[cfg(test)]
        let retention = measured_card(retention, self.storage_manager.cards[1].clone());
        let locations = self.storage_locations(cx);
        #[cfg(test)]
        let locations = measured_card(locations, self.storage_manager.cards[2].clone());
        column().gap(px(16.)).pb(px(4.))
            .child(space)
            .child(retention)
            .child(locations)
            .child(row().justify_end().child(self.button("reset-storage-settings","恢复默认",false,Action::Storage(StorageAction::ResetStorageSettings),cx)
                .tooltip(|window,cx|Tooltip::new("历史明细开启、保留3个月；秒级保存开启、保留3天；统一容量限制开启、上限2 GB。目录迁移需另行确认。").build(window,cx))))
    }
    fn storage_help(&self) -> Div {
        let p = self.p;
        column().gap(px(12.)).p(px(14.)).rounded(px(8.)).bg(state_surface(p))
            .child(row().items_start().gap(px(18.)).flex_wrap()
                .child(help_fact("摘要长期保留","总量、应用用量、每日汇总、峰值与均速，不随明细清理。",p))
                .child(help_fact("秒级记录不补录","开启后开始积累；关闭后，已有记录继续按保留策略维护。",p)))
            .child(settings_separator(p))
            .child(help_rule("保留策略","1个月按30天换算。“长期”取消到期清理，仍受统一容量上限约束。",p))
            .child(help_rule("清理保护","先清秒级，再清今天以前的历史明细；当天历史明细和长期汇总受保护。普通清理不暂停采集，回收文件空间需另行确认。",p))
            .child(column().gap(px(5.))
                .child(label("容量参考",p.text).text_size(px(12.)).font_weight(FontWeight::MEDIUM))
                .child(row().gap(px(18.)).flex_wrap()
                    .child(label("10项 ≈ 1.6 GB／月",p.text).font_weight(FontWeight::MEDIUM))
                    .child(label("20项 ≈ 3 GB／月",p.text).font_weight(FontWeight::MEDIUM)))
                .child(label("按24小时持续活跃估算；原始进程和宿主各算一项，实际随活动变化。",p.muted).text_size(px(12.)).whitespace_normal()))
    }
    fn storage_locations(&self, cx: &Context<Self>) -> Stateful<Div> {
        let p = self.p;
        let export_directory = crate::csv_export::directory()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|e| format!("下载目录不可用：{e}"));
        self.card().id("storage-locations-card").gap(px(12.))
            .child(row().justify_between().gap(px(12.)).flex_wrap()
                .child(heading("文件位置",p))
                .child(self.button("choose-data","更换数据目录",false,Action::Storage(StorageAction::ChooseDataDirectory),cx)))
            .child(column().gap(px(0.)).border_t_1().border_color(surface_border(p))
                .child(location_row("数据目录","保存流量数据库和设置。更换目录会迁移数据并重启程序。",self.backend.dir.display().to_string(),self.button("open-data","打开",false,Action::OpenDir(""),cx),p))
                .child(settings_separator(p))
                .child(location_row("CSV 导出","默认保存到系统下载文件夹。历史页统计列表右上方的“导出 CSV”跟随当前日期、维度和搜索条件。",export_directory,self.button("open-exports","打开",false,Action::OpenExportDirectory,cx),p)))
            .when_some(self.data_directory_draft.as_ref(),|d,target|d.child(column().gap(px(10.)).p(px(12.)).rounded(px(8.)).border_1().border_color(surface_border(p))
                .child(label(format!("迁移至：{}",target.display()),p.text).whitespace_normal())
                .child(label("迁移数据和设置后自动重启。目标已有数据先备份，原目录保留回退副本。",p.muted).text_size(px(12.)).whitespace_normal())
                .child(row().gap(px(8.)).flex_wrap()
                    .child(self.button("migrate-data","迁移并重启",true,Action::Storage(StorageAction::MigrateDataDirectory),cx))
                    .child(self.button("cancel-migrate-data","取消",false,Action::Storage(StorageAction::CancelDataDirectory),cx)))))
            .when(!self.data_directory_error.is_empty(),|d|d.child(label(self.data_directory_error.clone(),p.amber).text_size(px(12.)).whitespace_normal()))
    }
}

pub(super) fn option_row(
    title: &str,
    description: &str,
    control: impl IntoElement,
    p: Palette,
) -> Div {
    row()
        .flex_wrap()
        .gap(px(12.))
        .min_h(px(44.))
        .child(
            column()
                .flex_1()
                .min_w(px(160.))
                .gap(px(4.))
                .child(label(title.to_string(), p.text).font_weight(FontWeight::MEDIUM))
                .child(
                    label(description.to_string(), p.muted)
                        .text_size(px(12.))
                        .whitespace_normal(),
                ),
        )
        .child(div().flex_none().child(control))
}

#[cfg(test)]
fn measured_card(card: Stateful<Div>, bounds: Rc<Cell<Bounds<Pixels>>>) -> Stateful<Div> {
    card.relative().child(
        canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {})
            .absolute()
            .size_full(),
    )
}

fn location_row(
    title: &str,
    hint: &'static str,
    path: String,
    action: Stateful<Div>,
    p: Palette,
) -> Div {
    row()
        .gap(px(12.))
        .py(px(8.))
        .min_h(px(48.))
        .child(
            label(title.to_string(), p.text)
                .id(SharedString::from(format!("storage-location-{title}")))
                .w(px(64.))
                .flex_none()
                .tooltip(move |window, cx| Tooltip::new(hint).build(window, cx)),
        )
        .child(
            label(path, p.muted)
                .text_size(px(12.))
                .flex_1()
                .min_w_0()
                .whitespace_normal(),
        )
        .child(action.w(px(64.)).flex_none())
}

fn help_fact(title: &str, description: &str, p: Palette) -> Div {
    column()
        .gap(px(5.))
        .flex_1()
        .min_w(px(170.))
        .child(label(title.to_string(), p.text).font_weight(FontWeight::SEMIBOLD))
        .child(
            label(description.to_string(), p.muted)
                .text_size(px(12.))
                .whitespace_normal(),
        )
}
fn help_rule(title: &str, description: &str, p: Palette) -> Div {
    row()
        .items_start()
        .gap(px(12.))
        .child(
            label(title.to_string(), p.text)
                .w(px(52.))
                .flex_none()
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM),
        )
        .child(
            label(description.to_string(), p.muted)
                .text_size(px(12.))
                .flex_1()
                .min_w_0()
                .whitespace_normal(),
        )
}
