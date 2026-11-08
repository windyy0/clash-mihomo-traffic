use super::*;

#[path = "live_chart.rs"]
mod live;
pub(super) use live::LiveChart;

// Previous renderer, retained only for the before/after visual regression.
#[cfg(test)]
fn cosine_points(points: &[Point<Pixels>], reveal: f32) -> Vec<Point<Pixels>> {
    let Some(&first) = points.first() else {
        return Vec::new();
    };
    let mut result = vec![first];
    for (index, pair) in points.windows(2).enumerate() {
        let [a, b] = [pair[0], pair[1]];
        let end = if index + 2 == points.len() {
            reveal
        } else {
            1.
        }
        .clamp(0., 1.);
        let steps = ((f32::from(b.x - a.x).abs() / 2.).ceil() as usize).clamp(4, 16);
        for step in 1..=steps {
            let t = (step as f32 / steps as f32).min(end);
            let eased = (1. - (t * std::f32::consts::PI).cos()) / 2.;
            result.push(point(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * eased));
            if t >= end {
                break;
            }
        }
    }
    result
}

// The previous interpolation is retained for visual regression comparisons.
// Shared tangents carry a rising
// or falling slope through intermediate points instead of flattening every
// second into another rounded spike. Extrema and flat intervals have zero slope.
#[cfg(test)]
fn interpolated_points(points: &[Point<Pixels>]) -> Vec<Point<Pixels>> {
    if points.len() < 2 || points.windows(2).any(|pair| pair[1].x <= pair[0].x) {
        return points.to_vec();
    }
    let widths: Vec<_> = points
        .windows(2)
        .map(|pair| f32::from(pair[1].x - pair[0].x))
        .collect();
    let slopes: Vec<_> = points
        .windows(2)
        .zip(&widths)
        .map(|(pair, width)| f32::from(pair[1].y - pair[0].y) / width)
        .collect();
    let mut tangents = vec![0.; points.len()];
    for index in 1..points.len() - 1 {
        let (left, right) = (slopes[index - 1], slopes[index]);
        if left != 0. && right != 0. && left.signum() == right.signum() {
            let w1 = 2. * widths[index] + widths[index - 1];
            let w2 = widths[index] + 2. * widths[index - 1];
            tangents[index] = (w1 + w2) / (w1 / left + w2 / right);
        }
    }
    let endpoint = |h0: f32, h1: f32, d0: f32, d1: f32| {
        let value = ((2. * h0 + h1) * d0 - h0 * d1) / (h0 + h1);
        if d0 == 0. || value.signum() != d0.signum() {
            0.
        } else if d0.signum() != d1.signum() && value.abs() > 3. * d0.abs() {
            3. * d0
        } else {
            value
        }
    };
    let last = slopes.len() - 1;
    tangents[0] = if last == 0 {
        slopes[0]
    } else {
        endpoint(widths[0], widths[1], slopes[0], slopes[1])
    };
    tangents[points.len() - 1] = if last == 0 {
        slopes[0]
    } else {
        endpoint(
            widths[last],
            widths[last - 1],
            slopes[last],
            slopes[last - 1],
        )
    };
    let mut result = vec![points[0]];
    for (index, pair) in points.windows(2).enumerate() {
        let [a, b] = [pair[0], pair[1]];
        let width = widths[index];
        let steps = ((width / 2.).ceil() as usize).clamp(4, 32);
        for step in 1..=steps {
            if step == steps {
                result.push(b);
                continue;
            }
            let t = step as f32 / steps as f32;
            let t2 = t * t;
            let t3 = t2 * t;
            let y = (2. * t3 - 3. * t2 + 1.) * f32::from(a.y)
                + (t3 - 2. * t2 + t) * width * tangents[index]
                + (-2. * t3 + 3. * t2) * f32::from(b.y)
                + (t3 - t2) * width * tangents[index + 1];
            result.push(point(
                a.x + px(width * t),
                px(y.clamp(f32::from(a.y.min(b.y)), f32::from(a.y.max(b.y)))),
            ));
        }
    }
    result
}

// A uniform cubic B-spline gives a locally weighted, C2-continuous outline.
// Positive basis weights keep it inside the neighbouring control values.
// Repeated endpoint controls retain the actual first and latest positions;
// separate sampling segments are never joined across missing coverage.
pub(super) fn smooth_points(points: &[Point<Pixels>]) -> Vec<Point<Pixels>> {
    if points.len() < 2 || points.windows(2).any(|pair| pair[1].x <= pair[0].x) {
        return points.to_vec();
    }
    let first = points[0];
    let last = *points.last().unwrap();
    let mut controls = Vec::with_capacity(points.len() + 4);
    controls.extend([first, first]);
    controls.extend_from_slice(points);
    controls.extend([last, last]);
    let mut result = vec![first];
    for group in controls.windows(4) {
        let start = (group[0].x + group[1].x * 4. + group[2].x) / 6.;
        let end = (group[1].x + group[2].x * 4. + group[3].x) / 6.;
        let steps = ((f32::from(end - start) / 2.).ceil() as usize).clamp(4, 32);
        let min_y = group.iter().map(|point| point.y).min().unwrap();
        let max_y = group.iter().map(|point| point.y).max().unwrap();
        for step in 1..=steps {
            let t = step as f32 / steps as f32;
            let t2 = t * t;
            let t3 = t2 * t;
            let weights = [
                (1. - t).powi(3) / 6.,
                (3. * t3 - 6. * t2 + 4.) / 6.,
                (-3. * t3 + 3. * t2 + 3. * t + 1.) / 6.,
                t3 / 6.,
            ];
            let mut x = px(0.);
            let mut y = px(0.);
            for (control, weight) in group.iter().zip(weights) {
                x += control.x * weight;
                y += control.y * weight;
            }
            let pos = point(x.clamp(first.x, last.x), y.clamp(min_y, max_y));
            if pos.x > result.last().unwrap().x {
                result.push(pos);
            }
        }
    }
    *result.last_mut().unwrap() = last;
    result
}

fn traffic_path(points: &[Point<Pixels>]) -> PathBuilder {
    let mut path = PathBuilder::stroke(px(1.75));
    for (index, pos) in points.iter().enumerate() {
        if index == 0 {
            path.move_to(*pos);
        } else {
            path.line_to(*pos);
        }
    }
    path
}
pub(super) struct ChartScale {
    pub(super) ceiling: f64,
    pub(super) labels: [String; 4],
}
impl ChartScale {
    fn for_ceiling(ceiling: f64, live: bool) -> Self {
        let units = ["B", "KB", "MB", "GB", "TB", "PB", "EB"];
        let mut index = 0;
        let mut unit = 1.;
        while index + 1 < units.len() && ceiling >= unit * 1024. {
            index += 1;
            unit *= 1024.;
        }
        Self {
            ceiling,
            labels: std::array::from_fn(|tick| {
                if tick == 3 {
                    return "0".into();
                }
                let number = format!("{:.2}", ceiling * (3 - tick) as f64 / (3. * unit));
                format!(
                    "{} {}{}",
                    number.trim_end_matches('0').trim_end_matches('.'),
                    units[index],
                    if live { "/s" } else { "" }
                )
            }),
        }
    }

    pub(super) fn compact_axis_width(&self) -> f32 {
        self.labels
            .iter()
            .map(|label| label.chars().count() as f32 * 6. + 4.)
            .fold(40., f32::max)
            .min(64.)
    }

    fn new(points: &[SeriesPoint], live: bool) -> Self {
        let peak = points
            .iter()
            .flat_map(|point| [point.down, point.up])
            .filter(|value| value.is_finite())
            .fold(0f64, f64::max);
        Self::for_peak(peak, live)
    }
    pub(super) fn for_peak(peak: f64, live: bool) -> Self {
        let target = if peak > 0. { peak * 1.15 } else { 3. * 1024. };
        let units = ["B", "KB", "MB", "GB", "TB", "PB", "EB"];
        let mut index = 0;
        let mut unit = 1.;
        while index + 1 < units.len() && target >= unit * 1024. {
            index += 1;
            unit *= 1024.;
        }
        loop {
            let raw_step = target / (3. * unit);
            let magnitude = 10f64.powf(raw_step.log10().floor());
            let step = [1., 2., 2.5, 5., 10.]
                .into_iter()
                .find(|step| step * magnitude >= raw_step)
                .unwrap_or(10.)
                * magnitude;
            let ceiling = step * 3. * unit;
            if index + 1 < units.len() && ceiling >= unit * 1024. {
                index += 1;
                unit *= 1024.;
                continue;
            }
            return Self::for_ceiling(ceiling, live);
        }
    }
}

struct ChartData {
    segments: Rc<Vec<Vec<SeriesPoint>>>,
    from: i64,
    to: i64,
    live: bool,
    ceiling: Option<f64>,
    animate: Option<Rc<Cell<bool>>>,
}

fn time_position(time: i64, from: i64, to: i64) -> f32 {
    if to <= from {
        return 0.5;
    }
    (time.saturating_sub(from) as f64 / to.saturating_sub(from) as f64) as f32
}

fn live_ticks(from: i64, to: i64, width: f32) -> Vec<(f32, i64)> {
    let span = to.saturating_sub(from);
    if span <= 0 || width < 228. {
        return vec![];
    }
    // Fixed fractions keep the labels stationary while the time window rolls.
    // At least one minute between labels prevents duplicate HH:mm values.
    let divisions = (span / 60_000).min((width / 76.).floor() as i64).max(1);
    (1..divisions)
        .filter_map(|index| {
            let position = index as f32 / divisions as f32;
            let x = position * width;
            (x >= 76. && width - x >= 64.).then_some((position, from + span * index / divisions))
        })
        .collect()
}

impl TrafficApp {
    pub(super) fn live_chart(&self, duration: i64, height: f32, legend: bool) -> Div {
        self.live_chart_view.element(
            &self.data,
            self.p,
            duration,
            height,
            legend,
            self.page == Page::Overview,
        )
    }
}

fn render_chart(data: ChartData, height: f32, legend: bool, compact_axis: bool, p: Palette) -> Div {
    let ChartData {
        segments,
        from: first,
        to: last,
        live,
        ceiling,
        animate,
    } = data;
    let waiting = live
        && !segments
            .iter()
            .any(|segment| segment.last().is_some_and(|point| point.time >= first));
    let scale = ceiling.map_or_else(
        || {
            ChartScale::new(
                &segments.iter().flatten().cloned().collect::<Vec<_>>(),
                live,
            )
        },
        |max| ChartScale::for_ceiling(max, live),
    );
    let max = scale.ceiling;
    let fmt = |t: i64| {
        chrono::DateTime::from_timestamp_millis(t)
            .map(|x| {
                x.with_timezone(&chrono::Local)
                    .format(if live { "%H:%M" } else { "%m/%d %H:%M" })
                    .to_string()
            })
            .unwrap_or_default()
    };
    let axis_width = if compact_axis {
        scale.compact_axis_width()
    } else if legend || live {
        64.
    } else {
        50.
    };
    let axis = div()
        .relative()
        .w(px(axis_width))
        .h(px(height))
        .flex_none()
        .children(scale.labels.into_iter().enumerate().map(|(tick, text)| {
            label(text, p.muted)
                .absolute()
                .top(relative(tick as f32 / 3.))
                .mt(px(-7.5))
                .w_full()
                .text_right()
                .text_size(px(10.5))
                .truncate()
        }));
    column()
        .gap(px(10.))
        .when(legend, |chart| {
            chart.child(
                row()
                    .when(live, |row| row.h(px(18.)))
                    .justify_end()
                    .when(live, |row| {
                        row.justify_between().child(
                            label(
                                format!(
                                    "{}秒均值 · 平滑",
                                    crate::live_chart::display_interval_ms(last - first) / 1_000
                                ),
                                p.muted,
                            )
                            .text_size(px(11.)),
                        )
                    })
                    .when(!live, |row| {
                        row.justify_between()
                            .child(label("平滑趋势", p.muted).text_size(px(11.)))
                    })
                    .child(
                        row()
                            .gap(px(14.))
                            .child(label("● 下载", p.download_text))
                            .child(label("● 上传", p.upload_text)),
                    ),
            )
        })
        .child(
            row().items_start().gap(px(8.)).child(axis).child(
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .h(px(height))
                    .child(
                        canvas(
                            |_, _, _| (),
                            move |bounds, _, window, cx| {
                                // Scrolled-out charts must not keep requesting frames.
                                if !bounds.intersects(&window.content_mask().bounds) {
                                    return;
                                }
                                if let Some(pending) = animate
                                    && native::visible(native::hwnd(window))
                                    && !pending.replace(true)
                                {
                                    let view = window.current_view();
                                    // Bound drawing work independently of monitor refresh rate.
                                    // A single outstanding timer also prevents polling redraws
                                    // from starting duplicate animation loops.
                                    window
                                        .spawn(cx, async move |cx| {
                                            Timer::after(Duration::from_secs_f64(1. / 30.)).await;
                                            pending.set(false);
                                            cx.update(|_, cx| cx.notify(view)).ok();
                                        })
                                        .detach();
                                }
                                let left = bounds.left();
                                let top = bounds.top();
                                let w = bounds.size.width;
                                let h = bounds.size.height;
                                for n in 0..=3 {
                                    let y = top + h * (n as f32 / 3.);
                                    let mut path = PathBuilder::stroke(px(1.));
                                    path.move_to(point(left, y));
                                    path.line_to(point(left + w, y));
                                    if let Ok(path) = path.build() {
                                        let mut color = rgb(p.line);
                                        color.a = if n == 3 { 0.7 } else { 0.45 };
                                        window.paint_path(path, color);
                                    }
                                }
                                window.with_content_mask(Some(ContentMask { bounds }), |window| {
                                    for segment in segments.iter() {
                                        let start = if live {
                                            segment
                                                .partition_point(|point| point.time < first)
                                                .saturating_sub(3)
                                        } else {
                                            0
                                        };
                                        let segment = &segment[start..];
                                        for upload in [false, true] {
                                            let coords: Vec<_> = segment
                                                .iter()
                                                .enumerate()
                                                .map(|(i, v)| {
                                                    point(
                                                        left + w * if live {
                                                            // Retain the spline's controls outside the left edge;
                                                            // clipping preserves the curve as it scrolls out.
                                                            time_position(v.time, first, last)
                                                        } else if segment.len() > 1 {
                                                            i as f32 / (segment.len() - 1) as f32
                                                        } else {
                                                            0.5
                                                        },
                                                        top + h
                                                            * (1.
                                                                - (if upload {
                                                                    v.up
                                                                } else {
                                                                    v.down
                                                                } / max)
                                                                    as f32),
                                                    )
                                                })
                                                .collect();
                                            let coords = smooth_points(&coords);
                                            if coords.len() == 1 {
                                                let pos = coords[0];
                                                window.paint_quad(
                                                    fill(
                                                        Bounds::new(
                                                            point(pos.x - px(2.), pos.y - px(2.)),
                                                            size(px(4.), px(4.)),
                                                        ),
                                                        rgb(if upload {
                                                            p.upload
                                                        } else {
                                                            p.download
                                                        }),
                                                    )
                                                    .corner_radii(px(2.)),
                                                );
                                                continue;
                                            }
                                            if coords.is_empty() {
                                                continue;
                                            }
                                            if !upload || live {
                                                let mut area = PathBuilder::fill();
                                                area.move_to(point(coords[0].x, top + h));
                                                for pos in &coords {
                                                    area.line_to(*pos);
                                                }
                                                area.line_to(point(
                                                    coords.last().unwrap().x,
                                                    top + h,
                                                ));
                                                area.close();
                                                if let Ok(path) = area.build() {
                                                    let mut color = rgb(if upload {
                                                        p.upload
                                                    } else {
                                                        p.download
                                                    });
                                                    color.a = if live {
                                                        match (upload, p.dark) {
                                                            (false, true) => 0.20,
                                                            (false, false) => 0.13,
                                                            (true, true) => 0.12,
                                                            (true, false) => 0.08,
                                                        }
                                                    } else if p.dark {
                                                        0.10
                                                    } else {
                                                        0.07
                                                    };
                                                    window.paint_path(path, color);
                                                }
                                            }
                                            let path = traffic_path(&coords);
                                            if let Ok(path) = path.build() {
                                                window.paint_path(
                                                    path,
                                                    rgb(if upload { p.upload } else { p.download }),
                                                );
                                            }
                                        }
                                    }
                                });
                            },
                        )
                        .h(px(height))
                        .w_full(),
                    )
                    .when(waiting, |plot| {
                        plot.child(
                            div()
                                .absolute()
                                .inset_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(label("等待有效采样", p.muted).text_size(px(12.))),
                        )
                    }),
            ),
        )
        .child(
            div()
                .relative()
                .ml(px(axis_width + 8.))
                .h(px(if legend { 18. } else { 28. }))
                .text_size(px(11.))
                .child(
                    label(fmt(first), p.muted)
                        .text_size(px(11.))
                        .line_height(px(16.))
                        .absolute()
                        .left(px(if live { 12. } else { 0. })),
                )
                .child(
                    label(if live { "现在".into() } else { fmt(last) }, p.muted)
                        .text_size(px(11.))
                        .line_height(px(16.))
                        .absolute()
                        .right_0(),
                )
                .when(live, |footer| {
                    footer.child(
                        canvas(
                            |bounds, _, _| bounds,
                            move |bounds, _, window, cx| {
                                for (position, time) in
                                    live_ticks(first, last, f32::from(bounds.size.width))
                                {
                                    let text = chrono::DateTime::from_timestamp_millis(time)
                                        .map(|time| {
                                            time.with_timezone(&chrono::Local)
                                                .format("%H:%M")
                                                .to_string()
                                        })
                                        .unwrap_or_default();
                                    let mut style = window.text_style();
                                    style.color = rgb(p.muted).into();
                                    let run = style.to_run(text.len());
                                    let line = window.text_system().shape_line(
                                        text.into(),
                                        px(11.),
                                        &[run],
                                        None,
                                    );
                                    let x = bounds.left() + bounds.size.width * position
                                        - line.width / 2.;
                                    let _ = line.paint(point(x, bounds.top()), px(16.), window, cx);
                                }
                            },
                        )
                        .size_full(),
                    )
                }),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn shape_fixture() -> std::collections::VecDeque<LiveSample> {
        (0..300)
            .filter(|i| !(140..155).contains(i))
            .map(|i| {
                let t = i as f64;
                let hill = |center: f64, width: f64, height: f64| {
                    height * (-0.5 * ((t - center) / width).powi(2)).exp()
                };
                let envelope =
                    hill(55., 4., 35_000.) + hill(96., 8., 85_000.) + hill(225., 13., 300_000.);
                let burst = if i % 4 == 0 { 4. } else { 0.25 };
                LiveSample {
                    from: i * 1_000,
                    to: (i + 1) * 1_000,
                    down: 400. + envelope * burst,
                    up: 0.,
                    valid: true,
                }
            })
            .collect()
    }

    fn export_comparison(
        file: &str,
        before: &[Vec<SeriesPoint>],
        after: &[Vec<SeriesPoint>],
        old_curve: fn(&[Point<Pixels>]) -> Vec<Point<Pixels>>,
        new_curve: fn(&[Point<Pixels>]) -> Vec<Point<Pixels>>,
    ) {
        use std::fmt::Write;
        let mut csv = String::from("kind,segment,x,y,ceiling\n");
        for (kind, segments) in [("before", before), ("after", after)] {
            let peak = segments
                .iter()
                .flatten()
                .map(|point| point.down)
                .fold(0f64, f64::max);
            let ceiling = ChartScale::for_peak(peak, true).ceiling;
            for (segment, points) in segments.iter().enumerate() {
                let points: Vec<_> = points
                    .iter()
                    .map(|p| {
                        point(
                            px(p.time as f32 / 300_000. * 960.),
                            px(190. * (1. - p.down / ceiling) as f32),
                        )
                    })
                    .collect();
                let curve = if kind == "before" {
                    old_curve(&points)
                } else {
                    new_curve(&points)
                };
                for p in curve {
                    writeln!(
                        &mut csv,
                        "{kind},{segment},{},{},{ceiling}",
                        f32::from(p.x),
                        f32::from(p.y)
                    )
                    .unwrap();
                }
            }
            println!(
                "{kind}: {} display vertices",
                segments.iter().map(Vec::len).sum::<usize>()
            );
        }
        std::fs::write(file, csv).unwrap();
    }

    #[::core::prelude::v1::test]
    #[ignore = "exports before/after paths from the actual live display algorithms for visual QA"]
    fn write_live_shape_comparison() {
        let raw = shape_fixture();
        export_comparison(
            "target/live-chart-shape.csv",
            &crate::live_chart::trend(&raw, 0, 300_000),
            &crate::live_chart::display_trend(&raw, 300_000),
            |points| cosine_points(points, 1.),
            interpolated_points,
        );
    }

    #[::core::prelude::v1::test]
    #[ignore = "exports the previous interpolation and the softer trend outline for visual QA"]
    fn write_extra_smooth_comparison() {
        let points = crate::live_chart::display_trend(&shape_fixture(), 300_000);
        export_comparison(
            "target/extra-smooth-shape.csv",
            &points,
            &points,
            interpolated_points,
            smooth_points,
        );
    }
    #[::core::prelude::v1::test]
    fn trend_outline_preserves_endpoints_and_stays_inside_the_sample_envelope() {
        let raw = vec![
            point(px(-10.), px(100.)),
            point(px(40.), px(0.)),
            point(px(60.), px(100.)),
            point(px(100.), px(100.)),
            point(px(140.), px(100.)),
            point(px(180.), px(100.)),
        ];
        let curve = smooth_points(&raw);
        assert!(curve.len() > raw.len());
        assert_eq!(curve.first(), raw.first());
        assert_eq!(curve.last(), raw.last());
        assert!(curve.windows(2).all(|pair| pair[0].x < pair[1].x));
        assert!(
            curve
                .iter()
                .all(|sample| sample.y >= px(0.) && sample.y <= px(100.))
        );
        assert!(
            curve
                .iter()
                .filter(|sample| sample.x >= px(120.))
                .all(|sample| sample.y == px(100.))
        );
        assert!(smooth_points(&[],).is_empty());
        assert_eq!(smooth_points(&raw[..1]), raw[..1]);
        let zero = [
            point(px(0.), px(0.)),
            point(px(40.), px(0.)),
            point(px(80.), px(0.)),
        ];
        assert!(smooth_points(&zero).iter().all(|point| point.y == px(0.)));
    }

    #[::core::prelude::v1::test]
    fn rising_slopes_continue_through_the_middle_of_a_hill() {
        let raw = vec![
            point(px(0.), px(0.)),
            point(px(40.), px(20.)),
            point(px(80.), px(50.)),
            point(px(120.), px(0.)),
        ];
        let curve = smooth_points(&raw);
        let join = curve
            .iter()
            .position(|point| (point.x - raw[1].x).abs() < px(0.001))
            .unwrap();
        let slope =
            |a: Point<Pixels>, b: Point<Pixels>| f32::from(b.y - a.y) / f32::from(b.x - a.x);
        let left = slope(curve[join - 1], curve[join]);
        let right = slope(curve[join], curve[join + 1]);
        assert!(
            left > 0.25 && right > 0.25,
            "the hillside must not flatten at every sample"
        );
        assert!((left - right).abs() < 0.1, "the slope must stay continuous");
    }

    #[::core::prelude::v1::test]
    fn rounded_shoulders_have_continuous_curvature() {
        let raw: Vec<_> = [0., 10., 60., 55., 0.]
            .iter()
            .enumerate()
            .map(|(index, y)| point(px(index as f32 * 40.), px(*y)))
            .collect();
        let curve = smooth_points(&raw);
        let join = curve
            .iter()
            .position(|point| (point.x - px(80.)).abs() < px(0.001))
            .unwrap();
        let curvature = |a: Point<Pixels>, b: Point<Pixels>, c: Point<Pixels>| {
            let left = f32::from(b.y - a.y) / f32::from(b.x - a.x);
            let right = f32::from(c.y - b.y) / f32::from(c.x - b.x);
            2. * (right - left) / f32::from(c.x - a.x)
        };
        let left = curvature(curve[join - 2], curve[join - 1], curve[join]);
        let right = curvature(curve[join], curve[join + 1], curve[join + 2]);
        assert!(
            (left - right).abs() < 0.01,
            "a shoulder must not have a curvature corner"
        );
    }

    #[::core::prelude::v1::test]
    fn scrolling_keeps_the_same_visible_outline_when_left_controls_are_clipped() {
        let points: Vec<_> = (0..12)
            .map(|i| point(px(i as f32 * 40.), px((i * 37 % 101) as f32)))
            .collect();
        let from = px(160.3);
        let start = points
            .partition_point(|point| point.x < from)
            .saturating_sub(3);
        let full = smooth_points(&points);
        let clipped = smooth_points(&points[start..]);
        assert_eq!(
            full.into_iter()
                .filter(|point| point.x >= from)
                .collect::<Vec<_>>(),
            clipped
                .into_iter()
                .filter(|point| point.x >= from)
                .collect::<Vec<_>>(),
        );
    }

    #[::core::prelude::v1::test]
    fn small_bumps_are_reduced_while_the_main_hill_remains() {
        let raw: Vec<_> = [0., 15., 25., 22., 70., 100., 65., 40., 0., 0.]
            .iter()
            .enumerate()
            .map(|(i, y)| point(px(i as f32 * 40.), px(*y)))
            .collect();
        let old = interpolated_points(&raw);
        let new = smooth_points(&raw);
        let peaks = |points: &[Point<Pixels>]| {
            points
                .windows(3)
                .filter(|p| p[1].y > p[0].y && p[1].y > p[2].y)
                .count()
        };
        assert!(peaks(&new) < peaks(&old));
        assert!(new.iter().map(|point| point.y).max().unwrap() >= px(80.));
        let increasing: Vec<_> = [0., 1., 1., 2., 80.]
            .iter()
            .enumerate()
            .map(|(i, y)| point(px(i as f32 * 40.), px(*y)))
            .collect();
        assert!(
            smooth_points(&increasing)
                .windows(2)
                .all(|p| p[1].y >= p[0].y)
        );
    }

    #[::core::prelude::v1::test]
    fn realtime_axis_uses_actual_time_and_ticks_do_not_collide() {
        let from = 17_234;
        let to = from + 300_000;
        assert_eq!(time_position(from, from, to), 0.);
        assert_eq!(time_position(from + 60_000, from, to), 0.2);
        assert_eq!(time_position(to, from, to), 1.);
        assert!(time_position(from - 1, from, to) < 0.);
        assert!(time_position(to + 1, from, to) > 1.);
        assert_eq!(time_position(from, from, from), 0.5);
        for width in [180., 228., 320., 600., 1000.] {
            let ticks = live_ticks(from, to, width);
            let positions: Vec<_> = ticks.iter().map(|(position, _)| position * width).collect();
            assert!(positions.iter().all(|x| *x >= 76. && width - *x >= 64.));
            assert!(positions.windows(2).all(|pair| pair[1] - pair[0] >= 75.));
            let later = live_ticks(from + 17_000, to + 17_000, width);
            assert_eq!(
                ticks
                    .iter()
                    .map(|(position, _)| *position)
                    .collect::<Vec<_>>(),
                later
                    .iter()
                    .map(|(position, _)| *position)
                    .collect::<Vec<_>>(),
                "middle time labels must not slide as the chart advances"
            );
            assert!(ticks.windows(2).all(|pair| pair[1].1 - pair[0].1 >= 60_000));
            assert!(
                ticks
                    .iter()
                    .zip(&later)
                    .all(|(before, after)| after.1 - before.1 == 17_000)
            );
        }
        assert!(live_ticks(from, to, 180.).is_empty());
        assert!(live_ticks(from, from, 600.).is_empty());
    }

    #[::core::prelude::v1::test]
    fn dynamic_scale_keeps_both_directions_in_range_and_uses_matching_units() {
        for (value, unit) in [
            (1., "B/s"),
            (800. * 1024., "MB/s"),
            (2. * 1024f64.powi(2), "MB/s"),
            (8. * 1024f64.powi(3), "GB/s"),
        ] {
            let scale = ChartScale::new(
                &[SeriesPoint {
                    time: 0,
                    down: value / 3.,
                    up: value,
                }],
                true,
            );
            assert!(scale.ceiling >= value * 1.15);
            assert!(scale.labels[..3].iter().all(|label| label.ends_with(unit)));
            assert_eq!(scale.labels[3], "0");
        }
        let scale = ChartScale::new(
            &[SeriesPoint {
                time: 0,
                down: 2. * 1024f64.powi(2),
                up: 0.,
            }],
            true,
        );
        assert_eq!(scale.labels, ["3 MB/s", "2 MB/s", "1 MB/s", "0"]);
        let history = ChartScale::new(
            &[SeriesPoint {
                time: 0,
                down: 2. * 1024f64.powi(2),
                up: 0.,
            }],
            false,
        );
        assert_eq!(history.labels, ["3 MB", "2 MB", "1 MB", "0"]);
    }
    #[::core::prelude::v1::test]
    fn empty_chart_has_a_finite_readable_axis() {
        let scale = ChartScale::new(&[], true);
        assert_eq!(scale.labels, ["3 KB/s", "2 KB/s", "1 KB/s", "0"]);
        assert!(scale.ceiling.is_finite() && scale.ceiling > 0.);
    }
}
