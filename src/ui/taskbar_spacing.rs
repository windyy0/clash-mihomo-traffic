use super::*;
use crate::taskbar_text::ContentFrame;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SpacingKind {
    Segment,
    Prefix,
}

impl SpacingKind {
    fn name(self) -> &'static str {
        match self {
            Self::Segment => "段间距",
            Self::Prefix => "前缀间距",
        }
    }
    fn value(self, content: &TaskbarContent) -> u8 {
        match self {
            Self::Segment => content.gap,
            Self::Prefix => content.prefix_gap,
        }
    }
    fn apply(self, content: &mut TaskbarContent, value: u8) {
        match self {
            Self::Segment => content.gap = value,
            Self::Prefix => content.prefix_gap = value,
        }
    }
    fn color(self, palette: Palette) -> u32 {
        match self {
            Self::Segment => {
                if palette.dark {
                    0x86b9ef
                } else {
                    0x3478c7
                }
            }
            Self::Prefix => palette.amber,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct GuideKey {
    pub(super) kind: SpacingKind,
    pub(super) column: usize,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Guide {
    key: GuideKey,
    left: f32,
    right: f32,
}

impl Guide {
    fn center(self) -> f32 {
        (self.left + self.right) / 2.
    }
}

fn guides(frame: &ContentFrame) -> Vec<Guide> {
    // Both rows share column reservations. The longest row includes every
    // boundary, including a prefix reserved by an item in the other row.
    let Some(cells) = frame.rows.iter().max_by_key(|row| row.len()) else {
        return Vec::new();
    };
    let mut guides = Vec::new();
    for (column, cell) in cells.iter().enumerate() {
        if let [prefix, number, ..] = cell.segments.as_slice()
            && prefix.width > 0.
        {
            guides.push(Guide {
                key: GuideKey {
                    kind: SpacingKind::Prefix,
                    column,
                },
                left: prefix.x + prefix.width,
                right: number.x,
            });
        }
        if let Some(next) = cells.get(column + 1) {
            guides.push(Guide {
                key: GuideKey {
                    kind: SpacingKind::Segment,
                    column,
                },
                left: cell.x + cell.width,
                right: next.x,
            });
        }
    }
    guides
}

struct SpacingDrag {
    guide: GuideKey,
    original: TaskbarContent,
    start_x: Pixels,
    value: u8,
    scale: f32,
}

#[derive(Clone, Copy)]
struct SpacingTarget {
    guide: GuideKey,
    bounds: Bounds<Pixels>,
}

impl SpacingDrag {
    fn update(&mut self, x: Pixels) {
        // Always measure from the original pointer position, independently of
        // reflow, the changing preview width, or the guide's current hitbox.
        self.value = (f32::from(self.guide.kind.value(&self.original))
            + f32::from(x - self.start_x) / self.scale)
            .round()
            .clamp(0., 32.) as u8;
    }
    fn content(&self) -> TaskbarContent {
        let mut content = self.original.clone();
        self.guide.kind.apply(&mut content, self.value);
        content
    }
}

#[derive(Default)]
pub(super) struct SpacingState {
    preview_hovered: bool,
    hovered: Option<SpacingTarget>,
    retain_hover: bool,
    wheel_remainder: f32,
    drag: Option<SpacingDrag>,
    pub(super) card_bounds: Rc<Cell<Bounds<Pixels>>>,
    pub(super) footer_bounds: Rc<Cell<Bounds<Pixels>>>,
    occlusion: Rc<Cell<Option<Bounds<Pixels>>>>,
}

impl SpacingState {
    pub(super) fn preview_is_hovered(&self) -> bool {
        self.preview_hovered
    }
    fn target_at(
        &self,
        position: Point<Pixels>,
        candidate: Option<SpacingTarget>,
    ) -> Option<SpacingTarget> {
        // Retain the original hit region during wheel/key reflow, even when
        // the shrinking or right-anchored preview moves the visible guide.
        self.hovered
            .filter(|target| self.retain_hover && target.bounds.contains(&position))
            .or(candidate)
    }
    fn hover(&mut self, target: Option<SpacingTarget>) -> bool {
        let changed = self.hovered.map(|target| target.guide) != target.map(|target| target.guide);
        if changed {
            self.retain_hover = false;
            self.wheel_remainder = 0.;
        }
        self.hovered = target;
        changed
    }
    pub(super) fn clear_occlusion(&self) {
        self.occlusion.set(None);
    }
    pub(super) fn icon_mask(&self) -> Rc<Cell<Option<Bounds<Pixels>>>> {
        self.occlusion.clone()
    }
    pub(super) fn label(&self, text: impl Into<SharedString>, color: u32) -> Div {
        div()
            .text_size(px(13.))
            .line_height(relative(1.45))
            .text_color(rgb(color))
            .child(MaskedText {
                text: text.into(),
                occlusion: self.occlusion.clone(),
            })
    }
    #[cfg(test)]
    pub(super) fn occluded_bounds(&self) -> Option<Bounds<Pixels>> {
        self.occlusion.get()
    }
    pub(super) fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }
    pub(super) fn set_preview_hovered(&mut self, hovered: bool) {
        self.preview_hovered = hovered;
        if !hovered {
            self.hover(None);
            self.clear_occlusion();
        }
    }
    pub(super) fn conflicts_with(&self, content: &TaskbarContent) -> bool {
        self.drag
            .as_ref()
            .is_some_and(|drag| &drag.original != content)
    }
    pub(super) fn preview_content(&self) -> Option<TaskbarContent> {
        self.drag.as_ref().map(SpacingDrag::content)
    }
}

impl TrafficApp {
    fn hover_taskbar_spacing(
        &mut self,
        position: Point<Pixels>,
        candidate: Option<SpacingTarget>,
        cx: &mut Context<Self>,
    ) {
        let target = self.editor.spacing.target_at(position, candidate);
        if self.editor.spacing.hover(target) {
            cx.notify();
        }
    }

    #[cfg(test)]
    pub(super) fn exercise_taskbar_spacing(
        &mut self,
        prefix: bool,
        position: Point<Pixels>,
        delta: Option<ScrollDelta>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let frame = self.content_frame(self.display_content());
        let candidate = guides(&frame)
            .into_iter()
            .filter(|guide| {
                guide.key.column == 0 && (guide.key.kind == SpacingKind::Prefix) == prefix
            })
            .find_map(|guide| {
                let visual = grip_bounds(
                    guide,
                    self.editor.traffic_bounds.get().left(),
                    self.editor.traffic_bounds.get().top() - px(1.),
                    self.editor.spacing.footer_bounds.get().bottom() - px(2.),
                    self.preview_scale(),
                );
                let expanded = self
                    .editor
                    .spacing
                    .hovered
                    .is_some_and(|target| target.guide == guide.key);
                let rail = grip_hit_bounds(
                    visual,
                    self.editor.preview_scroll.bounds().top() + px(56.),
                    expanded,
                );
                rail.contains(&position).then_some(SpacingTarget {
                    guide: guide.key,
                    bounds: rail,
                })
            });
        if let Some(delta) = delta {
            self.scroll_taskbar_spacing(
                &ScrollWheelEvent {
                    position,
                    delta,
                    ..Default::default()
                },
                candidate,
                window,
                cx,
            )
        } else {
            self.hover_taskbar_spacing(position, candidate, cx);
            false
        }
    }

    pub(super) fn start_taskbar_spacing(
        &mut self,
        guide: GuideKey,
        x: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.editor.editing || self.editor.drag.is_some() {
            return;
        }
        self.close_picker(window, cx);
        self.taskbar_position.hold_generation += 1;
        self.editor.spacing.drag = Some(SpacingDrag {
            guide,
            original: self.display_content().clone(),
            start_x: x,
            scale: self.preview_scale(),
            value: guide.kind.value(self.display_content()),
        });
        window.blur();
        window.prevent_default();
        cx.notify();
    }

    pub(super) fn cancel_taskbar_spacing(&mut self, cx: &mut Context<Self>) {
        let dragged = self.editor.spacing.drag.take().is_some();
        if self.editor.spacing.hover(None) || dragged {
            self.editor.spacing.clear_occlusion();
            cx.notify();
        }
    }

    fn step_taskbar_spacing(&mut self, amount: i32, cx: &mut Context<Self>) {
        let Some(target) = self.editor.spacing.hovered else {
            return;
        };
        self.editor.spacing.retain_hover = true;
        self.taskbar_position.hold_generation += 1;
        let current = target.guide.kind.value(self.display_content());
        let next = (i32::from(current) + amount).clamp(0, 32) as u8;
        if next != current {
            target.guide.kind.apply(self.display_content_mut(), next);
            self.save_display_content(cx);
        }
        cx.notify();
    }

    pub(super) fn taskbar_spacing_keystroke(
        &mut self,
        event: &KeystrokeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if event.keystroke.key == "escape" && self.editor.spacing.is_dragging() {
            self.cancel_taskbar_spacing(cx);
            return true;
        }
        if !self.editor.editing
            || self.page != Page::Display
            || self.editor.drag.is_some()
            || self.editor.spacing.hovered.is_none()
            || self.picker.is_some()
            || event.keystroke.modifiers.modified()
            || event.context_stack.iter().any(|context| {
                ["Input", "Popover", "Modal", "Drawer"]
                    .iter()
                    .any(|name| context.contains(name))
            })
        {
            return false;
        }
        let step = match event.keystroke.key.as_str() {
            "up" => 1,
            "down" => -1,
            "escape" => {
                self.cancel_taskbar_spacing(cx);
                return true;
            }
            _ => return false,
        };
        if !self.editor.spacing.is_dragging() {
            self.step_taskbar_spacing(step, cx);
        }
        window.prevent_default();
        true
    }

    fn scroll_taskbar_spacing(
        &mut self,
        event: &ScrollWheelEvent,
        candidate: Option<SpacingTarget>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.editor.editing
            || self.editor.drag.is_some()
            || self.picker.is_some()
            || event.modifiers.modified()
        {
            return false;
        }
        let target = self.editor.spacing.target_at(event.position, candidate);
        let Some(target) = target else {
            return false;
        };
        let delta = match event.delta {
            ScrollDelta::Lines(delta) => delta.y / 3.,
            ScrollDelta::Pixels(delta) => f32::from(delta.y) / 24.,
        };
        if !delta.is_finite() || delta == 0. {
            return false;
        }
        if self.editor.spacing.is_dragging() {
            return true;
        }
        window.blur();
        self.editor.spacing.hover(Some(target));
        self.editor.spacing.retain_hover = true;
        let remainder = &mut self.editor.spacing.wheel_remainder;
        if remainder.signum() != delta.signum() {
            *remainder = 0.;
        }
        *remainder = (*remainder + delta).clamp(-32., 32.);
        let step = remainder.trunc() as i32;
        *remainder -= step as f32;
        self.step_taskbar_spacing(step, cx);
        true
    }

    pub(super) fn update_taskbar_spacing(&mut self, x: Pixels, cx: &mut Context<Self>) {
        if let Some(drag) = self.editor.spacing.drag.as_mut() {
            drag.update(x);
            cx.notify();
        }
    }

    pub(super) fn finish_taskbar_spacing(&mut self, x: Pixels, cx: &mut Context<Self>) {
        let Some(mut drag) = self.editor.spacing.drag.take() else {
            return;
        };
        drag.update(x);
        if self.editor.editing && *self.display_content() == drag.original {
            let next = drag.content();
            if &next != self.display_content() {
                *self.display_content_mut() = next;
                self.save_display_content(cx);
            }
        }
        self.editor.spacing.hover(None);
        cx.notify();
    }

    pub(super) fn release_taskbar_spacing(
        &mut self,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let target = self.editor.spacing.drag.as_ref().map(|drag| SpacingTarget {
            guide: drag.guide,
            bounds: Bounds::centered_at(position, size(px(14.), px(14.))),
        });
        self.finish_taskbar_spacing(position.x, cx);
        if let Some(target) = target
            && self.editor.spacing.card_bounds.get().contains(&position)
        {
            // Keep wheel/keys usable at the release point after reflow.
            self.editor.spacing.hover(Some(target));
            self.editor.spacing.retain_hover = true;
        }
    }

    pub(super) fn taskbar_spacing_hint(&self) -> Div {
        let state = &self.editor.spacing;
        let guide = state
            .drag
            .as_ref()
            .map(|drag| drag.guide)
            .or(state.hovered.map(|target| target.guide));
        let (text, color) = if self.editor.editing
            && self.editor.drag.is_none()
            && let Some(guide) = guide
        {
            let preview = state.preview_content();
            let content = preview.as_ref().unwrap_or(self.display_content());
            (
                format!(
                    "{} {} px · {}",
                    guide.kind.name(),
                    guide.kind.value(content),
                    if state.is_dragging() {
                        "松手保存 · Esc 取消"
                    } else {
                        "拖动 / 滚轮 / ↑↓ 调整"
                    }
                ),
                guide.kind.color(self.p),
            )
        } else {
            (self.taskbar_drag_hint(), self.p.muted)
        };
        div().overflow_hidden().child(
            state
                .label(text, color)
                .text_size(px(12.))
                .whitespace_nowrap(),
        )
    }
}

pub(super) struct SpacingOverlay {
    app: WeakEntity<TrafficApp>,
    guides: Vec<Guide>,
    traffic_bounds: Rc<Cell<Bounds<Pixels>>>,
    footer_bounds: Rc<Cell<Bounds<Pixels>>>,
    occlusion: Rc<Cell<Option<Bounds<Pixels>>>>,
    scroll: ScrollHandle,
    palette: Palette,
    visible: bool,
    scale: f32,
    floating: bool,
    hovered: Option<GuideKey>,
    active: Option<GuideKey>,
}

impl SpacingOverlay {
    pub(super) fn new(app: &TrafficApp, frame: &ContentFrame, cx: &Context<TrafficApp>) -> Self {
        Self {
            app: cx.weak_entity(),
            guides: guides(frame),
            traffic_bounds: app.editor.traffic_bounds.clone(),
            footer_bounds: app.editor.spacing.footer_bounds.clone(),
            occlusion: app.editor.spacing.occlusion.clone(),
            scroll: app.editor.preview_scroll.clone(),
            palette: app.p,
            scale: app.preview_scale(),
            floating: app.display_tab == DisplayTab::Floating,
            visible: app.editor.spacing.preview_hovered || app.editor.spacing.is_dragging(),
            hovered: app.editor.spacing.hovered.map(|target| target.guide),
            active: app.editor.spacing.drag.as_ref().map(|drag| drag.guide),
        }
    }
}

pub(super) struct SpacingGeometry {
    guides: Vec<(Guide, Hitbox)>,
    clip: Bounds<Pixels>,
    origin: Pixels,
    pill_top: Pixels,
    line_top: Pixels,
    bottom: Pixels,
}

fn grip_bounds(
    guide: Guide,
    origin: Pixels,
    top: Pixels,
    bottom: Pixels,
    scale: f32,
) -> Bounds<Pixels> {
    let x = origin + px(guide.center() * scale);
    let width = px(((guide.right - guide.left).max(0.) * scale).max(1.));
    Bounds::new(
        point(x - width / 2., top),
        size(width, (bottom - top).max(px(0.))),
    )
}

fn grip_hit_bounds(visual: Bounds<Pixels>, line_top: Pixels, expanded: bool) -> Bounds<Pixels> {
    let width = visual.size.width.max(px(14.));
    let top = if expanded { visual.top() } else { line_top };
    Bounds::new(
        point(visual.center().x - width / 2., top),
        size(width, visual.bottom() - top + px(3.)),
    )
}

fn hovered_guide(
    guides: &[(Guide, Hitbox)],
    origin: Pixels,
    position: Point<Pixels>,
    window: &Window,
    scrolling: bool,
    scale: f32,
) -> Option<SpacingTarget> {
    guides
        .iter()
        .filter(|(_, hitbox)| {
            if scrolling {
                hitbox.should_handle_scroll(window)
            } else {
                hitbox.is_hovered(window)
            }
        })
        .min_by(|(a, _), (b, _)| {
            let distance =
                |guide: &Guide| f32::from(origin + px(guide.center() * scale) - position.x).abs();
            distance(a).total_cmp(&distance(b))
        })
        .map(|(guide, hitbox)| SpacingTarget {
            guide: guide.key,
            bounds: hitbox.bounds.intersect(&hitbox.content_mask.bounds),
        })
}

impl IntoElement for SpacingOverlay {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for SpacingOverlay {
    type RequestLayoutState = ();
    type PrepaintState = Option<SpacingGeometry>;
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (
            window.request_layout(
                Style {
                    position: Position::Absolute,
                    inset: Edges::all(px(0.).into()),
                    size: size(relative(1.).into(), relative(1.).into()),
                    ..Style::default()
                },
                None,
                cx,
            ),
            (),
        )
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        _: &mut App,
    ) -> Self::PrepaintState {
        self.occlusion.set(None);
        if !self.visible {
            return None;
        }
        let traffic = self.traffic_bounds.get();
        let viewport = self.scroll.bounds();
        let bottom = self.footer_bounds.get().bottom() - px(2.);
        let line_top = if self.floating {
            viewport.bottom() - px(14.)
        } else {
            viewport.top() + px(56.)
        };
        let pill_top = if self.floating {
            traffic.top().max(viewport.top())
        } else {
            traffic.top()
        } - px(1.);
        if bottom <= line_top || viewport.size.width <= px(0.) {
            return None;
        }
        let clip = Bounds::new(
            point(viewport.left(), pill_top),
            size(viewport.size.width, bottom - pill_top + px(3.)),
        );
        let origin = traffic.left();
        let guides = window.with_content_mask(Some(ContentMask { bounds: clip }), |window| {
            self.guides
                .iter()
                .map(|guide| {
                    let expanded = self.active.or(self.hovered) == Some(guide.key);
                    let visual = grip_bounds(*guide, origin, pill_top, bottom, self.scale);
                    let rail = grip_hit_bounds(visual, line_top, expanded);
                    if expanded {
                        let mask = visual.intersect(&clip);
                        if mask.size.width > px(0.) && mask.size.height > px(0.) {
                            self.occlusion.set(Some(mask));
                        }
                    }
                    (*guide, window.insert_hitbox(rail, HitboxBehavior::Normal))
                })
                .collect()
        });
        Some(SpacingGeometry {
            guides,
            clip,
            origin,
            pill_top,
            line_top,
            bottom,
        })
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        geometry: &mut Self::PrepaintState,
        window: &mut Window,
        _: &mut App,
    ) {
        let scale = self.scale;
        let Some(geometry) = geometry else {
            return;
        };
        if self.active.is_some() {
            window.set_window_cursor_style(CursorStyle::ResizeLeftRight);
        }
        window.with_content_mask(
            Some(ContentMask {
                bounds: geometry.clip,
            }),
            |window| {
                for (guide, hitbox) in &geometry.guides {
                    let emphasized = self.active.or(self.hovered) == Some(guide.key);
                    let x = geometry.origin + px(guide.center() * scale);
                    let visual = grip_bounds(
                        *guide,
                        geometry.origin,
                        geometry.pill_top,
                        geometry.bottom,
                        self.scale,
                    );
                    let mut color = rgb(guide.key.kind.color(self.palette));
                    if emphasized {
                        color.a = if self.active.is_some() { 0.4 } else { 0.28 };
                        window.paint_quad(
                            fill(visual, color).corner_radii(px(3.).min(visual.size.width / 2.)),
                        );
                    }
                    color.a = if emphasized { 0.95 } else { 0.6 };
                    window.paint_quad(fill(
                        Bounds::new(
                            point(x - px(0.5), geometry.line_top),
                            size(px(1.), geometry.bottom - geometry.line_top),
                        ),
                        color,
                    ));
                    window.paint_quad(
                        fill(
                            Bounds::new(
                                point(visual.left(), geometry.bottom - px(0.5)),
                                size(visual.size.width, px(1.)),
                            ),
                            color,
                        )
                        .corner_radii(px(0.5)),
                    );
                    window.set_cursor_style(CursorStyle::ResizeLeftRight, hitbox);
                }
            },
        );
        let app = self.app.clone();
        let hitboxes = geometry.guides.clone();
        let origin = geometry.origin;
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            let candidate = hovered_guide(&hitboxes, origin, event.position, window, false, scale);
            let _ = app.update(cx, |app, cx| {
                let target = app.editor.spacing.target_at(event.position, candidate);
                if phase == DispatchPhase::Capture {
                    if target.is_none() || event.button != MouseButton::Left {
                        app.cancel_taskbar_spacing(cx);
                    }
                } else if event.button == MouseButton::Left
                    && let Some(target) = target
                {
                    app.editor.spacing.hover(Some(target));
                    app.start_taskbar_spacing(target.guide, event.position.x, window, cx);
                    cx.stop_propagation();
                }
            });
        });
        let app = self.app.clone();
        let hitboxes = geometry.guides.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Capture {
                return;
            }
            let candidate = hovered_guide(&hitboxes, origin, event.position, window, false, scale);
            let _ = app.update(cx, |app, cx| {
                let state = &mut app.editor.spacing;
                if state.is_dragging() {
                    if event.pressed_button == Some(MouseButton::Left) {
                        app.update_taskbar_spacing(event.position.x, cx);
                        cx.stop_propagation();
                    } else {
                        app.cancel_taskbar_spacing(cx);
                    }
                } else {
                    app.hover_taskbar_spacing(event.position, candidate, cx);
                }
            });
        });
        let app = self.app.clone();
        let hitboxes = geometry.guides.clone();
        window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            let candidate = hovered_guide(&hitboxes, origin, event.position, window, true, scale);
            let _ = app.update(cx, |app, cx| {
                if app.scroll_taskbar_spacing(event, candidate, window, cx) {
                    window.prevent_default();
                    cx.stop_propagation();
                }
            });
        });
        let app = self.app.clone();
        window.on_mouse_event(move |_: &MouseExitEvent, phase, _, cx| {
            if phase == DispatchPhase::Capture {
                let _ = app.update(cx, |app, cx| {
                    if app.editor.spacing.hover(None) {
                        app.editor.spacing.clear_occlusion();
                        cx.notify();
                    }
                });
            }
        });
        let app = self.app.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
            if phase != DispatchPhase::Capture || event.button != MouseButton::Left {
                return;
            }
            let _ = app.update(cx, |app, cx| {
                if app.editor.spacing.is_dragging() {
                    app.release_taskbar_spacing(event.position, cx);
                    cx.stop_propagation();
                }
            });
        });
    }
}

/// Disjoint scissor rectangles outside the grip. Text and icons use these masks;
/// interactive controls paint once, so event listeners cannot be duplicated.
pub(super) fn uncovered(clip: Bounds<Pixels>, cover: Bounds<Pixels>) -> Vec<Bounds<Pixels>> {
    let hole = clip.intersect(&cover);
    if hole.size.width <= px(0.) || hole.size.height <= px(0.) {
        return vec![clip];
    }
    [
        Bounds::new(clip.origin, size(clip.size.width, hole.top() - clip.top())),
        Bounds::new(
            point(clip.left(), hole.bottom()),
            size(clip.size.width, clip.bottom() - hole.bottom()),
        ),
        Bounds::new(
            point(clip.left(), hole.top()),
            size(hole.left() - clip.left(), hole.size.height),
        ),
        Bounds::new(
            point(hole.right(), hole.top()),
            size(clip.right() - hole.right(), hole.size.height),
        ),
    ]
    .into_iter()
    .filter(|bounds| bounds.size.width > px(0.) && bounds.size.height > px(0.))
    .collect()
}

struct MaskedText {
    text: SharedString,
    occlusion: Rc<Cell<Option<Bounds<Pixels>>>>,
}
impl IntoElement for MaskedText {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for MaskedText {
    type RequestLayoutState = <SharedString as Element>::RequestLayoutState;
    type PrepaintState = ();
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.text.request_layout(id, inspector, window, cx)
    }
    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.text.prepaint(id, inspector, bounds, state, window, cx);
    }
    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        prepaint: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(cover) = self.occlusion.get().filter(|cover| {
            let overlap = bounds.intersect(cover);
            overlap.size.width > px(0.) && overlap.size.height > px(0.)
        }) else {
            self.text
                .paint(id, inspector, bounds, state, prepaint, window, cx);
            return;
        };
        for clip in uncovered(window.content_mask().bounds, cover) {
            let overlap = bounds.intersect(&clip);
            if overlap.size.width > px(0.) && overlap.size.height > px(0.) {
                window.with_content_mask(Some(ContentMask { bounds: clip }), |window| {
                    self.text
                        .paint(id, inspector, bounds, state, prepaint, window, cx);
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn grip_mask_removes_only_the_covered_area_without_overlapping_text_passes() {
        let clip = Bounds::new(point(px(0.), px(0.)), size(px(100.), px(80.)));
        for cover in [
            Bounds::new(point(px(30.), px(10.)), size(px(20.), px(50.))),
            Bounds::new(point(px(-10.), px(20.)), size(px(20.), px(100.))),
            Bounds::new(point(px(150.), px(20.)), size(px(20.), px(40.))),
            clip,
        ] {
            let pieces = uncovered(clip, cover);
            let area = |bounds: Bounds<Pixels>| {
                f32::from(bounds.size.width).max(0.) * f32::from(bounds.size.height).max(0.)
            };
            assert!(
                (pieces.iter().copied().map(area).sum::<f32>() + area(clip.intersect(&cover))
                    - area(clip))
                .abs()
                    < 0.001
            );
            for (index, piece) in pieces.iter().enumerate() {
                assert_eq!(area(piece.intersect(&cover)), 0.);
                for other in &pieces[index + 1..] {
                    assert_eq!(area(piece.intersect(other)), 0.);
                }
            }
        }
    }
}
