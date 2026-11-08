use super::*;

const FADE_HEIGHT: f32 = 28.;
const SHADE_HEIGHT: f32 = 32.;

fn feather_opacity(distance: f32, width: f32) -> f32 {
    let value = (distance / width).clamp(0., 1.);
    value * value * (3. - 2. * value)
}

pub(super) fn bottom_shade(handle: &ScrollHandle, palette: Palette) -> impl IntoElement {
    let handle = handle.clone();
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            if !controls::has_more_below(&handle) || bounds.size.width <= px(0.) {
                return;
            }
            let remaining = f32::from(handle.max_offset().height + handle.offset().y).max(0.);
            let strength = (remaining / SHADE_HEIGHT).min(1.);
            let mut gray = rgb(0x808080);
            gray.a = if palette.dark { 0.09 } else { 0.07 } * strength;
            let mut clear = gray;
            clear.a = 0.;
            let width = f32::from(bounds.size.width);
            let edge = 32f32.min(width / 2.);
            let scale = window.scale_factor();
            let draw = |x: f32, width: f32, opacity: f32, window: &mut Window| {
                if width <= 0. {
                    return;
                }
                let mut color = gray;
                color.a *= opacity;
                window.paint_quad(fill(
                    Bounds::new(
                        point(bounds.left() + px(x), bounds.top()),
                        size(px(width), bounds.size.height),
                    ),
                    linear_gradient(
                        180.,
                        linear_color_stop(clear, 0.),
                        linear_color_stop(color, 1.),
                    ),
                ));
            };
            // Paint the center once; only the two soft edges need pixel strips.
            draw(edge, width - edge * 2., 1., window);
            for column in 0..(edge * scale).ceil() as i32 {
                let x = column as f32 / scale;
                let end = ((column + 1) as f32 / scale).min(edge);
                let opacity = feather_opacity((x + end) / 2., edge);
                draw(x, end - x, opacity, window);
                draw(width - end, end - x, opacity, window);
            }
        },
    )
    .absolute()
    .bottom_0()
    .left_0()
    .right(px(12.))
    .h(px(SHADE_HEIGHT))
}

#[derive(Clone, Copy)]
struct FadeRegion {
    bottom: f32,
    height: f32,
    strength: f32,
}

impl FadeRegion {
    fn opacity(self, y: f32) -> f32 {
        let distance = ((self.bottom - y) / self.height).clamp(0., 1.);
        let smooth = distance * distance * (3. - 2. * distance);
        1. - self.strength * (1. - smooth)
    }
}

thread_local! {
    static REGION: Cell<Option<FadeRegion>> = const { Cell::new(None) };
}

pub(super) struct FadingContent {
    child: AnyElement,
    handle: ScrollHandle,
}

impl FadingContent {
    pub fn new(child: impl IntoElement, handle: &ScrollHandle) -> Self {
        Self {
            child: child.into_any_element(),
            handle: handle.clone(),
        }
    }
}

impl IntoElement for FadingContent {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for FadingContent {
    type RequestLayoutState = ();
    type PrepaintState = ();
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
        (self.child.request_layout(window, cx), ())
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.prepaint(window, cx);
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let remaining = f32::from(self.handle.max_offset().height + self.handle.offset().y).max(0.);
        let region = controls::has_more_below(&self.handle).then_some(FadeRegion {
            bottom: self.handle.bounds().bottom().into(),
            height: FADE_HEIGHT
                .min(self.handle.bounds().size.height.into())
                .max(1.),
            strength: (remaining / FADE_HEIGHT).min(1.),
        });
        struct Restore(Option<FadeRegion>);
        impl Drop for Restore {
            fn drop(&mut self) {
                REGION.set(self.0);
            }
        }
        let _restore = Restore(REGION.replace(region));
        self.child.paint(window, cx);
    }
}

/// Paint text through physical-pixel strips with actual alpha. The content
/// remains transparent over Mica, independently of the optional gray shade.
pub(super) struct FadingText(pub SharedString);

impl IntoElement for FadingText {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for FadingText {
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
        self.0.request_layout(id, inspector, window, cx)
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
        self.0.prepaint(id, inspector, bounds, state, window, cx)
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
        let Some(region) = REGION
            .get()
            .filter(|region| f32::from(bounds.bottom()) > region.bottom - region.height)
        else {
            self.0
                .paint(id, inspector, bounds, state, prepaint, window, cx);
            return;
        };
        let clip = window.content_mask().bounds;
        let top = f32::from(bounds.top().max(clip.top()));
        let bottom = f32::from(bounds.bottom().min(clip.bottom()));
        if bottom <= top {
            return;
        }
        let fade_top = region.bottom - region.height;
        if top < fade_top {
            let mask = ContentMask {
                bounds: Bounds::new(
                    point(clip.left(), px(top)),
                    size(clip.size.width, px(bottom.min(fade_top) - top)),
                ),
            };
            window.with_content_mask(Some(mask), |window| {
                self.0
                    .paint(id, inspector, bounds, state, prepaint, window, cx);
            });
        }
        let top = top.max(fade_top);
        let scale = window.scale_factor();
        let start = (top * scale).floor() as i32;
        let end = (bottom * scale).ceil() as i32;
        let mut opacity = gpui::Interactivity::default();
        for row in start..end {
            let sample_y = (row as f32 + 0.5) / scale;
            let y = (row as f32 / scale).max(top);
            let end_y = ((row + 1) as f32 / scale).min(bottom);
            opacity.base_style.opacity = Some(region.opacity(sample_y));
            let mask = ContentMask {
                bounds: Bounds::new(
                    point(clip.left(), px(y)),
                    size(clip.size.width, px(end_y - y)),
                ),
            };
            window.with_content_mask(Some(mask), |window| {
                opacity.paint(None, None, bounds, None, window, cx, |_, window, cx| {
                    self.0
                        .paint(id, inspector, bounds, state, prepaint, window, cx);
                });
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn gray_shade_feathers_away_at_the_sides() {
        assert_eq!(feather_opacity(0., 18.), 0.);
        assert_eq!(feather_opacity(18., 18.), 1.);
        assert!((feather_opacity(9., 18.) - 0.5).abs() < 0.001);
        for x in 0..18 {
            assert!(feather_opacity(x as f32, 18.) <= feather_opacity(x as f32 + 1., 18.));
        }
        assert!(feather_opacity(0.1, 18.) < 0.001);
        assert!(feather_opacity(17.9, 18.) > 0.999);
    }
    #[::core::prelude::v1::test]
    fn fade_is_monotonic_and_relaxes_when_reaching_the_end() {
        let region = FadeRegion {
            bottom: 100.,
            height: 28.,
            strength: 1.,
        };
        assert_eq!(region.opacity(72.), 1.);
        assert_eq!(region.opacity(100.), 0.);
        assert!((region.opacity(86.) - 0.5).abs() < 0.001);
        for y in 72..100 {
            assert!(region.opacity(y as f32) >= region.opacity(y as f32 + 1.));
        }
        let near_end = FadeRegion {
            strength: 0.1,
            ..region
        };
        assert!((near_end.opacity(100.) - 0.9).abs() < 0.001);
        let end = FadeRegion {
            strength: 0.,
            ..region
        };
        assert_eq!(end.opacity(100.), 1.);
    }
}
