use super::*;

pub(super) struct State {
    pub(super) input: Entity<InputState>,
    slider: Entity<SliderState>,
    pub(super) error: String,
    pub(super) focus: FocusHandle,
    feedback: Option<(i32, std::time::Instant)>,
    pub(super) hold_generation: u64,
}

impl State {
    pub(super) fn new(
        input: Entity<InputState>,
        slider: Entity<SliderState>,
        focus: FocusHandle,
    ) -> Self {
        Self {
            input,
            slider,
            error: String::new(),
            focus,
            feedback: None,
            hold_generation: 0,
        }
    }
}

impl TrafficApp {
    pub(super) fn manual_x(&self, geometry: TaskbarGeometry) -> i32 {
        crate::taskbar_layout::manual(
            crate::taskbar_layout::anchor(
                self.config.taskbar_left,
                geometry.tray_start,
                geometry.widget_width,
                geometry.margin,
            ),
            self.config.taskbar_offset,
            geometry.max_x,
        )
    }
    pub(super) fn sync_position_controls(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let geometry = self.backend.shared.lock().unwrap().taskbar_geometry;
        if !geometry.ready {
            return;
        }
        let x = self.manual_x(geometry);
        if self.taskbar_position.error.is_empty()
            && !self
                .taskbar_position
                .input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
            && self.taskbar_position.input.read(cx).value().as_ref() != x.to_string()
        {
            self.taskbar_position
                .input
                .update(cx, |s, cx| s.set_value(x.to_string(), window, cx));
        }
        let normalized = x as f32 / geometry.max_x.max(1) as f32;
        if (self.taskbar_position.slider.read(cx).value().end() - normalized).abs() > 0.00001 {
            self.taskbar_position
                .slider
                .update(cx, |s, cx| s.set_value(normalized, window, cx));
        }
    }
    pub(super) fn set_manual_position(
        &mut self,
        x: i32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.config.taskbar_position_mode != TaskbarPositionMode::Manual {
            return false;
        }
        let geometry = self.backend.shared.lock().unwrap().taskbar_geometry;
        if !geometry.ready {
            return false;
        }
        let x = x.clamp(0, geometry.max_x);
        let changed = x != self.manual_x(geometry);
        if changed {
            self.config.taskbar_offset = x - crate::taskbar_layout::anchor(
                self.config.taskbar_left,
                geometry.tray_start,
                geometry.widget_width,
                geometry.margin,
            );
            self.save_config();
        }
        self.taskbar_position.error.clear();
        self.taskbar_position
            .input
            .update(cx, |s, cx| s.set_value(x.to_string(), window, cx));
        self.sync_position_controls(window, cx);
        cx.notify();
        changed
    }
    pub(super) fn apply_position_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.config.taskbar_position_mode != TaskbarPositionMode::Manual {
            return;
        }
        let geometry = self.backend.shared.lock().unwrap().taskbar_geometry;
        if !geometry.ready {
            self.taskbar_position.error = "正在等待任务栏，请稍后调整".into();
            return;
        }
        let value = self
            .taskbar_position
            .input
            .read(cx)
            .value()
            .trim()
            .parse::<i32>();
        match value {
            Ok(x) if (0..=geometry.max_x).contains(&x) => {
                self.set_manual_position(x, window, cx);
            }
            _ => {
                self.taskbar_position.error = format!("请输入 0–{} 之间的整数像素", geometry.max_x)
            }
        }
    }
    fn begin_position_hold(
        &mut self,
        direction: i32,
        shift: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.taskbar_position.hold_generation += 1;
        let generation = self.taskbar_position.hold_generation;
        self.taskbar_position.focus.focus(window);
        if !self.step_position(direction, shift, window, cx) {
            return;
        }
        cx.spawn_in(window, async move |this, cx| {
            Timer::after(Duration::from_millis(350)).await;
            loop {
                let keep_going = this
                    .update_in(cx, |app, window, cx| {
                        let (pressed, shift) = native::adjustment_keys();
                        if !pressed
                            || !window.is_window_active()
                            || generation != app.taskbar_position.hold_generation
                        {
                            return false;
                        }
                        app.step_position(direction, shift, window, cx)
                    })
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
                Timer::after(Duration::from_millis(60)).await;
            }
        })
        .detach();
    }
    fn step_position(
        &mut self,
        direction: i32,
        shift: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let geometry = self.backend.shared.lock().unwrap().taskbar_geometry;
        let changed = self.set_manual_position(
            self.manual_x(geometry) + direction * if shift { 8 } else { 1 },
            window,
            cx,
        );
        if changed {
            let feedback = (direction, std::time::Instant::now());
            self.taskbar_position.feedback = Some(feedback);
            cx.spawn_in(window, async move |this, cx| {
                Timer::after(Duration::from_millis(160)).await;
                let _ = this.update_in(cx, |app, _, cx| {
                    if app.taskbar_position.feedback == Some(feedback) {
                        app.taskbar_position.feedback = None;
                        cx.notify();
                    }
                });
            })
            .detach();
        }
        changed
    }
    pub(super) fn position_keystroke(
        &mut self,
        event: &KeystrokeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let modifiers = event.keystroke.modifiers;
        if self.page != Page::Display
            || self.editor.spacing.is_dragging()
            || self.display_tab != DisplayTab::Taskbar
            || self.config.taskbar_position_mode != TaskbarPositionMode::Manual
            || modifiers.control
            || modifiers.alt
            || modifiers.platform
            || modifiers.function
            || native::hwnd(window) != self.main_hwnd
            || event.context_stack.iter().any(|context| {
                ["Popover", "Modal", "Drawer", "TaskbarEditor"]
                    .iter()
                    .any(|name| context.contains(name))
            })
        {
            return;
        }
        let direction = match event.keystroke.key.as_str() {
            "left" => -1,
            "right" => 1,
            _ => return,
        };
        let input_focused = self
            .taskbar_position
            .input
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);
        if !input_focused
            && event
                .context_stack
                .iter()
                .any(|context| context.contains("Input"))
        {
            return;
        }
        cx.stop_propagation();
        window.prevent_default();
        // Numeric arrows commit valid pending text before applying the step.
        if input_focused {
            self.apply_position_input(window, cx);
            if !self.taskbar_position.error.is_empty() {
                cx.notify();
                return;
            }
        }
        self.step_position(direction, modifiers.shift, window, cx);
    }

    pub(super) fn taskbar_position_controls(&self, cx: &Context<Self>) -> Div {
        let p = self.p;
        column()
            .w_full()
            .child(
                row()
                    .justify_between()
                    .child(
                        row()
                            .gap(px(6.))
                            .child(heading("定位方式", p))
                            .child(self.position_help()),
                    )
                    .child(
                        row()
                            .gap(px(4.))
                            .p(px(3.))
                            .rounded(px(12.))
                            .bg(state_surface(p))
                            .child(self.button(
                                "position-auto",
                                "自动避让",
                                self.config.taskbar_position_mode == TaskbarPositionMode::AutoAvoid,
                                Action::Display(DisplayAction::TaskbarMode(
                                    TaskbarPositionMode::AutoAvoid,
                                )),
                                cx,
                            ))
                            .child(self.button(
                                "position-manual",
                                "手动调整",
                                self.config.taskbar_position_mode == TaskbarPositionMode::Manual,
                                Action::Display(DisplayAction::TaskbarMode(
                                    TaskbarPositionMode::Manual,
                                )),
                                cx,
                            )),
                    ),
            )
            .when(
                self.config.taskbar_position_mode == TaskbarPositionMode::AutoAvoid,
                |d| {
                    let status = self.data.taskbar_status.clone();
                    let warning = ["失败", "无法", "没有足够", "超时"]
                        .iter()
                        .any(|word| status.contains(word));
                    let positioned = status.starts_with("自动避让 · ");
                    let position = if positioned {
                        format!("距左侧 {} px", self.data.taskbar_geometry.x)
                    } else {
                        status
                    };
                    d.child(
                        row()
                            .justify_between()
                            .gap(px(16.))
                            .pt(px(12.))
                            .border_t_1()
                            .border_color(surface_border(p))
                            .child(
                                row().min_w_0().flex_1().gap(px(6.)).child(
                                    label(
                                        position.clone(),
                                        if warning { p.amber } else { p.muted },
                                    )
                                    .id("automatic-position-status")
                                    .text_size(px(if warning { 13. } else { 14. }))
                                    .text_color(rgb(if warning { p.amber } else { p.text }))
                                    .when(positioned, |d| {
                                        d.px(px(10.))
                                            .py(px(6.))
                                            .rounded(px(8.))
                                            .bg(state_surface(p))
                                            .text_color(rgb(p.accent_text))
                                            .font_weight(FontWeight::MEDIUM)
                                    })
                                    .min_w_0()
                                    .truncate()
                                    .tooltip(
                                        move |window, cx| {
                                            Tooltip::new(position.clone()).build(window, cx)
                                        },
                                    ),
                                ),
                            )
                            .child(
                                row()
                                    .flex_none()
                                    .gap(px(0.))
                                    .child(self.position_aux_button(
                                        "position-rescan",
                                        "重新检测",
                                        Action::Display(DisplayAction::TaskbarRescan),
                                        cx,
                                    ))
                                    .child(position_action_separator(p))
                                    .child(
                                        position_choice_group()
                                            .child(self.position_option(
                                                "prefer-left",
                                                "优先靠左",
                                                self.config.taskbar_auto_prefer_left,
                                                Action::Display(DisplayAction::TaskbarPreference(
                                                    true,
                                                )),
                                                cx,
                                            ))
                                            .child(self.position_option(
                                                "prefer-tray",
                                                "靠近通知区",
                                                !self.config.taskbar_auto_prefer_left,
                                                Action::Display(DisplayAction::TaskbarPreference(
                                                    false,
                                                )),
                                                cx,
                                            )),
                                    ),
                            ),
                    )
                },
            )
            .when(
                self.config.taskbar_position_mode == TaskbarPositionMode::Manual,
                |d| {
                    let geometry = self.data.taskbar_geometry;
                    let current = self.manual_x(geometry);
                    let anchor_x = |left| {
                        crate::taskbar_layout::manual(
                            crate::taskbar_layout::anchor(
                                left,
                                geometry.tray_start,
                                geometry.widget_width,
                                geometry.margin,
                            ),
                            0,
                            geometry.max_x,
                        )
                    };
                    d.child(
                        column()
                            .gap(px(12.))
                            .id("manual-position-controls")
                            .track_focus(&self.taskbar_position.focus)
                            .tab_stop(true)
                            .child(
                                row()
                                    .justify_between()
                                    .child(label("距任务栏左侧", p.muted))
                                    .child(
                                        row()
                                            .gap(px(6.))
                                            .child(self.position_step_button(
                                                "position-minus",
                                                -1,
                                                cx,
                                            ))
                                            .child(self.position_step_button(
                                                "position-plus",
                                                1,
                                                cx,
                                            ))
                                            .child(div().w(px(94.)).child(
                                                self.text_input(&self.taskbar_position.input),
                                            ))
                                            .child(label("px", p.muted)),
                                    ),
                            )
                            .child(
                                Slider::new(&self.taskbar_position.slider)
                                    .disabled(!geometry.ready),
                            )
                            .child(
                                row()
                                    .justify_between()
                                    .pt(px(10.))
                                    .border_t_1()
                                    .border_color(surface_border(p))
                                    .child(label("0 px", p.muted).text_size(px(12.)))
                                    .child(
                                        row()
                                            .gap(px(0.))
                                            .flex_none()
                                            .child(self.position_aux_button(
                                                "position-reset",
                                                "重置",
                                                Action::Display(DisplayAction::TaskbarPreset(
                                                    Config::default().taskbar_left,
                                                )),
                                                cx,
                                            ))
                                            .child(position_action_separator(p))
                                            .child(
                                                position_choice_group()
                                                    .child(self.position_option(
                                                        "position-left",
                                                        "靠左",
                                                        geometry.ready && current == anchor_x(true),
                                                        Action::Display(
                                                            DisplayAction::TaskbarPreset(true),
                                                        ),
                                                        cx,
                                                    ))
                                                    .child(self.position_option(
                                                        "position-tray",
                                                        "靠近通知区",
                                                        geometry.ready
                                                            && current == anchor_x(false),
                                                        Action::Display(
                                                            DisplayAction::TaskbarPreset(false),
                                                        ),
                                                        cx,
                                                    )),
                                            )
                                            .child(
                                                label(format!("{} px", geometry.max_x), p.muted)
                                                    .text_size(px(12.))
                                                    .ml(px(14.)),
                                            ),
                                    ),
                            )
                            .when(!self.taskbar_position.error.is_empty(), |d| {
                                d.child(label(self.taskbar_position.error.clone(), p.amber))
                            })
                            .when(
                                self.config.taskbar
                                    && ["失败", "无法", "不支持", "等待 Explorer"]
                                        .iter()
                                        .any(|word| self.data.taskbar_status.contains(word)),
                                |d| {
                                    d.child(
                                        label(self.data.taskbar_status.clone(), p.amber)
                                            .whitespace_normal(),
                                    )
                                },
                            ),
                    )
                },
            )
    }
    fn position_help(&self) -> impl IntoElement {
        let p = self.p;
        let manual = self.config.taskbar_position_mode == TaskbarPositionMode::Manual;
        let mut lift = rgb(0xffffff);
        lift.a = if p.dark { 0.04 } else { 0.72 };
        let background = rgb(p.bg).blend(lift);
        Popover::new("position-help-popover")
            .no_style()
            .trigger(InfoHelpTrigger { id: "position-help", palette: p, selected: false })
            .content(move |window, cx| cx.new(|cx| PopoverContent::new(window, cx, move |_, cx| {
                column().id("position-help-card").gap(px(8.)).p(px(14.)).w(px(320.))
                    .bg(background).border_1().border_color(card_border(p))
                    .rounded(px(10.)).shadow_sm()
                    .when(manual, |d| d.child(label("每次1px · Shift加速至8px · 支持长按", p.text).whitespace_normal()))
                    .child(label(if manual { "键盘 ← / → 调整，Shift 加速。数字输入回车或失焦应用。" } else { "没有足够空位时临时隐藏，空位稳定后恢复；识别失败时可使用手动调整。" }, p.muted).whitespace_normal())
                    // no_style removes the component's outside-click handler,
                    // so keep dismissal on this card while styling it locally.
                    .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(DismissEvent)))
                    .into_any_element()
            }).p_0()))
    }
    fn position_aux_button(
        &self,
        id: &'static str,
        text: &'static str,
        action: Action,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let p = self.p;
        self.button(id, text, false, action, cx)
            .h(px(32.))
            .px(px(11.))
            .flex_none()
            .active(move |s| s.bg(switch_off_surface(p)).text_color(rgb(p.text)))
    }
    fn position_option(
        &self,
        id: &'static str,
        text: &'static str,
        selected: bool,
        action: Action,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let p = self.p;
        self.button(id, text, selected, action, cx)
            .h(px(32.))
            .px(px(12.))
            .rounded(px(8.))
            .bg(rgba(0))
            .text_color(rgb(if selected { p.accent_text } else { p.muted }))
            .active(move |s| s.bg(switch_off_surface(p)).text_color(rgb(p.accent_text)))
    }
    fn position_step_button(
        &self,
        id: &'static str,
        direction: i32,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let p = self.p;
        let geometry = self.data.taskbar_geometry;
        let x = self.manual_x(geometry);
        let enabled = geometry.ready
            && if direction < 0 {
                x > 0
            } else {
                x < geometry.max_x
            };
        let feedback = self
            .taskbar_position
            .feedback
            .is_some_and(|(value, _)| value == direction);
        let mut pressed = rgb(p.accent);
        pressed.a = if p.dark { 0.28 } else { 0.18 };
        div()
            .id(id)
            .focusable()
            .tab_stop(enabled)
            .size(px(32.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(9.))
            .border_1()
            .border_color(if feedback {
                rgb(p.accent)
            } else {
                surface_border(p)
            })
            .bg(if feedback { pressed } else { rgba(0) })
            .text_color(rgb(if feedback { p.accent_text } else { p.muted }))
            .when(!enabled, |d| d.opacity(0.4))
            .when(self.keyboard_navigation, |d| {
                d.focus(move |s| s.border_color(rgb(p.accent)))
            })
            .child(
                Icon::default()
                    .path(if direction < 0 {
                        "icons/position-left.svg"
                    } else {
                        "icons/position-right.svg"
                    })
                    .size(px(16.)),
            )
            .when(enabled, |d| {
                d.cursor_pointer()
                    .hover(move |s| {
                        s.bg(if feedback { pressed } else { state_surface(p) })
                            .border_color(rgb(p.accent))
                            .text_color(rgb(p.accent_text))
                    })
                    .active(move |s| {
                        s.bg(pressed)
                            .border_color(rgb(p.accent))
                            .text_color(rgb(p.accent_text))
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |app, event: &MouseDownEvent, window, cx| {
                            app.begin_position_hold(direction, event.modifiers.shift, window, cx);
                        }),
                    )
                    .on_key_down(cx.listener(move |app, event: &KeyDownEvent, window, cx| {
                        if !event.is_held
                            && !event.keystroke.modifiers.modified()
                            && matches!(event.keystroke.key.as_str(), "enter" | "space")
                        {
                            cx.stop_propagation();
                            window.prevent_default();
                            app.step_position(direction, false, window, cx);
                        }
                    }))
            })
    }
}
