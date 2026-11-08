use super::*;
use chrono::{Datelike, Local, NaiveDate, NaiveDateTime, Timelike};

pub(super) struct Locator {
    pub parts: [Entity<InputState>; 6],
    values: [String; 6],
    pub following: bool,
    pub preview_at: Option<i64>,
    pub error: String,
    pub bounds: [Rc<Cell<Bounds<Pixels>>>; 6],
}

impl Locator {
    pub fn new(window: &mut Window, cx: &mut Context<TrafficApp>) -> Self {
        Self {
            parts: std::array::from_fn(|_| input("", "", false, window, cx)),
            values: std::array::from_fn(|_| String::new()),
            following: true,
            preview_at: None,
            error: String::new(),
            bounds: std::array::from_fn(|_| Rc::new(Cell::new(Bounds::default()))),
        }
    }

    pub fn reset(&mut self) {
        self.following = true;
        self.preview_at = None;
        self.error.clear();
    }
}

fn local_time(at: i64) -> Option<NaiveDateTime> {
    chrono::DateTime::from_timestamp_millis(at).map(|d| d.with_timezone(&Local).naive_local())
}

fn time_parts(at: i64) -> [String; 6] {
    local_time(at).map_or_else(|| std::array::from_fn(|_| String::new()), datetime_parts)
}

fn datetime_parts(d: NaiveDateTime) -> [String; 6] {
    [
        format!("{:04}", d.year()),
        format!("{:02}", d.month()),
        format!("{:02}", d.day()),
        format!("{:02}", d.hour()),
        format!("{:02}", d.minute()),
        format!("{:02}", d.second()),
    ]
}

fn parse_time(text: &str) -> Option<NaiveDateTime> {
    [
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
        "%Y/%m/%d %H:%M:%S",
        "%Y/%m/%d %H:%M",
    ]
    .into_iter()
    .find_map(|format| NaiveDateTime::parse_from_str(text.trim(), format).ok())
}

fn parse_parts(values: &[String; 6]) -> Option<NaiveDateTime> {
    let n: Vec<u32> = values
        .iter()
        .map(|v| v.trim().parse().ok())
        .collect::<Option<_>>()?;
    let year = i32::try_from(n[0])
        .ok()
        .filter(|y| (1..=9999).contains(y))?;
    NaiveDate::from_ymd_opt(year, n[1], n[2])?.and_hms_opt(n[3], n[4], n[5])
}

fn step_time(d: NaiveDateTime, part: usize, step: i32) -> Option<NaiveDateTime> {
    let next = match part {
        0 | 1 => {
            let months = chrono::Months::new(step.unsigned_abs() * if part == 0 { 12 } else { 1 });
            let date = if step < 0 {
                d.date().checked_sub_months(months)?
            } else {
                d.date().checked_add_months(months)?
            };
            date.and_time(d.time())
        }
        2..=5 => {
            let seconds = i64::from(step) * [86_400, 3600, 60, 1][part - 2];
            d.checked_add_signed(chrono::Duration::seconds(seconds))?
        }
        _ => return None,
    };
    (1..=9999).contains(&next.year()).then_some(next)
}

pub(super) fn cursor_time(
    from: i64,
    to: i64,
    bounds: Bounds<Pixels>,
    position: Point<Pixels>,
) -> Option<i64> {
    if to <= from || bounds.size.width <= px(0.) || !bounds.contains(&position) {
        return None;
    }
    let fraction = f64::from(f32::from(position.x - bounds.left()) / f32::from(bounds.size.width));
    Some((from + ((to - from) as f64 * fraction.clamp(0., 1.)) as i64).min(to - 1))
}

pub(super) fn compact_range(from: i64, to: i64, seconds: bool) -> String {
    let (Some(a), Some(b)) = (local_time(from), local_time(to)) else {
        return "—".into();
    };
    let start = if seconds {
        "%Y-%m-%d %H:%M:%S"
    } else {
        "%Y-%m-%d %H:%M"
    };
    let end = match (a.date() == b.date(), a.year() == b.year(), seconds) {
        (true, _, true) => "%H:%M:%S",
        (true, _, false) => "%H:%M",
        (false, true, true) => "%m-%d %H:%M:%S",
        (false, true, false) => "%m-%d %H:%M",
        (false, false, _) => start,
    };
    format!("{}–{}", a.format(start), b.format(end))
}

impl TrafficApp {
    pub(super) fn drill_locator_time(&self) -> Option<i64> {
        if self.seconds.pinned {
            match self.seconds.hover {
                Some(crate::traffic_drill::Target::Second(seq)) => {
                    if let Some(sample) = self
                        .seconds
                        .data
                        .as_ref()?
                        .samples
                        .iter()
                        .find(|s| s.seq == seq)
                    {
                        return Some(sample.from);
                    }
                }
                Some(crate::traffic_drill::Target::Minute(from, _)) => return Some(from),
                None => {}
            }
        }
        self.seconds
            .locator
            .preview_at
            .or_else(|| self.seconds.view.map(|v| v.from + (v.to - v.from) / 2))
    }

    pub(super) fn preview_drill_time(&mut self, at: i64, cx: &mut Context<Self>) {
        let locator = &mut self.seconds.locator;
        let at = at.div_euclid(1000) * 1000;
        if locator.following && !self.seconds.pinned && locator.preview_at != Some(at) {
            locator.preview_at = Some(at);
            cx.notify();
        }
    }

    fn set_drill_time_values(
        &mut self,
        values: [String; 6],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.seconds.locator.values = values.clone();
        for (field, value) in self.seconds.locator.parts.clone().into_iter().zip(values) {
            if field.read(cx).value().as_ref() != value {
                field.update(cx, |s, cx| s.set_value(value, window, cx));
            }
        }
    }

    pub(super) fn edit_drill_time(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.seconds.locator.following {
            let Some(at) = self.drill_locator_time() else {
                return;
            };
            self.seconds.locator.following = false;
            self.set_drill_time_values(time_parts(at), window, cx);
            cx.notify();
        }
    }

    pub(super) fn follow_drill_time(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.seconds.locator.following = true;
        self.seconds.locator.error.clear();
        if self.seconds.is_modal() {
            self.seconds.focus.focus(window);
        } else {
            window.blur();
        }
        cx.notify();
    }

    pub(super) fn subscribe_drill_locator(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for (index, field) in self.seconds.locator.parts.clone().into_iter().enumerate() {
            self._subscriptions.push(cx.subscribe_in(
                &field,
                window,
                move |app, field, event, window, cx| {
                    if app.seconds.scope.is_none() {
                        return;
                    }
                    match event {
                        InputEvent::Focus => {
                            app.edit_drill_time(window, cx);
                            let field = field.clone();
                            window.on_next_frame(move |window, cx| {
                                if field.focus_handle(cx).is_focused(window) {
                                    window.dispatch_action(
                                        Box::new(gpui_component::input::SelectAll),
                                        cx,
                                    );
                                }
                            });
                        }
                        InputEvent::Change => {
                            let value = field.read(cx).value().to_string();
                            if app.seconds.locator.values[index] == value {
                                return;
                            }
                            app.seconds.locator.following = false;
                            app.seconds.locator.values[index] = value.clone();
                            app.seconds.locator.error.clear();
                            if let Some(d) = parse_time(&value) {
                                app.set_drill_time_values(datetime_parts(d), window, cx);
                            }
                        }
                        _ => {}
                    }
                    cx.notify();
                },
            ));
        }
    }

    pub(super) fn step_drill_time(
        &mut self,
        part: usize,
        step: i32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit_drill_time(window, cx);
        let values = self
            .seconds
            .locator
            .parts
            .each_ref()
            .map(|f| f.read(cx).value().to_string());
        let Some(next) = parse_parts(&values).and_then(|d| step_time(d, part, step)) else {
            self.seconds.locator.error = "请输入有效日期时间".into();
            cx.notify();
            return;
        };
        self.set_drill_time_values(datetime_parts(next), window, cx);
        self.seconds.locator.error.clear();
        if self.seconds.locator.parts[part]
            .focus_handle(cx)
            .is_focused(window)
        {
            window.dispatch_action(Box::new(gpui_component::input::SelectAll), cx);
        }
        cx.notify();
    }

    pub(super) fn drill_time_keystroke(
        &mut self,
        event: &KeystrokeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.seconds.scope.is_none()
            || (!self.seconds.is_modal() && !self.seconds.locate_open)
            || event.keystroke.modifiers != Modifiers::default()
        {
            return false;
        }
        let Some(index) = self
            .seconds
            .locator
            .parts
            .iter()
            .position(|f| f.focus_handle(cx).is_focused(window))
        else {
            return false;
        };
        match event.keystroke.key.as_str() {
            "up" | "down" => self.step_drill_time(
                index,
                if event.keystroke.key == "up" { 1 } else { -1 },
                window,
                cx,
            ),
            "enter" => self.locate_seconds(window, cx),
            _ => return false,
        }
        true
    }

    pub(super) fn drill_time_result(&self, cx: &App) -> Option<i64> {
        if self.seconds.locator.following {
            return self.drill_locator_time();
        }
        let values = self
            .seconds
            .locator
            .parts
            .each_ref()
            .map(|f| f.read(cx).value().to_string());
        parse_parts(&values)?
            .and_local_timezone(Local)
            .single()
            .map(|d| d.timestamp_millis())
    }

    fn drill_time_field(&self, index: usize, value: String, cx: &Context<Self>) -> Stateful<Div> {
        let p = self.p;
        let field = self.seconds.locator.parts[index].clone();
        let width = px(if index == 0 { 36. } else { 22. });
        let content = if self.seconds.locator.following {
            label(value, p.text)
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(13.))
                .into_any_element()
        } else {
            TextInput::new(&field)
                .appearance(false)
                .focus_bordered(false)
                .px(px(0.))
                .pl(px(2.))
                .py(px(0.))
                .h(px(28.))
                .w(width + px(8.))
                .absolute()
                .top_0()
                .left_0()
                .text_size(px(13.))
                .text_color(rgb(p.text))
                .into_any_element()
        };
        let weak = cx.entity().downgrade();
        let measured = self.seconds.locator.bounds[index].clone();
        let hit = canvas(
            move |bounds, window, _| {
                measured.set(bounds);
                window.insert_hitbox(bounds, HitboxBehavior::Normal)
            },
            move |_, hitbox, window, _| {
                let hitbox = hitbox.clone();
                let weak = weak.clone();
                window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
                    if phase != DispatchPhase::Capture || !hitbox.should_handle_scroll(window) {
                        return;
                    }
                    let delta = event.delta.pixel_delta(window.line_height());
                    if delta.y == px(0.) || delta.x.abs() > delta.y.abs() {
                        return;
                    }
                    let _ = weak.update(cx, |app, cx| {
                        let step = app.time_wheel_step(
                            history_state::TimeWheelTarget::DrillPart(index),
                            event.delta,
                        );
                        app.edit_drill_time(window, cx);
                        if step != 0 {
                            app.step_drill_time(index, step, window, cx);
                        }
                    });
                    cx.stop_propagation();
                });
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        div()
            .id(("drill-time-part", index))
            .relative()
            .w(width)
            .h(px(28.))
            .flex_none()
            .overflow_hidden()
            .rounded(px(3.))
            .font_family("Segoe UI")
            .font_weight(FontWeight::NORMAL)
            .track_focus(&field.focus_handle(cx))
            .hover(move |d| d.bg(state_surface(p)))
            .child(content)
            .child(hit)
            .on_hover(cx.listener(|app, entered: &bool, window, cx| {
                if *entered {
                    app.edit_drill_time(window, cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |app, _, window, cx| {
                    app.edit_drill_time(window, cx);
                    field.focus_handle(cx).focus(window);
                }),
            )
    }

    pub(super) fn drill_locator(&self, cx: &Context<Self>) -> Div {
        let p = self.p;
        let values = if self.seconds.locator.following {
            time_parts(self.drill_locator_time().unwrap_or(0))
        } else {
            self.seconds
                .locator
                .parts
                .each_ref()
                .map(|f| f.read(cx).value().to_string())
        };
        let mut fields = row().gap(px(0.)).flex_none();
        for (index, value) in values.into_iter().enumerate() {
            if index > 0 {
                fields = fields.child(
                    label(
                        match index {
                            1 | 2 => "-",
                            3 => "",
                            _ => ":",
                        },
                        p.muted,
                    )
                    .w(px(if index == 3 { 8. } else { 6. }))
                    .text_size(px(13.))
                    .flex_none()
                    .text_center(),
                );
            }
            fields = fields.child(self.drill_time_field(index, value, cx));
        }
        let hint = "指向图表显示时间；点击数字输入／粘贴，滚轮或↑↓微调；Enter定位";
        let editor = row()
            .gap(px(4.))
            .px(px(5.))
            .h(px(34.))
            .rounded(px(7.))
            .border_1()
            .border_color(control_border(p))
            .bg(control_surface(p))
            .flex_none()
            .child(fields)
            .child(
                div()
                    .id("drill-time-hint")
                    .size(px(20.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        Icon::default()
                            .path("icons/clock.svg")
                            .size(px(14.))
                            .text_color(rgb(p.muted)),
                    )
                    .tooltip(move |window, cx| Tooltip::new(hint).build(window, cx)),
            );
        column()
            .flex_none()
            .gap(px(4.))
            .child(
                row()
                    .gap(px(6.))
                    .flex_none()
                    .child(editor)
                    .child(
                        self.button(
                            "seconds-locate",
                            "定位",
                            false,
                            Action::Drill(DrillAction::SecondsLocate),
                            cx,
                        )
                        .track_focus(&self.seconds.controls[0]),
                    )
                    .child(
                        self.button(
                            "seconds-follow",
                            "跟随鼠标",
                            self.seconds.locator.following,
                            Action::Drill(DrillAction::DrillFollowTime),
                            cx,
                        )
                        .track_focus(&self.seconds.controls[5])
                        .text_size(px(11.))
                        .px(px(6.)),
                    ),
            )
            .when(!self.seconds.locator.error.is_empty(), |d| {
                d.child(
                    label(self.seconds.locator.error.clone(), p.amber)
                        .text_size(px(11.))
                        .whitespace_normal(),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> i64 {
        parse_time(text)
            .unwrap()
            .and_local_timezone(Local)
            .single()
            .unwrap()
            .timestamp_millis()
    }

    #[::core::prelude::v1::test]
    fn compact_dates_keep_only_needed_date_components() {
        for (from, to, expected) in [
            (
                "2026-11-03 14:20",
                "2026-11-03 14:30",
                "2026-11-03 14:20–14:30",
            ),
            (
                "2026-11-03 23:55",
                "2026-11-04 00:05",
                "2026-11-03 23:55–11-04 00:05",
            ),
            (
                "2026-12-31 23:55",
                "2027-01-01 00:05",
                "2026-12-31 23:55–2027-01-01 00:05",
            ),
        ] {
            assert_eq!(compact_range(at(from), at(to), false), expected);
        }
        assert_eq!(
            compact_range(at("2026-11-03 14:20:01"), at("2026-11-03 14:30:02"), true),
            "2026-11-03 14:20:01–14:30:02"
        );
    }

    #[::core::prelude::v1::test]
    fn cursor_time_maps_the_plot_even_without_samples_and_stays_half_open() {
        let bounds = Bounds::new(point(px(20.), px(30.)), size(px(200.), px(80.)));
        assert_eq!(
            cursor_time(1000, 601000, bounds, bounds.center()),
            Some(301000)
        );
        assert_eq!(cursor_time(1000, 601000, bounds, bounds.origin), Some(1000));
        let last = cursor_time(
            1000,
            601000,
            bounds,
            point(bounds.right() - px(0.001), bounds.top()),
        )
        .unwrap();
        assert!((600990..601000).contains(&last));
        assert_eq!(
            cursor_time(1000, 601000, bounds, point(px(19.), px(40.))),
            None
        );
        assert_eq!(cursor_time(1000, 1000, bounds, bounds.center()), None);
        assert_eq!(
            cursor_time(1000, 601000, Bounds::default(), point(px(0.), px(0.))),
            None
        );
    }

    #[::core::prelude::v1::test]
    fn calendar_steps_clamp_month_ends_and_carry_time_across_dates() {
        for (text, part, step, expected) in [
            ("2024-02-29 12:34:56", 0, 1, "2025-02-28 12:34:56"),
            ("2024-01-31 12:34:56", 1, 1, "2024-02-29 12:34:56"),
            ("2026-03-31 12:34:56", 1, -1, "2026-02-28 12:34:56"),
            ("2026-12-31 23:59:59", 5, 1, "2027-01-01 00:00:00"),
            ("2026-11-03 00:00:00", 4, -1, "2026-11-02 23:59:00"),
            ("2026-11-03 00:00:00", 3, -1, "2026-11-02 23:00:00"),
            ("2024-02-28 00:00:00", 2, 1, "2024-02-29 00:00:00"),
        ] {
            assert_eq!(
                step_time(parse_time(text).unwrap(), part, step),
                parse_time(expected)
            );
        }
        assert!(step_time(parse_time("0001-01-01 00:00:00").unwrap(), 0, -1).is_none());
    }

    #[::core::prelude::v1::test]
    fn pasted_times_and_segment_validation_are_strict() {
        let d = parse_time("2024/02/29 23:59:58").unwrap();
        assert_eq!(parse_parts(&datetime_parts(d)), Some(d));
        assert_eq!(parse_time("2024-02-29 23:59").unwrap().second(), 0);
        assert!(parse_time("2026-02-29 00:00").is_none());
        let mut values = datetime_parts(d);
        values[3] = "24".into();
        assert!(parse_parts(&values).is_none());
        values[3].clear();
        assert!(parse_parts(&values).is_none());
    }
}
