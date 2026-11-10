use super::*;

impl TrafficApp {
    fn color_option(&self, color_theme: ColorTheme, cx: &Context<Self>) -> Stateful<Div> {
        let p = self.p;
        let sample = Palette::new(color_theme, Theme::global(cx).is_dark())
            .with_wallpaper_accent(self.wallpaper.color())
            .main_window();
        let sample_mica = color_theme == ColorTheme::Mica;
        let hover_bg = state_surface(p);
        let selected = self.config.color_theme == color_theme;
        let preview_background = if sample_mica {
            linear_gradient(
                135.,
                linear_color_stop(rgb(sample.tint), 0.),
                linear_color_stop(rgb(sample.bg), 1.),
            )
        } else {
            rgb(sample.bg).into()
        };
        let preview = row()
            .gap(px(0.))
            .w_full()
            .h(px(68.))
            .flex_none()
            .rounded(px(5.))
            .overflow_hidden()
            .border_1()
            .border_color(surface_border(sample))
            .bg(preview_background)
            .child(
                column()
                    .w(relative(0.28))
                    .h_full()
                    .flex_none()
                    .gap(px(5.))
                    .p(px(4.))
                    .when(!sample_mica, |sidebar| sidebar.bg(rgb(sample.tint)))
                    .child(
                        div()
                            .w_full()
                            .h(px(12.))
                            .rounded(px(3.))
                            .bg(rgb(sample.accent)),
                    )
                    .children((0..3).map(|_| {
                        div()
                            .w_full()
                            .h(px(2.))
                            .rounded(px(1.))
                            .bg(rgba((sample.muted << 8) | 0x66))
                    })),
            )
            .child(
                column()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .gap(px(5.))
                    .p(px(5.))
                    .child(
                        div()
                            .w(relative(0.45))
                            .h(px(4.))
                            .rounded(px(1.))
                            .bg(rgb(sample.muted)),
                    )
                    .child(
                        row()
                            .flex_1()
                            .min_h_0()
                            .gap(px(3.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .h_full()
                                    .rounded(px(2.))
                                    .border_1()
                                    .border_color(card_border(sample))
                                    .bg(card_surface(sample, sample_mica))
                                    .child(
                                        div()
                                            .w(relative(0.72))
                                            .h(px(5.))
                                            .mt(px(9.))
                                            .ml(px(3.))
                                            .rounded_full()
                                            .bg(rgb(sample.accent)),
                                    ),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .h_full()
                                    .rounded(px(2.))
                                    .border_1()
                                    .border_color(card_border(sample))
                                    .bg(rgb(sample.tint)),
                            ),
                    ),
            );
        column()
            .id(color_theme.id())
            .flex_1()
            .min_w_0()
            .gap(px(8.))
            .p(px(8.))
            .rounded(px(8.))
            .border_2()
            .border_color(if selected { rgb(p.accent) } else { rgba(0) })
            .cursor_pointer()
            .hover(move |s| s.bg(hover_bg))
            .child(preview)
            .child(
                label(
                    color_theme.label(),
                    if selected { p.accent_text } else { p.text },
                )
                .w_full()
                .text_center()
                .font_weight(FontWeight::NORMAL),
            )
            .when(color_theme == ColorTheme::Mica, |d| {
                d.tooltip(|window, cx| {
                    Tooltip::new(
                        "Windows 系统云母材质贯通背景，控件强调色随当前显示器的壁纸自动调整。",
                    )
                    .build(window, cx)
                })
            })
            .on_click(cx.listener(move |s, _, window, cx| {
                s.act(Action::ColorTheme(color_theme), window, cx)
            }))
    }
    pub(super) fn settings(&self, cx: &Context<Self>) -> Div {
        let p = self.p;
        let color_picker = row().gap(px(10.)).w_full().children(
            ColorTheme::ALL
                .into_iter()
                .map(|theme| self.color_option(theme, cx)),
        );
        let theme_picker = row()
            .gap(px(2.))
            .p(px(3.))
            .rounded(px(10.))
            .bg(rgba(0))
            .border_1()
            .border_color(surface_border(p))
            .children(
                [("浅色", "light"), ("深色", "dark"), ("跟随系统", "system")]
                    .into_iter()
                    .map(|(title, mode)| {
                        self.settings_segment(
                            mode,
                            title,
                            self.config.theme == mode,
                            Action::Theme(mode),
                            cx,
                        )
                    }),
            );
        let tray_picker = row()
            .gap(px(2.))
            .p(px(3.))
            .rounded(px(10.))
            .border_1()
            .border_color(surface_border(p))
            .children(
                [
                    ("tray-monochrome", "黑白（自动）", TrayIconStyle::Monochrome),
                    ("tray-color", "彩色", TrayIconStyle::Color),
                ]
                .into_iter()
                .map(|(id, title, style)| self.tray_icon_option(id, title, style, cx)),
            );
        let general = self
            .card()
            .gap(px(0.))
            .child(
                column()
                    .gap(px(12.))
                    .pb(px(16.))
                    .child(heading("配色方案", p))
                    .child(color_picker),
            )
            .child(settings_separator(p))
            .child(settings_row("外观模式", "", theme_picker, p))
            .child(settings_separator(p))
            .child(settings_row(
                "开机启动",
                "登录 Windows 后在后台运行",
                self.switch_control(
                    "settings-autorun",
                    "开机启动",
                    self.config.autorun,
                    Action::Config("autorun"),
                    cx,
                ),
                p,
            ))
            .child(settings_separator(p))
            .child(settings_row(
                "关闭窗口后继续统计",
                "隐藏主窗口并继续采集，点击托盘可快速恢复",
                self.switch_control(
                    "settings-close-tray",
                    "关闭窗口后继续统计",
                    self.config.close_to_tray,
                    Action::Config("close"),
                    cx,
                ),
                p,
            ))
            .child(settings_separator(p))
            .child(settings_row(
                "显示托盘图标",
                "单击打开概览，右键打开菜单",
                self.switch_control(
                    "settings-tray-visible",
                    "显示托盘图标",
                    self.config.tray_visible,
                    Action::Config("tray"),
                    cx,
                ),
                p,
            ))
            .child(settings_separator(p))
            .child(settings_row(
                "托盘图标样式",
                "黑白图标自动适配 Windows 任务栏的深浅色",
                tray_picker,
                p,
            ))
            .child(settings_separator(p))
            .child(self.recent_usage_refresh_settings(cx))
            .child(settings_separator(p))
            .child(self.open_hotkey_settings(cx));
        let connection = self.connection_settings(cx);

        let storage = self.storage_settings(cx);

        let backup = self.backup_settings(cx);
        let content = match self.settings_section {
            SettingsSection::General => general,
            SettingsSection::Connection => connection,
            SettingsSection::Storage => storage,
            SettingsSection::Backup => backup,
            SettingsSection::About => self.about_settings(cx),
        };

        let content_scroll = div()
            .id(SharedString::from(format!(
                "settings-content-{}",
                self.settings_section.id()
            )))
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .child(content);
        #[cfg(test)]
        let content_scroll = content_scroll.track_scroll(&self.connection_test_scroll);

        row()
            .items_start()
            .flex_1()
            .min_h_0()
            .w_full()
            .max_w(px(1100.))
            .gap(px(16.))
            .child(
                column()
                    .w(px(144.))
                    .flex_none()
                    .gap(px(4.))
                    .p(px(8.))
                    .bg(self.card_surface())
                    .border_1()
                    .border_color(card_border(p))
                    .rounded(px(p.radius as f32))
                    .children(SettingsSection::ALL.into_iter().map(|section| {
                        let selected = self.settings_section == section;
                        self.button(
                            format!("settings-section-{}", section.id()),
                            if section == SettingsSection::About && self.updates.has_unread(cx) {
                                "关于 · 新版"
                            } else {
                                section.label()
                            },
                            selected,
                            Action::SettingsSection(section),
                            cx,
                        )
                        .w_full()
                        .h(px(40.))
                        .justify_start()
                        .font_weight(FontWeight::NORMAL)
                    })),
            )
            .child(content_scroll)
    }

    pub(super) fn recent_usage_refresh_settings(&self, cx: &Context<Self>) -> Div {
        let mode = self.config.recent_usage_refresh_mode;
        settings_row(
            "应用-近期用量刷新方式",
            "点击近期用量时按所选方式更新；首次打开均会生成快照",
            div().w(px(178.)).child(
                self.taskbar_picker(
                    "recent-usage-refresh",
                    mode.label().into(),
                    RecentUsageRefreshMode::ALL
                        .into_iter()
                        .map(|choice| {
                            (
                                choice.label().into(),
                                Action::RecentUsageRefreshMode(choice),
                                choice == mode,
                            )
                        })
                        .collect(),
                    cx,
                ),
            ),
            self.p,
        )
    }

    pub(super) fn tray_icon_option(
        &self,
        id: &'static str,
        title: &'static str,
        style: TrayIconStyle,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let p = self.p;
        let selected = self.config.tray_icon_style == style;
        let preview = row()
            .gap(px(4.))
            .when(style == TrayIconStyle::Monochrome, |d| {
                d.children(
                    [
                        ("images/tray-dark.png", 0xf4f4f5),
                        ("images/tray-light.png", 0x252529),
                    ]
                    .into_iter()
                    .map(|(path, background)| {
                        div()
                            .size(px(26.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(5.))
                            .bg(rgb(background))
                            .child(img(path).size(px(20.)))
                    }),
                )
            })
            .when(style == TrayIconStyle::Color, |d| {
                d.child(img("images/traffic.png").size(px(26.)))
            });
        row()
            .id(id)
            .focusable()
            .tab_stop(true)
            .gap(px(8.))
            .px(px(10.))
            .h(px(38.))
            .rounded(px(7.))
            .cursor_pointer()
            .bg(if selected { state_surface(p) } else { rgba(0) })
            .hover(move |s| s.bg(state_surface(p)))
            .when(self.keyboard_navigation, |d| {
                d.focus(move |s| s.bg(state_surface(p)))
            })
            .child(preview)
            .child(label(title, if selected { p.accent_text } else { p.muted }))
            .on_click(cx.listener(move |app, _, window, cx| {
                app.act(Action::TrayIconStyle(style), window, cx)
            }))
            .on_key_down(cx.listener(move |app, event: &KeyDownEvent, window, cx| {
                if !event.is_held
                    && !event.keystroke.modifiers.modified()
                    && matches!(event.keystroke.key.as_str(), "enter" | "space")
                {
                    cx.stop_propagation();
                    app.act(Action::TrayIconStyle(style), window, cx);
                }
            }))
    }

    pub(super) fn start_hotkey_recording(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.hotkey_pending.is_some() {
            return;
        }
        self.hotkey_focus.focus(window);
        self.hotkey_recording = true;
        self.open_hotkey_error.clear();
        self.backend.set_hotkey_recording(true);
        cx.notify();
    }

    pub(super) fn cancel_hotkey_recording(&mut self, cx: &mut Context<Self>) {
        if self.hotkey_recording {
            self.hotkey_recording = false;
            self.open_hotkey_error.clear();
            self.backend.set_hotkey_recording(false);
            cx.notify();
        }
    }

    pub(super) fn record_open_hotkey(
        &mut self,
        key: crate::hotkey::Hotkey,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if !self.hotkey_recording
            || !window.is_window_active()
            || !self.hotkey_focus.is_focused(window)
        {
            return;
        }
        self.hotkey_draft = key.to_string();
        cx.notify();
        self.open_hotkey_error.clear();
        self.cancel_hotkey_recording(cx);
    }

    pub(super) fn hotkey_recording_keystroke(
        &mut self,
        event: &KeystrokeEvent,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.hotkey_recording {
            return false;
        }
        if !self.hotkey_focus.is_focused(window) {
            self.cancel_hotkey_recording(cx);
            return false;
        }
        let key = &event.keystroke;
        if key.key == "escape" {
            self.cancel_hotkey_recording(cx);
            return true;
        }
        if key.key == "tab"
            && !key.modifiers.control
            && !key.modifiers.alt
            && !key.modifiers.platform
        {
            self.cancel_hotkey_recording(cx);
            return false;
        }
        match crate::hotkey::Hotkey::recorded(
            &key.key,
            key.modifiers.control,
            key.modifiers.alt,
            key.modifiers.shift,
            key.modifiers.platform,
        ) {
            Ok(key) => self.record_open_hotkey(key, window, cx),
            Err(error) => {
                self.open_hotkey_error = error.to_string();
                cx.notify();
            }
        }
        true
    }

    pub(super) fn apply_open_hotkey(&mut self, cx: &mut Context<Self>) {
        if self.hotkey_pending.is_some() {
            return;
        }
        let result = crate::hotkey::Hotkey::parse(&self.hotkey_draft);
        match result {
            Ok(key) => {
                self.hotkey_request = self.hotkey_request.wrapping_add(1);
                match self.backend.apply_hotkey(self.hotkey_request, key) {
                    Ok(()) => {
                        self.hotkey_pending = Some((self.hotkey_request, key));
                        self.open_hotkey_error.clear();
                    }
                    Err(error) => self.open_hotkey_error = error.to_string(),
                }
            }
            Err(error) => self.open_hotkey_error = error.to_string(),
        }
        cx.notify();
    }

    pub(super) fn open_hotkey_applied(
        &mut self,
        request: u32,
        error: u32,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((pending, key)) = self.hotkey_pending else {
            return;
        };
        if pending != request {
            return;
        }
        self.hotkey_pending = None;
        cx.notify();
        if error != 0 {
            self.open_hotkey_error = crate::hotkey::failure(error);
            return;
        }
        self.config.open_hotkey = key.to_string();
        self.save_config();
        self.open_hotkey_error.clear();
        self.hotkey_draft = key.to_string();
    }

    pub(super) fn open_hotkey_settings(&self, cx: &Context<Self>) -> Div {
        let p = self.p;
        let pending = self.hotkey_pending.is_some();
        let error = if self.open_hotkey_error.is_empty() {
            &self.data.open_hotkey_error
        } else {
            &self.open_hotkey_error
        };
        let recording = self.hotkey_recording;
        let status = if recording {
            Some("正在录制，按 Esc 取消".into())
        } else if pending {
            Some("正在注册快捷键…".into())
        } else if self.hotkey_draft != self.config.open_hotkey || !error.is_empty() {
            Some(if self.data.open_hotkey_active.is_empty() {
                "当前没有可用的全局快捷键".into()
            } else {
                format!("当前可用：{}", self.data.open_hotkey_active)
            })
        } else {
            None
        };
        let text = if recording {
            "请按组合键…".into()
        } else {
            crate::hotkey::Hotkey::parse(&self.hotkey_draft)
                .map(|key| key.to_string())
                .unwrap_or_else(|_| self.hotkey_draft.clone())
        };
        let field = row()
            .id("open-hotkey-recorder")
            .track_focus(&self.hotkey_focus)
            .tab_stop(true)
            .w_full()
            .min_w_0()
            .h(px(34.))
            .gap(px(8.))
            .px(px(10.))
            .rounded(px(8.))
            .border_1()
            .border_color(if recording {
                rgb(p.accent)
            } else if !error.is_empty() {
                rgb(p.amber)
            } else {
                control_border(p)
            })
            .bg(control_surface(p))
            .cursor_pointer()
            .child(
                label(text, p.text)
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap(),
            )
            .child(
                label(
                    if recording {
                        "录制中"
                    } else {
                        "点击录制"
                    },
                    p.muted,
                )
                .flex_none(),
            )
            .on_click(
                cx.listener(|app, _, window, cx| app.act(Action::RecordOpenHotkey, window, cx)),
            )
            .on_mouse_down_out(cx.listener(|app, _, _, cx| app.cancel_hotkey_recording(cx)))
            .on_key_down(cx.listener(|app, event: &KeyDownEvent, window, cx| {
                if !event.is_held
                    && !app.hotkey_recording
                    && !event.keystroke.modifiers.modified()
                    && matches!(event.keystroke.key.as_str(), "enter" | "space")
                {
                    cx.stop_propagation();
                    app.act(Action::RecordOpenHotkey, window, cx);
                }
            }));
        column()
            .gap(px(8.))
            .py(px(12.))
            .child(heading("打开主界面快捷键", p))
            .child(
                label(
                    "点击下方区域后按组合键，再点应用；主窗口和托盘隐藏后仍可呼出。",
                    p.muted,
                )
                .whitespace_normal(),
            )
            .child(
                row()
                    .gap(px(8.))
                    .child(div().flex_1().min_w_0().child(field))
                    .child(
                        self.button(
                            if recording {
                                "cancel-open-hotkey"
                            } else {
                                "apply-open-hotkey"
                            },
                            if pending {
                                "应用中…"
                            } else if recording {
                                "取消"
                            } else {
                                "应用"
                            },
                            true,
                            if recording {
                                Action::CancelOpenHotkeyRecording
                            } else {
                                Action::ApplyOpenHotkey
                            },
                            cx,
                        )
                        .when(pending, |d| d.opacity(0.5)),
                    )
                    .child(
                        self.button(
                            "reset-open-hotkey",
                            "恢复默认",
                            false,
                            Action::ResetOpenHotkey,
                            cx,
                        )
                        .when(pending, |d| d.opacity(0.5)),
                    ),
            )
            .children(status.map(|text| label(text, p.muted).whitespace_normal()))
            .when(!error.is_empty(), |d| {
                d.child(label(error.clone(), p.amber).whitespace_normal())
            })
    }
}
