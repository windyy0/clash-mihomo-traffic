use super::*;
use crate::history::{Grain, Trend};
use std::sync::Arc;
fn stamp(time: i64, grain: Grain) -> String {
    chrono::DateTime::from_timestamp_millis(time)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format(match grain {
                    Grain::Minute | Grain::Hour => "%H:%M",
                    Grain::Months(_) => "%Y/%m",
                    _ => "%m/%d",
                })
                .to_string()
        })
        .unwrap_or_default()
}
impl TrafficApp {
    pub(super) fn history_histogram(
        &self,
        data: Option<Arc<Trend>>,
        compact: bool,
        cx: &Context<Self>,
    ) -> Div {
        let p = self.p;
        let height = if self.history_state.view == history_state::CalendarView::Year {
            128.
        } else {
            144.
        };
        let gap = if self.history_state.view == history_state::CalendarView::Year {
            8.
        } else {
            6.
        };
        let scope = (self.query.from, self.query.to);
        let revision = self.data.storage_revision;
        let motion = self.history_state.chart_motion.clone();
        let data = motion.borrow_mut().prepare(
            data,
            scope,
            revision,
            self.report.query.as_ref() != Some(&self.query),
            Instant::now(),
        );
        let Some(data) = data else {
            return column()
                .gap(px(gap))
                .child(self.history_chart_heading(compact, cx))
                .child(
                    column()
                        .h(px(height + 19. + gap))
                        .justify_center()
                        .child(label("正在读取历史趋势…", p.muted).text_size(px(12.))),
                );
        };
        let frame = Rc::new(
            motion
                .borrow_mut()
                .sample(&data, scope, revision, Instant::now()),
        );
        let placements = frame.bars.clone();
        let waiting = frame.waiting;
        let peak = data
            .buckets
            .iter()
            .map(|b| b.bytes.total() as f64)
            .fold(0., f64::max);
        let scale = chart::ChartScale::for_peak(peak, false);
        let axis_width = scale.compact_axis_width().max(52.);
        let axis = div()
            .relative()
            .w(px(axis_width))
            .h(px(height))
            .flex_none()
            .opacity(if waiting { 0.5 } else { 1. })
            .children(scale.labels.into_iter().enumerate().map(|(i, text)| {
                label(text, p.muted)
                    .absolute()
                    .top(relative(i as f32 / 3.))
                    .mt(px(-7.))
                    .w_full()
                    .text_right()
                    .text_size(px(11.))
                    .whitespace_nowrap()
            }));
        let bars = data.clone();
        let view = cx.weak_entity();
        let measured = self.history_state.chart_plot_bounds.clone();
        let hovered = self
            .valid_history_chart_hover()
            .filter(|h| h.visible)
            .map(|h| h.index);
        let focused = self
            .query
            .history_context
            .map(|_| (self.query.from, self.query.to));
        let mut plot = div().relative().h(px(height)).flex_1().min_w_0().child(
            canvas(
                move |bounds, _, _| {
                    measured.set(bounds);
                },
                move |bounds, _, window, _| {
                    for i in 0..=3 {
                        let y = bounds.top() + bounds.size.height * (i as f32 / 3.);
                        let mut path = PathBuilder::stroke(px(if i == 3 { 1. } else { 0.75 }));
                        if i != 3 {
                            path = path.dash_array(&[px(3.), px(4.)]);
                        }
                        path.move_to(point(bounds.left(), y));
                        path.line_to(point(bounds.right(), y));
                        if let Ok(path) = path.build() {
                            let mut color = rgb(p.line);
                            color.a = if i == 3 { 0.65 } else { 0.42 };
                            window.paint_path(path, color);
                        }
                    }
                    history_chart_motion::paint_bars(
                        bounds, &frame, p, hovered, focused, &bars, window,
                    );
                    motion
                        .borrow_mut()
                        .record(bars.clone(), &frame, scope, revision);
                    if frame.running || frame.finished {
                        let view = view.clone();
                        window.on_next_frame(move |window, cx| {
                            let pointer = window.mouse_position();
                            let _ = view.update(cx, |app, cx| {
                                app.track_history_chart_pointer(pointer, cx);
                                cx.notify();
                            });
                        });
                        window.request_animation_frame();
                    }
                },
            )
            .size_full(),
        );
        if waiting {
            plot = plot.child(
                label("正在读取所选时段…", p.muted)
                    .absolute()
                    .top(px(8.))
                    .right(px(6.))
                    .text_size(px(11.)),
            );
        }
        for (i, b) in data
            .buckets
            .iter()
            .enumerate()
            .filter(|(i, _)| !waiting && placements.get(*i).is_some_and(|b| b.opacity > 0.1))
        {
            let placement = placements[i];
            let from = b.from;
            let to = b.to;
            let grain = data.grain;
            plot = plot.child(
                div()
                    .id(SharedString::from(format!("history-trend-bin-{i}")))
                    .absolute()
                    .top_0()
                    .left(relative(placement.left))
                    .h_full()
                    .w(relative(placement.width))
                    .rounded(px(4.))
                    .on_click(cx.listener(move |app, _, _, cx| app.pin_history_chart(i, cx)))
                    .when(from < data.generated_at, |d| {
                        d.cursor_pointer().on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |app, _, window, cx| {
                                window.prevent_default();
                                cx.stop_propagation();
                                app.act(
                                    Action::History(HistoryAction::Bucket(from, to, grain)),
                                    window,
                                    cx,
                                )
                            }),
                        )
                    }),
            );
        }
        let mut labels = div()
            .relative()
            .h(px(19.))
            .ml(px(axis_width + 10.))
            .opacity(if waiting { 0.5 } else { 1. });
        let count = data.buckets.len().min(4);
        for i in 0..count {
            let n = if count <= 1 {
                0
            } else {
                i * (data.buckets.len() - 1) / (count - 1)
            };
            let text = stamp(data.buckets[n].from, data.grain);
            labels = labels.child(
                label(text, p.muted)
                    .absolute()
                    .top_0()
                    .left(relative(
                        (n as f32 + 0.5) / data.buckets.len().max(1) as f32,
                    ))
                    .when(i == 0, |d| d.left_0())
                    .when(i == count - 1, |d| d.left_auto().right_0().text_right())
                    .when(i > 0 && i < count - 1, |d| d.ml(px(-20.)))
                    .text_size(px(11.)),
            );
        }
        column()
            .gap(px(gap))
            .child(self.history_chart_heading(compact, cx))
            .child(row().gap(px(10.)).child(axis).child(plot))
            .child(labels)
    }
}
