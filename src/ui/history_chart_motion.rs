use super::*;
use crate::history::Trend;
use std::sync::Arc;

pub(super) const DURATION: Duration = Duration::from_millis(200);
type Scope = (i64, i64);

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Bar {
    pub left: f32,
    pub width: f32,
    pub down: f32,
    pub up: f32,
    pub opacity: f32,
    pub missing: f32,
    pub zero: f32,
}

fn bars(data: &Trend) -> Vec<Bar> {
    let peak = data
        .buckets
        .iter()
        .map(|b| b.bytes.total())
        .max()
        .unwrap_or(0);
    let ceiling = chart::ChartScale::for_peak(peak as f64, false).ceiling;
    let width = 1. / data.buckets.len().max(1) as f32;
    data.buckets
        .iter()
        .enumerate()
        .map(|(i, b)| Bar {
            left: i as f32 * width,
            width,
            down: if b.observed {
                (b.bytes.down as f64 / ceiling) as f32
            } else {
                0.
            },
            up: if b.observed {
                (b.bytes.up as f64 / ceiling) as f32
            } else {
                0.
            },
            opacity: 1.,
            missing: f32::from(!b.observed && b.from < data.generated_at),
            zero: f32::from(b.observed && b.bytes.total() == 0),
        })
        .collect()
}

// Match the demo's CSS ease for placement, and cubic-bezier(.2,.7,.2,1) for height.
fn bezier(delta: f32, x1: f32, y1: f32, x2: f32, y2: f32) -> f32 {
    let delta = delta.clamp(0., 1.);
    if delta == 0. || delta == 1. {
        return delta;
    }
    let coordinate = |t: f32, a: f32, b: f32| {
        3. * (1. - t).powi(2) * t * a + 3. * (1. - t) * t * t * b + t.powi(3)
    };
    let (mut lo, mut hi) = (0., 1.);
    for _ in 0..16 {
        let mid = (lo + hi) / 2.;
        if coordinate(mid, x1, x2) < delta {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    coordinate((lo + hi) / 2., y1, y2)
}

fn interpolate(from: &[Bar], to: &[Bar], delta: f32) -> Vec<Bar> {
    if delta >= 1. {
        return to.to_vec();
    }
    let placement = bezier(delta, 0.25, 0.1, 0.25, 1.);
    let height = bezier(delta, 0.2, 0.7, 0.2, 1.);
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    (0..from.len().max(to.len()))
        .map(|i| {
            let a = from.get(i).copied().unwrap_or(Bar {
                width: 1. / from.len().max(1) as f32,
                ..Bar::default()
            });
            let b = to.get(i).copied().unwrap_or(Bar {
                width: 1. / to.len().max(1) as f32,
                ..Bar::default()
            });
            Bar {
                left: lerp(a.left, b.left, placement),
                width: lerp(a.width, b.width, placement),
                down: lerp(a.down, b.down, height),
                up: lerp(a.up, b.up, height),
                opacity: lerp(a.opacity, b.opacity, placement),
                missing: lerp(a.missing, b.missing, placement),
                zero: lerp(a.zero, b.zero, placement),
            }
        })
        .collect()
}

#[derive(Clone)]
struct Scene {
    data: Arc<Trend>,
    bars: Vec<Bar>,
    scope: Scope,
    revision: u64,
}

struct Transition {
    from: Scene,
    target: Scope,
    revision: u64,
    started: Option<Instant>,
}

#[derive(Default)]
pub(super) struct ChartMotion {
    displayed: Option<Scene>,
    transition: Option<Transition>,
    prepared: Option<(Scope, u64)>,
    #[cfg(test)]
    pub observed_frames: Vec<(bool, Vec<Bar>)>,
}

pub(super) struct Frame {
    pub bars: Vec<Bar>,
    pub running: bool,
    pub finished: bool,
    pub waiting: bool,
}

impl ChartMotion {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn begin(
        &mut self,
        scope: Scope,
        target: Scope,
        revision: u64,
        fallback: Option<Arc<Trend>>,
        enabled: bool,
    ) {
        if !enabled || scope == target {
            self.clear();
            return;
        }
        let from = self
            .displayed
            .as_ref()
            .filter(|s| s.scope == scope && s.revision == revision)
            .cloned()
            .or_else(|| {
                self.transition
                    .as_ref()
                    .filter(|t| t.target == scope && t.revision == revision)
                    .map(|t| t.from.clone())
            })
            .or_else(|| {
                fallback.map(|data| Scene {
                    bars: bars(&data),
                    data,
                    scope,
                    revision,
                })
            });
        self.transition = from.map(|from| Transition {
            from,
            target,
            revision,
            started: None,
        });
        #[cfg(test)]
        self.observed_frames.clear();
    }

    pub fn prepare(
        &mut self,
        incoming: Option<Arc<Trend>>,
        scope: Scope,
        revision: u64,
        loading: bool,
        now: Instant,
    ) -> Option<Arc<Trend>> {
        self.prepared = Some((scope, revision));
        if self
            .transition
            .as_ref()
            .is_some_and(|t| t.target != scope || t.revision != revision)
        {
            self.clear();
            self.prepared = Some((scope, revision));
        }
        if let Some(data) = incoming {
            if let Some(t) = &mut self.transition {
                t.started.get_or_insert(now);
            }
            return Some(data);
        }
        if loading && let Some(t) = &self.transition {
            return Some(t.from.data.clone());
        }
        self.transition = None;
        self.displayed = None;
        None
    }

    pub fn sample(
        &mut self,
        data: &Arc<Trend>,
        scope: Scope,
        revision: u64,
        now: Instant,
    ) -> Frame {
        let target = bars(data);
        let mut frame = Frame {
            bars: target.clone(),
            running: false,
            finished: false,
            waiting: false,
        };
        if self.prepared != Some((scope, revision)) {
            return frame;
        }
        if let Some(t) = &self.transition {
            if let Some(started) = t.started {
                let delta =
                    now.saturating_duration_since(started).as_secs_f32() / DURATION.as_secs_f32();
                frame.bars = interpolate(&t.from.bars, &target, delta);
                frame.running = delta < 1.;
                frame.finished = !frame.running;
                if !frame.running {
                    self.transition = None;
                }
            } else {
                frame.bars = t.from.bars.clone();
                frame.waiting = true;
            }
        }
        frame
    }

    pub fn record(&mut self, data: Arc<Trend>, frame: &Frame, scope: Scope, revision: u64) {
        if self.prepared == Some((scope, revision)) {
            #[cfg(test)]
            if frame.running || self.observed_frames.last().is_some_and(|f| f.0) {
                self.observed_frames
                    .push((frame.running, frame.bars.clone()));
                if self.observed_frames.len() > 64 {
                    self.observed_frames.remove(0);
                }
            }
            self.displayed = Some(Scene {
                data,
                bars: frame.bars.clone(),
                scope,
                revision,
            });
        }
    }

    pub fn slot(&self, index: usize, scope: Scope, revision: u64) -> Option<(f32, f32)> {
        self.transition.as_ref()?.started?;
        let scene = self
            .displayed
            .as_ref()
            .filter(|s| s.scope == scope && s.revision == revision)?;
        let b = scene.bars.get(index)?;
        (b.opacity > 0.1).then_some((b.left, b.width))
    }

    pub fn hit(&self, x: f32, count: usize, scope: Scope, revision: u64) -> Option<usize> {
        (0..count).rev().find(|&i| {
            self.slot(i, scope, revision)
                .is_some_and(|(left, width)| x >= left && x < left + width)
        })
    }

    pub fn animating(&self) -> bool {
        self.transition
            .as_ref()
            .and_then(|t| t.started)
            .is_some_and(|start| start.elapsed() < DURATION)
    }
}

pub(super) fn paint_bars(
    bounds: Bounds<Pixels>,
    frame: &Frame,
    p: Palette,
    hovered: Option<usize>,
    focused: Option<Scope>,
    data: &Trend,
    window: &mut Window,
) {
    let opacity = if frame.waiting { 0.4 } else { 1. };
    for (i, b) in frame.bars.iter().enumerate() {
        let alpha = b.opacity * opacity;
        if alpha <= 0. {
            continue;
        }
        let slot = bounds.size.width * b.width;
        let x = bounds.left() + bounds.size.width * b.left;
        let width = (slot * 0.58).min(px(26.));
        let left = x + (slot - width) / 2.;
        if !frame.waiting
            && (hovered == Some(i)
                || data.buckets.get(i).is_some_and(|bucket| {
                    focused.is_some_and(|(from, to)| bucket.from < to && bucket.to > from)
                }))
        {
            let mut color = rgb(p.accent);
            color.a = 0.10 * alpha;
            window.paint_quad(fill(
                Bounds::new(point(x, bounds.top()), size(slot, bounds.size.height)),
                color,
            ));
        }
        if b.missing > 0. {
            let mut color = rgb(p.muted);
            color.a = 0.6 * alpha * b.missing;
            window.paint_quad(
                outline(
                    Bounds::new(
                        point(x + slot / 2. - px(1.), bounds.bottom() - px(3.)),
                        size(px(2.), px(2.)),
                    ),
                    color,
                    BorderStyle::Solid,
                )
                .corner_radii(px(1.))
                .border_widths(px(0.6)),
            );
        }
        if b.zero > 0. {
            let mut color = rgb(p.muted);
            color.a *= alpha * b.zero;
            window.paint_quad(fill(
                Bounds::new(
                    point(x + slot / 2. - px(3.), bounds.bottom() - px(1.)),
                    size(px(6.), px(1.)),
                ),
                color,
            ));
        }
        let down = bounds.size.height * b.down;
        let up = bounds.size.height * b.up;
        let height = down + up;
        if height <= px(0.) {
            continue;
        }
        let silhouette = Bounds::new(point(left, bounds.bottom() - height), size(width, height));
        let radius = px(4.).min(width * 0.4).min(height / 2.);
        let corners = Corners {
            top_left: radius,
            top_right: radius,
            bottom_left: px(0.),
            bottom_right: px(0.),
        };
        let seam = px(1. / window.scale_factor()).min(down).min(up);
        let mut bottom = bounds.bottom();
        for (index, (height, tint)) in [(down, p.download), (up, p.upload)].into_iter().enumerate()
        {
            if height <= px(0.) {
                continue;
            }
            let mut segment = Bounds::new(point(left, bottom - height), size(width, height));
            if index == 0 {
                segment.origin.y += seam / 2.;
            }
            segment.size.height -= seam / 2.;
            let mut color = rgb(tint);
            color.a = 0.92 * alpha;
            window.with_content_mask(Some(ContentMask { bounds: segment }), |window| {
                window.paint_quad(fill(silhouette, color).corner_radii(corners));
            });
            bottom -= height;
        }
    }
}

impl TrafficApp {
    pub(super) fn begin_history_chart_motion(&mut self, target: Scope) {
        let fallback = (self.report.query.as_ref() == Some(&self.query))
            .then(|| self.report.history.clone())
            .flatten();
        self.history_state.chart_motion.borrow_mut().begin(
            (self.query.from, self.query.to),
            target,
            self.data.storage_revision,
            fallback,
            crate::native::client_area_animations_enabled(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        history::{Bucket, Grain},
        model::Bytes,
    };
    use core::prelude::v1::test;
    fn trend(n: usize, offset: i64) -> Arc<Trend> {
        Arc::new(Trend {
            from: offset,
            to: offset + n as i64,
            generated_at: i64::MAX,
            grain: Grain::Day,
            buckets: (0..n)
                .map(|i| Bucket {
                    from: offset + i as i64,
                    to: offset + i as i64 + 1,
                    bytes: Bytes {
                        down: (i as u64 + 1) * 1000,
                        up: 200,
                    },
                    covered_ms: 1,
                    expected_ms: 1,
                    observed: true,
                })
                .collect(),
        })
    }
    #[test]
    fn drill_and_back_morph_geometry_and_finish_exactly() {
        for (a, b) in [(7, 24), (24, 60), (60, 24), (24, 7)] {
            let (old, next) = (trend(a, 0), trend(b, 100));
            let from = bars(&old);
            let to = bars(&next);
            assert_eq!(interpolate(&from, &to, 1.), to);
            let start = interpolate(&from, &to, 0.);
            assert_eq!(&start[..a], &from);
            let mid = interpolate(&from, &to, 0.5);
            assert_ne!(mid[1].left, from[1].left);
            assert_ne!(mid[1].left, to[1].left);
            assert!(
                mid.iter()
                    .all(|v| v.down >= 0. && v.up >= 0. && (0. ..=1.).contains(&v.opacity))
            );
        }
    }
    #[test]
    fn waiting_keeps_old_scene_and_refresh_does_not_restart_motion() {
        let (old, next) = (trend(7, 0), trend(24, 100));
        let mut state = ChartMotion::default();
        let now = Instant::now();
        state.begin((0, 7), (100, 124), 1, Some(old.clone()), true);
        assert!(Arc::ptr_eq(
            &state.prepare(None, (100, 124), 1, true, now).unwrap(),
            &old
        ));
        assert!(state.sample(&old, (100, 124), 1, now).waiting);
        state.prepare(Some(next.clone()), (100, 124), 1, false, now);
        let started = state.transition.as_ref().unwrap().started;
        state.prepare(
            Some(trend(24, 100)),
            (100, 124),
            1,
            false,
            now + DURATION / 2,
        );
        assert_eq!(state.transition.as_ref().unwrap().started, started);
        assert!(!state.sample(&next, (100, 124), 1, now + DURATION).running);
        assert!(state.transition.is_none());
    }
    #[test]
    fn rapid_navigation_uses_the_visible_frame_and_old_sources_are_rejected() {
        let (old, next) = (trend(7, 0), trend(24, 100));
        let mut state = ChartMotion::default();
        let now = Instant::now();
        state.begin((0, 7), (100, 124), 1, Some(old), true);
        state.prepare(Some(next.clone()), (100, 124), 1, false, now);
        let mid = state.sample(&next, (100, 124), 1, now + DURATION / 2);
        state.record(next.clone(), &mid, (100, 124), 1);
        state.begin((100, 124), (200, 260), 1, Some(next), true);
        assert_eq!(state.transition.as_ref().unwrap().from.bars, mid.bars);
        assert!(state.prepare(None, (200, 260), 2, true, now).is_none());
        assert!(state.transition.is_none() && state.displayed.is_none());
    }
    #[test]
    fn unrelated_range_failure_and_reduced_motion_do_not_keep_old_bars() {
        let mut state = ChartMotion::default();
        let now = Instant::now();
        state.begin((0, 7), (100, 124), 1, Some(trend(7, 0)), false);
        assert!(state.transition.is_none());
        for (scope, loading) in [((100, 124), false), ((300, 400), true)] {
            state.begin((0, 7), (100, 124), 1, Some(trend(7, 0)), true);
            assert!(state.prepare(None, scope, 1, loading, now).is_none());
            assert!(state.transition.is_none());
        }
    }
    #[test]
    fn missing_zero_and_future_markers_keep_their_meaning() {
        let mut data = (*trend(3, 0)).clone();
        data.generated_at = 2;
        for b in &mut data.buckets {
            b.bytes = Bytes::default();
        }
        data.buckets[1].observed = false;
        data.buckets[1].bytes.down = 20_000;
        data.buckets[2].observed = false;
        let b = bars(&data);
        assert_eq!((b[0].zero, b[0].missing), (1., 0.));
        assert_eq!((b[1].zero, b[1].missing), (0., 1.));
        assert_eq!((b[2].zero, b[2].missing), (0., 0.));
        assert!(b.iter().all(|bar| bar.down == 0. && bar.up == 0.));
    }
    #[test]
    fn pointer_slots_follow_the_painted_bars_and_stale_frames_cannot_overwrite() {
        let (old, next) = (trend(24, 0), trend(7, 100));
        let now = Instant::now();
        let mut state = ChartMotion::default();
        state.begin((0, 24), (100, 107), 1, Some(old), true);
        state.prepare(Some(next.clone()), (100, 107), 1, false, now);
        let frame = state.sample(&next, (100, 107), 1, now + DURATION / 2);
        state.record(next.clone(), &frame, (100, 107), 1);
        for (i, bar) in frame.bars.iter().take(7).enumerate() {
            assert_eq!(
                state.hit(bar.left + bar.width / 2., 7, (100, 107), 1),
                Some(i)
            );
        }
        state.prepare(None, (200, 210), 2, true, now);
        state.record(next, &frame, (100, 107), 1);
        assert!(state.displayed.is_none());
    }
}
