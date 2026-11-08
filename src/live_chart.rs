use crate::model::{LiveSample, SeriesPoint};
use std::collections::VecDeque;

#[cfg(test)]
const AVERAGE_MS: i64 = 3_000;

/// Keep roughly thirty intervals across either live viewport. The values are
/// trailing interval averages, so the unfinished interval never starts over at
/// a raw one-second spike.
pub fn display_interval_ms(duration: i64) -> i64 {
    (duration / 30).max(1_000)
}

pub fn display_trend(samples: &VecDeque<LiveSample>, duration: i64) -> Vec<Vec<SeriesPoint>> {
    let interval = display_interval_ms(duration);
    averaged(samples, i64::MIN, i64::MAX, interval)
        .into_iter()
        .map(|segment| {
            let mut displayed = Vec::new();
            let mut pending: Option<SeriesPoint> = None;
            let mut bucket = None;
            for point in segment {
                let next = point.time.saturating_sub(1).div_euclid(interval);
                if displayed.is_empty() {
                    displayed.push(point.clone());
                }
                if bucket != Some(next) {
                    if let Some(previous) = pending.take()
                        && displayed
                            .last()
                            .is_none_or(|last| last.time != previous.time)
                    {
                        displayed.push(previous);
                    }
                    bucket = Some(next);
                }
                pending = Some(point);
            }
            if let Some(last) = pending
                && displayed.last().is_none_or(|point| point.time != last.time)
            {
                displayed.push(last);
            }
            displayed
        })
        .collect()
}

/// Keep display samples separate from the accounting and analysis caches.
pub fn record(samples: &mut VecDeque<LiveSample>, sample: LiveSample, reset: bool) {
    if reset || samples.back().is_some_and(|last| sample.to <= last.to) {
        samples.clear();
    }
    samples.push_back(sample);
    while samples.len() > 900 {
        samples.pop_front();
    }
}

/// A trailing, time-weighted average, split at every unobserved interval.
/// Filtering happens after averaging so the left edge retains its full context.
#[cfg(test)]
pub fn trend(samples: &VecDeque<LiveSample>, from: i64, to: i64) -> Vec<Vec<SeriesPoint>> {
    averaged(samples, from, to, AVERAGE_MS)
}

fn averaged(
    samples: &VecDeque<LiveSample>,
    from: i64,
    to: i64,
    average_ms: i64,
) -> Vec<Vec<SeriesPoint>> {
    let mut segments: Vec<Vec<SeriesPoint>> = Vec::new();
    let mut window: VecDeque<&LiveSample> = VecDeque::new();
    let mut previous_end = None;
    let mut continuing = false;
    for sample in samples {
        if !sample.valid
            || sample.to <= sample.from
            || !sample.down.is_finite()
            || !sample.up.is_finite()
            || sample.down < 0.
            || sample.up < 0.
        {
            window.clear();
            previous_end = None;
            continuing = false;
            continue;
        }
        if previous_end != Some(sample.from) {
            window.clear();
            continuing = false;
        }
        previous_end = Some(sample.to);
        window.push_back(sample);
        let start = sample.to.saturating_sub(average_ms);
        while window.front().is_some_and(|entry| entry.to <= start) {
            window.pop_front();
        }
        let mut duration = 0.;
        let mut down = 0.;
        let mut up = 0.;
        for entry in &window {
            let overlap = (entry.to - entry.from.max(start)) as f64;
            duration += overlap;
            down += entry.down * overlap;
            up += entry.up * overlap;
        }
        if sample.to < from || sample.to > to {
            continue;
        }
        if !continuing {
            segments.push(Vec::new());
            continuing = true;
        }
        segments.last_mut().unwrap().push(SeriesPoint {
            time: sample.to,
            down: down / duration,
            up: up / duration,
        });
    }
    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(from: i64, to: i64, down: f64) -> LiveSample {
        LiveSample {
            from,
            to,
            down,
            up: down / 2.,
            valid: true,
        }
    }

    #[test]
    fn display_density_tracks_the_viewport_and_keeps_the_current_endpoint() {
        for duration in [120_000, 300_000] {
            let raw: VecDeque<_> = (0..duration / 1_000)
                .map(|i| sample(i * 1_000, (i + 1) * 1_000, (i % 7) as f64 * 100.))
                .collect();
            let original = raw.clone();
            let display = display_trend(&raw, duration);
            assert_eq!(display.len(), 1);
            assert!(
                display[0].len() <= 32,
                "a full viewport must not retain hundreds of vertices"
            );
            assert_eq!(display[0].first().unwrap().time, 1_000);
            assert_eq!(display[0].last().unwrap().time, duration);
            assert_eq!(raw, original);
        }
    }

    #[test]
    fn new_display_intervals_keep_their_context_instead_of_restarting_at_a_raw_spike() {
        let mut raw: VecDeque<_> = (0..10)
            .map(|i| sample(i * 1_000, (i + 1) * 1_000, 10.))
            .collect();
        let completed = display_trend(&raw, 300_000)[0].last().unwrap().clone();
        raw.push_back(sample(10_000, 11_000, 1_000.));
        let after = display_trend(&raw, 300_000);
        assert_eq!(after[0].len(), 3);
        assert_eq!(
            (after[0][1].time, after[0][1].down),
            (completed.time, completed.down)
        );
        assert_eq!(
            after[0][2].down, 109.,
            "nine seconds of context must survive a bucket boundary"
        );
    }

    #[test]
    fn display_averages_preserve_zero_and_never_bridge_missing_coverage() {
        let raw = VecDeque::from([
            sample(0, 1_000, 0.),
            sample(1_000, 2_000, 0.),
            sample(3_000, 4_000, 800.),
            sample(4_000, 5_000, 800.),
            LiveSample {
                valid: false,
                ..sample(5_000, 6_000, 0.)
            },
            sample(6_000, 7_000, 0.),
        ]);
        let segments = display_trend(&raw, 300_000);
        assert_eq!(segments.iter().map(Vec::len).collect::<Vec<_>>(), [2, 2, 1]);
        assert!(
            segments[0]
                .iter()
                .chain(&segments[2])
                .all(|point| point.down == 0.)
        );
        assert!(segments[1].iter().all(|point| point.down == 800.));
    }

    #[test]
    fn average_uses_actual_time_partial_intervals_and_preserves_raw_samples() {
        let raw = VecDeque::from([
            sample(0, 500, 600.),
            sample(500, 2_000, 100.),
            sample(2_000, 4_000, 400.),
        ]);
        let original = raw.clone();
        let lines = trend(&raw, 0, 4_000);
        assert_eq!(lines.len(), 1);
        let points = &lines[0];
        assert_eq!(points[0].down, 600., "startup must not be padded with zero");
        assert_eq!(points[1].down, 225.);
        assert_eq!(points[2].down, 300., "only the last 3000ms contribute");
        assert_eq!(points[2].up, 150.);
        assert_eq!(raw, original);
        assert_eq!(trend(&raw, 3_000, 4_000)[0][0].down, 300.);
    }

    #[test]
    fn zero_is_observed_but_gaps_and_invalid_samples_restart_the_average() {
        let mut raw = VecDeque::from([
            sample(0, 1_000, 300.),
            sample(1_000, 2_000, 0.),
            sample(3_000, 4_000, 900.),
            LiveSample {
                valid: false,
                ..sample(4_000, 5_000, 0.)
            },
            sample(5_000, 6_000, 120.),
        ]);
        let lines = trend(&raw, 0, 6_000);
        assert_eq!(lines.iter().map(Vec::len).collect::<Vec<_>>(), [2, 1, 1]);
        assert_eq!(lines[0][1].down, 150.);
        assert_eq!(lines[1][0].down, 900.);
        assert_eq!(lines[2][0].down, 120.);
        raw.push_back(sample(6_000, 7_000, f64::NAN));
        raw.push_back(sample(7_000, 8_000, 0.));
        let lines = trend(&raw, 0, 8_000);
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[3][0].down, 0.);
        assert!(trend(&raw, 9_000, 10_000).is_empty());
        assert!(trend(&VecDeque::new(), 0, 1_000).is_empty());
    }

    #[test]
    fn clock_rollback_source_change_and_restore_discard_old_display_data() {
        let mut raw = VecDeque::new();
        record(&mut raw, sample(10_000, 11_000, 100.), false);
        record(&mut raw, sample(11_000, 12_000, 200.), false);
        record(&mut raw, sample(2_000, 3_000, 300.), false);
        assert_eq!(raw.len(), 1);
        record(&mut raw, sample(3_000, 4_000, 400.), true);
        assert_eq!(raw.len(), 1);
        assert_eq!(trend(&raw, 0, 5_000)[0][0].down, 400.);
        for index in 0..1_000 {
            record(
                &mut raw,
                sample(5_000 + index * 1_000, 6_000 + index * 1_000, 1.),
                false,
            );
        }
        assert_eq!(raw.len(), 900);
    }
}
