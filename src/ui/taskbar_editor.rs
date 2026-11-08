use super::*;

impl TrafficApp {
    fn taskbar_preview(&self, cx: &Context<Self>) -> Stateful<Div> {
        let p = self.p;
        self.editor.spacing.clear_occlusion();
        let card_bounds = self.editor.spacing.card_bounds.clone();
        let footer_bounds = self.editor.spacing.footer_bounds.clone();
        let spacing_preview = self.editor.spacing.preview_content();
        let content = spacing_preview.as_ref().unwrap_or(self.display_content());
        let frame = self
            .editor
            .drag
            .as_ref()
            .map(|drag| drag.frame.clone())
            .unwrap_or_else(|| self.content_frame(content));
        let scale = crate::taskbar_text::PREVIEW_SCALE;
        let bar_height = (frame.line_height * frame.rows.len() as f32 * scale + 4.).max(56.);
        let left = if self.config.taskbar_position_mode == TaskbarPositionMode::AutoAvoid {
            self.config.taskbar_auto_prefer_left
        } else {
            self.config.taskbar_left
        };
        let traffic = self.taskbar_readout(&frame, true, cx);
        let apps = div()
            .flex()
            .flex_1()
            .min_w(px(80.))
            .items_center()
            .justify_center()
            .child(label("应用区域", p.muted));
        let now = chrono::Local::now();
        let mut bar = row()
            .id("taskbar-preview-bar")
            .w_full()
            .min_w(px(frame.width * scale + 210.))
            .h(px(bar_height))
            .flex_none()
            .gap(px(16.))
            .px(px(12.))
            .rounded(px(8.))
            .border_1()
            .border_color(surface_border(p));
        let geometry = self.data.taskbar_geometry;
        bar = if self.config.taskbar_position_mode == TaskbarPositionMode::Manual
            && geometry.ready
            && geometry.max_x > 0
        {
            let ratio = (self.manual_x(geometry) as f32 / geometry.max_x as f32).clamp(0., 1.);
            let region = |weight: f32, visible: bool| {
                let mut region = div()
                    .flex()
                    .flex_basis(px(0.))
                    .min_w_0()
                    .items_center()
                    .justify_center()
                    .when(visible, |d| d.child(label("应用区域", p.muted)));
                region.style().flex_grow = Some(weight);
                region
            };
            bar.when(ratio > 0., |d| d.child(region(ratio, ratio >= 0.5)))
                .child(traffic)
                .when(ratio < 1., |d| d.child(region(1. - ratio, ratio < 0.5)))
        } else if left {
            bar.child(traffic).child(apps)
        } else {
            bar.child(apps).child(traffic)
        };
        bar = bar
            .child(div().w(px(1.)).h(px(28.)).flex_none().bg(surface_border(p)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_end()
                    .w(px(56.))
                    .flex_none()
                    .font_family("Segoe UI")
                    .child(label(now.format("%H:%M").to_string(), p.text))
                    .child(label(now.format("%m/%d").to_string(), p.muted)),
            );
        let mut background = rgb(p.text);
        background.a = if p.dark { 0.04 } else { 0.025 };
        let hover_bounds = card_bounds;
        let hover_app = cx.weak_entity();
        let has_spacing_overlay = self.editor.editing && self.editor.drag.is_none();
        column()
            .id("taskbar-preview-card")
            .flex_none()
            .relative()
            .w_full()
            // Negative child margins collapse this auto-height flex card in
            // Taffy. Use a small base gap and positive spacing where needed.
            .gap(px(2.))
            .p(px(12.))
            .pb(px(8.))
            .rounded(px(12.))
            .border_1()
            .border_color(surface_border(p))
            .bg(background)
            .child(
                row()
                    .mb(px(8.))
                    .justify_between()
                    .flex_wrap()
                    .child(label("布局预览", p.text).font_weight(FontWeight::MEDIUM))
                    .child(
                        row()
                            .gap(px(12.))
                            .child(
                                row()
                                    .gap(px(5.))
                                    .child(div().size(px(5.)).rounded_full().bg(rgb(p.accent)))
                                    .child(label("即时预览", p.accent_text).text_size(px(12.))),
                            )
                            .child(
                                Button::new("taskbar-edit-toggle")
                                    .child(label(
                                        if self.editor.editing {
                                            "收起编辑"
                                        } else {
                                            "编辑布局"
                                        },
                                        p.accent_text,
                                    ))
                                    .icon(
                                        Icon::default()
                                            .path(if self.editor.editing {
                                                "icons/chevron-down.svg"
                                            } else {
                                                "icons/edit.svg"
                                            })
                                            .size(px(14.))
                                            .text_color(rgb(p.accent_text)),
                                    )
                                    .custom(
                                        ButtonCustomVariant::new(cx)
                                            .color(rgb(p.tint).into())
                                            .foreground(rgb(p.accent_text).into())
                                            .border(state_surface(p).into())
                                            .hover(rgb(p.tint).blend(state_surface(p)).into())
                                            .active(rgb(p.tint).blend(state_surface(p)).into()),
                                    )
                                    .on_click(cx.listener(|app, _, window, cx| {
                                        app.act(
                                            Action::Display(DisplayAction::TaskbarEditing),
                                            window,
                                            cx,
                                        )
                                    })),
                            ),
                    ),
            )
            .child(
                div()
                    .relative()
                    .w_full()
                    .h(px(bar_height + 14.))
                    .flex_none()
                    .child(
                        div()
                            .id("taskbar-preview-scroll")
                            .size_full()
                            .pb(px(14.))
                            .overflow_x_scroll()
                            .track_scroll(&self.editor.preview_scroll)
                            .child(bar),
                    )
                    .child(controls::scroll_overlay(Scrollbar::horizontal(
                        &self.editor.preview_scrollbar,
                        &self.editor.preview_scroll,
                    ))),
            )
            .children(self.taskbar_drop_candidate().map(|candidate| {
                let frame = self.content_frame(&candidate);
                column()
                    .mt(px(8.))
                    .gap(px(4.))
                    .child(label("落位预览 · 松手后生效", p.accent_text).text_size(px(12.)))
                    .child(
                        div()
                            .relative()
                            .h(px(frame.rows.len() as f32 * frame.line_height * scale + 14.))
                            .w_full()
                            .child(
                                div()
                                    .id("taskbar-candidate-scroll")
                                    .size_full()
                                    .pb(px(14.))
                                    .overflow_x_scroll()
                                    .track_scroll(&self.editor.candidate_scroll)
                                    .child(self.taskbar_readout(&frame, false, cx)),
                            )
                            .child(controls::scroll_overlay(Scrollbar::horizontal(
                                &self.editor.candidate_scrollbar,
                                &self.editor.candidate_scroll,
                            ))),
                    )
            }))
            .child(
                row()
                    .relative()
                    .h(px(18.))
                    .flex_none()
                    .justify_end()
                    .gap(px(12.))
                    .when(self.editor.editing, |d| {
                        d.child(self.taskbar_spacing_hint().flex_1().min_w_0())
                    })
                    .child(
                        self.editor
                            .spacing
                            .label(
                                format!(
                                    "{} 行 · {} 项",
                                    content.rows.len(),
                                    content.rows.iter().map(Vec::len).sum::<usize>()
                                ),
                                p.muted,
                            )
                            .text_size(px(12.))
                            .flex_none(),
                    )
                    .child(
                        canvas(move |rect, _, _| footer_bounds.set(rect), |_, _, _, _| {})
                            .absolute()
                            .top_0()
                            .left_0()
                            .bottom_0()
                            .right_0(),
                    ),
            )
            .when(
                self.config.taskbar
                    && ["失败", "无法", "空位", "不支持", "等待 Explorer", "超时"]
                        .iter()
                        .any(|word| self.data.taskbar_status.contains(word)),
                |d| {
                    d.child(
                        label(self.data.taskbar_status.clone(), p.amber)
                            .mt(px(8.))
                            .text_size(px(12.))
                            .whitespace_normal(),
                    )
                },
            )
            .when(has_spacing_overlay, |d| {
                d.child(taskbar_spacing::SpacingOverlay::new(self, &frame, cx))
            })
            .child(
                canvas(
                    |_, _, _| {},
                    move |_, _, window, _| {
                        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                            if phase != DispatchPhase::Capture {
                                return;
                            }
                            let _ = hover_app.update(cx, |app, cx| {
                                let hovered = hover_bounds.get().contains(&event.position);
                                if app.editor.spacing.preview_is_hovered() != hovered {
                                    app.editor.spacing.set_preview_hovered(hovered);
                                    cx.notify();
                                }
                            });
                        });
                    },
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
    }

    fn taskbar_toolbar_group(&self, index: usize, child: impl IntoElement) -> Div {
        let groups = self.editor.toolbar_groups.clone();
        div()
            .flex()
            .flex_none()
            .child(child)
            .on_children_prepainted(move |bounds, _, _| {
                if let Some(bounds) = bounds.first() {
                    let mut measured = groups.get();
                    measured[index] = *bounds;
                    groups.set(measured);
                }
            })
    }

    fn taskbar_toolbar(&self, cx: &Context<Self>) -> Div {
        if self.display_tab == DisplayTab::Floating {
            return self.floating_toolbar(cx);
        }
        let p = self.p;
        let content = self.display_content();
        let layout = self.taskbar_toolbar_group(
            0,
            row()
                .flex_none()
                .gap(px(3.))
                .child(label("排列", p.muted).text_size(px(12.)))
                .child(
                    self.button(
                        "taskbar-single-row",
                        "单行",
                        content.rows.len() == 1,
                        Action::Display(DisplayAction::TaskbarRows(1)),
                        cx,
                    )
                    .min_w(px(40.))
                    .px(px(8.))
                    .h(px(30.)),
                )
                .child(
                    self.button(
                        "taskbar-double-row",
                        "双行",
                        content.rows.len() == 2,
                        Action::Display(DisplayAction::TaskbarRows(2)),
                        cx,
                    )
                    .min_w(px(40.))
                    .px(px(8.))
                    .h(px(30.)),
                ),
        );
        let fonts = self.taskbar_toolbar_group(1, self.taskbar_font_controls(cx));
        let spacing = self.taskbar_toolbar_group(
            2,
            row()
                .flex_none()
                .gap(px(6.))
                .child(label("段间距", p.muted).text_size(px(12.)))
                .child(div().w(px(70.)).child(Slider::new(&self.editor.gap)))
                .child(label(format!("{} px", content.gap), p.muted).text_size(px(12.))),
        );
        let reset = self.taskbar_toolbar_group(
            3,
            self.button(
                "taskbar-reset-content",
                "恢复默认",
                false,
                Action::Display(DisplayAction::TaskbarReset),
                cx,
            )
            .flex_none()
            .min_w(px(64.))
            .px(px(8.))
            .h(px(30.)),
        );
        let compact = self.editor.toolbar_compact;
        let toolbar = if compact {
            column()
                .w_full()
                .min_w_0()
                .gap(px(10.))
                .child(
                    row()
                        .w_full()
                        .gap(px(10.))
                        .child(layout)
                        .child(spacing)
                        .child(div().flex_1())
                        .child(reset),
                )
                .child(fonts)
        } else {
            row()
                .w_full()
                .min_w_0()
                .flex_wrap()
                .gap(px(10.))
                .child(layout)
                .child(fonts)
                .child(spacing)
                .child(div().flex_1())
                .child(reset)
        };
        let groups = self.editor.toolbar_groups.clone();
        let view = cx.entity().downgrade();
        let target = self.display_tab;
        div()
            .w_full()
            .min_w_0()
            .py(px(12.))
            .child(toolbar)
            .on_children_prepainted(move |bounds, window, _| {
                let Some(available) = bounds.first() else {
                    return;
                };
                let measured = groups.get();
                if measured.iter().any(|group| group.size.width <= px(0.)) {
                    return;
                }
                let required = measured
                    .iter()
                    .fold(px(40.), |total, group| total + group.size.width);
                let next = required > available.size.width + px(0.5);
                if next != compact {
                    let view = view.clone();
                    window.on_next_frame(move |_, cx| {
                        let _ = view.update(cx, |app, cx| {
                            if app.display_tab == target && app.editor.toolbar_compact != next {
                                app.editor.toolbar_compact = next;
                                cx.notify();
                            }
                        });
                    });
                }
            })
    }

    fn taskbar_library(&self, cx: &Context<Self>) -> Div {
        let p = self.p;
        let content = self.display_content();
        let count = content.rows.iter().map(Vec::len).sum::<usize>();
        column()
            .w(px(192.))
            .flex_none()
            .gap(px(6.))
            .py(px(12.))
            .pr(px(14.))
            .child(
                row()
                    .justify_between()
                    .child(label("显示项目", p.text).font_weight(FontWeight::BOLD))
                    .child(label(format!("{count} / 13"), p.muted).text_size(px(12.))),
            )
            .children(
                ["网络", "流量", "系统", "显卡", "磁盘"]
                    .into_iter()
                    .map(|group| {
                        stretch()
                            .items_start()
                            .gap(px(6.))
                            .child(
                                label(group, p.muted)
                                    .text_size(px(12.))
                                    .w(px(26.))
                                    .pt(px(5.))
                                    .flex_none(),
                            )
                            .child(
                                column().flex_1().min_w_0().gap(px(2.)).children(
                                    Metric::ALL
                                        .into_iter()
                                        .filter(|metric| metric.group() == group)
                                        .map(|metric| {
                                            let added = content.item(metric).is_some();
                                            let drag = TaskbarDrag {
                                                target: self.display_tab,
                                                metric,
                                                text: self.taskbar_item(metric).text(
                                                    &self.taskbar_item(metric).value(&self.data),
                                                    true,
                                                ),
                                                palette: p,
                                                font_family: content.font_family.clone(),
                                                font_size: content.font_size
                                                    * crate::taskbar_text::PREVIEW_SCALE,
                                                font_weight: content.font_weight,
                                                text_color: self.taskbar_item(metric).text_color,
                                                background_color: self
                                                    .taskbar_item(metric)
                                                    .background_color,
                                            };
                                            self.button(
                                                format!("taskbar-library-{metric:?}"),
                                                metric.name(),
                                                self.editor.selected == Some(metric),
                                                Action::Display(DisplayAction::TaskbarSelect(
                                                    metric,
                                                )),
                                                cx,
                                            )
                                            .w_full()
                                            .min_w_0()
                                            .h(px(26.))
                                            .justify_start()
                                            .px(px(8.))
                                            .rounded(px(6.))
                                            .text_size(px(12.))
                                            .border_1()
                                            .bg(controls::project_surface(
                                                p,
                                                added,
                                                self.editor.selected == Some(metric),
                                            ))
                                            .text_color(rgb(
                                                if added || self.editor.selected == Some(metric) {
                                                    p.accent_text
                                                } else {
                                                    p.muted
                                                },
                                            ))
                                            .border_color(if self.editor.selected == Some(metric) {
                                                rgb(p.accent)
                                            } else {
                                                surface_border(p)
                                            })
                                            .child(div().flex_1())
                                            .when(added, |d| {
                                                d.child(
                                                    Icon::default()
                                                        .path("icons/check.svg")
                                                        .size(px(12.))
                                                        .text_color(rgb(p.accent_text)),
                                                )
                                            })
                                            .on_drag(
                                                drag,
                                                {
                                                    let app = cx.entity().downgrade();
                                                    move |drag, _, window, cx| {
                                                        let _ = app.update(cx, |app, cx| {
                                                            if app.display_tab != drag.target {
                                                                return;
                                                            }
                                                            app.start_taskbar_drag(
                                                                drag.metric,
                                                                window,
                                                                cx,
                                                            )
                                                        });
                                                        cx.new(|_| drag.clone())
                                                    }
                                                },
                                            )
                                        }),
                                ),
                            )
                    }),
            )
    }
    fn taskbar_inspector(&self, metric: Metric, cx: &Context<Self>) -> Div {
        let p = self.p;
        let content = self.display_content();
        let item = self.taskbar_item(metric);
        let position = content.position(metric);
        let added = position.is_some();
        let count = content.rows.iter().map(Vec::len).sum::<usize>();
        let formats = item.metric.formats();
        let current_format = item.effective_format();
        let format_label = formats
            .iter()
            .find(|(format, _)| *format == current_format)
            .map(|(_, name)| *name)
            .unwrap_or(formats[0].1);
        let mut normalized = item.clone();
        normalized.normalize_legacy();
        let precision = normalized.precision;
        let format = self.taskbar_picker(
            "taskbar-format",
            format_label.into(),
            formats
                .into_iter()
                .map(|(format, text)| {
                    (
                        text.into(),
                        Action::Display(DisplayAction::TaskbarValueFormat(format)),
                        current_format == format,
                    )
                })
                .collect(),
            cx,
        );
        let decimals = self.taskbar_picker(
            "taskbar-precision",
            precision
                .map(|v| format!("{v} 位小数"))
                .unwrap_or_else(|| "自动".into()),
            [None, Some(0), Some(1), Some(2)]
                .into_iter()
                .map(|value| {
                    (
                        value
                            .map(|v| format!("{v} 位小数"))
                            .unwrap_or_else(|| "自动".into()),
                        Action::Display(DisplayAction::TaskbarPrecision(value)),
                        value == precision,
                    )
                })
                .collect(),
            cx,
        );
        let capacity = item.metric.is_rate()
            || item.metric.group() == "流量"
            || (item.metric.is_memory() && current_format != ValueFormat::Percent);
        let unit = self.taskbar_picker(
            "taskbar-unit",
            ByteUnit::ALL
                .iter()
                .find(|(unit, _)| *unit == item.effective_unit())
                .unwrap()
                .1
                .into(),
            item.metric
                .unit_choices()
                .iter()
                .map(|&(unit, text)| {
                    (
                        text.into(),
                        Action::Display(DisplayAction::TaskbarUnit(unit)),
                        unit == item.effective_unit(),
                    )
                })
                .collect(),
            cx,
        );
        let unit_style = Button::new("taskbar-unit-style")
            .w(px(50.))
            .h(px(34.))
            .flex_none()
            .px(px(8.))
            .rounded(px(8.))
            .border_1()
            .border_color(control_border(p))
            .custom(
                ButtonCustomVariant::new(cx)
                    .color(control_surface(p).into())
                    .hover(state_surface(p).into())
                    .active(state_surface(p).into()),
            )
            .child(label(item.unit_style.label(), p.text).text_size(px(13.)))
            .on_click(cx.listener(|app, _, window, cx| {
                app.act(Action::Display(DisplayAction::TaskbarUnitStyle), window, cx)
            }));
        let mut fields = column()
            .flex_1()
            .min_w(px(280.))
            .gap(px(12.))
            .child(label("内容与格式", p.text).font_weight(FontWeight::MEDIUM))
            .child(
                column()
                    .gap(px(6.))
                    .child(
                        row()
                            .justify_between()
                            .child(label("前缀文案", p.muted).text_size(px(12.)))
                            .child(label("留空仅显示数值", p.muted).text_size(px(11.))),
                    )
                    .child(self.text_input(&self.editor.label)),
            )
            .when(item.metric.is_memory(), |d| {
                d.child(
                    column()
                        .gap(px(6.))
                        .child(label("显示内容", p.muted).text_size(px(12.)))
                        .child(format),
                )
            })
            .child(
                stretch()
                    .gap(px(10.))
                    .child(
                        column()
                            .flex_1()
                            .min_w_0()
                            .gap(px(6.))
                            .child(label("显示单位", p.muted).text_size(px(12.)))
                            .when(capacity, |d| {
                                d.child(
                                    row()
                                        .gap(px(6.))
                                        .min_w_0()
                                        .child(div().flex_1().min_w_0().child(unit))
                                        .child(unit_style),
                                )
                            })
                            .when(!capacity, |d| {
                                d.child(
                                    self.taskbar_picker(
                                        "taskbar-percent-unit",
                                        if item.hide_percent { "无" } else { "%" }.into(),
                                        [false, true]
                                            .into_iter()
                                            .map(|hidden| {
                                                (
                                                    if hidden { "无（隐藏 %）" } else { "%" }
                                                        .into(),
                                                    Action::Display(
                                                        DisplayAction::TaskbarHidePercent(hidden),
                                                    ),
                                                    hidden == item.hide_percent,
                                                )
                                            })
                                            .collect(),
                                        cx,
                                    ),
                                )
                            }),
                    )
                    .child(
                        column()
                            .w(px(112.))
                            .flex_none()
                            .gap(px(6.))
                            .child(label("小数位", p.muted).text_size(px(12.)))
                            .child(decimals),
                    ),
            );
        if item.metric.group() == "显卡" || item.metric.group() == "磁盘" {
            let gpu = item.metric.group() == "显卡";
            let auto = if gpu {
                "自动选择显卡"
            } else {
                "所有磁盘"
            };
            let mut devices = vec![(String::new(), auto.to_string())];
            if gpu {
                devices.extend(
                    self.data
                        .system
                        .gpus
                        .iter()
                        .map(|device| (device.id.clone(), device.name.clone())),
                );
            } else {
                devices.extend(
                    self.data
                        .system
                        .disks
                        .iter()
                        .filter(|device| !device.id.is_empty())
                        .map(|device| (device.id.clone(), device.name.clone())),
                );
            }
            if !devices.iter().any(|(id, _)| *id == item.device) {
                devices.push((item.device.clone(), "原设备暂不可用".into()));
            }
            let current = devices
                .iter()
                .find(|(id, _)| *id == item.device)
                .unwrap()
                .1
                .clone();
            let choices = devices
                .into_iter()
                .map(|(id, name)| {
                    let selected = id == item.device;
                    (
                        name,
                        Action::Display(DisplayAction::TaskbarDevice(id)),
                        selected,
                    )
                })
                .collect();
            fields = fields.child(
                column()
                    .gap(px(6.))
                    .child(
                        label(if gpu { "监测显卡" } else { "监测磁盘" }, p.muted)
                            .text_size(px(12.)),
                    )
                    .child(self.taskbar_picker("taskbar-device", current, choices, cx)),
            );
        }
        let mut scope = item.metric.scope().to_string();
        if item.metric == Metric::Vram && item.gpu(&self.data).is_some_and(|gpu| gpu.shared_memory)
        {
            scope = "所选显卡 · 当前使用共享显存".into();
        }
        let metric = item.metric;
        let movement = [
            (MoveDirection::Left, "左移", "icons/arrow-left.svg"),
            (MoveDirection::Right, "右移", "icons/arrow-right.svg"),
            (MoveDirection::Up, "上移", "icons/arrow-up.svg"),
            (MoveDirection::Down, "下移", "icons/arrow-down.svg"),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, (direction, text, icon))| {
            let disabled = content.directional_target(metric, direction).is_none();
            Button::new(("taskbar-move", index))
                .ghost()
                .compact()
                .px(px(4.))
                .h(px(30.))
                .child(
                    row()
                        .gap(px(3.))
                        .child(
                            Icon::default()
                                .path(icon)
                                .size(px(12.))
                                .text_color(rgb(p.text)),
                        )
                        .child(label(text, p.text).text_size(px(12.))),
                )
                .disabled(disabled)
                .when(disabled, |b| b.opacity(0.4))
                .on_click(cx.listener(move |app, _, window, cx| {
                    app.act(
                        Action::Display(DisplayAction::TaskbarMove(direction)),
                        window,
                        cx,
                    )
                }))
        });
        let membership = Button::new("taskbar-item-membership")
            .h(px(30.))
            .px(px(8.))
            .child(
                label(
                    if added { "移除" } else { "添加" },
                    if added { p.amber } else { p.accent_text },
                )
                .text_size(px(12.)),
            )
            .custom(
                ButtonCustomVariant::new(cx)
                    .color(if added { rgba(0) } else { state_surface(p) }.into())
                    .hover(state_surface(p).into())
                    .active(state_surface(p).into()),
            )
            .disabled(added && count == 1)
            .when(added && count == 1, |b| b.opacity(0.4))
            .on_click(cx.listener(move |app, _, window, cx| {
                app.act(
                    if added {
                        Action::Display(DisplayAction::TaskbarRemove)
                    } else {
                        Action::Display(DisplayAction::TaskbarAdd(metric))
                    },
                    window,
                    cx,
                )
            }));
        column()
            .flex_1()
            .min_w(px(320.))
            .gap(px(12.))
            .py(px(14.))
            .pl(px(16.))
            .border_l_1()
            .border_color(surface_border(p))
            .child(
                row()
                    .gap(px(12.))
                    .child(
                        column()
                            .gap(px(2.))
                            .flex_1()
                            .min_w_0()
                            .child(label(item.metric.name(), p.text).font_weight(FontWeight::BOLD))
                            .child(
                                label(
                                    position.map_or_else(
                                        || "未添加 · 设置将在添加后生效".to_string(),
                                        |(row, column)| {
                                            format!(
                                                "已添加 · 第 {} 行 · 第 {} 列",
                                                row + 1,
                                                column + 1
                                            )
                                        },
                                    ),
                                    p.muted,
                                )
                                .text_size(px(11.)),
                            ),
                    )
                    .child(membership),
            )
            .child(
                row()
                    .gap(px(8.))
                    .flex_wrap()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(150.))
                            .px(px(8.))
                            .py(px(5.))
                            .rounded(px(8.))
                            .bg(control_surface(p))
                            .id("taskbar-item-example")
                            .overflow_x_scroll()
                            .child(self.taskbar_readout(&self.example_frame(&item), false, cx)),
                    )
                    .child(row().gap(px(0.)).flex_none().children(movement)),
            )
            .child(settings_separator(p))
            .child(
                stretch().gap(px(20.)).flex_wrap().child(fields).child(
                    column()
                        .flex_1()
                        .min_w(px(220.))
                        .gap(px(12.))
                        .child(label("颜色", p.text).font_weight(FontWeight::MEDIUM))
                        .child(self.taskbar_color_control(TaskbarColorTarget::Text, &item, cx))
                        .child(self.taskbar_color_control(
                            TaskbarColorTarget::Background,
                            &item,
                            cx,
                        )),
                ),
            )
            .child(div().flex_1().min_h_0())
            .child(
                label(scope, p.muted)
                    .text_size(px(11.))
                    .whitespace_normal()
                    .flex_none(),
            )
    }
    pub(super) fn taskbar_editor(&self, cx: &Context<Self>) -> Div {
        let p = self.p;
        let library = self.taskbar_library(cx);
        let inspector = self.editor.selected.map_or_else(
            || {
                column()
                    .flex_1()
                    .min_w(px(320.))
                    .py(px(14.))
                    .px(px(16.))
                    .border_l_1()
                    .border_color(surface_border(p))
                    .items_center()
                    .justify_center()
                    .child(label("选择一个项目查看设置", p.muted))
            },
            |metric| self.taskbar_inspector(metric, cx),
        );
        column()
            .key_context("TaskbarEditor")
            .gap(px(0.))
            .child(self.taskbar_toolbar(cx))
            .child(settings_separator(p))
            .child(stretch().gap(px(0.)).child(library).child(inspector))
    }

    pub(super) fn taskbar_settings(&self, cx: &Context<Self>) -> Div {
        let p = self.p;
        let card_bounds = self.editor.spacing.card_bounds.clone();
        column()
            .on_children_prepainted(move |bounds, _, _| {
                // Read the preview's allocated box, not its overflowing content.
                if let Some(bounds) = bounds.get(1) {
                    card_bounds.set(*bounds);
                }
            })
            .w_full()
            .flex_1()
            .min_h_0()
            .gap(px(0.))
            .child(
                self.toggle(
                    "taskbar-on",
                    "显示任务栏信息",
                    "",
                    self.config.taskbar,
                    Action::Config("taskbar"),
                    cx,
                )
                .flex_none(),
            )
            .child(self.taskbar_preview(cx))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .id("taskbar-settings-scroll")
                            .h_full()
                            .pr(px(14.))
                            .track_scroll(&self.editor.settings_scroll)
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .child(scroll_fade::FadingContent::new(
                                column()
                                    .gap(px(0.))
                                    .when(self.editor.editing, |d| d.child(self.taskbar_editor(cx)))
                                    .child(settings_separator(p).mt(px(16.)))
                                    .child(self.taskbar_position_controls(cx).pt(px(16.))),
                                &self.editor.settings_scroll,
                            )),
                    )
                    .child(scroll_fade::bottom_shade(&self.editor.settings_scroll, p))
                    .child(controls::scroll_overlay(Scrollbar::vertical(
                        &self.editor.settings_scrollbar,
                        &self.editor.settings_scroll,
                    ))),
            )
    }
}
