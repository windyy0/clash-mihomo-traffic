use super::*;

#[derive(Clone, Copy)]
pub(super) enum TaskbarColorTarget {
    Text,
    Background,
}

impl TaskbarColorTarget {
    pub(super) const ALL: [Self; 2] = [Self::Text, Self::Background];

    pub(super) fn index(self) -> usize {
        match self {
            Self::Text => 0,
            Self::Background => 1,
        }
    }

    pub(super) fn value(self, item: &TaskbarItem) -> Option<u32> {
        match self {
            Self::Text => item.text_color,
            Self::Background => item.background_color,
        }
    }

    pub(super) fn set(self, item: &mut TaskbarItem, color: Option<u32>) {
        match self {
            Self::Text => item.text_color = color,
            Self::Background => item.background_color = color,
        }
    }

    fn default_label(self) -> &'static str {
        match self {
            Self::Text => "跟随主题",
            Self::Background => "透明",
        }
    }
}

fn color_string(color: Option<u32>) -> String {
    color
        .map(|color| format!("#{color:06X}"))
        .unwrap_or_default()
}

impl TrafficApp {
    pub(super) fn update_taskbar_font_size_input(
        &mut self,
        submitted: bool,
        cx: &mut Context<Self>,
    ) {
        let value = self.editor.size.read(cx).value();
        let size = value
            .trim()
            .parse::<f32>()
            .ok()
            .filter(|size| (8. ..=20.).contains(size));
        self.editor.size_error = submitted && size.is_none();
        if let Some(size) = size
            && size != self.display_content().font_size
        {
            self.display_content_mut().font_size = size;
            self.save_display_content(cx);
        }
        cx.notify();
    }

    pub(super) fn sync_taskbar_font_size(
        &mut self,
        force: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if force
            || (!self.editor.size_error
                && !self
                    .editor
                    .size
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window))
        {
            let value = self.display_content().font_size.to_string();
            if self.editor.size.read(cx).value().as_ref() != value {
                self.editor
                    .size
                    .update(cx, |input, cx| input.set_value(value, window, cx));
            }
            if force {
                self.editor.size_error = false;
            }
        }
    }

    fn step_taskbar_weight(&mut self, step: i32, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.editor.weight.read(cx).value();
        let Some(weight) = value
            .trim()
            .parse::<u16>()
            .ok()
            .and_then(TaskbarFontWeight::new)
        else {
            self.editor.weight_error = true;
            self.editor.weight_wheel_remainder = 0.;
            cx.notify();
            return;
        };
        let next = (i32::from(weight.value()) + step).clamp(100, 900) as u16;
        if next == 100 || next == 900 {
            self.editor.weight_wheel_remainder = 0.;
        }
        self.editor.weight_error = false;
        let weight = TaskbarFontWeight::new(next).unwrap();
        if weight != self.display_content().font_weight {
            self.display_content_mut().font_weight = weight;
            self.save_display_content(cx);
        }
        self.sync_taskbar_weight(true, window, cx);
        cx.notify();
    }

    pub(super) fn taskbar_weight_keystroke(
        &mut self,
        event: &KeystrokeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let modifiers = event.keystroke.modifiers;
        if self.page != Page::Display
            || !self.editor.editing
            || self.picker.is_some()
            || modifiers.control
            || modifiers.alt
            || modifiers.platform
            || modifiers.function
            || !self
                .editor
                .weight
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        {
            return false;
        }
        let direction = match event.keystroke.key.as_str() {
            "up" => 1,
            "down" => -1,
            _ => return false,
        };
        self.editor.weight_wheel_remainder = 0.;
        self.step_taskbar_weight(direction * if modifiers.shift { 10 } else { 1 }, window, cx);
        window.prevent_default();
        true
    }

    pub(super) fn scroll_taskbar_weight(
        &mut self,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let modifiers = event.modifiers;
        if self.page != Page::Display
            || !self.editor.editing
            || self.picker.is_some()
            || self.editor.drag.is_some()
            || self.editor.spacing.is_dragging()
            || modifiers.control
            || modifiers.alt
            || modifiers.platform
            || modifiers.function
        {
            return false;
        }
        let delta = match event.delta {
            // Windows converts Shift + vertical wheel into a horizontal delta.
            ScrollDelta::Lines(delta) => {
                (if modifiers.shift && delta.y == 0. {
                    delta.x
                } else {
                    delta.y
                }) / 3.
            }
            ScrollDelta::Pixels(delta) => {
                f32::from(if modifiers.shift && delta.y == px(0.) {
                    delta.x
                } else {
                    delta.y
                }) / 24.
            }
        };
        if !delta.is_finite() || delta == 0. {
            return false;
        }
        let remainder = &mut self.editor.weight_wheel_remainder;
        if remainder.signum() != delta.signum() {
            *remainder = 0.;
        }
        *remainder = (*remainder + delta).clamp(-800., 800.);
        let step = remainder.trunc() as i32;
        *remainder -= step as f32;
        if step != 0 {
            self.step_taskbar_weight(step * if modifiers.shift { 10 } else { 1 }, window, cx);
        }
        true
    }

    pub(super) fn update_taskbar_weight_input(&mut self, submitted: bool, cx: &mut Context<Self>) {
        let value = self.editor.weight.read(cx).value();
        let weight = value
            .trim()
            .parse::<u16>()
            .ok()
            .and_then(TaskbarFontWeight::new);
        self.editor.weight_error = submitted && weight.is_none();
        if let Some(weight) = weight
            && weight != self.display_content().font_weight
        {
            self.display_content_mut().font_weight = weight;
            self.save_display_content(cx);
        }
        cx.notify();
    }

    pub(super) fn sync_taskbar_weight(
        &mut self,
        force: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if force
            || (!self.editor.weight_error
                && !self
                    .editor
                    .weight
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window))
        {
            let value = self.display_content().font_weight.value().to_string();
            if self.editor.weight.read(cx).value().as_ref() != value {
                self.editor
                    .weight
                    .update(cx, |state, cx| state.set_value(value, window, cx));
            }
            if force {
                self.editor.weight_error = false;
            }
        }
    }

    pub(super) fn update_taskbar_color_input(
        &mut self,
        target: TaskbarColorTarget,
        submitted: bool,
        cx: &mut Context<Self>,
    ) {
        let index = target.index();
        let value = self.editor.colors[index].read(cx).value();
        let value = value.trim();
        let hex = value.strip_prefix('#').unwrap_or(value);
        let color = if value.is_empty() {
            Some(None)
        } else if hex.len() == 6 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            u32::from_str_radix(hex, 16).ok().map(Some)
        } else {
            None
        };
        self.editor.color_errors[index] = if color.is_none() && submitted {
            "请输入六位颜色，如 #3B82F6".into()
        } else {
            String::new()
        };
        if let Some(color) = color {
            self.edit_taskbar_item(cx, |item| target.set(item, color));
        }
        cx.notify();
    }

    pub(super) fn sync_taskbar_colors(
        &mut self,
        force: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(metric) = self.editor.selected else {
            return;
        };
        let item = self.taskbar_item(metric);
        for target in TaskbarColorTarget::ALL {
            let index = target.index();
            let state = &self.editor.colors[index];
            if force
                || (!state.read(cx).focus_handle(cx).is_focused(window)
                    && self.editor.color_errors[index].is_empty())
            {
                let value = color_string(target.value(&item));
                // Accept equivalent hex spelling without interrupting editing.
                if !state.read(cx).value().eq_ignore_ascii_case(&value) {
                    state.update(cx, |state, cx| state.set_value(value, window, cx));
                }
                if force {
                    self.editor.color_errors[index].clear();
                }
            }
        }
    }

    pub(super) fn taskbar_color_control(
        &self,
        target: TaskbarColorTarget,
        item: &TaskbarItem,
        cx: &Context<Self>,
    ) -> Div {
        let p = self.p;
        let index = target.index();
        let color = target.value(item);
        let default_label = target.default_label();
        column()
            .gap(px(4.))
            .child(
                row()
                    .gap(px(8.))
                    .child(
                        label(
                            match target {
                                TaskbarColorTarget::Text => "文本颜色",
                                TaskbarColorTarget::Background => "背景颜色",
                            },
                            p.muted,
                        )
                        .text_size(px(12.)),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new(("taskbar-color-reset", index))
                            .ghost()
                            .compact()
                            .h(px(22.))
                            .px(px(4.))
                            .child(
                                label(
                                    default_label,
                                    if color.is_none() {
                                        p.accent_text
                                    } else {
                                        p.muted
                                    },
                                )
                                .text_size(px(12.)),
                            )
                            .on_click(cx.listener(move |app, _, window, cx| {
                                app.act(
                                    Action::Display(DisplayAction::TaskbarColor(target, None)),
                                    window,
                                    cx,
                                )
                            })),
                    ),
            )
            .child(
                row()
                    .gap(px(8.))
                    .child(
                        div()
                            .size(px(24.))
                            .flex_none()
                            .rounded(px(4.))
                            .border_1()
                            .border_color(control_border(p))
                            .bg(color.map(rgb).unwrap_or_else(|| match target {
                                TaskbarColorTarget::Text => rgb(p.text),
                                TaskbarColorTarget::Background => rgba(0),
                            })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(self.text_input(&self.editor.colors[index])),
                    ),
            )
            .child(
                row().gap(px(4.)).children(
                    [
                        0xFFFFFF, 0x111827, 0x64748B, 0xEF4444, 0xF59E0B, 0x22C55E, 0x3B82F6,
                        0xA855F7,
                    ]
                    .into_iter()
                    .enumerate()
                    .map(|(choice, value)| {
                        div()
                            .id(("taskbar-color-swatch", index * 8 + choice))
                            .size(px(18.))
                            .flex_none()
                            .rounded(px(4.))
                            .border_1()
                            .border_color(if color == Some(value) {
                                rgb(p.accent)
                            } else {
                                control_border(p)
                            })
                            .bg(rgb(value))
                            .cursor_pointer()
                            .hover(move |d| d.border_color(rgb(p.accent)))
                            .tooltip(move |window, cx| {
                                Tooltip::new(format!("#{value:06X}")).build(window, cx)
                            })
                            .on_click(cx.listener(move |app, _, window, cx| {
                                app.act(
                                    Action::Display(DisplayAction::TaskbarColor(
                                        target,
                                        Some(value),
                                    )),
                                    window,
                                    cx,
                                )
                            }))
                    }),
                ),
            )
            .when(!self.editor.color_errors[index].is_empty(), |d| {
                d.child(label(self.editor.color_errors[index].clone(), p.amber).text_size(px(12.)))
            })
    }

    pub(super) fn taskbar_font_controls(&self, cx: &Context<Self>) -> Div {
        let content = self.display_content();
        let mut size_input = self.text_input(&self.editor.size);
        size_input.invalid = self.editor.size_error;
        let size_hint = if self.editor.size_error {
            "请输入 8–20 的数值，支持小数；未应用此值"
        } else {
            "8–20 px，支持小数，如 12.25、12.5；随系统缩放"
        };
        let mut weight_input = self.text_input(&self.editor.weight);
        weight_input.invalid = self.editor.weight_error;
        let weight_hint = if self.editor.weight_error {
            "请输入 100–900 的整数；未应用此值"
        } else {
            "100–900；↑↓／滚轮微调 1，Shift 微调 10；实际粗细取决于字体支持"
        };
        let fonts = self
            .taskbar_fonts
            .iter()
            .map(|family| {
                (
                    family.clone(),
                    Action::Display(DisplayAction::TaskbarFont(family.clone())),
                    *family == content.font_family,
                )
            })
            .collect();
        let sizes = (8..=20)
            .map(|size| {
                let value = size as f32;
                (
                    format!("{size} px"),
                    Action::Display(DisplayAction::TaskbarFontSize(value)),
                    value == content.font_size,
                )
            })
            .collect();
        let weights = TaskbarFontWeight::ALL
            .into_iter()
            .map(|weight| {
                (
                    weight.label().into(),
                    Action::Display(DisplayAction::TaskbarFontWeight(weight)),
                    weight == content.font_weight,
                )
            })
            .collect();
        let app = cx.weak_entity();
        let weight_bounds = self.editor.weight_bounds.clone();
        row()
            .gap(px(8.))
            .flex_none()
            .child(label("字体", self.p.muted).text_size(px(12.)))
            .child(div().w(px(150.)).flex_none().child(self.taskbar_picker(
                "taskbar-font",
                content.font_family.clone(),
                fonts,
                cx,
            )))
            .child(label("字号", self.p.muted).text_size(px(12.)))
            .child(
                row()
                    .w(px(108.))
                    .flex_none()
                    .gap(px(4.))
                    .child(
                        div()
                            .id("taskbar-font-size-input")
                            .w(px(72.))
                            .flex_none()
                            .tooltip(move |window, cx| Tooltip::new(size_hint).build(window, cx))
                            .child(size_input),
                    )
                    .child(
                        div()
                            .id("taskbar-font-size-presets")
                            .w(px(32.))
                            .flex_none()
                            .tooltip(|window, cx| {
                                Tooltip::new("选择常用字号（px）").build(window, cx)
                            })
                            .child(self.taskbar_picker(
                                "taskbar-font-size",
                                String::new(),
                                sizes,
                                cx,
                            )),
                    ),
            )
            .child(label("字重", self.p.muted).text_size(px(12.)))
            .child(
                div()
                    .id("taskbar-font-weight-input")
                    .relative()
                    .w(px(64.))
                    .flex_none()
                    .tooltip(move |window, cx| Tooltip::new(weight_hint).build(window, cx))
                    .on_hover(cx.listener(|app, hovered, _, _| {
                        if !hovered {
                            app.editor.weight_wheel_remainder = 0.;
                        }
                    }))
                    // TextInput consumes wheel events in bubble phase. Capture
                    // first, within the visible, unobscured numeric field.
                    .child(
                        canvas(
                            move |bounds, window, _| {
                                weight_bounds.set(bounds);
                                window.insert_hitbox(bounds, HitboxBehavior::Normal)
                            },
                            move |_, hitbox, window, _| {
                                window.on_mouse_event(
                                    move |event: &ScrollWheelEvent, phase, window, cx| {
                                        if phase != DispatchPhase::Capture
                                            || !hitbox.should_handle_scroll(window)
                                        {
                                            return;
                                        }
                                        let _ = app.update(cx, |app, cx| {
                                            if app.scroll_taskbar_weight(event, window, cx) {
                                                window.prevent_default();
                                                cx.stop_propagation();
                                            }
                                        });
                                    },
                                );
                            },
                        )
                        .absolute()
                        .size_full(),
                    )
                    .child(weight_input),
            )
            .child(div().w(px(96.)).flex_none().child(self.taskbar_picker(
                "taskbar-font-weight",
                content.font_weight.label().into(),
                weights,
                cx,
            )))
    }
}
