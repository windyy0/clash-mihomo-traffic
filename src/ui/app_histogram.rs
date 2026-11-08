use super::*;
use crate::app_trend::{Bucket, BucketSize, Histogram};
use std::sync::Arc;

fn date(time: i64, format: &str) -> String {
    chrono::DateTime::from_timestamp_millis(time)
        .map(|value| {
            value
                .with_timezone(&chrono::Local)
                .format(format)
                .to_string()
        })
        .unwrap_or_default()
}

fn bucket_center(bounds: Bounds<Pixels>, index: usize, count: usize) -> Pixels {
    bounds.left() + bounds.size.width * ((index as f32 + 0.5) / count.max(1) as f32)
}

fn total_controls(
    histogram: &Histogram,
    bounds: Bounds<Pixels>,
    ceiling: f64,
) -> Vec<Point<Pixels>> {
    histogram
        .buckets
        .iter()
        .enumerate()
        .map(|(index, bucket)| {
            let total = bucket.bytes.down as f64 + bucket.bytes.up as f64;
            point(
                bucket_center(bounds, index, histogram.buckets.len()),
                bounds.bottom() - bounds.size.height * (total / ceiling).clamp(0., 1.) as f32,
            )
        })
        .collect()
}

fn bucket_tooltip(bucket: &Bucket) -> String {
    let end_format = if date(bucket.from, "%Y%m%d") == date(bucket.to, "%Y%m%d") {
        "%H:%M"
    } else {
        "%Y/%m/%d %H:%M"
    };
    format!(
        "{} — {}\n下载 {}\n上传 {}\n合计 {}\n右键查看详情",
        date(bucket.from, "%Y/%m/%d %H:%M"),
        date(bucket.to, end_format),
        human_bytes(bucket.bytes.down),
        human_bytes(bucket.bytes.up),
        human_bytes(bucket.bytes.total())
    )
}

impl TrafficApp {
    pub(super) fn application_trend_chart(
        &self,
        histogram: Histogram,
        height: f32,
        cx: &Context<Self>,
    ) -> Div {
        let p = self.p;
        let data = Arc::new(histogram);
        let peak = data
            .buckets
            .iter()
            .map(|bucket| bucket.bytes.up as f64 + bucket.bytes.down as f64)
            .fold(0., f64::max);
        let scale = chart::ChartScale::for_peak(peak, false);
        let ceiling = scale.ceiling;
        let axis_width = scale.compact_axis_width();
        let mut total_color = rgb(p.text);
        total_color.a = if p.dark { 0.78 } else { 0.64 };

        let axis = div()
            .relative()
            .w(px(axis_width))
            .h(px(height))
            .flex_none()
            .children(scale.labels.into_iter().enumerate().map(|(index, text)| {
                label(text, p.muted)
                    .absolute()
                    .top(relative(index as f32 / 3.))
                    .mt(px(-7.5))
                    .w_full()
                    .text_right()
                    .text_size(px(10.5))
                    .truncate()
            }));
        let bars = data.clone();
        let mut plot = div().relative().flex_1().min_w_0().h(px(height)).child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    for index in 0..=3 {
                        let y = bounds.top() + bounds.size.height * (index as f32 / 3.);
                        let mut path = PathBuilder::stroke(px(1.));
                        path.move_to(point(bounds.left(), y));
                        path.line_to(point(bounds.right(), y));
                        if let Ok(path) = path.build() {
                            let mut color = rgb(p.line);
                            color.a = if index == 3 { 0.7 } else { 0.45 };
                            window.paint_path(path, color);
                        }
                    }
                    let slot = bounds.size.width / bars.buckets.len().max(1) as f32;
                    for (index, bucket) in bars.buckets.iter().enumerate() {
                        let total = bucket.bytes.up as f64 + bucket.bytes.down as f64;
                        if total <= 0. {
                            continue;
                        }
                        let width = (slot * if slot < px(8.) { 0.78 } else { 0.66 }).min(px(32.));
                        let x = bucket_center(bounds, index, bars.buckets.len()) - width / 2.;
                        let total_height =
                            bounds.size.height * (total / ceiling).clamp(0., 1.) as f32;
                        let silhouette = Bounds::new(
                            point(x, bounds.bottom() - total_height),
                            size(width, total_height),
                        );
                        let radius = px(3.).min(width / 3.).min(total_height / 2.);
                        let corners = Corners {
                            top_left: radius,
                            top_right: radius,
                            bottom_left: px(0.),
                            bottom_right: px(0.),
                        };
                        let mut bottom = bounds.bottom();
                        let gap = if bucket.bytes.down > 0 && bucket.bytes.up > 0 {
                            px(1. / window.scale_factor())
                                .min(
                                    bounds.size.height
                                        * (bucket.bytes.down as f64 / ceiling) as f32,
                                )
                                .min(bounds.size.height * (bucket.bytes.up as f64 / ceiling) as f32)
                        } else {
                            px(0.)
                        };
                        for (index, (bytes, color)) in
                            [(bucket.bytes.down, p.download), (bucket.bytes.up, p.upload)]
                                .into_iter()
                                .enumerate()
                        {
                            if bytes == 0 {
                                continue;
                            }
                            let segment_height =
                                bounds.size.height * (bytes as f64 / ceiling).clamp(0., 1.) as f32;
                            let segment = Bounds::new(
                                point(
                                    x,
                                    bottom - segment_height
                                        + if index == 0 { gap / 2. } else { px(0.) },
                                ),
                                size(width, (segment_height - gap / 2.).max(px(0.))),
                            );
                            window.with_content_mask(
                                Some(ContentMask { bounds: segment }),
                                |window| {
                                    let mut color = rgb(color);
                                    color.a = if p.dark { 0.42 } else { 0.32 };
                                    window.paint_quad(fill(silhouette, color).corner_radii(corners))
                                },
                            );
                            bottom -= segment_height;
                        }
                    }
                    window.with_content_mask(Some(ContentMask { bounds }), |window| {
                        let controls = total_controls(&bars, bounds, ceiling);
                        let outline = chart::smooth_points(&controls);
                        if outline.len() == 1 {
                            let position = outline[0];
                            window.paint_quad(
                                fill(
                                    Bounds::new(
                                        point(position.x - px(2.), position.y - px(2.)),
                                        size(px(4.), px(4.)),
                                    ),
                                    total_color,
                                )
                                .corner_radii(px(2.)),
                            );
                        } else if !outline.is_empty() {
                            let mut path = PathBuilder::stroke(px(1.75));
                            path.move_to(outline[0]);
                            for position in &outline[1..] {
                                path.line_to(*position);
                            }
                            if let Ok(path) = path.build() {
                                window.paint_path(path, total_color);
                            }
                        }
                    });
                },
            )
            .size_full(),
        );
        for (index, bucket) in data.buckets.iter().enumerate() {
            let bucket = *bucket;
            let mut hover = rgb(p.download);
            hover.a = 0.055;
            plot = plot.child(
                div()
                    .id(SharedString::from(format!(
                        "application-volume-bin-{}-{index}",
                        bucket.from
                    )))
                    .absolute()
                    .left(relative(index as f32 / data.buckets.len() as f32))
                    .top_0()
                    .w(relative(1. / data.buckets.len() as f32))
                    .h_full()
                    .hover(move |s| s.bg(hover))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |app, _, window, cx| {
                            window.prevent_default();
                            cx.stop_propagation();
                            app.open_application_drill(bucket.from, bucket.to, window, cx);
                        }),
                    )
                    .tooltip(move |window, cx| {
                        Tooltip::new(bucket_tooltip(&bucket)).build(window, cx)
                    }),
            );
        }
        let ticks = data.clone();
        let labels = div().ml(px(axis_width + 8.)).mr(px(0.)).h(px(18.)).child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, cx| {
                    let count = ticks.buckets.len();
                    if count == 0 {
                        return;
                    }
                    let tick_count = count
                        .min((f32::from(bounds.size.width) / 76.).floor().max(1.) as usize)
                        .min(5);
                    let cross_day = ticks.buckets.first().zip(ticks.buckets.last()).is_some_and(
                        |(first, last)| date(first.from, "%Y%m%d") != date(last.from, "%Y%m%d"),
                    );
                    for tick in 0..tick_count {
                        let index = if tick_count == 1 {
                            count / 2
                        } else {
                            ((count - 1) as f32 * tick as f32 / (tick_count - 1) as f32).round()
                                as usize
                        };
                        let text = date(
                            ticks.buckets[index].from,
                            match ticks.size {
                                BucketSize::Hour if cross_day => "%m/%d %H时",
                                BucketSize::Hour => "%H:%M",
                                BucketSize::Day => "%m/%d",
                                BucketSize::Month => "%Y/%m",
                            },
                        );
                        let mut style = window.text_style();
                        style.color = rgb(p.muted).into();
                        let run = style.to_run(text.len());
                        let line =
                            window
                                .text_system()
                                .shape_line(text.into(), px(10.5), &[run], None);
                        let center = bucket_center(bounds, index, count);
                        let x = (center - line.width / 2.)
                            .max(bounds.left())
                            .min((bounds.right() - line.width).max(bounds.left()));
                        let _ = line.paint(point(x, bounds.top()), px(16.), window, cx);
                    }
                },
            )
            .size_full(),
        );
        column()
            .gap(px(10.))
            .child(
                row()
                    .justify_end()
                    .gap(px(14.))
                    .flex_wrap()
                    .child(label("● 下载用量", p.download_text))
                    .child(label("● 上传用量", p.upload_text))
                    .child(
                        row()
                            .gap(px(5.))
                            .child(
                                div()
                                    .w(px(14.))
                                    .h(px(1.75))
                                    .rounded_full()
                                    .bg(total_color),
                            )
                            .child(label("合计趋势", p.muted))
                            .id("application-total-curve-help")
                            .tooltip(|window,cx|Tooltip::new("合计趋势对应各柱的下载与上传用量之和，与柱形共用时间轴和用量刻度；平滑只辅助观察走势，悬浮显示准确用量。").build(window,cx)),
                    ),
            )
            .child(row().items_start().gap(px(8.)).child(axis).child(plot))
            .child(labels)
    }
}

#[cfg(test)]
#[path = "app_histogram_tests.rs"]
mod tests;
