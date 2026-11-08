use super::assets::UiAssets;
use super::*;
use gpui_component::{Root, ThemeMode, TitleBar, scroll::ScrollbarShow};

pub(super) fn apply_theme(
    p: Palette,
    dark: bool,
    mica: bool,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    // GPUI's Blurred appearance requests Acrylic. Leave that disabled and use DWM Mica.
    window.set_background_appearance(WindowBackgroundAppearance::Opaque);
    native::titlebar_theme(native::hwnd(window), dark);
    let mica_active = native::set_mica(native::hwnd(window), mica);
    Theme::change(
        if dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        },
        Some(window),
        cx,
    );
    let theme = Theme::global_mut(cx);
    theme.font_family = "Microsoft YaHei UI".into();
    theme.font_size = px(13.);
    let mut input = rgb(p.muted);
    input.a = 0.35;
    let mut selection = rgb(p.accent);
    selection.a = 0.2;
    // Root also paints a background, so it must let the system backdrop through.
    theme.background = if mica_active { rgba(0) } else { rgb(p.panel) }.into();
    theme.foreground = rgb(p.text).into();
    theme.border = surface_border(p).into();
    theme.input = control_border(p).into();
    theme.scrollbar_show = ScrollbarShow::Always;
    theme.scrollbar = rgba(0).into();
    let mut thumb = rgb(p.muted);
    thumb.a = 0.45;
    theme.scrollbar_thumb = thumb.into();
    thumb.a = 0.65;
    theme.scrollbar_thumb_hover = thumb.into();
    theme.primary = rgb(p.accent).into();
    theme.primary_hover = rgb(if p.mica {
        p.accent_hover()
    } else {
        p.accent_text
    })
    .into();
    theme.primary_active = theme.primary_hover;
    theme.primary_foreground = rgb(if p.mica {
        p.accent_foreground()
    } else if dark {
        p.bg
    } else {
        0x202d3e
    })
    .into();
    theme.secondary = rgba(0).into();
    theme.secondary_foreground = rgb(p.text).into();
    theme.secondary_hover = state_surface(p).into();
    theme.secondary_active = state_surface(p).into();
    theme.muted = rgba(0).into();
    theme.muted_foreground = rgb(p.muted).into();
    theme.accent = state_surface(p).into();
    theme.accent_foreground = rgb(p.accent_text).into();
    theme.caret = rgb(p.accent_text).into();
    theme.ring = rgb(p.accent).into();
    theme.selection = selection.into();
    theme.switch = input.into();
    theme.popover = popup_surface(p).into();
    theme.popover_foreground = rgb(p.text).into();
    theme.title_bar = if mica_active { rgba(0) } else { rgb(p.bg) }.into();
    theme.title_bar_border = rgba(0).into();
    mica_active
}

impl TrafficApp {
    fn new(
        backend: Backend,
        shell: mpsc::Receiver<UiEvent>,
        float_view: Entity<Floating>,
        float_hwnd: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let config = backend.config_snapshot();
        let updates = updates::State::new(&backend.dir, cx);
        let dark = config.theme == "dark" || (config.theme == "system" && native::dark_system());
        let mut wallpaper = WallpaperWatcher::new();
        wallpaper.request(native::hwnd(window));
        let p = Palette::new(config.color_theme, dark)
            .with_wallpaper_accent(wallpaper.color())
            .main_window();
        let mica_active = apply_theme(p, dark, config.color_theme == ColorTheme::Mica, window, cx);
        let search = input("", "搜索应用、域名或目标…", false, window, cx);
        let page_number = input("", "", false, window, cx);
        let connection_form = connection::ConnectionForm::new(&config, window, cx);
        let retention = input(
            &storage_settings::format_amount(
                u64::from(config.retention_days),
                config.retention_time_unit.multiplier(),
            ),
            "保留时间",
            false,
            window,
            cx,
        );
        let restore_path = input("", "备份文件的完整路径 .sqlite3", false, window, cx);
        let retention_size = input(
            &storage_settings::format_amount(
                config.storage_limit_mb,
                config.storage_limit_unit.multiplier(),
            ),
            "存储空间",
            false,
            window,
            cx,
        );
        let hotkey_draft = config.open_hotkey.clone();
        let hotkey_focus = cx.focus_handle();
        let hotkey_subscription = cx.on_blur(&hotkey_focus, window, |app, _, cx| {
            app.cancel_hotkey_recording(cx)
        });
        let position_input = input("0", "距左侧 px", false, window, cx);
        let (editor, editor_subscriptions) =
            DisplayEditor::new(&config.taskbar_content, DisplayTab::Taskbar, window, cx);
        let (inactive_editor, floating_subscriptions) =
            DisplayEditor::new(&config.floating_content, DisplayTab::Floating, window, cx);
        let mut taskbar_fonts = cx.text_system().all_font_names();
        taskbar_fonts.retain(|name| {
            !name.starts_with(['.', '@'])
                && !name.trim().is_empty()
                && name.chars().count() <= 128
                && !name.chars().any(char::is_control)
        });
        taskbar_fonts.extend([
            config.taskbar_content.font_family.clone(),
            config.floating_content.font_family.clone(),
        ]);
        taskbar_fonts.sort();
        taskbar_fonts.dedup();
        let font_search = input("", "搜索字体…", false, window, cx);
        let font_search_subscription =
            cx.subscribe_in(&font_search, window, |app, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    app.filter_font_picker(cx);
                }
            });
        let floating_background = input(
            &config
                .floating_background_color
                .map(|v| format!("#{v:06X}"))
                .unwrap_or_default(),
            "跟随主题 · #RRGGBB",
            false,
            window,
            cx,
        );
        let background_subscription =
            cx.subscribe_in(&floating_background, window, |app, _, event, _, cx| {
                app.update_floating_background(
                    matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. }),
                    cx,
                );
            });
        let floating_transparency = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(100.)
                .step(1.)
                .default_value(f32::from(config.floating_background_transparency))
        });
        let transparency_subscription =
            cx.subscribe_in(&floating_transparency, window, |app, _, event, _, cx| {
                let SliderEvent::Change(value) = event;
                let value = value.end().round().clamp(0., 100.) as u8;
                if value != app.config.floating_background_transparency {
                    app.config.floating_background_transparency = value;
                    app.save_display_content(cx);
                }
            });
        let floating_row_gap = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(32.)
                .step(1.)
                .default_value(f32::from(config.floating_row_gap))
        });
        let row_gap_subscription =
            cx.subscribe_in(&floating_row_gap, window, |app, _, event, _, cx| {
                let SliderEvent::Change(value) = event;
                let value = value.end().round().clamp(0., 32.) as u8;
                if value != app.config.floating_row_gap {
                    app.config.floating_row_gap = value;
                    app.save_display_content(cx);
                }
            });
        let position_slider = cx.new(|_| SliderState::new().min(0.).max(1.).step(0.0001));
        let position_subscription =
            cx.subscribe_in(&position_input, window, |app, _, event, window, cx| {
                match event {
                    InputEvent::Blur | InputEvent::PressEnter { .. } => {
                        app.apply_position_input(window, cx)
                    }
                    InputEvent::Change => app.taskbar_position.error.clear(),
                    _ => {}
                }
                cx.notify();
            });
        let slider_subscription =
            cx.subscribe_in(&position_slider, window, |app, _, event, window, cx| {
                let SliderEvent::Change(value) = event;
                let geometry = app.backend.shared.lock().unwrap().taskbar_geometry;
                if geometry.ready {
                    app.taskbar_position.focus.focus(window);
                    app.set_manual_position(
                        (value.end() * geometry.max_x as f32).round() as i32,
                        window,
                        cx,
                    );
                }
            });
        let retention_subscription =
            cx.subscribe_in(&retention, window, |app, _, event, window, cx| {
                match event {
                    InputEvent::Blur | InputEvent::PressEnter { .. } => {
                        app.save_retention(window, cx)
                    }
                    InputEvent::Change => app.retention_error.clear(),
                    _ => {}
                }
                cx.notify();
            });
        let retention_size_subscription =
            cx.subscribe_in(&retention_size, window, |app, _, event, window, cx| {
                match event {
                    InputEvent::Blur | InputEvent::PressEnter { .. } => {
                        app.save_retention_size(window, cx)
                    }
                    InputEvent::Change => app.retention_size_error.clear(),
                    _ => {}
                }
                cx.notify();
            });
        let restore_subscription = cx.subscribe_in(
            &restore_path,
            window,
            |app, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    app.restore_armed = false;
                }
                cx.notify()
            },
        );
        let activation_subscription = cx.observe_window_activation(window, |app, window, cx| {
            if !window.is_window_active() {
                app.history_state.drag = None;
                app.cancel_hotkey_recording(cx);
                app.taskbar_position.hold_generation += 1;
                app.close_picker(window, cx);
                app.cancel_taskbar_drag(window, cx);
                app.cancel_taskbar_spacing(cx);
                app.editor.spacing.set_preview_hovered(false);
            }
            if window.is_window_active() {
                app.wallpaper.request(app.main_hwnd);
            }
        });
        // Tab is consumed by Root's action binding before key-down listeners run.
        let keyboard_subscription = cx.observe_keystrokes(|app, event, window, cx| {
            let modifiers = event.keystroke.modifiers;
            if event.keystroke.key == "tab"
                && !modifiers.control
                && !modifiers.alt
                && !modifiers.platform
                && !modifiers.function
                && native::hwnd(window) == app.main_hwnd
                && !app.keyboard_navigation
            {
                app.keyboard_navigation = true;
                cx.notify();
            }
        });
        let h = native::hwnd(window);
        native::set_app_icon(h);
        // Intercept before component key bindings without depending on an inner
        // Div's focus path: clicking the page may otherwise blur that Div.
        let keyboard_view = cx.weak_entity();
        let position_keyboard_subscription = cx.intercept_keystrokes(move |event, window, cx| {
            let _ = keyboard_view.update(cx, |app, cx| {
                if native::hwnd(window) != app.main_hwnd {
                    return;
                }
                if app.migration_busy {
                    cx.stop_propagation();
                    return;
                }
                if app.update_notes_is_open() {
                    if app.update_notes_keystroke(event, window, cx) {
                        cx.stop_propagation();
                    }
                    return;
                }
                if app.storage_manager.dialog.is_none()
                    && app.seconds.scope.is_none()
                    && app.page == Page::Settings
                    && app.settings_section == SettingsSection::Storage
                    && event.keystroke.key == "enter"
                {
                    let handled = if app.retention.read(cx).focus_handle(cx).is_focused(window) {
                        app.save_retention(window, cx);
                        true
                    } else if app
                        .retention_size
                        .read(cx)
                        .focus_handle(cx)
                        .is_focused(window)
                    {
                        app.save_retention_size(window, cx);
                        true
                    } else if app
                        .seconds
                        .days
                        .read(cx)
                        .focus_handle(cx)
                        .is_focused(window)
                    {
                        app.save_seconds_setting(window, cx);
                        true
                    } else {
                        false
                    };
                    if handled {
                        cx.stop_propagation();
                        cx.notify();
                        return;
                    }
                }
                if app.storage_manager.dialog.is_some() {
                    if app.storage_keystroke(event, window, cx) {
                        cx.stop_propagation();
                    }
                    return;
                }
                if app.drill_sample_keystroke(event, window, cx) {
                    cx.stop_propagation();
                    return;
                }
                if app.drill_time_keystroke(event, window, cx) {
                    cx.stop_propagation();
                    return;
                }
                if app.seconds.is_modal() {
                    if app.seconds_keystroke(event, window, cx) {
                        cx.stop_propagation();
                    }
                    return;
                }
                if event.keystroke.key == "escape"
                    && app.page == Page::Overview
                    && app.picker.is_none()
                    && app.overview_state.hover.is_some()
                {
                    app.dismiss_overview_hover(window.mouse_position(), cx);
                    cx.stop_propagation();
                    return;
                }
                if app.hotkey_recording_keystroke(event, window, cx)
                    || app.range_editor_keystroke(event, window, cx)
                    || app.history_keystroke(event, window, cx)
                    || app.taskbar_weight_keystroke(event, window, cx)
                    || app.taskbar_spacing_keystroke(event, window, cx)
                {
                    cx.stop_propagation();
                } else if event.keystroke.key == "escape" && app.editor.drag.is_some() {
                    app.cancel_taskbar_drag(window, cx);
                    cx.stop_propagation();
                } else if app.picker_keystroke(event, window, cx) {
                    if event.keystroke.key != "tab" {
                        cx.stop_propagation();
                    }
                } else if event.keystroke.key == "escape" && app.usage_detail_open() {
                    app.close_usage_detail();
                    cx.notify();
                    cx.stop_propagation();
                } else if event.keystroke.key == "escape"
                    && app.page == Page::Connections
                    && app.connections_view.expanded.is_some()
                {
                    app.connections_view.expanded = None;
                    cx.notify();
                    cx.stop_propagation();
                } else {
                    app.position_keystroke(event, window, cx);
                }
            });
        });
        native::titlebar_theme(h, dark);
        let close_backend = backend.clone();
        window.on_window_should_close(cx, move |_, cx| {
            native::set_hotkey_recording(false);
            if close_backend.config.lock().unwrap().close_to_tray {
                native::show(h, false);
                false
            } else {
                cx.quit();
                true
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            loop {
                Timer::after(Duration::from_millis(500)).await;
                if this.update_in(cx, |s, w, cx| s.tick(w, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        let query_ready = backend.query_ready.clone();
        let query_stop = backend.stop.clone();
        cx.spawn_in(window, async move |this, cx| {
            loop {
                if query_stop.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
                query_ready.notified().await;
                let active = this.update_in(cx, |app, _, cx| {
                    if app.backend.stop.load(std::sync::atomic::Ordering::Relaxed) {
                        return false;
                    }
                    app.poll_query_results(cx);
                    app.poll_analysis(cx);
                    app.poll_history_calendar(false, cx);
                    app.poll_recent_usage(cx);
                    cx.notify();
                    true
                });
                if !matches!(active, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
        let mut app = Self {
            #[cfg(test)]
            connection_test_scroll: ScrollHandle::new(),
            page_scroll: ScrollHandle::new(),
            page_scrollbar: ScrollbarState::default(),
            app_usage_bounds: Rc::new(Cell::new(Bounds::default())),
            app_usage_height: Rc::new(Cell::new(px(0.))),
            app_usage_anchor_pending: false,
            app_list_offset: point(px(0.), px(0.)),
            usage_detail: apps_page::UsageDetailState::default(),
            connections_view: connections_page::State::default(),
            overview_state: overview_page::State::default(),
            seconds: seconds_page::State::new(&config, window, cx),
            storage_manager: storage_management_page::State::new(window, cx),
            expanded_application_path: None,
            backend,
            shell,
            page: Page::Overview,
            display_tab: DisplayTab::Taskbar,
            settings_section: SettingsSection::General,
            p,
            mica_active,
            wallpaper,
            config,
            editor,
            inactive_editor,
            taskbar_fonts,
            floating_background,
            floating_background_error: false,
            floating_transparency,
            floating_row_gap,
            picker: None,
            font_search,
            picker_focus: cx.focus_handle(),
            picker_bounds: [
                "retention-time-unit",
                "seconds-time-unit",
                "storage-clean-range",
                "retention-size-unit",
                "taskbar-format",
                "taskbar-unit",
                "taskbar-precision",
                "taskbar-device",
                "taskbar-percent-unit",
                "taskbar-font",
                "taskbar-font-size",
                "taskbar-font-weight",
                "floating-rows",
                "analysis-bucket",
                "analysis-sort",
                "connections-sort",
                "history-range",
                "history-annual-scope",
                "recent-usage-refresh",
                "update-check-frequency",
            ]
            .into_iter()
            .map(|id| (id, Rc::new(Cell::new(Bounds::default()))))
            .collect(),
            picker_scroll: ScrollHandle::new(),
            picker_scrollbar: ScrollbarState::default(),
            taskbar_position: taskbar_position::State::new(
                position_input,
                position_slider,
                cx.focus_handle(),
            ),
            query: Query::default(),
            query_id: 0,
            request_sequence: 0,
            detail_id: 0,
            report: Report::default(),
            detail: Report::default(),
            detail_cache: Default::default(),
            received_report_id: 0,
            received_detail_id: 0,
            detail_requested_at: None,
            unsent_query: None,
            pending_query: None,
            unsent_detail_query: None,
            pending_detail_query: None,
            data: Shared::default(),
            live_chart_view: chart::LiveChart::new(cx),
            search,
            page_number,
            connection_form,
            retention,
            retention_size,
            retention_size_error: String::new(),
            data_directory_draft: None,
            data_directory_error: String::new(),
            migration_busy: false,
            hotkey_draft,
            hotkey_recording: false,
            hotkey_focus,
            hotkey_pending: None,
            hotkey_request: 0,
            open_hotkey_error: String::new(),
            restore_path,
            period: TimePeriod::Today,
            app_custom_range: None,
            selected: String::new(),
            detail_dimension: "domain",
            history_state: history_page::HistoryState::new(window, cx),
            analysis: analysis_page::AnalysisState::new(window, cx),
            app_icons: crate::app_icons::AppIcons::new(),
            icon_scale: window.scale_factor(),
            activity: activity_page::ActivityPageState::new(window, cx),
            live_app_order: RefCell::new(ActivityOrder::default()),
            restore_armed: false,
            about_details_open: false,
            updates,
            backup_directory_busy: false,
            backup_directory_error: String::new(),
            retention_error: String::new(),
            keyboard_navigation: false,
            tick: 0,
            last_search: String::new(),
            main_hwnd: h,
            float_hwnd,
            float_view,
            _subscriptions: vec![
                retention_subscription,
                retention_size_subscription,
                hotkey_subscription,
                restore_subscription,
                activation_subscription,
                keyboard_subscription,
                position_subscription,
                slider_subscription,
                position_keyboard_subscription,
                background_subscription,
                transparency_subscription,
                row_gap_subscription,
                font_search_subscription,
            ]
            .into_iter()
            .chain(editor_subscriptions)
            .chain(floating_subscriptions)
            .collect(),
        };
        app.subscribe_range_editor(window, cx);
        app.subscribe_seconds(window, cx);
        let sub = cx.subscribe_in(&app.storage_manager.date, window, |app, _, event, _, cx| {
            if app.storage_manager.preset == 2
                && matches!(
                    event,
                    InputEvent::Change | InputEvent::PressEnter { .. } | InputEvent::Blur
                )
            {
                app.preview_storage(cx);
            }
        });
        app._subscriptions.push(sub);
        app._subscriptions.push(cx.subscribe_in(
            &app.page_number,
            window,
            |app, _, event, window, cx| match event {
                InputEvent::PressEnter { .. } => app.act(Action::JumpToPage, window, cx),
                InputEvent::Change => cx.notify(),
                _ => {}
            },
        ));
        app.refresh();
        #[cfg(test)]
        if position_key_tests::RUN.load(std::sync::atomic::Ordering::Acquire) {
            position_key_tests::schedule(window, cx);
        }
        #[cfg(test)]
        if editor_tests::RUN.load(std::sync::atomic::Ordering::Acquire) {
            editor_tests::schedule(window, cx);
        }
        #[cfg(test)]
        if connection_tests::RUN.load(std::sync::atomic::Ordering::Acquire) {
            connection_tests::schedule(window, cx);
        }
        #[cfg(test)]
        if connections_page_tests::RUN.load(std::sync::atomic::Ordering::Acquire) {
            connections_page_tests::schedule(window, cx);
        }
        #[cfg(test)]
        if overview_tests::RUN.load(std::sync::atomic::Ordering::Acquire) {
            overview_tests::schedule(window, cx);
        }
        #[cfg(test)]
        if storage_tests::RUN.load(std::sync::atomic::Ordering::Acquire) {
            storage_tests::schedule(window, cx);
        }
        #[cfg(test)]
        if backup_about_tests::RUN.load(std::sync::atomic::Ordering::Acquire) {
            backup_about_tests::schedule(window, cx);
        }
        #[cfg(test)]
        if apps_tests::RUN.load(std::sync::atomic::Ordering::Acquire) {
            apps_tests::schedule(window, cx);
        }
        app
    }

    fn window_control(
        &self,
        id: &'static str,
        icon: &'static str,
        area: WindowControlArea,
    ) -> Stateful<Div> {
        let p = self.p;
        let close = area == WindowControlArea::Close;
        let (hover, pressed, hover_text) = if close {
            (rgb(0xb94a48), rgb(0xa13f3d), 0xffffff)
        } else {
            let mut hover = rgb(p.text);
            hover.a = if p.dark { 0.10 } else { 0.065 };
            let mut pressed = hover;
            pressed.a *= 1.5;
            (hover, pressed, p.text)
        };
        div()
            .id(id)
            .group(id)
            .flex()
            .items_center()
            .justify_center()
            .size(px(32.))
            .flex_none()
            .rounded(px(8.))
            .bg(rgba(0))
            .text_color(rgb(p.text))
            .window_control_area(area)
            .hover(move |d| d.bg(hover).text_color(rgb(hover_text)))
            .active(move |d| d.bg(pressed).text_color(rgb(hover_text)))
            .child(
                svg()
                    .path(icon)
                    .id((id, 0usize))
                    .size(px(16.))
                    .flex_none()
                    .text_color(rgb(p.text))
                    .group_hover(id, move |s| s.text_color(rgb(hover_text)))
                    .group_active(id, move |s| s.text_color(rgb(hover_text))),
            )
    }
    pub(super) fn titlebar(&self, window: &Window, cx: &Context<Self>) -> Stateful<Div> {
        let p = self.p;
        let collapsed = self.config.collapsed;
        row()
            .id("title-bar")
            .w_full()
            .h(px(52.))
            .flex_none()
            .gap(px(0.))
            .bg(if self.mica_active { rgba(0) } else { rgb(p.bg) })
            .child(
                row()
                    .w(self.sidebar_width())
                    .h_full()
                    .flex_none()
                    .gap(px(10.))
                    .pl(px(20.))
                    .bg(if self.mica_active {
                        rgba(0)
                    } else {
                        rgb(p.side)
                    })
                    .border_r_1()
                    .border_color(card_border(p))
                    .window_control_area(WindowControlArea::Drag)
                    .child(
                        img(ImageSource::Resource(Resource::Embedded(
                            "images/traffic.png".into(),
                        )))
                        .size(px(24.))
                        .flex_none(),
                    )
                    .when(!collapsed, |d| {
                        d.child(
                            div()
                                .text_size(px(14.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("流量监控"),
                        )
                    }),
            )
            // Only non-interactive regions participate in native caption hit testing.
            .child(
                row()
                    .id("page-title")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .pl(px(24.))
                    .pr(px(16.))
                    .text_size(px(18.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .when(self.page != Page::History, |d| {
                        d.window_control_area(WindowControlArea::Drag)
                    })
                    .gap(px(10.))
                    .child(
                        div()
                            .h_full()
                            .flex()
                            .items_center()
                            .window_control_area(WindowControlArea::Drag)
                            .child(self.page.label()),
                    )
                    .when(self.page == Page::History, |d| {
                        d.child(
                            div()
                                .h_full()
                                .flex_1()
                                .window_control_area(WindowControlArea::Drag),
                        )
                    }),
            )
            .when(
                (self.page.has_time_range() || self.page == Page::History)
                    && !(self.page == Page::History
                        && self.history_state.tab == HistoryTab::Analysis)
                    && !(self.page == Page::Apps
                        && self.activity.details
                        && self.selected.is_empty()),
                |d| d.child(self.period_tools(cx)),
            )
            .child(
                div()
                    .w(px(16.))
                    .h_full()
                    .flex_none()
                    .window_control_area(WindowControlArea::Drag),
            )
            .child(
                column()
                    .h_full()
                    .flex_none()
                    .gap(px(0.))
                    .child(
                        row()
                            .h(px(40.))
                            .flex_none()
                            .pt(px(6.))
                            .pr(px(8.))
                            .gap(px(4.))
                            .child(self.window_control(
                                "window-minimize",
                                "icons/window-minimize.svg",
                                WindowControlArea::Min,
                            ))
                            .child(self.window_control(
                                "window-maximize",
                                if window.is_maximized() {
                                    "icons/window-restore.svg"
                                } else {
                                    "icons/window-maximize.svg"
                                },
                                WindowControlArea::Max,
                            ))
                            .child(self.window_control(
                                "window-close",
                                "icons/window-close.svg",
                                WindowControlArea::Close,
                            )),
                    )
                    .child(
                        div()
                            .w_full()
                            .flex_1()
                            .window_control_area(WindowControlArea::Drag),
                    ),
            )
    }
}

pub fn run(backend: Backend) -> Result<(), anyhow::Error> {
    let shell = native::start_shell(
        backend.shared.clone(),
        backend.config.clone(),
        backend.stop.clone(),
    );
    let main = std::sync::Arc::new(std::sync::atomic::AtomicIsize::new(0));
    let shell = std::cell::RefCell::new(Some(shell));
    Application::new()
        .with_assets(UiAssets)
        .run(move |cx: &mut App| {
            gpui_component::init(cx);
            let config = backend.config_snapshot();
            let float_backend = backend.clone();
            let main_handle = main.clone();
            let float = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                            point(
                                px(config.floating_x.unwrap_or(0) as f32),
                                px(config.floating_y.unwrap_or(0) as f32),
                            ),
                            size(px(270.), px(84.)),
                        ))),
                        titlebar: None,
                        window_background: WindowBackgroundAppearance::Transparent,
                        focus: false,
                        // Initialize before the saved visibility is applied by the first tick.
                        show: false,
                        is_resizable: false,
                        is_minimizable: false,
                        ..Default::default()
                    },
                    move |_, cx| cx.new(|_| Floating::new(float_backend, main_handle)),
                )
                .expect("无法创建悬浮窗");
            let (float_view, float_hwnd) = float
                .update(cx, |_, window, cx| {
                    let h = native::hwnd(window);
                    native::setup_float(
                        h,
                        config.floating_x.unwrap_or(0),
                        config.floating_y.unwrap_or(0),
                    );
                    (cx.entity(), h)
                })
                .expect("悬浮窗初始化失败");
            let b = backend.clone();
            let shell = shell.borrow_mut().take().unwrap();
            let main_handle = main.clone();
            let background = std::env::args().any(|s| s == "--background");
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::centered(size(px(1240.), px(870.)), cx)),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Mihomo Traffic".into()),
                        ..TitleBar::title_bar_options()
                    }),
                    window_background: WindowBackgroundAppearance::Opaque,
                    window_min_size: Some(size(px(if cfg!(test) { 720. } else { 900. }), px(700.))),
                    show: true,
                    ..Default::default()
                },
                move |window, cx| {
                    main_handle.store(native::hwnd(window), std::sync::atomic::Ordering::Relaxed);
                    let view =
                        cx.new(|cx| TrafficApp::new(b, shell, float_view, float_hwnd, window, cx));
                    cx.new(|cx| Root::new(view.into(), window, cx))
                },
            )
            .expect("无法创建主窗口");
            if !background {
                cx.activate(true);
                native::foreground(main.load(std::sync::atomic::Ordering::Relaxed));
            } else {
                native::show(main.load(std::sync::atomic::Ordering::Relaxed), false);
            }
        });
    Ok(())
}
