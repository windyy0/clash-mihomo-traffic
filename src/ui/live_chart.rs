use super::*;

const REVEAL: Duration = Duration::from_millis(280);

#[derive(Clone)]
struct Source {
    revision: u64,
    first: Option<LiveSample>,
    last: Option<LiveSample>,
    len: usize,
}

impl PartialEq for Source {
    fn eq(&self, other: &Self) -> bool {
        // Invalid samples may contain NaN; keep their identity stable as well.
        let identity = |sample: LiveSample| {
            (
                sample.from,
                sample.to,
                sample.down.to_bits(),
                sample.up.to_bits(),
                sample.valid,
            )
        };
        self.revision == other.revision
            && self.len == other.len
            && self.first.map(identity) == other.first.map(identity)
            && self.last.map(identity) == other.last.map(identity)
    }
}

struct Input {
    source: Source,
    segments: Rc<Vec<Vec<SeriesPoint>>>,
    generation: u64,
    received: Instant,
    tail_from: Option<SeriesPoint>,
    duration: i64,
    height: f32,
    legend: bool,
    compact_axis: bool,
    palette: Palette,
    connected: bool,
    animations: bool,
}

// The parent supplies snapshots at its existing cadence. Only this view asks
// for animation frames; collecting traffic and querying the ledger stay unchanged.
pub(in crate::ui) struct LiveChart {
    input: Rc<RefCell<Option<Input>>>,
    view: Entity<LiveChartView>,
}

impl LiveChart {
    pub(in crate::ui) fn new(cx: &mut Context<TrafficApp>) -> Self {
        let input = Rc::new(RefCell::new(None));
        let view = cx.new(|_| LiveChartView {
            input: input.clone(),
            frame_pending: Rc::new(Cell::new(false)),
            scale: None,
            generation: 0,
            #[cfg(test)]
            frames: (0, 0),
        });
        Self { input, view }
    }

    #[cfg(test)]
    pub(in crate::ui) fn frame(&self, cx: &App) -> (u64, i64, Instant) {
        let (frames, time) = self.view.read(cx).frames;
        (frames, time, self.input.borrow().as_ref().unwrap().received)
    }

    pub(in crate::ui) fn element(
        &self,
        data: &Shared,
        palette: Palette,
        duration: i64,
        height: f32,
        legend: bool,
        compact_axis: bool,
    ) -> Div {
        let source = Source {
            revision: data.storage_revision,
            first: data.live.front().copied(),
            last: data.live.back().copied(),
            len: data.live.len(),
        };
        let mut slot = self.input.borrow_mut();
        let changed = slot
            .as_ref()
            .is_none_or(|old| old.source != source || old.duration != duration);
        if changed {
            let reset = slot.as_ref().is_none_or(|old| {
                old.duration != duration
                    || old.source.revision != source.revision
                    || old
                        .source
                        .last
                        .is_none_or(|last| !data.live.iter().any(|sample| sample.to == last.to))
            });
            let generation = slot.as_ref().map_or(0, |old| old.generation) + u64::from(reset);
            let segments = Rc::new(crate::live_chart::display_trend(&data.live, duration));
            let tail_from = if reset {
                None
            } else {
                slot.as_ref()
                    .and_then(|old| previous_tail(&old.segments, &segments))
            };
            let received = if reset {
                // Entering a chart or changing sources shows its existing data immediately.
                Instant::now() - REVEAL
            } else {
                Instant::now()
            };
            *slot = Some(Input {
                source,
                segments,
                generation,
                received,
                tail_from,
                duration,
                height,
                legend,
                compact_axis,
                palette,
                connected: data.connected,
                animations: native::client_area_animations_enabled(),
            });
        }
        if let Some(input) = slot.as_mut() {
            input.duration = duration;
            input.height = height;
            input.legend = legend;
            input.compact_axis = compact_axis;
            input.palette = palette;
            input.connected = data.connected;
            input.animations = native::client_area_animations_enabled();
        }
        div().w_full().child(self.view.clone())
    }
}

fn previous_tail(old: &[Vec<SeriesPoint>], next: &[Vec<SeriesPoint>]) -> Option<SeriesPoint> {
    let old = old.last()?.last()?;
    let segment = next.last()?;
    let last = segment.last()?;
    (old.time < last.time && segment.first()?.time <= old.time).then(|| old.clone())
}

fn display_frame(
    segments: &Rc<Vec<Vec<SeriesPoint>>>,
    old: Option<&SeriesPoint>,
    fraction: f32,
) -> Rc<Vec<Vec<SeriesPoint>>> {
    let Some(old) = old.filter(|_| fraction < 1.) else {
        return segments.clone();
    };
    let mut frame = segments.as_ref().clone();
    let Some(segment) = frame.last_mut() else {
        return segments.clone();
    };
    let Some(last) = segment.last_mut() else {
        return segments.clone();
    };
    let t = f64::from(fraction.clamp(0., 1.));
    let t = t * t * (3. - 2. * t);
    last.time = old.time + ((last.time - old.time) as f64 * t).round() as i64;
    last.down = old.down + (last.down - old.down) * t;
    last.up = old.up + (last.up - old.up) * t;
    // At a bucket boundary the old tail is now the previous completed point.
    // Do not leave two vertices at the same x coordinate during the first frame.
    if segment.len() > 1 && segment[segment.len() - 2].time >= segment.last().unwrap().time {
        segment.pop();
    }
    Rc::new(frame)
}

struct ScaleMotion {
    from: f64,
    target: f64,
    started: Instant,
    duration: Duration,
}

impl ScaleMotion {
    fn value(&self, now: Instant) -> f64 {
        let t = (now.saturating_duration_since(self.started).as_secs_f64()
            / self.duration.as_secs_f64())
        .clamp(0., 1.);
        let eased = t * t * (3. - 2. * t);
        self.from + (self.target - self.from) * eased
    }

    fn update(&mut self, target: f64, now: Instant, animations: bool) {
        if !animations {
            self.from = target;
            self.target = target;
            self.started = now;
        } else if self.target != target {
            self.from = self.value(now);
            self.target = target;
            let growing = target > self.from;
            self.started = now
                + if growing {
                    Duration::ZERO
                } else {
                    Duration::from_millis(800)
                };
            self.duration = if growing {
                REVEAL
            } else {
                Duration::from_millis(900)
            };
        }
    }
}

struct LiveChartView {
    input: Rc<RefCell<Option<Input>>>,
    frame_pending: Rc<Cell<bool>>,
    scale: Option<ScaleMotion>,
    generation: u64,
    #[cfg(test)]
    frames: (u64, i64),
}

impl Render for LiveChartView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let slot = self.input.borrow();
        let Some(input) = slot.as_ref() else {
            return div();
        };
        let now = Instant::now();
        let to = chrono::Utc::now().timestamp_millis();
        #[cfg(test)]
        {
            self.frames = (self.frames.0 + 1, to);
        }
        let from = to.saturating_sub(input.duration);
        let fraction = (now.saturating_duration_since(input.received).as_secs_f32()
            / REVEAL.as_secs_f32())
        .clamp(0., 1.);
        let segments = display_frame(
            &input.segments,
            input.tail_from.as_ref(),
            if input.animations { fraction } else { 1. },
        );
        let peak = segments
            .iter()
            .filter(|segment| segment.last().is_some_and(|point| point.time >= from))
            .flat_map(|segment| {
                let start = segment
                    .partition_point(|point| point.time < from)
                    .saturating_sub(3);
                segment[start..]
                    .iter()
                    .flat_map(|point| [point.down, point.up])
            })
            .fold(0f64, f64::max);
        let target = ChartScale::for_peak(peak, true).ceiling;
        if self.generation != input.generation {
            self.scale = None;
            self.generation = input.generation;
        }
        let scale = self.scale.get_or_insert(ScaleMotion {
            from: target,
            target,
            started: now,
            duration: REVEAL,
        });
        scale.update(target, now, input.animations);
        // A sudden peak must remain visible even while the extra headroom eases in.
        let ceiling = scale.value(now).max(peak);
        let fresh = input
            .source
            .last
            .is_some_and(|sample| sample.valid && to.saturating_sub(sample.to) < 5_000);
        let animate = input.animations
            && ((input.connected && fresh) || now < scale.started + scale.duration);
        render_chart(
            ChartData {
                segments,
                from,
                to,
                live: true,
                ceiling: Some(ceiling),
                animate: animate.then(|| self.frame_pending.clone()),
            },
            input.height,
            input.legend,
            input.compact_axis,
            input.palette,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point_at(time: i64, down: f64) -> SeriesPoint {
        SeriesPoint {
            time,
            down,
            up: down / 2.,
        }
    }

    #[::core::prelude::v1::test]
    fn a_partial_interval_moves_from_the_visible_tail_instead_of_retracting_to_its_start() {
        let old = vec![vec![point_at(10_000, 100.), point_at(15_000, 200.)]];
        let next = Rc::new(vec![vec![point_at(10_000, 100.), point_at(16_000, 220.)]]);
        let from = previous_tail(&old, &next).unwrap();
        let first = display_frame(&next, Some(&from), 0.);
        let middle = display_frame(&next, Some(&from), 0.5);
        let end = display_frame(&next, Some(&from), 1.);
        assert_eq!(first[0].last().unwrap().time, 15_000);
        assert_eq!(middle[0].last().unwrap().time, 15_500);
        assert_eq!(middle[0].last().unwrap().down, 210.);
        assert_eq!(end[0].last().unwrap().time, 16_000);
        let rolled = Rc::new(vec![vec![
            point_at(10_000, 100.),
            point_at(15_000, 200.),
            point_at(16_000, 220.),
        ]]);
        assert_eq!(display_frame(&rolled, Some(&from), 0.)[0].len(), 2);
        let gap = vec![vec![point_at(17_000, 500.)]];
        assert!(
            previous_tail(&old, &gap).is_none(),
            "a new segment must not animate across a gap"
        );
    }

    #[::core::prelude::v1::test]
    fn scale_retargets_from_the_visible_value_and_delays_shrinking() {
        let now = Instant::now();
        let mut scale = ScaleMotion {
            from: 100.,
            target: 100.,
            started: now,
            duration: REVEAL,
        };
        scale.update(400., now, true);
        let halfway = scale.value(now + REVEAL / 2);
        assert!(halfway > 100. && halfway < 400.);
        scale.update(800., now + REVEAL / 2, true);
        assert_eq!(scale.value(now + REVEAL / 2), halfway);
        assert_eq!(scale.value(now + REVEAL * 2), 800.);
        scale.update(100., now + REVEAL * 2, true);
        assert_eq!(
            scale.value(now + REVEAL * 2 + Duration::from_millis(799)),
            800.
        );
        assert_eq!(scale.value(now + Duration::from_secs(3)), 100.);
        scale.update(300., now, false);
        assert_eq!(scale.value(now), 300.);
    }
}
