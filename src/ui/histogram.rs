use super::*;
use crate::analysis::{BucketValue, ComparisonBucket};

fn coverage(value: BucketValue, duration: i64) -> String {
    if value.complete(duration) {
        "完整覆盖".into()
    } else if !value.observed() {
        "缺失采样".into()
    } else {
        format!(
            "部分覆盖 {:.1} / {:.1}秒 · 已观测量",
            value.covered_ms as f64 / 1000.,
            duration as f64 / 1000.
        )
    }
}
fn bucket_change(bucket: ComparisonBucket) -> String {
    if !bucket.current.observed() || !bucket.previous.observed() {
        return "较前期：数据不足".into();
    }
    format!(
        "较前期 {} · {}",
        signed_bytes(bucket.current.bytes.total() as i128 - bucket.previous.bytes.total() as i128),
        if bucket.comparable() {
            change_text(
                bucket.current.bytes.total(),
                bucket.previous.bytes.total(),
                true,
            )
        } else {
            "仅观测变化，覆盖不完整".into()
        }
    )
}
fn bucket_tooltip(bucket: ComparisonBucket) -> String {
    let describe = |title, from, value: BucketValue| {
        format!(
            "{title} {} — {}\n下载 {} · 上传 {} · 合计 {}\n{}",
            stamp(from),
            stamp(from + bucket.duration),
            human_bytes(value.bytes.down),
            human_bytes(value.bytes.up),
            human_bytes(value.bytes.total()),
            coverage(value, bucket.duration)
        )
    };
    format!(
        "{}\n\n{}\n\n{}",
        describe("本期", bucket.from, bucket.current),
        describe("前期", bucket.previous_from, bucket.previous),
        bucket_change(bucket)
    )
}
fn paint_bar(
    p: Palette,
    rect: Bounds<Pixels>,
    value: BucketValue,
    duration: i64,
    ceiling: f64,
    previous: bool,
    window: &mut Window,
) {
    if !value.observed() {
        let diameter = px(2.5).min(rect.size.width);
        let mut color = rgb(p.muted);
        color.a = 0.3;
        window.paint_quad(
            outline(
                Bounds::new(
                    point(
                        rect.left() + (rect.size.width - diameter) / 2.,
                        rect.bottom() - diameter,
                    ),
                    size(diameter, diameter),
                ),
                color,
                BorderStyle::Solid,
            )
            .corner_radii(diameter / 2.)
            .border_widths(px(0.6).min(diameter / 3.)),
        );
        return;
    }
    if value.bytes.total() == 0 {
        let mut color = rgb(p.muted);
        color.a = 0.25;
        let width = px(3.).min(rect.size.width);
        window.paint_quad(fill(
            Bounds::new(
                point(
                    rect.left() + (rect.size.width - width) / 2.,
                    rect.bottom() - px(1.),
                ),
                size(width, px(1.)),
            ),
            color,
        ));
        return;
    }
    let total_height = rect.size.height
        * ((value.bytes.down as f64 + value.bytes.up as f64) / ceiling).clamp(0., 1.) as f32;
    let silhouette = Bounds::new(
        point(rect.left(), rect.bottom() - total_height),
        size(rect.size.width, total_height),
    );
    let radius = px(3.).min(rect.size.width * 0.4).min(total_height / 2.);
    let corners = Corners {
        top_left: radius,
        top_right: radius,
        bottom_left: px(0.),
        bottom_right: px(0.),
    };
    let mut bottom = rect.bottom();
    for (bytes, color) in [(value.bytes.down, p.download), (value.bytes.up, p.upload)] {
        if bytes == 0 {
            continue;
        }
        let height = rect.size.height * (bytes as f64 / ceiling).clamp(0., 1.) as f32;
        let segment = Bounds::new(
            point(rect.left(), bottom - height),
            size(rect.size.width, height),
        );
        let mut background = rgb(color);
        background.a = if previous {
            if p.dark { 0.32 } else { 0.24 }
        } else {
            0.92
        };
        // Clip each color to the same rounded silhouette. Even a tiny upload cap
        // keeps the outer corners smooth while the stack boundary stays flat.
        window.with_content_mask(Some(ContentMask { bounds: segment }), |window| {
            window.paint_quad(fill(silhouette, background).corner_radii(corners));
        });
        bottom -= height;
    }
    if !value.complete(duration) {
        let diameter = px(2.5).min(rect.size.width);
        window.paint_quad(
            fill(
                Bounds::new(
                    point(
                        rect.left() + (rect.size.width - diameter) / 2.,
                        bottom - px(4.),
                    ),
                    size(diameter, diameter),
                ),
                rgb(p.amber),
            )
            .corner_radii(diameter / 2.),
        );
    }
}

impl TrafficApp {
    pub(super) fn analysis_chart(
        &self,
        snapshot: &Snapshot,
        compact: bool,
        cx: &Context<Self>,
    ) -> Div {
        let p = self.p;
        let bucket_size = self.analysis_bucket(snapshot);
        let bins = Arc::new(snapshot.histogram(bucket_size, self.analysis.focus.as_deref()));
        let compare = self.analysis.compare_previous;
        let peak = bins
            .iter()
            .flat_map(|bucket| {
                [
                    bucket.current.bytes.down as f64 + bucket.current.bytes.up as f64,
                    if compare {
                        bucket.previous.bytes.down as f64 + bucket.previous.bytes.up as f64
                    } else {
                        0.
                    },
                ]
            })
            .fold(0., f64::max);
        let scale = crate::ui::chart::ChartScale::for_peak(peak, false);
        let ceiling = scale.ceiling;
        let marks: Vec<_> = self
            .analysis_bursts()
            .iter()
            .map(|event| event.time)
            .collect();
        let pinned = self.analysis.pinned_bucket;
        let highlight = self.analysis.highlight;
        let detail = pinned
            .and_then(|time| bins.iter().find(|bucket| bucket.contains(time)))
            .copied();
        let axis_width = 50.;
        let height = if compact { 170. } else { 190. };
        let mut plot = div().relative().h(px(height)).flex_1().min_w_0();
        let paint_bins = bins.clone();
        plot = plot.child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    for line in 0..=3 {
                        let y = bounds.top() + bounds.size.height * (line as f32 / 3.);
                        let mut path = PathBuilder::stroke(px(1.));
                        path.move_to(point(bounds.left(), y));
                        path.line_to(point(bounds.right(), y));
                        if let Ok(path) = path.build() {
                            let mut color = rgb(p.line);
                            color.a = if line == 3 { 0.7 } else { 0.45 };
                            window.paint_path(path, color);
                        }
                    }
                    let slot = bounds.size.width / paint_bins.len().max(1) as f32;
                    for (index, bucket) in paint_bins.iter().enumerate() {
                        let x = bounds.left() + slot * index as f32;
                        let focused = pinned.is_some_and(|time| bucket.contains(time))
                            || highlight.is_some_and(|time| {
                                time < bucket.from + bucket.duration && time + 60_000 > bucket.from
                            });
                        if focused {
                            let mut tint = rgb(p.amber);
                            tint.a = 0.055;
                            window.paint_quad(fill(
                                Bounds::new(point(x, bounds.top()), size(slot, bounds.size.height)),
                                tint,
                            ));
                        }
                        let occupancy = if slot < px(8.) { 0.84 } else { 0.64 };
                        let inner = if compare {
                            px(3.).min(slot * 0.1)
                        } else {
                            px(0.)
                        };
                        let width = if compare {
                            ((slot * occupancy - inner) / 2.).min(px(18.))
                        } else {
                            (slot * occupancy).min(px(22.))
                        };
                        let group_width = if compare { width * 2. + inner } else { width };
                        let rect = Bounds::new(
                            point(x + (slot - group_width) / 2., bounds.top()),
                            size(width, bounds.size.height),
                        );
                        paint_bar(
                            p,
                            rect,
                            bucket.current,
                            bucket.duration,
                            ceiling,
                            false,
                            window,
                        );
                        if compare {
                            let rect = Bounds::new(
                                point(rect.left() + width + inner, bounds.top()),
                                rect.size,
                            );
                            paint_bar(
                                p,
                                rect,
                                bucket.previous,
                                bucket.duration,
                                ceiling,
                                true,
                                window,
                            );
                        }
                        if marks.iter().any(|time| {
                            *time < bucket.from + bucket.duration && *time + 60_000 > bucket.from
                        }) {
                            window.paint_quad(
                                fill(
                                    Bounds::new(
                                        point(
                                            x + slot / 2. - px(1.).min(slot / 4.),
                                            bounds.top() - px(5.),
                                        ),
                                        size(px(3.).min(slot / 2.), px(3.)),
                                    ),
                                    rgb(p.amber),
                                )
                                .corner_radii(px(1.5)),
                            );
                        }
                    }
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        );
        for (index, bucket) in bins.iter().copied().enumerate() {
            let tooltip = bucket_tooltip(bucket);
            let mut hover = rgb(p.text);
            hover.a = if p.dark { 0.045 } else { 0.025 };
            plot = plot.child(
                div()
                    .id(SharedString::from(format!(
                        "histogram-bin-{}-{}-{}-{}-{index}",
                        snapshot.request.id,
                        bucket.from,
                        bucket.duration,
                        self.analysis.focus.as_deref().unwrap_or("all")
                    )))
                    .absolute()
                    .left(relative(index as f32 / bins.len() as f32))
                    .top_0()
                    .w(relative(1. / bins.len() as f32))
                    .h_full()
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
                    .on_click(cx.listener(move |app, _, window, cx| {
                        app.act(
                            Action::Analysis(AnalysisAction::Pin(bucket.from)),
                            window,
                            cx,
                        )
                    })),
            );
        }
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
            }));
        // Three well-spaced interior ticks plus endpoints remain readable at 900px.
        let ticks = row()
            .ml(px(axis_width + 8.))
            .justify_between()
            .children((0..=4).map(|index| {
                let time = snapshot.current_from() + snapshot.request.duration() * index / 4;
                let text = if index == 4 {
                    if snapshot.request.duration() > 86_400_000 {
                        history_page::stamp(snapshot.request.anchor)
                    } else {
                        clock(snapshot.request.anchor)
                    }
                } else {
                    if snapshot.request.duration() > 86_400_000 {
                        history_page::stamp(time)
                    } else {
                        clock(time)
                    }
                };
                label(text, p.muted).text_size(px(10.5))
            }));
        let chart_help = "纵轴为每段用量；每组左侧本期、右侧浅色前期。\n悬停查看两期详情，点击柱组固定明细。\n琥珀色柱顶为覆盖不完整，顶部标记对应暴增分钟。\n分钟峰值及暴增始终按原始1分钟数据计算。".to_string();
        let mut card = self
            .card()
            .relative()
            .p(px(16.))
            .gap(px(14.))
            .child(
                row()
                    .justify_between()
                    .flex_wrap()
                    .gap(px(8.))
                    .child(
                        self.taskbar_picker(
                            "analysis-bucket",
                            format!("每柱 {}", bucket_size.label()),
                            snapshot
                                .request
                                .bucket_sizes()
                                .iter()
                                .map(|size| {
                                    (
                                        size.label().into(),
                                        Action::Analysis(AnalysisAction::Bucket(*size)),
                                        *size == bucket_size,
                                    )
                                })
                                .collect(),
                            cx,
                        ),
                    )
                    .child(
                        row()
                            .gap(px(12.))
                            .child(label("● 下载", p.download_text).text_size(px(11.)))
                            .child(label("● 上传", p.upload_text).text_size(px(11.)))
                            .child(
                                Checkbox::new("analysis-compare")
                                    .label("前期对照")
                                    .checked(compare)
                                    .on_click(cx.listener(|app, _, window, cx| {
                                        app.act(
                                            Action::Analysis(AnalysisAction::Compare),
                                            window,
                                            cx,
                                        )
                                    })),
                            )
                            .child(
                                div()
                                    .id("analysis-chart-help")
                                    .child(
                                        Icon::default()
                                            .path("icons/position-info.svg")
                                            .size(px(15.))
                                            .text_color(rgb(p.muted)),
                                    )
                                    .tooltip(move |window, cx| {
                                        Tooltip::new(chart_help.clone()).build(window, cx)
                                    }),
                            ),
                    ),
            )
            .child(
                label(
                    if compare {
                        "每组左侧本期，右侧浅色前期"
                    } else {
                        "当前仅显示本期"
                    },
                    p.muted,
                )
                .text_size(px(11.)),
            )
            .child(row().items_start().gap(px(8.)).child(axis).child(plot))
            .child(ticks);
        if let Some(bucket) = detail {
            let render_period = |title: &str, from, value: BucketValue| {
                column()
                    .flex_1()
                    .min_w_0()
                    .gap(px(3.))
                    .child(
                        label(
                            format!(
                                "{title} {} — {}",
                                stamp(from),
                                stamp(from + bucket.duration)
                            ),
                            p.muted,
                        )
                        .text_size(px(11.))
                        .whitespace_normal(),
                    )
                    .child(
                        row()
                            .gap(px(8.))
                            .flex_wrap()
                            .child(
                                label(
                                    format!("↓ {}", human_bytes(value.bytes.down)),
                                    p.download_text,
                                )
                                .text_size(px(11.)),
                            )
                            .child(
                                label(format!("↑ {}", human_bytes(value.bytes.up)), p.upload_text)
                                    .text_size(px(11.)),
                            )
                            .child(
                                label(format!("合计 {}", human_bytes(value.bytes.total())), p.text)
                                    .text_size(px(11.)),
                            ),
                    )
                    .child(
                        label(
                            coverage(value, bucket.duration),
                            if value.complete(bucket.duration) {
                                p.muted
                            } else {
                                p.amber
                            },
                        )
                        .text_size(px(11.)),
                    )
            };
            card = card.child(
                column()
                    .gap(px(6.))
                    .p(px(10.))
                    .rounded(px(7.))
                    .bg(control_surface(p))
                    .child(
                        row()
                            .justify_between()
                            .child(label("时段详情", p.muted).text_size(px(11.)))
                            .child(
                                self.button(
                                    "analysis-hide-bin",
                                    "收起",
                                    false,
                                    Action::Analysis(AnalysisAction::Pin(bucket.from)),
                                    cx,
                                )
                                .h(px(24.)),
                            ),
                    )
                    .child(
                        row()
                            .items_start()
                            .gap(px(12.))
                            .child(render_period("本期", bucket.from, bucket.current))
                            .child(render_period("前期", bucket.previous_from, bucket.previous)),
                    )
                    .child(label(bucket_change(bucket), p.text).text_size(px(12.))),
            );
        }
        card
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn bucket_details_do_not_treat_missing_or_partial_data_as_zero() {
        let mut bucket = ComparisonBucket {
            from: 0,
            previous_from: -10_000,
            duration: 10_000,
            current: BucketValue {
                bytes: Bytes { down: 100, up: 0 },
                covered_ms: 10_000,
            },
            previous: BucketValue::default(),
        };
        assert!(bucket_change(bucket).contains("数据不足"));
        bucket.previous.covered_ms = 10_000;
        assert!(bucket_change(bucket).contains("新增"));
        bucket.current.covered_ms = 1000;
        assert!(bucket_change(bucket).contains("仅观测变化"));
        assert!(bucket_tooltip(bucket).contains("部分覆盖"));
    }
}
