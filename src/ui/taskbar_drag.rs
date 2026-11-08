use super::*;
use crate::taskbar_text::ContentFrame;

#[derive(Clone)]
pub(super) struct TaskbarDrag {
    pub(super) target: DisplayTab,
    pub(super) metric: Metric,
    pub(super) text: String,
    pub(super) palette: Palette,
    pub(super) font_family: String,
    pub(super) font_size: f32,
    pub(super) font_weight: TaskbarFontWeight,
    pub(super) text_color: Option<u32>,
    pub(super) background_color: Option<u32>,
}

pub(super) struct TaskbarDragSession {
    pub(super) item: TaskbarItem,
    pub(super) original: TaskbarContent,
    pub(super) frame: crate::taskbar_text::ContentFrame,
    pub(super) scroll_direction: f32,
    pub(super) scroll_y: f32,
    pub(super) generation: u64,
}

impl Render for TaskbarDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(px(8.))
            .py(px(3.))
            .rounded(px(6.))
            .bg(rgb(self.background_color.unwrap_or(self.palette.bg)))
            .border_1()
            .border_color(rgb(self.palette.accent))
            .font_family(self.font_family.clone())
            .font_weight(FontWeight(f32::from(self.font_weight.value())))
            .text_size(px(self.font_size))
            .text_color(rgb(self.text_color.unwrap_or(self.palette.text)))
            .child(self.text.clone())
    }
}
impl TrafficApp {
    pub(super) fn drop_taskbar_item(
        &mut self,
        metric: Metric,
        row: usize,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_taskbar_drop(metric, DropIntent::Insert { row, index }, window, cx);
    }
    pub(super) fn apply_taskbar_drop(
        &mut self,
        metric: Metric,
        intent: DropIntent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.editor.editing {
            return;
        }
        let item = self.taskbar_item(metric);
        let Some(next) = self.display_content().placed(&item, intent) else {
            return;
        };
        if &next != self.display_content() {
            *self.display_content_mut() = next;
            self.editor.drafts.remove(&metric);
            self.editor.feedback = Some((metric, Instant::now()));
            self.save_display_content(cx);
        }
        self.editor.drag = None;
        self.editor.drop = None;
        self.select_taskbar_item(metric, window, cx);
        cx.notify();
    }

    pub(super) fn start_taskbar_drag(
        &mut self,
        metric: Metric,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.editor.editing {
            return;
        }
        self.close_picker(window, cx);
        self.cancel_taskbar_spacing(cx);
        self.taskbar_position.hold_generation += 1;
        let generation = self.taskbar_position.hold_generation;
        self.editor.drop = None;
        self.editor.drag = Some(TaskbarDragSession {
            item: self.taskbar_item(metric),
            original: self.display_content().clone(),
            frame: self.content_frame(self.display_content()),
            scroll_direction: 0.,
            scroll_y: 0.,
            generation,
        });
        window.blur();
        cx.notify();
        // Keep scrolling while the pointer rests near an edge, even without
        // further native mouse-move events. A generation prevents stale timers.
        cx.spawn_in(window, async move |this, cx| {
            loop {
                Timer::after(Duration::from_millis(30)).await;
                let keep_going = this
                    .update_in(cx, |app, window, cx| {
                        let Some(drag) = app
                            .editor
                            .drag
                            .as_ref()
                            .filter(|drag| drag.generation == generation)
                        else {
                            return false;
                        };
                        if !cx.has_active_drag() {
                            app.cancel_taskbar_drag(window, cx);
                            return false;
                        }
                        let direction = drag.scroll_direction;
                        let vertical = drag.scroll_y;
                        if direction != 0. || vertical != 0. {
                            let mut offset = app.editor.preview_scroll.offset();
                            offset.x += px(direction * 6.);
                            offset.y += px(vertical * 6.);
                            app.editor.preview_scroll.set_offset(offset);
                            app.update_taskbar_drag(window.mouse_position(), cx);
                            cx.notify();
                        }
                        true
                    })
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
        })
        .detach();
    }

    pub(super) fn cancel_taskbar_drag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor.drag.take().is_some() {
            self.editor.drop = None;
            cx.stop_active_drag(window);
            cx.notify();
        }
    }

    pub(super) fn update_taskbar_drag(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let scale = self.preview_scale();
        let row_gap = self.display_row_gap();
        let floating = self.display_tab == DisplayTab::Floating;
        let Some(drag) = self.editor.drag.as_mut() else {
            return;
        };
        let viewport = self.editor.preview_scroll.bounds();
        drag.scroll_direction = if viewport.contains(&position) {
            if position.x < viewport.origin.x + px(24.) {
                1.
            } else if position.x > viewport.right() - px(24.) {
                -1.
            } else {
                0.
            }
        } else {
            0.
        };
        drag.scroll_y = if floating && viewport.contains(&position) {
            if position.y < viewport.top() + px(24.) {
                1.
            } else if position.y > viewport.bottom() - px(24.) {
                -1.
            } else {
                0.
            }
        } else {
            0.
        };
        let bounds = self.editor.traffic_bounds.get();
        let target = if bounds.contains(&position) && viewport.contains(&position) {
            let row = (((position.y - bounds.origin.y)
                / px((drag.frame.line_height + row_gap) * scale))
            .floor() as usize)
                .min(drag.frame.rows.len() - 1);
            let x = f32::from(position.x - bounds.origin.x) / scale;
            let cells = &drag.frame.rows[row];
            let added = drag.original.item(drag.item.metric).is_some();
            let mut intent = DropIntent::Insert {
                row,
                index: cells.len(),
            };
            for (index, cell) in cells.iter().enumerate() {
                if x < cell.x {
                    intent = DropIntent::Insert { row, index };
                    break;
                }
                if x <= cell.x + cell.width {
                    intent = DropIntent::over_cell(
                        cell.metric,
                        row,
                        index,
                        (x - cell.x) / cell.width,
                        added,
                    );
                    break;
                }
            }
            Some(intent)
        } else {
            None
        };
        if target != self.editor.drop {
            self.editor.drop = target;
            cx.notify();
        }
    }

    pub(super) fn taskbar_drop_candidate(&self) -> Option<TaskbarContent> {
        let drag = self.editor.drag.as_ref()?;
        drag.original.placed(&drag.item, self.editor.drop?)
    }

    pub(super) fn taskbar_drag_hint(&self) -> String {
        if let Some(drag) = &self.editor.drag {
            return match self.editor.drop {
                Some(DropIntent::Swap(target)) if target == drag.item.metric => {
                    "保持当前位置 · 松手不改变布局".into()
                }
                Some(DropIntent::Swap(target)) if target != drag.item.metric => {
                    format!("松手与 {} 交换", target.name())
                }
                Some(_) => self
                    .taskbar_drop_candidate()
                    .and_then(|layout| layout.position(drag.item.metric))
                    .map_or_else(
                        || "拖到指标中央交换，拖到边缘或间隙插入".into(),
                        |(row, column)| format!("松手插入第 {} 行 · 第 {} 列", row + 1, column + 1),
                    ),
                None => "拖到指标中央交换，拖到边缘或间隙插入 · Esc 取消".into(),
            };
        }
        if let Some((metric, time)) = self.editor.feedback
            && time.elapsed() < Duration::from_millis(1600)
            && let Some((row, column)) = self.display_content().position(metric)
        {
            return format!(
                "{} 已放到第 {} 行 · 第 {} 列",
                metric.name(),
                row + 1,
                column + 1
            );
        }
        "拖动排序".into()
    }

    pub(super) fn taskbar_readout(
        &self,
        frame: &ContentFrame,
        interactive: bool,
        cx: &Context<Self>,
    ) -> Div {
        let p = self.p;
        let scale = self.preview_scale();
        let row_height = frame.line_height * scale;
        let bounds = self.editor.traffic_bounds.clone();
        let mut readout = column()
            .relative()
            .gap(px(self.display_row_gap() * scale))
            .flex_none()
            .w(px(frame.width * scale + if interactive { 24. } else { 0. }))
            .font_family(frame.font_family.clone())
            .font_weight(FontWeight(f32::from(frame.font_weight.value())))
            .children(frame.rows.iter().enumerate().map(|(row_index, cells)| {
                let mut preview_row = div()
                    .relative()
                    .h(px(row_height))
                    .w_full()
                    .flex_none()
                    .children(cells.iter().map(|cell| {
                        let metric = cell.metric;
                        let source = interactive
                            && self
                                .editor
                                .drag
                                .as_ref()
                                .is_some_and(|drag| drag.item.metric == metric);
                        let selected = interactive
                            && self.editor.editing
                            && self.editor.selected == Some(metric);
                        let target =
                            interactive && self.editor.drop == Some(DropIntent::Swap(metric));
                        let feedback = interactive
                            && self.editor.feedback.is_some_and(|(item, time)| {
                                item == metric && time.elapsed() < Duration::from_millis(1600)
                            });
                        let drag = TaskbarDrag {
                            target: self.display_tab,
                            metric,
                            text: cell.text.clone(),
                            palette: p,
                            font_family: frame.font_family.clone(),
                            font_size: frame.font_size * scale,
                            font_weight: frame.font_weight,
                            text_color: cell.text_color,
                            background_color: cell.background_color,
                        };
                        div()
                            .id((
                                if interactive {
                                    "taskbar-cell"
                                } else {
                                    "taskbar-candidate-cell"
                                },
                                metric as usize,
                            ))
                            .absolute()
                            .left(px(cell.x * scale))
                            .top_0()
                            .w(px(cell.width * scale))
                            .h(px(row_height))
                            .key_context("TaskbarEditor")
                            .focusable()
                            .tab_stop(interactive && self.editor.editing)
                            .flex()
                            .items_center()
                            .rounded(px(4.))
                            .border_1()
                            .border_color(if target || selected || feedback {
                                rgb(p.accent)
                            } else {
                                rgba(0)
                            })
                            .bg(if let Some(background) = cell.background_color {
                                rgb(background)
                            } else if target || selected || feedback {
                                state_surface(p)
                            } else {
                                rgba(0)
                            })
                            .when(source, |d| d.opacity(0.35))
                            .children(display_editor::cell_segments(
                                cell,
                                frame,
                                scale,
                                p,
                                interactive.then_some(&self.editor.spacing),
                            ))
                            .when(interactive && self.editor.editing, |d| {
                                let app = cx.entity().downgrade();
                                d.cursor_move()
                                    .on_click(cx.listener(move |app, _, window, cx| {
                                        app.act(
                                            Action::Display(DisplayAction::TaskbarSelect(metric)),
                                            window,
                                            cx,
                                        )
                                    }))
                                    .on_key_down(cx.listener(
                                        move |app, event: &KeyDownEvent, window, cx| {
                                            if matches!(
                                                event.keystroke.key.as_str(),
                                                "enter" | "space"
                                            ) {
                                                app.act(
                                                    Action::Display(DisplayAction::TaskbarSelect(
                                                        metric,
                                                    )),
                                                    window,
                                                    cx,
                                                );
                                            }
                                        },
                                    ))
                                    .on_drag(drag, move |drag, _, window, cx| {
                                        let _ = app.update(cx, |app, cx| {
                                            if app.display_tab != drag.target {
                                                return;
                                            }
                                            app.start_taskbar_drag(drag.metric, window, cx)
                                        });
                                        cx.new(|_| drag.clone())
                                    })
                            })
                    }));
                if interactive && self.editor.editing && cells.is_empty() {
                    preview_row = preview_row.child(label("拖到此行", p.muted).text_size(px(12.)));
                }
                if interactive
                    && let Some(DropIntent::Insert { row, index }) = self.editor.drop
                    && row == row_index
                {
                    let x = cells.get(index).map(|cell| cell.x).unwrap_or_else(|| {
                        cells.last().map_or(2., |cell| cell.x + cell.width + 4.)
                    });
                    preview_row = preview_row.child(
                        div()
                            .absolute()
                            .left(px(x * scale - 4.))
                            .top_0()
                            .w(px(9.))
                            .h(px(row_height))
                            .rounded(px(3.))
                            .border_2()
                            .border_color(rgb(p.accent))
                            .bg(state_surface(p)),
                    );
                }
                preview_row
            }));
        if interactive {
            readout = readout
                .child(
                    canvas(move |rect, _, _| bounds.set(rect), |_, _, _, _| {})
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full(),
                )
                .on_drag_move(
                    cx.listener(|app, event: &DragMoveEvent<TaskbarDrag>, _, cx| {
                        app.update_taskbar_drag(event.event.position, cx)
                    }),
                )
                .on_drop(cx.listener(|app, drag: &TaskbarDrag, window, cx| {
                    if app.display_tab != drag.target {
                        return;
                    }
                    app.update_taskbar_drag(window.mouse_position(), cx);
                    if let Some(intent) = app.editor.drop {
                        match intent {
                            DropIntent::Insert { row, index } => {
                                app.drop_taskbar_item(drag.metric, row, index, window, cx)
                            }
                            DropIntent::Swap(_) => {
                                app.apply_taskbar_drop(drag.metric, intent, window, cx)
                            }
                        }
                    }
                }));
        }
        readout
    }
}
