use super::*;
use std::cell::RefCell;

#[derive(Clone, Default)]
pub(super) struct ScrollbarState(Rc<RefCell<Interaction>>);

#[derive(Default)]
struct Interaction {
    hovered: bool,
    drag: Option<(Pixels, Pixels)>,
    last_offset: Option<Pixels>,
    last_scroll: Option<Instant>,
    motion: Option<ScrollMotion>,
    frame_pending: bool,
    idle_timer_pending: bool,
}

impl ScrollbarState {
    /// Native overflow owns scrolling; this only reveals the thumb for real wheel input.
    pub(super) fn note_native_scroll(&self, handle: &ScrollHandle, delta: ScrollDelta) -> bool {
        let delta = delta.pixel_delta(px(1.));
        if handle.max_offset().height <= px(0.) || (delta.x.is_zero() && delta.y.is_zero()) {
            return false;
        }
        self.0.borrow_mut().last_scroll = Some(Instant::now());
        true
    }

    pub(super) fn scroll(
        &self,
        handle: &ScrollHandle,
        vertical: bool,
        delta: ScrollDelta,
        line_height: Pixels,
    ) -> bool {
        let smooth = matches!(delta, ScrollDelta::Lines(_));
        let delta = delta.pixel_delta(line_height);
        let delta = if vertical {
            if delta.y.is_zero() { delta.x } else { delta.y }
        } else if delta.x.is_zero() {
            delta.y
        } else {
            delta.x
        };
        if delta.is_zero() {
            return false;
        }
        let maximum = if vertical {
            handle.max_offset().height
        } else {
            handle.max_offset().width
        };
        let current = axis_offset(handle, vertical);
        let mut interaction = self.0.borrow_mut();
        interaction.last_scroll = Some(Instant::now());
        interaction.last_offset = Some(current);
        if smooth {
            let previous_target = interaction
                .motion
                .as_ref()
                .map_or(current, |motion| motion.target);
            let base = if (previous_target > current && delta < px(0.))
                || (previous_target < current && delta > px(0.))
            {
                current
            } else {
                previous_target
            };
            let target = (base + delta).clamp(-maximum, px(0.));
            interaction.motion = (target != current).then_some(ScrollMotion {
                from: current,
                target,
                started: Instant::now(),
            });
        } else {
            // Precision touchpads already supply smooth pixel deltas and native momentum.
            interaction.motion = None;
            let value = (current + delta).clamp(-maximum, px(0.));
            set_axis_offset(handle, vertical, value);
            interaction.last_offset = Some(value);
        }
        drop(interaction);
        true
    }
}

const SCROLL_DURATION: Duration = Duration::from_millis(130);
const ACTIVE_HOLD: Duration = Duration::from_millis(450);
const ACTIVE_FADE: Duration = Duration::from_millis(220);
const HIDDEN_HOLD: Duration = Duration::from_millis(150);
const HIDDEN_FADE: Duration = Duration::from_millis(150);

struct ScrollMotion {
    from: Pixels,
    target: Pixels,
    started: Instant,
}

impl ScrollMotion {
    fn sample(&self, now: Instant) -> (Pixels, bool) {
        let progress = (now.duration_since(self.started).as_secs_f32()
            / SCROLL_DURATION.as_secs_f32())
        .clamp(0., 1.);
        let eased = 1. - (1. - progress).powi(3);
        (self.from + (self.target - self.from) * eased, progress < 1.)
    }
}

impl Interaction {
    fn activity(&self, now: Instant, scrolling_only: bool) -> f32 {
        let (hold, fade) = if scrolling_only {
            (HIDDEN_HOLD, HIDDEN_FADE)
        } else {
            (ACTIVE_HOLD, ACTIVE_FADE)
        };
        self.last_scroll.map_or(0., |last| {
            1. - (now.duration_since(last).saturating_sub(hold).as_secs_f32() / fade.as_secs_f32())
                .clamp(0., 1.)
        })
    }
    fn opacity(&self, now: Instant, scrolling_only: bool, dark: bool) -> f32 {
        let activity = self.activity(now, scrolling_only);
        if self.drag.is_some() {
            return 0.65;
        }
        if scrolling_only {
            return 0.42 * activity;
        }
        let idle = if dark { 0.16 } else { 0.10 };
        let active = idle + (0.42 - idle) * activity;
        if self.hovered {
            0.38f32.max(active)
        } else {
            active
        }
    }
}

fn axis_offset(handle: &ScrollHandle, vertical: bool) -> Pixels {
    if vertical {
        handle.offset().y
    } else {
        handle.offset().x
    }
}

fn set_axis_offset(handle: &ScrollHandle, vertical: bool, value: Pixels) {
    let mut offset = handle.offset();
    if vertical {
        offset.y = value;
    } else {
        offset.x = value;
    }
    handle.set_offset(offset);
}

fn animate_scroll(
    state: ScrollbarState,
    handle: ScrollHandle,
    vertical: bool,
    view: EntityId,
    window: &Window,
) {
    if state.0.borrow().frame_pending {
        return;
    }
    state.0.borrow_mut().frame_pending = true;
    window.on_next_frame(move |_, cx| {
        let mut interaction = state.0.borrow_mut();
        interaction.frame_pending = false;
        let Some(motion) = &interaction.motion else {
            return;
        };
        // Keyboard navigation, dragging or content changes can supersede a wheel animation.
        if interaction
            .last_offset
            .is_some_and(|offset| offset != axis_offset(&handle, vertical))
        {
            interaction.motion = None;
            return;
        }
        let (value, running) = motion.sample(Instant::now());
        let maximum = if vertical {
            handle.max_offset().height
        } else {
            handle.max_offset().width
        };
        let value = value.clamp(-maximum, px(0.));
        set_axis_offset(&handle, vertical, value);
        interaction.last_offset = Some(value);
        interaction.last_scroll = Some(Instant::now());
        if !running {
            interaction.motion = None;
        }
        cx.notify(view);
    });
}

pub(super) struct Scrollbar {
    state: ScrollbarState,
    handle: ScrollHandle,
    vertical: bool,
    handle_wheel: bool,
    scrolling_only: bool,
}

pub(super) struct ScrollbarGeometry {
    rail: Bounds<Pixels>,
    thumb: Bounds<Pixels>,
    hitbox: Hitbox,
    viewport: Hitbox,
    maximum: Pixels,
    travel: Pixels,
}

pub(super) fn thumb_geometry(length: f32, maximum: f32, offset: f32) -> Option<(f32, f32)> {
    if length <= 0. || maximum <= 0. {
        return None;
    }
    let thumb = (length * length / (length + maximum)).max(24.).min(length);
    let start = (-offset / maximum).clamp(0., 1.) * (length - thumb);
    Some((start, thumb))
}

impl Scrollbar {
    pub fn vertical(state: &ScrollbarState, handle: &ScrollHandle) -> Self {
        Self {
            state: state.clone(),
            handle: handle.clone(),
            vertical: true,
            handle_wheel: true,
            scrolling_only: false,
        }
    }
    pub fn horizontal(state: &ScrollbarState, handle: &ScrollHandle) -> Self {
        Self {
            state: state.clone(),
            handle: handle.clone(),
            vertical: false,
            handle_wheel: true,
            scrolling_only: false,
        }
    }

    /// Hide even on hover, and do not mistake programmatic offset resets for user scrolling.
    pub fn scrolling_only(mut self, enabled: bool) -> Self {
        self.scrolling_only = enabled;
        self
    }

    /// Keep native overflow wheel routing, including nested scroll containers.
    pub fn native_wheel(mut self) -> Self {
        self.handle_wheel = false;
        self
    }
}

impl IntoElement for Scrollbar {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for Scrollbar {
    type RequestLayoutState = ();
    type PrepaintState = Option<ScrollbarGeometry>;
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
        let style = Style {
            position: Position::Absolute,
            size: size(relative(1.).into(), relative(1.).into()),
            ..Style::default()
        };
        (window.request_layout(style, None, cx), ())
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        _: &mut App,
    ) -> Self::PrepaintState {
        let vertical = self.vertical;
        let length = if vertical {
            bounds.size.height
        } else {
            bounds.size.width
        };
        let maximum = if vertical {
            self.handle.max_offset().height
        } else {
            self.handle.max_offset().width
        };
        let offset = if vertical {
            self.handle.offset().y
        } else {
            self.handle.offset().x
        };
        let Some((start, thumb_length)) =
            thumb_geometry(length.into(), maximum.into(), offset.into())
        else {
            let mut interaction = self.state.0.borrow_mut();
            interaction.drag = None;
            interaction.motion = None;
            interaction.last_offset = None;
            return None;
        };
        let rail = if vertical {
            Bounds::new(
                point(bounds.right() - px(12.), bounds.top()),
                size(px(12.), length),
            )
        } else {
            Bounds::new(
                point(bounds.left(), bounds.bottom() - px(12.)),
                size(length, px(12.)),
            )
        };
        let mut interaction = self.state.0.borrow_mut();
        if !self.scrolling_only && interaction.last_offset.is_some_and(|last| last != offset) {
            interaction.last_scroll = Some(Instant::now());
        }
        interaction.last_offset = Some(offset);
        let width = px(if interaction.drag.is_some() {
            5.
        } else if interaction.hovered {
            4.
        } else {
            3.
        });
        let thumb = if vertical {
            Bounds::new(
                point(rail.left() + (px(12.) - width) / 2., rail.top() + px(start)),
                size(width, px(thumb_length)),
            )
        } else {
            Bounds::new(
                point(rail.left() + px(start), rail.top() + (px(12.) - width) / 2.),
                size(px(thumb_length), width),
            )
        };
        let hitbox = window.insert_hitbox(rail, HitboxBehavior::Normal);
        let viewport = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        Some(ScrollbarGeometry {
            rail,
            thumb,
            hitbox,
            viewport,
            maximum,
            travel: length - px(thumb_length),
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
        cx: &mut App,
    ) {
        let Some(geometry) = geometry else {
            return;
        };
        let interaction = self.state.0.borrow();
        let activity = interaction.activity(Instant::now(), self.scrolling_only);
        let hold = if self.scrolling_only {
            HIDDEN_HOLD
        } else {
            ACTIVE_HOLD
        };
        let mut color: Rgba = Theme::global(cx).muted_foreground.into();
        color.a = interaction.opacity(
            Instant::now(),
            self.scrolling_only,
            Theme::global(cx).is_dark(),
        );
        let interactive = color.a > 0.;
        drop(interaction);
        let view = window.current_view();
        if self.state.0.borrow().motion.is_some() {
            animate_scroll(
                self.state.clone(),
                self.handle.clone(),
                self.vertical,
                view,
                window,
            );
        }
        if activity > 0. {
            let mut interaction = self.state.0.borrow_mut();
            let elapsed = Instant::now().duration_since(interaction.last_scroll.unwrap());
            if elapsed >= hold {
                window.request_animation_frame();
            } else if !interaction.idle_timer_pending {
                interaction.idle_timer_pending = true;
                let state = self.state.clone();
                window
                    .spawn(cx, async move |cx| {
                        Timer::after(hold - elapsed).await;
                        state.0.borrow_mut().idle_timer_pending = false;
                        cx.update(|_, cx| cx.notify(view)).ok();
                    })
                    .detach();
            }
        }
        window.paint_quad(fill(geometry.thumb, color).corner_radii(px(3.)));
        let vertical = self.vertical;
        let coordinate =
            move |position: Point<Pixels>| if vertical { position.y } else { position.x };
        let hitbox = geometry.hitbox.clone();
        let thumb = geometry.thumb;
        let rail = geometry.rail;
        let maximum = geometry.maximum;
        let travel = geometry.travel;
        let state = self.state.clone();
        let handle = self.handle.clone();
        let viewport = geometry.viewport.clone();
        if self.handle_wheel {
            window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble || !viewport.should_handle_scroll(window) {
                    return;
                }
                if handle.max_offset().width > px(0.) && handle.max_offset().height > px(0.) {
                    let delta = event.delta.pixel_delta(window.line_height());
                    let horizontal = event.modifiers.shift || delta.x.abs() > delta.y.abs();
                    if horizontal == vertical {
                        return;
                    }
                }
                if !state.scroll(&handle, vertical, event.delta, window.line_height()) {
                    return;
                }
                cx.notify(view);
                cx.stop_propagation();
            });
        }
        let state = self.state.clone();
        let handle = self.handle.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble
                || event.button != MouseButton::Left
                || !hitbox.is_hovered(window)
                || travel <= px(0.)
                || !interactive
            {
                return;
            }
            let mouse = coordinate(event.position);
            let mut offset = handle.offset();
            if !thumb.dilate(px(3.)).contains(&event.position) {
                let thumb_length = if vertical {
                    thumb.size.height
                } else {
                    thumb.size.width
                };
                let origin = if vertical { rail.top() } else { rail.left() };
                let next = -((mouse - origin - thumb_length / 2.) / travel).clamp(0., 1.) * maximum;
                if vertical {
                    offset.y = next;
                } else {
                    offset.x = next;
                }
                handle.set_offset(offset);
            }
            let mut interaction = state.0.borrow_mut();
            interaction.motion = None;
            interaction.last_scroll = Some(Instant::now());
            interaction.drag = Some((mouse, if vertical { offset.y } else { offset.x }));
            window.prevent_default();
            cx.notify(view);
            cx.stop_propagation();
        });
        let state = self.state.clone();
        let handle = self.handle.clone();
        let hitbox = geometry.hitbox.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Capture {
                return;
            }
            let mut interaction = state.0.borrow_mut();
            if interaction.drag.is_some() && event.pressed_button != Some(MouseButton::Left) {
                interaction.drag = None;
            }
            if let Some((start, offset)) = interaction.drag {
                interaction.last_scroll = Some(Instant::now());
                let mut next = handle.offset();
                let value = (offset + (start - coordinate(event.position)) / travel * maximum)
                    .clamp(-maximum, px(0.));
                if vertical {
                    next.y = value;
                } else {
                    next.x = value;
                }
                handle.set_offset(next);
                cx.notify(view);
            }
            let hovered = hitbox.is_hovered(window);
            if hovered != interaction.hovered {
                interaction.hovered = hovered;
                cx.notify(view);
            }
        });
        let state = self.state.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
            let released = phase == DispatchPhase::Capture
                && event.button == MouseButton::Left
                && state.0.borrow_mut().drag.take().is_some();
            if released {
                state.0.borrow_mut().last_scroll = Some(Instant::now());
                cx.notify(view);
                cx.stop_propagation();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn scroll_thumb_stays_in_track_and_reaches_both_ends() {
        assert_eq!(thumb_geometry(100., 0., 0.), None);
        assert_eq!(thumb_geometry(100., 300., 0.), Some((0., 25.)));
        assert_eq!(thumb_geometry(100., 300., -300.), Some((75., 25.)));
        assert_eq!(thumb_geometry(100., 300., -900.), Some((75., 25.)));
        assert_eq!(thumb_geometry(10., 1000., -1000.), Some((0., 10.)));
    }

    #[::core::prelude::v1::test]
    fn wheel_motion_is_continuous_and_stops_exactly_without_overshoot() {
        let start = Instant::now();
        for (from, target) in [(0., -240.), (-240., 0.)] {
            let motion = ScrollMotion {
                from: px(from),
                target: px(target),
                started: start,
            };
            assert_eq!(motion.sample(start), (px(from), true));
            let middle = motion.sample(start + SCROLL_DURATION / 2);
            assert!(middle.1 && middle.0 > px(from.min(target)) && middle.0 < px(from.max(target)));
            assert_eq!(motion.sample(start + SCROLL_DURATION), (px(target), false));
            assert_eq!(
                motion.sample(start + SCROLL_DURATION * 2),
                (px(target), false)
            );
        }
        let idle = Interaction::default();
        assert_eq!(idle.activity(start, false), 0.);
        let active = Interaction {
            last_scroll: Some(start),
            ..Default::default()
        };
        assert_eq!(active.activity(start + ACTIVE_HOLD, false), 1.);
        assert!(
            (active.activity(start + ACTIVE_HOLD + ACTIVE_FADE / 2, false) - 0.5).abs() < 0.001
        );
        assert_eq!(
            active.activity(start + ACTIVE_HOLD + ACTIVE_FADE, false),
            0.
        );
    }
    #[::core::prelude::v1::test]
    fn scrolling_only_thumb_hides_on_idle_and_hover_but_stays_visible_while_dragging() {
        let now = Instant::now();
        let mut interaction = Interaction {
            hovered: true,
            ..Default::default()
        };
        for dark in [false, true] {
            assert_eq!(interaction.opacity(now, true, dark), 0.);
            assert!(interaction.opacity(now, false, dark) > 0.);
        }
        interaction.last_scroll = Some(now);
        assert_eq!(interaction.opacity(now + HIDDEN_HOLD, true, false), 0.42);
        assert!(
            (interaction.opacity(now + HIDDEN_HOLD + HIDDEN_FADE / 2, true, false) - 0.21).abs()
                < 0.001
        );
        let idle_at = now + HIDDEN_HOLD + HIDDEN_FADE;
        assert_eq!(interaction.opacity(idle_at, true, false), 0.);
        interaction.drag = Some((px(0.), px(0.)));
        assert_eq!(interaction.opacity(idle_at, true, false), 0.65);
        interaction.drag = None;
        assert_eq!(interaction.opacity(idle_at, true, false), 0.);
    }
}
