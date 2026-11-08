//! Shared editor state. Swapping tabs swaps state, never the persisted contents.
use super::*;
use crate::taskbar_text::{ContentCell, ContentFrame, ContentMeasure};

pub(super) struct DisplayEditor {
    pub selected: Option<Metric>,
    pub label: Entity<InputState>,
    pub size: Entity<InputState>,
    pub size_error: bool,
    pub weight: Entity<InputState>,
    pub weight_error: bool,
    pub weight_wheel_remainder: f32,
    pub weight_bounds: Rc<Cell<Bounds<Pixels>>>,
    pub colors: [Entity<InputState>; 2],
    pub color_errors: [String; 2],
    pub toolbar_compact: bool,
    pub toolbar_groups: Rc<Cell<[Bounds<Pixels>; 4]>>,
    pub editing: bool,
    pub gap: Entity<SliderState>,
    pub drafts: HashMap<Metric, TaskbarItem>,
    pub drop: Option<DropIntent>,
    pub drag: Option<TaskbarDragSession>,
    pub spacing: taskbar_spacing::SpacingState,
    pub feedback: Option<(Metric, Instant)>,
    pub settings_scroll: ScrollHandle,
    pub settings_scrollbar: ScrollbarState,
    pub preview_scroll: ScrollHandle,
    pub preview_scrollbar: ScrollbarState,
    pub preview_vertical_scrollbar: ScrollbarState,
    pub traffic_bounds: Rc<Cell<Bounds<Pixels>>>,
    pub candidate_scroll: ScrollHandle,
    pub candidate_scrollbar: ScrollbarState,
    pub measure: ContentMeasure,
    pub candidate_measure: ContentMeasure,
    pub example_measure: ContentMeasure,
}

impl DisplayEditor {
    pub fn new(
        content: &TaskbarContent,
        target: DisplayTab,
        window: &mut Window,
        cx: &mut Context<TrafficApp>,
    ) -> (Self, Vec<Subscription>) {
        let first = content
            .rows
            .iter()
            .flatten()
            .next()
            .expect("validated display content");
        let label = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder("留空时只显示数值")
                .validate(|value, _| TaskbarContent::valid_label(value));
            state.set_value(first.prefix_label().into_owned(), window, cx);
            state
        });
        let size = input(&content.font_size.to_string(), "12", false, window, cx);
        let weight = input(
            &content.font_weight.value().to_string(),
            "400",
            false,
            window,
            cx,
        );
        let colors = TaskbarColorTarget::ALL.map(|target| {
            input(
                &target
                    .value(first)
                    .map(|v| format!("#{v:06X}"))
                    .unwrap_or_default(),
                match target {
                    TaskbarColorTarget::Text => "跟随主题 · #RRGGBB",
                    TaskbarColorTarget::Background => "透明 · #RRGGBB",
                },
                false,
                window,
                cx,
            )
        });
        let gap = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(32.)
                .step(1.)
                .default_value(f32::from(content.gap))
        });
        let mut subscriptions = vec![
            cx.subscribe_in(&label, window, move |app, _, event, _, cx| {
                if app.display_tab != target {
                    return;
                }
                if matches!(event, InputEvent::Change) {
                    let value = app.editor.label.read(cx).value().to_string();
                    if TaskbarContent::valid_label(&value) {
                        app.edit_taskbar_item(cx, |item| item.label = value);
                    }
                    cx.notify();
                }
            }),
            cx.subscribe_in(&size, window, move |app, _, event, _, cx| {
                if app.display_tab != target {
                    return;
                }
                match event {
                    InputEvent::Change => app.update_taskbar_font_size_input(false, cx),
                    InputEvent::Blur | InputEvent::PressEnter { .. } => {
                        app.update_taskbar_font_size_input(true, cx)
                    }
                    _ => {}
                }
            }),
            cx.subscribe_in(&weight, window, move |app, _, event, _, cx| {
                if app.display_tab != target {
                    return;
                }
                match event {
                    InputEvent::Change => app.update_taskbar_weight_input(false, cx),
                    InputEvent::Blur | InputEvent::PressEnter { .. } => {
                        app.update_taskbar_weight_input(true, cx)
                    }
                    _ => {}
                }
            }),
            cx.subscribe_in(&gap, window, move |app, _, event, window, cx| {
                if app.display_tab != target {
                    return;
                }
                let SliderEvent::Change(value) = event;
                let gap = value.end().round().clamp(0., 32.) as u8;
                if gap != app.display_content().gap {
                    app.act(Action::Display(DisplayAction::TaskbarGap(gap)), window, cx);
                }
            }),
        ];
        subscriptions.extend(TaskbarColorTarget::ALL.map(|color| {
            cx.subscribe_in(
                &colors[color.index()],
                window,
                move |app, _, event, _, cx| {
                    if app.display_tab != target {
                        return;
                    }
                    match event {
                        InputEvent::Change => app.update_taskbar_color_input(color, false, cx),
                        InputEvent::Blur | InputEvent::PressEnter { .. } => {
                            app.update_taskbar_color_input(color, true, cx)
                        }
                        _ => {}
                    }
                },
            )
        }));
        (
            Self {
                selected: Some(first.metric),
                label,
                size,
                size_error: false,
                weight,
                weight_error: false,
                weight_wheel_remainder: 0.,
                weight_bounds: Rc::new(Cell::new(Bounds::default())),
                colors,
                color_errors: Default::default(),
                toolbar_compact: false,
                toolbar_groups: Rc::new(Cell::new([Bounds::default(); 4])),
                editing: false,
                gap,
                drafts: HashMap::new(),
                drop: None,
                drag: None,
                spacing: Default::default(),
                feedback: None,
                settings_scroll: ScrollHandle::new(),
                settings_scrollbar: Default::default(),
                preview_scroll: ScrollHandle::new(),
                preview_scrollbar: Default::default(),
                preview_vertical_scrollbar: Default::default(),
                traffic_bounds: Rc::new(Cell::new(Bounds::default())),
                candidate_scroll: ScrollHandle::new(),
                candidate_scrollbar: Default::default(),
                measure: Default::default(),
                candidate_measure: Default::default(),
                example_measure: Default::default(),
            },
            subscriptions,
        )
    }
}

#[derive(Clone)]
pub(super) enum DisplayAction {
    TaskbarMode(TaskbarPositionMode),
    TaskbarPreference(bool),
    TaskbarPreset(bool),
    TaskbarRescan,
    TaskbarSelect(Metric),
    TaskbarAdd(Metric),
    TaskbarRemove,
    TaskbarRows(usize),
    TaskbarGap(u8),
    TaskbarFont(String),
    TaskbarFontSize(f32),
    TaskbarFontWeight(TaskbarFontWeight),
    TaskbarColor(TaskbarColorTarget, Option<u32>),
    TaskbarHidePercent(bool),
    TaskbarEditing,
    TaskbarValueFormat(ValueFormat),
    TaskbarUnit(ByteUnit),
    TaskbarUnitStyle,
    TaskbarPrecision(Option<u8>),
    TaskbarDevice(String),
    TaskbarReset,
    TaskbarMove(MoveDirection),
    CopyTaskbar,
    FloatingBackgroundReset,
    ResetFloat,
}

impl TrafficApp {
    pub(super) fn display(&self, cx: &Context<Self>) -> Div {
        let p = self.p;
        let mut rail_bg = rgb(p.text);
        rail_bg.a = if p.dark { 0.06 } else { 0.04 };
        let mut selected_bg = rgb(p.accent);
        selected_bg.a = if p.dark { 0.18 } else { 0.12 };
        let mut selected_border = rgb(p.accent);
        selected_border.a = if p.dark { 0.32 } else { 0.24 };
        let tabs = row()
            .gap(px(4.))
            .mx(px(12.))
            .mt(px(12.))
            .p(px(4.))
            .h(px(44.))
            .flex_none()
            .rounded(px(12.))
            .bg(rail_bg)
            .children(
                [
                    (
                        DisplayTab::Taskbar,
                        "display-tab-taskbar",
                        "任务栏",
                        "icons/display-taskbar.svg",
                    ),
                    (
                        DisplayTab::Floating,
                        "display-tab-floating",
                        "悬浮窗",
                        "icons/display-floating.svg",
                    ),
                ]
                .into_iter()
                .map(|(tab, id, title, icon)| {
                    let selected = self.display_tab == tab;
                    div()
                        .id(id)
                        .flex()
                        .flex_1()
                        .min_w_0()
                        .items_center()
                        .justify_center()
                        .gap(px(8.))
                        .h_full()
                        .px(px(12.))
                        .rounded(px(8.))
                        .border_1()
                        .border_color(if selected { selected_border } else { rgba(0) })
                        .bg(if selected { selected_bg } else { rgba(0) })
                        .font_weight(FontWeight::NORMAL)
                        .text_color(rgb(if selected { p.accent_text } else { p.muted }))
                        .cursor_pointer()
                        .hover(move |s| {
                            s.bg(if selected {
                                selected_bg
                            } else {
                                state_surface(p)
                            })
                            .text_color(rgb(p.accent_text))
                        })
                        .child(Icon::default().path(icon).size(px(16.)))
                        .child(title)
                        .on_click(cx.listener(move |s, _, window, cx| {
                            s.act(Action::DisplayTab(tab), window, cx)
                        }))
                }),
            );
        let panel = match self.display_tab {
            DisplayTab::Taskbar => self.taskbar_settings(cx),
            DisplayTab::Floating => self.floating_settings(cx),
        };
        column()
            .gap(px(0.))
            .flex_1()
            .min_h_0()
            .w_full()
            .max_w(px(1100.))
            .bg(self.card_surface())
            .border_1()
            .border_color(card_border(p))
            .rounded(px(p.radius as f32))
            .overflow_hidden()
            .child(tabs)
            .child(
                div()
                    .id(match self.display_tab {
                        DisplayTab::Taskbar => "display-content-taskbar",
                        DisplayTab::Floating => "display-content-floating",
                    })
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .child(column().p(px(19.)).flex_1().min_h_0().child(panel)),
            )
    }

    pub(super) fn taskbar_item(&self, metric: Metric) -> TaskbarItem {
        self.display_content()
            .item(metric)
            .or_else(|| self.editor.drafts.get(&metric))
            .cloned()
            .unwrap_or_else(|| TaskbarItem::new(metric))
    }
    pub(super) fn edit_taskbar_item(
        &mut self,
        cx: &mut Context<Self>,
        edit: impl FnOnce(&mut TaskbarItem),
    ) {
        let Some(metric) = self.editor.selected else {
            return;
        };
        let previous = self.taskbar_item(metric);
        let mut next = previous.clone();
        edit(&mut next);
        if next == previous {
            return;
        }
        if let Some(item) = self.display_content_mut().item_mut(metric) {
            *item = next;
            self.save_display_content(cx);
        } else {
            self.editor.drafts.insert(metric, next);
        }
    }
    pub(super) fn select_taskbar_item(
        &mut self,
        metric: Metric,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_picker(window, cx);
        let item = self.taskbar_item(metric);
        self.editor.selected = Some(metric);
        self.editor.label.update(cx, |input, cx| {
            input.set_value(item.prefix_label().into_owned(), window, cx)
        });
        self.sync_taskbar_colors(true, window, cx);
    }
    pub(super) fn sync_taskbar_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_taskbar_font_size(false, window, cx);
        self.sync_taskbar_weight(false, window, cx);
        if self.editor.gap.read(cx).value().end() != f32::from(self.display_content().gap) {
            self.editor.gap.update(cx, |slider, cx| {
                slider.set_value(f32::from(self.display_content().gap), window, cx)
            });
        }
        let Some(metric) = self.editor.selected else {
            return;
        };
        let selected = self.taskbar_item(metric);
        if !self
            .editor
            .label
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
            && self.editor.label.read(cx).value().as_ref() != selected.prefix_label().as_ref()
        {
            self.select_taskbar_item(selected.metric, window, cx);
        }
        self.sync_taskbar_colors(false, window, cx);
    }

    pub(super) fn handle_display_action(
        &mut self,
        action: DisplayAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> std::ops::ControlFlow<()> {
        match action {
            DisplayAction::TaskbarMode(mode) => {
                self.taskbar_position.hold_generation += 1;
                self.config.taskbar_position_mode = mode;
                self.save_config();
                self.sync_position_controls(window, cx);
                if mode == TaskbarPositionMode::Manual {
                    self.taskbar_position.focus.focus(window);
                }
            }
            DisplayAction::TaskbarPreference(left) => {
                self.config.taskbar_auto_prefer_left = left;
                self.save_config();
            }
            DisplayAction::TaskbarPreset(left) => {
                self.config.taskbar_left = left;
                self.config.taskbar_offset = 0;
                self.taskbar_position.error.clear();
                window.blur();
                self.save_config();
                self.sync_position_controls(window, cx);
                self.taskbar_position.focus.focus(window);
            }
            DisplayAction::TaskbarRescan => self.backend.taskbar_rescan(),
            DisplayAction::TaskbarSelect(metric) => {
                if self.editor.selected == Some(metric) {
                    self.editor.selected = None;
                    window.blur();
                } else {
                    self.select_taskbar_item(metric, window, cx);
                }
            }
            DisplayAction::TaskbarAdd(metric) => {
                if self.display_content().item(metric).is_none() {
                    let item = self.taskbar_item(metric);
                    self.display_content_mut().add(metric);
                    *self.display_content_mut().item_mut(metric).unwrap() = item;
                    self.editor.drafts.remove(&metric);
                    self.editor.feedback = Some((metric, Instant::now()));
                    self.save_display_content(cx);
                }
                self.select_taskbar_item(metric, window, cx);
            }
            DisplayAction::TaskbarRemove => {
                if let Some(metric) = self.editor.selected
                    && self
                        .display_content()
                        .rows
                        .iter()
                        .map(Vec::len)
                        .sum::<usize>()
                        > 1
                {
                    self.editor.drafts.insert(metric, self.taskbar_item(metric));
                    self.display_content_mut().remove(metric);
                    self.save_display_content(cx);
                }
            }
            DisplayAction::TaskbarRows(count) => {
                if !(1..=if self.display_tab == DisplayTab::Taskbar {
                    2
                } else {
                    13
                })
                    .contains(&count)
                {
                    return std::ops::ControlFlow::Break(());
                }
                self.display_content_mut().set_rows(count);
                self.save_display_content(cx);
            }
            DisplayAction::TaskbarGap(gap) => {
                self.display_content_mut().gap = gap;
                self.save_display_content(cx);
            }
            DisplayAction::TaskbarFont(family) => {
                self.display_content_mut().font_family = family;
                self.save_display_content(cx);
            }
            DisplayAction::TaskbarFontSize(size) => {
                if !(8. ..=20.).contains(&size) {
                    return std::ops::ControlFlow::Break(());
                }
                self.display_content_mut().font_size = size;
                self.sync_taskbar_font_size(true, window, cx);
                self.save_display_content(cx);
            }
            DisplayAction::TaskbarFontWeight(weight) => {
                self.display_content_mut().font_weight = weight;
                self.save_display_content(cx);
                self.sync_taskbar_weight(true, window, cx);
            }
            DisplayAction::TaskbarColor(target, color) => {
                self.edit_taskbar_item(cx, |item| target.set(item, color));
                self.sync_taskbar_colors(true, window, cx);
            }
            DisplayAction::TaskbarHidePercent(hidden) => {
                self.edit_taskbar_item(cx, |item| item.hide_percent = hidden)
            }
            DisplayAction::TaskbarEditing => {
                self.taskbar_position.hold_generation += 1;
                self.editor.editing = !self.editor.editing;
                window.blur();
            }
            DisplayAction::TaskbarValueFormat(format) => self.edit_taskbar_item(cx, |item| {
                item.normalize_legacy();
                item.format = format;
            }),
            DisplayAction::TaskbarUnit(unit) => self.edit_taskbar_item(cx, |item| {
                item.normalize_legacy();
                item.unit = Some(unit);
                if !item.metric.is_memory() {
                    item.format = ValueFormat::Auto;
                }
            }),
            DisplayAction::TaskbarUnitStyle => self.edit_taskbar_item(cx, |item| {
                item.normalize_legacy();
                item.unit_style = item.unit_style.toggled();
            }),
            DisplayAction::TaskbarPrecision(precision) => self.edit_taskbar_item(cx, |item| {
                item.normalize_legacy();
                item.precision = precision;
            }),
            DisplayAction::TaskbarDevice(device) => {
                self.edit_taskbar_item(cx, |item| item.device = device)
            }
            DisplayAction::TaskbarReset => {
                self.editor.drafts.clear();
                *self.display_content_mut() = if self.display_tab == DisplayTab::Taskbar {
                    TaskbarContent::default()
                } else {
                    TaskbarContent::floating()
                };
                self.sync_taskbar_font_size(true, window, cx);
                self.sync_taskbar_weight(true, window, cx);
                self.save_display_content(cx);
                let first = self
                    .display_content()
                    .rows
                    .iter()
                    .flatten()
                    .next()
                    .unwrap()
                    .metric;
                self.select_taskbar_item(first, window, cx);
            }
            DisplayAction::TaskbarMove(direction) => {
                if let Some(metric) = self.editor.selected
                    && let Some(target) =
                        self.display_content().directional_target(metric, direction)
                {
                    self.apply_taskbar_drop(metric, target, window, cx);
                }
            }
            DisplayAction::CopyTaskbar => {
                if self.display_tab == DisplayTab::Floating {
                    self.config.floating_content = self.config.taskbar_content.clone();
                    self.editor.drafts.clear();
                    self.editor.measure = Default::default();
                    self.editor.example_measure = Default::default();
                    self.sync_taskbar_font_size(true, window, cx);
                    self.sync_taskbar_weight(true, window, cx);
                    let first = self
                        .config
                        .floating_content
                        .rows
                        .iter()
                        .flatten()
                        .next()
                        .unwrap()
                        .metric;
                    self.select_taskbar_item(first, window, cx);
                    self.save_display_content(cx);
                }
            }
            DisplayAction::FloatingBackgroundReset => {
                self.config.floating_background_color = None;
                self.floating_background_error = false;
                self.floating_background
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.save_display_content(cx);
            }
            DisplayAction::ResetFloat => {
                self.config.floating_x = None;
                self.config.floating_y = None;
                self.float_view.update(cx, |view, cx| {
                    view.reset_position();
                    cx.notify();
                });
                self.save_config();
            }
        }
        std::ops::ControlFlow::Continue(())
    }

    pub(super) fn display_content(&self) -> &TaskbarContent {
        match self.display_tab {
            DisplayTab::Taskbar => &self.config.taskbar_content,
            DisplayTab::Floating => &self.config.floating_content,
        }
    }
    pub(super) fn display_content_mut(&mut self) -> &mut TaskbarContent {
        match self.display_tab {
            DisplayTab::Taskbar => &mut self.config.taskbar_content,
            DisplayTab::Floating => &mut self.config.floating_content,
        }
    }
    pub(super) fn content_frame(&self, content: &TaskbarContent) -> ContentFrame {
        if content == self.display_content() {
            self.editor.measure.frame(content, &self.data)
        } else {
            self.editor.candidate_measure.frame(content, &self.data)
        }
    }
    pub(super) fn example_frame(&self, item: &TaskbarItem) -> ContentFrame {
        let frame = if self.display_content().item(item.metric).is_some() {
            self.content_frame(self.display_content())
        } else {
            let mut content = self.display_content().clone();
            content.add(item.metric);
            *content.item_mut(item.metric).unwrap() = item.clone();
            self.editor.example_measure.frame(&content, &self.data)
        };
        frame
            .metric_frame(item.metric)
            .expect("example includes selected metric")
    }
    pub(super) fn preview_scale(&self) -> f32 {
        if self.display_tab == DisplayTab::Taskbar {
            crate::taskbar_text::PREVIEW_SCALE
        } else {
            1.
        }
    }
    pub(super) fn display_row_gap(&self) -> f32 {
        if self.display_tab == DisplayTab::Floating {
            f32::from(self.config.floating_row_gap)
        } else {
            0.
        }
    }
    pub(super) fn save_display_content(&mut self, cx: &mut Context<Self>) {
        self.config.taskbar_horizontal = self.config.taskbar_content.rows.len() == 1;
        self.config.floating_today = self.config.floating_content.item(Metric::Today).is_some();
        self.save_config();
        self.refresh_floating(cx);
        cx.notify();
    }
    pub(super) fn refresh_floating(&mut self, cx: &mut Context<Self>) {
        let visible = self.config.floating
            && !(self.config.fullscreen_hide && native::fullscreen_foreground());
        native::show(self.float_hwnd, visible);
        let palette = self.floating_palette();
        self.float_view.update(cx, |view, cx| {
            view.palette = palette;
            if !self.config.floating {
                view.clear_appearance();
            }
            cx.notify();
        });
    }

    pub(super) fn floating_palette(&self) -> Palette {
        Palette::new(self.config.color_theme, self.p.dark)
            .with_wallpaper_accent(self.wallpaper.color())
    }
}

pub(super) fn cell_segments(
    cell: &ContentCell,
    frame: &ContentFrame,
    scale: f32,
    palette: Palette,
    mask: Option<&taskbar_spacing::SpacingState>,
) -> Vec<Div> {
    cell.segments
        .iter()
        .enumerate()
        .map(|(index, segment)| {
            let container = div()
                .absolute()
                .left(px((segment.x - cell.x) * scale - 1.))
                .top(px(-1.))
                .w(px(segment.width * scale))
                .h(px(frame.line_height * scale))
                .font_family(frame.font_family.clone())
                .font_weight(FontWeight(f32::from(frame.font_weight.value())))
                .text_size(px(frame.font_size * scale))
                .flex()
                .items_center()
                .when(segment.right_aligned, |d| d.justify_end());
            if index == 0
                && (segment.icon.is_some()
                    || crate::taskbar_text::split_prefix(&segment.text).is_some())
            {
                return container.child(aligned_prefix(
                    segment.icon,
                    segment.text.clone(),
                    frame.font_size * scale,
                    frame.line_height * scale,
                    cell.text_color.unwrap_or(palette.text),
                    mask.map(taskbar_spacing::SpacingState::icon_mask),
                ));
            }
            let text = if let Some(mask) = mask {
                mask.label(
                    segment.text.clone(),
                    cell.text_color.unwrap_or(palette.text),
                )
            } else {
                label(
                    segment.text.clone(),
                    cell.text_color.unwrap_or(palette.text),
                )
            };
            container.child(
                text.font_family(frame.font_family.clone())
                    .font_weight(FontWeight(f32::from(frame.font_weight.value())))
                    .text_size(px(frame.font_size * scale))
                    .whitespace_nowrap(),
            )
        })
        .collect()
}

fn aligned_prefix(
    icon: Option<crate::traffic_icon::DirectionIcon>,
    text: String,
    size: f32,
    line_height: f32,
    color: u32,
    occlusion: Option<Rc<Cell<Option<Bounds<Pixels>>>>>,
) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, cx| {
            let (stem, colon) = crate::taskbar_text::split_prefix(&text).unwrap_or((&text, ""));
            let mut style = window.text_style();
            style.color = rgb(color).into();
            let shape = |text: &str| {
                window.text_system().shape_line(
                    text.to_owned().into(),
                    px(size),
                    &[style.to_run(text.len())],
                    None,
                )
            };
            let stem = shape(stem);
            let colon = shape(colon);
            let stem_width = if icon.is_some() { px(size) } else { stem.width };
            let stem_y = prefix_center_offset(&stem, window);
            let colon_y = prefix_center_offset(&colon, window);
            let icon_top = (bounds.size.height - px(size)) / 2.;
            let mut paint = |window: &mut Window| {
                if let Some(icon) = icon
                    && let Err(error) = window.paint_svg(
                        Bounds::new(
                            bounds.origin + point(px(0.), icon_top),
                            gpui::size(px(size), px(size)),
                        ),
                        icon.asset_path().into(),
                        TransformationMatrix::unit(),
                        rgb(color).into(),
                        cx,
                    )
                {
                    tracing::warn!(%error, "Display direction icon could not be painted");
                }
                for (line, x, offset) in [(&stem, px(0.), stem_y), (&colon, stem_width, colon_y)] {
                    if line.len() == 0 {
                        continue;
                    }
                    if let Err(error) = line.paint(
                        bounds.origin + point(x, -offset),
                        px(line_height),
                        window,
                        cx,
                    ) {
                        tracing::warn!(%error, "Display prefix text could not be painted");
                    }
                }
            };
            if let Some(cover) = occlusion.as_ref().and_then(|mask| mask.get()) {
                for clip in taskbar_spacing::uncovered(window.content_mask().bounds, cover) {
                    if clip.intersect(&bounds).size.width > px(0.)
                        && clip.intersect(&bounds).size.height > px(0.)
                    {
                        window.with_content_mask(Some(ContentMask { bounds: clip }), |window| {
                            paint(window)
                        });
                    }
                }
            } else {
                paint(window);
            }
        },
    )
    .w_full()
    .h(px(line_height))
    .flex_none()
}

/// GPUI's Windows glyph bounds use upward-positive y relative to the baseline.
/// Include every glyph and its resolved font so Chinese/custom text is centered
/// by its own ink, just like a network arrow or the separate colon.
pub(super) fn prefix_center_offset(line: &ShapedLine, window: &Window) -> Pixels {
    let mut ink: Option<(Pixels, Pixels)> = None;
    for run in &line.runs {
        for glyph in &run.glyphs {
            let Some(character) = line.text[glyph.index..].chars().next() else {
                continue;
            };
            let Ok(bounds) =
                window
                    .text_system()
                    .typographic_bounds(run.font_id, line.font_size, character)
            else {
                continue;
            };
            if bounds.size.height <= px(0.) {
                continue;
            }
            let bottom = bounds.origin.y;
            let top = bottom + bounds.size.height;
            ink = Some(ink.map_or((bottom, top), |(low, high)| {
                (low.min(bottom), high.max(top))
            }));
        }
    }
    ink.map_or(px(0.), |(bottom, top)| {
        (line.ascent - line.descent - bottom - top) / 2.
    })
}
