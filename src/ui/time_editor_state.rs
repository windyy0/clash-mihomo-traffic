use super::*;
use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use history_state::{CalendarAction, TimeQuickRange, TimeUnit, TimeWheelTarget};

fn parse_parts(values: &[String; 10], unit: TimeUnit, precise: bool) -> anyhow::Result<(i64, i64)> {
    let today = chrono::Local::now().date_naive();
    let endpoint = |offset: usize| -> anyhow::Result<i64> {
        let number = |part: usize| -> anyhow::Result<u32> {
            let value = values[offset + part].trim();
            let endpoint = if offset == 0 { "开始" } else { "结束" };
            let field = ["年份", "月份", "日期", "小时", "分钟"][part];
            anyhow::ensure!(!value.is_empty(), "请填写{endpoint}{field}");
            value
                .parse::<u32>()
                .map_err(|_| anyhow::anyhow!("请输入有效的{endpoint}{field}"))
        };
        let year = i32::try_from(number(0)?).map_err(|_| anyhow::anyhow!("请输入有效年份"))?;
        anyhow::ensure!(year >= 1, "请输入有效年份");
        let day = NaiveDate::from_ymd_opt(
            year,
            if unit == TimeUnit::Year {
                1
            } else {
                number(1)?
            },
            if unit == TimeUnit::Day { number(2)? } else { 1 },
        )
        .ok_or_else(|| anyhow::anyhow!("请输入有效日期"))?;
        let span = unit
            .span(day, today)
            .ok_or_else(|| anyhow::anyhow!("不选择未来日期"))?;
        let date = if offset == 0 { span.0 } else { span.1 };
        let time = if precise && unit == TimeUnit::Day {
            NaiveTime::from_hms_opt(number(3)?, number(4)?, 0)
                .ok_or_else(|| anyhow::anyhow!("请输入有效时分"))?
        } else if offset == 0 {
            NaiveTime::MIN
        } else {
            NaiveTime::from_hms_opt(23, 59, 0).unwrap()
        };
        date.and_time(time)
            .and_local_timezone(chrono::Local)
            .earliest()
            .map(|d| d.timestamp_millis())
            .ok_or_else(|| anyhow::anyhow!("本地时间无效"))
    };
    let (from, last) = (endpoint(0)?, endpoint(5)?);
    anyhow::ensure!(last >= from, "结束时间不能早于开始时间");
    anyhow::ensure!(last - from < 3660 * crate::history::DAY, "范围不超过十年");
    anyhow::ensure!(
        crate::history::local_date(last) <= today && from < chrono::Utc::now().timestamp_millis(),
        "不选择未来日期"
    );
    Ok((from, last + 60_000))
}

fn parts_for(from: &str, to: &str) -> Option<[String; 10]> {
    fn parse(s: &str) -> Option<NaiveDateTime> {
        ["%Y-%m-%d %H:%M", "%Y/%m/%d %H:%M", "%Y.%m.%d %H:%M"]
            .into_iter()
            .find_map(|fmt| NaiveDateTime::parse_from_str(s.trim(), fmt).ok())
    }
    let (a, b) = (parse(from)?, parse(to)?);
    Some([
        a.year().to_string(),
        format!("{:02}", a.month()),
        format!("{:02}", a.day()),
        format!("{:02}", a.hour()),
        format!("{:02}", a.minute()),
        b.year().to_string(),
        format!("{:02}", b.month()),
        format!("{:02}", b.day()),
        format!("{:02}", b.hour()),
        format!("{:02}", b.minute()),
    ])
}

impl TrafficApp {
    pub(super) fn subscribe_range_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for (index, field) in self
            .history_state
            .time_parts
            .clone()
            .into_iter()
            .enumerate()
        {
            self._subscriptions.push(cx.subscribe_in(
                &field,
                window,
                move |app, field, event, window, cx| {
                    match event {
                        InputEvent::Change => app.time_part_changed(index, window, cx),
                        InputEvent::Focus => {
                            app.history_state.error.clear();
                            app.history_state.time_pick_target = Some(index / 5);
                            app.history_state.time_anchor = None;
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
                        InputEvent::Blur => {
                            let value = field.read(cx).value();
                            if let Ok(n) = value.parse::<u32>() {
                                let value = if index % 5 == 0 {
                                    n.to_string()
                                } else {
                                    format!("{n:02}")
                                };
                                app.set_time_part(index, value, window, cx);
                            }
                            field.update(cx, |s, cx| s.unselect(window, cx));
                        }
                        _ => {}
                    }
                    cx.notify();
                },
            ));
        }
        let browse = std::iter::once(self.history_state.year_input.clone())
            .chain(self.history_state.time_browse.clone());
        for (index, field) in browse.enumerate() {
            self._subscriptions.push(cx.subscribe_in(
                &field,
                window,
                move |app, field, event, window, cx| {
                    if app.history_state.time_open
                        && app.history_state.time_expanded
                        && matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. })
                    {
                        app.edit_time_browse(index, window, cx);
                    }
                    if matches!(event, InputEvent::Blur) {
                        field.update(cx, |s, cx| s.unselect(window, cx));
                    }
                },
            ));
        }
    }

    pub(super) fn time_part_strings(&self, cx: &App) -> [String; 10] {
        let mut values = std::array::from_fn(|i| {
            self.history_state.time_parts[i]
                .read(cx)
                .value()
                .to_string()
        });
        if let Some((a, b)) = self.time_drag_preview() {
            for (offset, date) in [(0, a), (5, b)] {
                values[offset] = date.year().to_string();
                values[offset + 1] = format!("{:02}", date.month());
                values[offset + 2] = format!("{:02}", date.day());
            }
        }
        values
    }
    fn set_time_part(
        &mut self,
        index: usize,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.history_state.time_values[index] = value.clone();
        if self.history_state.time_parts[index]
            .read(cx)
            .value()
            .as_ref()
            != value
        {
            self.history_state.time_parts[index].update(cx, |s, cx| s.set_value(value, window, cx));
        }
    }
    pub(super) fn set_time_values(
        &mut self,
        values: [&str; 4],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(parts) = parts_for(
            &format!("{} {}", values[0], values[1]),
            &format!("{} {}", values[2], values[3]),
        ) {
            for (i, value) in parts.into_iter().enumerate() {
                self.set_time_part(i, value, window, cx);
            }
        }
    }
    pub(super) fn time_range_result(&self, cx: &App) -> anyhow::Result<(i64, i64)> {
        parse_parts(
            &self.time_part_strings(cx),
            self.history_state.time_unit,
            true,
        )
    }
    pub(super) fn time_range_dirty(&self, cx: &App) -> bool {
        self.time_range_result(cx).ok() != Some(self.history_state.time_applied)
    }
    pub(super) fn time_range_error(&self, window: &Window, cx: &App) -> Option<String> {
        if !self.history_state.error.is_empty() {
            return Some(self.history_state.error.clone());
        }
        // Allow partial input without adding/removing a validation row mid-edit.
        // Enter stores an explicit error above; blur shows the current result.
        if self
            .history_state
            .time_parts
            .iter()
            .any(|part| part.focus_handle(cx).is_focused(window))
        {
            return None;
        }
        self.time_range_result(cx)
            .err()
            .map(|error| error.to_string())
    }
    pub(super) fn history_time_selection(
        &self,
        cx: &Context<Self>,
    ) -> Option<(NaiveDate, NaiveDate)> {
        let v = self.time_part_strings(cx);
        let unit = self.history_state.time_unit;
        let date = |i: usize| {
            NaiveDate::from_ymd_opt(
                v[i].parse().ok()?,
                if unit == TimeUnit::Year {
                    1
                } else {
                    v[i + 1].parse().ok()?
                },
                if unit == TimeUnit::Day {
                    v[i + 2].parse().ok()?
                } else {
                    1
                },
            )
        };
        let today = chrono::Local::now().date_naive();
        Some((unit.span(date(0)?, today)?.0, unit.span(date(5)?, today)?.1))
    }
    pub(super) fn open_history_time(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.page == Page::Overview {
            return;
        }
        if self.history_state.time_open
            && self.history_state.time_apps == (self.page == Page::Apps)
            && (self.page == Page::Apps || self.history_state.time_target == self.history_state.tab)
        {
            self.history_state.time_focus.focus(window);
            return;
        }
        let range = if self.page == Page::History && self.history_state.tab == HistoryTab::Analysis
        {
            self.history_state
                .analysis_period
                .and_then(|p| p.bounds(chrono::Local::now()).ok().flatten())
                .unwrap_or(self.history_state.analysis_range)
        } else {
            (self.query.from, self.query.to)
        };
        self.history_state.time_applied = range;
        self.history_state.time_initialized = true;
        self.history_state.time_apps = self.page == Page::Apps;
        self.history_state.time_unit = TimeUnit::Day;
        self.history_state.time_target = self.history_state.tab;
        self.history_state.time_open = true;
        self.history_state.time_expanded = false;
        self.history_state.help_open = false;
        self.history_state.month_open = false;
        self.history_state.annual_hover = None;
        self.history_state.error.clear();
        self.reset_time_range(window, cx);
        self.history_state.time_focus.focus(window);
    }
    pub(super) fn reset_time_range(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.history_state.time_quick = None;
        let range = self.history_state.time_applied;
        let (from, to) = (
            history_state::stamp(range.0),
            history_state::stamp(range.1 - 60_000),
        );
        self.history_state.time_unit = TimeUnit::Day;
        self.history_state.time_precision = true;
        self.set_time_values([&from[..10], &from[11..], &to[..10], &to[11..]], window, cx);
        self.history_state.time_anchor = None;
        self.history_state.time_pick_target = None;
        self.history_state.time_drag = None;
        self.history_state.time_side = 0;
        self.history_state.error.clear();
        self.locate_time_calendar(window, cx);
    }
    pub(super) fn toggle_time_range(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let same = self.history_state.time_apps == (self.page == Page::Apps)
            && (self.page == Page::Apps
                || self.history_state.time_target == self.history_state.tab);
        if self.history_state.time_open && same {
            self.history_state.time_open = false;
            self.history_state.time_expanded = false;
            self.history_state.time_drag = None;
            self.history_state.time_dragged = false;
            window.blur();
            cx.notify();
            return;
        }
        let range = if self.page == Page::History && self.history_state.tab == HistoryTab::Analysis
        {
            self.history_state.analysis_range
        } else {
            (self.query.from, self.query.to)
        };
        if same && self.history_state.time_initialized && range == self.history_state.time_applied {
            self.history_state.time_open = true;
            self.history_state.time_expanded = false;
            self.history_state.time_focus.focus(window);
            cx.notify();
        } else {
            self.history_calendar_action(CalendarAction::OpenTime, window, cx);
        }
    }
    pub(super) fn select_time_quick(
        &mut self,
        quick: TimeQuickRange,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((from, to)) = quick.dates(chrono::Local::now().date_naive()) else {
            return;
        };
        self.history_state.time_unit = TimeUnit::Day;
        self.history_state.time_precision = true;
        let (from_text, to_text) = (
            from.format("%Y-%m-%d").to_string(),
            to.format("%Y-%m-%d").to_string(),
        );
        self.set_time_values([&from_text, "00:00", &to_text, "23:59"], window, cx);
        self.history_state.time_quick = Some(quick);
        self.history_state.time_anchor = None;
        self.history_state.time_pick_target = None;
        self.history_state.time_drag = None;
        self.history_state.time_dragged = false;

        self.history_state.time_expanded = true;
        self.history_state.error.clear();
        self.locate_time_calendar(window, cx);
        let index = TimeQuickRange::ALL
            .iter()
            .position(|q| *q == quick)
            .unwrap();
        self.history_state.time_quick_buttons[index].focus(window);
        cx.notify();
    }
    pub(super) fn active_time_quick(&self, today: NaiveDate, cx: &App) -> Option<TimeQuickRange> {
        let quick = self.history_state.time_quick?;
        let (from, to) = quick.dates(today)?;
        let expected = (
            crate::history::midnight(from).ok()?,
            crate::history::midnight(to.succ_opt()?).ok()?,
        );
        (self.time_range_result(cx).ok()? == expected).then_some(quick)
    }
    pub(super) fn locate_time_calendar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((from, to)) = self.history_time_selection(cx) {
            let (from, to) = (from.min(to), from.max(to));
            let first = from.with_day(1).unwrap();
            let last = to.with_day(1).unwrap();
            let left = if first == last {
                first
                    .checked_sub_months(chrono::Months::new(1))
                    .filter(|m| m.year() >= 1)
                    .unwrap_or(first)
            } else {
                first
            };
            self.history_state.time_months = [left, last];
            self.history_state.time_cursors = [if left == first { from } else { left }, to];
            self.history_state.time_year = to.year();
        }
        self.sync_time_browse(window, cx);
    }
    fn sync_time_browse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let [first, right] = self.history_state.time_months;
        let year = if self.history_state.time_unit == TimeUnit::Day {
            first.year()
        } else {
            self.history_state.time_year
        };
        for (field, value) in std::iter::once(self.history_state.year_input.clone())
            .chain(self.history_state.time_browse.clone())
            .zip([
                year.to_string(),
                format!("{:02}", first.month()),
                right.year().to_string(),
                format!("{:02}", right.month()),
            ])
        {
            if field.read(cx).value().as_ref() != value {
                field.update(cx, |s, cx| s.set_value(value, window, cx));
            }
        }
    }
    pub(super) fn edit_time_browse(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if index == 4 {
            self.locate_time_calendar(window, cx);
            cx.notify();
            return;
        }
        let field = if index == 0 {
            &self.history_state.year_input
        } else {
            &self.history_state.time_browse[index - 1]
        };
        let value = field.read(cx).value().parse::<i32>().ok();
        let side = index / 2;
        let shown = self.history_state.time_months[side];
        let year = if index.is_multiple_of(2) {
            value.filter(|v| (1..=chrono::Local::now().year()).contains(v))
        } else {
            Some(shown.year())
        };
        let month = if index % 2 == 1 {
            value.filter(|v| (1..=12).contains(v)).map(|v| v as u32)
        } else {
            Some(shown.month())
        };
        if let Some(date) = year
            .zip(month)
            .and_then(|(y, m)| NaiveDate::from_ymd_opt(y, m, 1))
        {
            if self.can_show_time_month(side, date) {
                self.history_state.time_months[side] = date;
                self.history_state.time_cursors[side] = date;
                self.history_state.time_side = side;
                self.history_state.time_year = date.year();
                self.history_state.error.clear();
            } else {
                self.history_state.error = "左侧月份不能晚于右侧月份".into();
            }
        } else {
            self.history_state.error = "请输入有效年月，不选择未来年份".into();
        }
        self.sync_time_browse(window, cx);
        cx.notify();
    }
    fn can_show_time_month(&self, side: usize, month: NaiveDate) -> bool {
        month.year() >= 1
            && month.year() <= chrono::Local::now().year()
            && if side == 0 {
                month <= self.history_state.time_months[1]
            } else {
                month >= self.history_state.time_months[0]
            }
    }
    pub(super) fn can_shift_time_calendar(&self, side: usize, delta: i32) -> bool {
        let month = self.history_state.time_months[side];
        let next = if delta < 0 {
            month.checked_sub_months(chrono::Months::new(delta.unsigned_abs()))
        } else {
            month.checked_add_months(chrono::Months::new(delta as u32))
        };
        next.is_some_and(|m| self.can_show_time_month(side, m))
    }
    pub(super) fn shift_time_calendar(
        &mut self,
        side: usize,
        delta: i32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.history_state.time_unit == TimeUnit::Day {
            if self.can_shift_time_calendar(side, delta) {
                let m = self.history_state.time_months[side];
                let next = if delta < 0 {
                    m.checked_sub_months(chrono::Months::new(delta.unsigned_abs()))
                } else {
                    m.checked_add_months(chrono::Months::new(delta as u32))
                }
                .unwrap();
                self.history_state.time_months[side] = next;
                self.history_state.time_cursors[side] = next;
                self.history_state.time_side = side;
            }
        } else {
            self.history_state.time_year = (self.history_state.time_year
                + delta
                    * if self.history_state.time_unit == TimeUnit::Year {
                        12
                    } else {
                        1
                    })
            .clamp(1, chrono::Local::now().year());
        }
        self.sync_time_browse(window, cx);
        cx.notify();
    }
    pub(super) fn time_all_day_enabled(&self, cx: &Context<Self>) -> bool {
        if self.history_state.time_unit != TimeUnit::Day
            || !self.history_time_selection(cx).is_some_and(|(a, b)| a <= b)
        {
            return false;
        }
        let values = self.time_part_strings(cx);
        [3, 4, 8, 9]
            .into_iter()
            .zip([0, 0, 23, 59])
            .any(|(i, n)| values[i].parse::<u32>() != Ok(n))
    }
    pub(super) fn set_time_all_day(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.time_all_day_enabled(cx) {
            return;
        }
        for (index, value) in [(3, "00"), (4, "00"), (8, "23"), (9, "59")] {
            self.set_time_part(index, value.into(), window, cx);
        }
        self.history_state.time_anchor = None;
        self.history_state.time_pick_target = None;
        self.history_state.time_drag = None;
        self.history_state.time_dragged = false;
        self.history_state.error.clear();
        self.history_state.time_all_day.focus(window);
        cx.notify();
    }
    pub(super) fn change_time_unit(
        &mut self,
        unit: TimeUnit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.history_state.time_unit = unit;
        if unit != TimeUnit::Day {
            self.history_state.time_side = 0;
        }
        self.history_state.time_anchor = None;
        self.history_state.time_pick_target = None;
        if unit != TimeUnit::Day {
            self.history_state.time_precision = false;
            if let Some((a, b)) = self.history_time_selection(cx)
                && let (Some(a), Some(b)) = (
                    unit.span(a, chrono::Local::now().date_naive()),
                    unit.span(b, chrono::Local::now().date_naive()),
                )
            {
                self.set_time_dates(a.0, b.1, window, cx);
            }
        }
        self.sync_time_browse(window, cx);
    }
    fn set_time_dates(
        &mut self,
        a: NaiveDate,
        b: NaiveDate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for (offset, date) in [(0, a), (5, b)] {
            for (part, value) in [
                date.year().to_string(),
                format!("{:02}", date.month()),
                format!("{:02}", date.day()),
            ]
            .into_iter()
            .enumerate()
            {
                self.set_time_part(offset + part, value, window, cx);
            }
        }
    }
    pub(super) fn pick_history_time(
        &mut self,
        date: NaiveDate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.history_state.time_dragged {
            return;
        }
        let Some(span) = self
            .history_state
            .time_unit
            .span(date, chrono::Local::now().date_naive())
        else {
            return;
        };
        let range = if let Some((a, b)) = self.history_state.time_anchor.take() {
            (a.min(span.0), b.max(span.1))
        } else if let Some(target) = self.history_state.time_pick_target.take() {
            let (a, b) = self.history_time_selection(cx).unwrap_or(span);
            if target == 0 {
                self.history_state.time_pick_target = Some(1);
                (span.0, b)
            } else {
                (a.min(span.0), a.max(span.1))
            }
        } else {
            self.history_state.time_anchor = Some(span);
            span
        };
        self.set_time_dates(range.0, range.1, window, cx);
        self.history_state.time_cursors[self.history_state.time_side] = date;
        self.history_state.error.clear();
        self.history_state.time_grids[self.history_state.time_side].focus(window);
        cx.notify();
    }
    pub(super) fn apply_history_dates(&mut self, cx: &Context<Self>) {
        let result = if self.history_state.time_open {
            self.time_range_result(cx)
        } else {
            let fields = &self.history_state.dates[self.history_state.index()];
            parts_for(&fields[0].read(cx).value(), &fields[1].read(cx).value())
                .ok_or_else(|| anyhow::anyhow!("请输入有效日期与时分"))
                .and_then(|v| parse_parts(&v, TimeUnit::Day, true))
        };
        match result {
            Ok(range) => {
                self.history_state.error.clear();
                self.history_state.time_applied = range;
                self.history_state.time_expanded = false;
                self.history_state.time_anchor = None;
                self.history_state.time_pick_target = None;
                if self.page == Page::Apps {
                    self.period = TimePeriod::Custom;
                    self.app_custom_range = Some(range);
                    self.query.from = range.0;
                    self.query.to = range.1;
                    self.query.page = 0;
                    self.refresh();
                } else if self.history_state.time_target == HistoryTab::Analysis
                    && self.history_state.time_open
                    || self.history_state.tab == HistoryTab::Analysis
                        && !self.history_state.time_open
                {
                    self.history_state.analysis_period = Some(TimePeriod::Custom);
                    self.history_state.analysis_range = range;
                    self.analysis.history_bucket = None;
                    self.request_analysis(self.analysis.range);
                } else {
                    self.history_state.record_period = TimePeriod::Custom;
                    self.history_state.anchor = crate::history::local_date(range.0);
                    self.history_state.cursor = crate::history::local_date(range.1 - 1);
                    self.history_state.month = self.history_state.cursor.with_day(1).unwrap();
                    self.set_history_range(range, true);
                }
            }
            Err(error) => self.history_state.error = error.to_string(),
        }
    }
    fn time_part_changed(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.history_state.time_parts[index]
            .read(cx)
            .value()
            .to_string();
        if self.history_state.time_values[index] == value {
            return;
        }
        self.history_state.time_values[index] = value.clone();
        if value.contains(['/', '-']) && value.len() >= 8 {
            let normalized = value.replace(" - ", "–").replace('至', "–");
            let items: Vec<_> = normalized.split(['–', '—']).map(str::trim).collect();
            let with_time = |s: &str, end: bool| {
                if s.contains(' ') {
                    s.to_string()
                } else {
                    format!("{s} {}", if end { "23:59" } else { "00:00" })
                }
            };
            let mut v = self.time_part_strings(cx);
            let mut parsed = false;
            if items.len() == 2 {
                if let Some(parts) =
                    parts_for(&with_time(items[0], false), &with_time(items[1], true))
                {
                    v = parts;
                    parsed = true;
                }
            } else if let Some(parts) = parts_for(
                &with_time(items[0], index >= 5),
                &with_time(items[0], index >= 5),
            ) {
                let start = index / 5 * 5;
                v[start..start + 5].clone_from_slice(&parts[..5]);
                parsed = true;
            }
            if parsed {
                self.history_state.time_unit = TimeUnit::Day;
                if value.contains(':') {
                    self.history_state.time_precision = true;
                }
                for (i, value) in v.into_iter().enumerate() {
                    self.set_time_part(i, value, window, cx);
                }
                self.locate_time_calendar(window, cx);
            }
        }
        self.history_state.error.clear();
        cx.notify();
    }
    pub(super) fn step_time_part(
        &mut self,
        index: usize,
        step: i32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut values = self.time_part_strings(cx);
        let offset = index / 5 * 5;
        let part = index % 5;
        if part >= 3 {
            let max = if part == 3 { 24 } else { 60 };
            let Ok(n) = values[index].parse::<i32>() else {
                return;
            };
            values[index] = format!("{:02}", (n + step).rem_euclid(max));
        } else {
            let Some((a, b)) = self.history_time_selection(cx) else {
                return;
            };
            let d = if offset == 0 { a } else { b };
            let next = if part == 2 {
                d.checked_add_signed(chrono::Duration::days(i64::from(step)))
            } else {
                let count = step.unsigned_abs() * if part == 0 { 12 } else { 1 };
                if step < 0 {
                    d.checked_sub_months(chrono::Months::new(count))
                } else {
                    d.checked_add_months(chrono::Months::new(count))
                }
            };
            let Some(next) =
                next.filter(|d| d.year() >= 1 && *d <= chrono::Local::now().date_naive())
            else {
                return;
            };
            values[offset] = next.year().to_string();
            values[offset + 1] = format!("{:02}", next.month());
            values[offset + 2] = format!("{:02}", next.day());
        }
        for (i, value) in values.into_iter().enumerate() {
            self.set_time_part(i, value, window, cx);
        }
        self.history_state.error.clear();
        if self.history_state.time_parts[index]
            .focus_handle(cx)
            .is_focused(window)
        {
            window.dispatch_action(Box::new(gpui_component::input::SelectAll), cx);
        }
        cx.notify();
    }
    pub(super) fn scroll_time_heading(
        &mut self,
        side: usize,
        position: Point<Pixels>,
        delta: ScrollDelta,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let field = (side * 2..side * 2 + 2).find(|&i| {
            (i.is_multiple_of(2) || self.history_state.time_unit == TimeUnit::Day)
                && self.history_state.time_browse_bounds[i]
                    .get()
                    .contains(&position)
        });
        let target = field.map_or(TimeWheelTarget::Calendar(side), TimeWheelTarget::Browse);
        let step = self.time_wheel_step(target, delta);
        if step != 0 {
            let months = match field {
                Some(i) if i.is_multiple_of(2) && self.history_state.time_unit == TimeUnit::Day => {
                    step * 12
                }
                Some(_) => step,
                None => -step,
            };
            self.shift_time_calendar(side, months, window, cx);
        }
    }
    pub(super) fn time_wheel_step(&mut self, target: TimeWheelTarget, delta: ScrollDelta) -> i32 {
        let now = Instant::now();
        let h = &mut self.history_state;
        let amount = f32::from(delta.pixel_delta(px(20.)).y);
        if h.time_wheel_target != Some(target)
            || h.time_wheel_at
                .is_none_or(|t| now.duration_since(t) > Duration::from_millis(240))
            || h.time_wheel != 0. && h.time_wheel.signum() != amount.signum()
        {
            h.time_wheel = 0.;
        }
        h.time_wheel_target = Some(target);
        h.time_wheel_at = Some(now);
        h.time_wheel += amount;
        if h.time_wheel.abs() < 32. {
            return 0;
        }
        let step = if h.time_wheel > 0. { 1 } else { -1 };
        h.time_wheel = 0.;
        step
    }
    pub(super) fn begin_time_drag(&mut self, side: usize, date: NaiveDate) {
        self.history_state.time_side = side;
        self.history_state.time_cursors[side] = date;
        self.history_state.time_drag = Some((date, date));
        self.history_state.time_dragged = false;
    }
    pub(super) fn time_drag_preview(&self) -> Option<(NaiveDate, NaiveDate)> {
        if !self.history_state.time_dragged {
            return None;
        }
        let (a, b) = self.history_state.time_drag?;
        let today = chrono::Local::now().date_naive();
        let unit = self.history_state.time_unit;
        Some((unit.span(a.min(b), today)?.0, unit.span(a.max(b), today)?.1))
    }
    pub(super) fn move_time_drag(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.history_state.time_open || !self.history_state.time_expanded {
            self.history_state.time_drag = None;
            return;
        }
        let Some((anchor, over)) = self.history_state.time_drag else {
            return;
        };
        let date = self
            .history_state
            .time_bounds
            .borrow()
            .iter()
            .find(|(_, b)| b.contains(&position))
            .map(|((side, date), _)| (*side, *date));
        if let Some((side, date)) = date.filter(|(side, d)| {
            (*d != over || *side != self.history_state.time_side)
                && *d <= chrono::Local::now().date_naive()
        }) {
            self.history_state.time_side = side;
            self.history_state.time_grids[side].focus(window);
            self.history_state.time_drag = Some((anchor, date));
            self.history_state.time_dragged = true;
            self.history_state.time_cursors[self.history_state.time_side] = date;
            cx.notify();
        }
    }
    pub(super) fn finish_time_drag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let preview = self.time_drag_preview();
        self.history_state.time_drag = None;
        if let Some((a, b)) = preview {
            self.set_time_dates(a, b, window, cx);
            self.history_state.time_anchor = None;
            self.history_state.time_pick_target = None;
            cx.defer_in(window, |app, _, _| {
                app.history_state.time_dragged = false;
            });
            cx.notify();
        }
    }
    pub(super) fn range_editor_keystroke(
        &mut self,
        event: &KeystrokeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.history_state.time_open
            || !matches!(self.page, Page::Apps | Page::History)
            || !self.history_state.time_focus.contains_focused(window, cx)
        {
            return false;
        }
        let key = event.keystroke.key.as_str();
        if key == "escape" {
            if self.history_state.time_expanded {
                self.history_state.time_expanded = false;
                self.history_state.time_drag = None;
                self.history_state.time_dragged = false;
                self.history_state.time_buttons[0].focus(window);
            } else {
                self.reset_time_range(window, cx);
            }
            cx.notify();
            return true;
        }
        let part = self
            .history_state
            .time_parts
            .iter()
            .position(|s| s.focus_handle(cx).is_focused(window));
        if let Some(index) = part
            && matches!(key, "up" | "down")
        {
            self.step_time_part(index, if key == "up" { 1 } else { -1 }, window, cx);
            window.prevent_default();
            return true;
        }
        let browse = std::iter::once(&self.history_state.year_input)
            .chain(self.history_state.time_browse.iter())
            .position(|s| s.focus_handle(cx).is_focused(window));
        if key == "enter" || (key == "space" && part.is_none() && browse.is_none()) {
            if let Some(index) = browse {
                self.edit_time_browse(index, window, cx);
            } else if let Some(i) = self
                .history_state
                .time_quick_buttons
                .iter()
                .position(|h| h.is_focused(window))
            {
                self.select_time_quick(TimeQuickRange::ALL[i], window, cx);
            } else if let Some(i) = self
                .history_state
                .time_buttons
                .iter()
                .position(|h| h.is_focused(window))
            {
                self.history_calendar_action(
                    match i {
                        0 => CalendarAction::TimeCalendar,
                        1 => CalendarAction::TimeReset,
                        _ => {
                            self.apply_history_dates(cx);
                            cx.notify();
                            return true;
                        }
                    },
                    window,
                    cx,
                );
            } else if self.history_state.time_all_day.is_focused(window) {
                self.set_time_all_day(window, cx);
            } else if let Some((side, dir)) = self
                .history_state
                .time_nav
                .iter()
                .enumerate()
                .find_map(|(side, handles)| {
                    handles
                        .iter()
                        .position(|h| h.is_focused(window))
                        .map(|dir| (side, dir))
                })
            {
                self.shift_time_calendar(side, if dir == 0 { -1 } else { 1 }, window, cx);
            } else if let Some(side) = self
                .history_state
                .time_grids
                .iter()
                .position(|h| h.is_focused(window))
            {
                self.history_state.time_side = side;
                self.pick_history_time(self.history_state.time_cursors[side], window, cx);
            } else if let Some(i) = self
                .history_state
                .time_controls
                .iter()
                .position(|h| h.is_focused(window))
            {
                let action = match i {
                    0 => CalendarAction::TimeUnit(TimeUnit::Day),
                    1 => CalendarAction::TimeUnit(TimeUnit::Month),
                    2 => CalendarAction::TimeUnit(TimeUnit::Year),
                    3 => CalendarAction::TimePrecision,
                    4 => CalendarAction::TimeShift(0, -1),
                    5 => CalendarAction::TimeShift(0, 1),
                    6 => {
                        self.locate_time_calendar(window, cx);
                        cx.notify();
                        return true;
                    }
                    _ => CalendarAction::TimePick(
                        self.history_state.time_cursors[self.history_state.time_side],
                    ),
                };
                self.history_calendar_action(action, window, cx);
            } else {
                self.apply_history_dates(cx);
            }
            window.prevent_default();
            cx.notify();
            return true;
        }
        if key == "tab" {
            self.keyboard_navigation = true;
            if self.history_state.time_focus.is_focused(window) && !event.keystroke.modifiers.shift
            {
                self.history_state.time_parts[0]
                    .focus_handle(cx)
                    .focus(window);
                window.prevent_default();
                cx.notify();
                return true;
            }
            let mut handles = Vec::new();
            for (i, s) in self.history_state.time_parts.iter().enumerate() {
                if self.visible_time_part(i) {
                    handles.push(s.focus_handle(cx));
                }
            }
            handles.push(self.history_state.time_buttons[0].clone());
            if self.time_range_dirty(cx) && self.time_range_result(cx).is_ok() {
                handles.push(self.history_state.time_buttons[2].clone());
            }
            if self.history_state.time_expanded {
                let browse = std::iter::once(&self.history_state.year_input)
                    .chain(self.history_state.time_browse.iter())
                    .collect::<Vec<_>>();
                for side in 0..if self.history_state.time_unit == TimeUnit::Day {
                    2
                } else {
                    1
                } {
                    handles.push(browse[side * 2].focus_handle(cx));
                    if self.history_state.time_unit == TimeUnit::Day {
                        handles.push(browse[side * 2 + 1].focus_handle(cx));
                    }
                    handles.extend(self.history_state.time_nav[side].iter().cloned());
                    handles.push(self.history_state.time_grids[side].clone());
                }
                handles.extend(self.history_state.time_quick_buttons.iter().cloned());
                if self.time_all_day_enabled(cx) {
                    handles.push(self.history_state.time_all_day.clone());
                }
                if self.time_range_dirty(cx) {
                    handles.push(self.history_state.time_buttons[1].clone());
                }
                handles.push(self.history_state.time_controls[6].clone());
            }
            if let Some(index) = handles.iter().position(|h| h.is_focused(window)) {
                let next = if event.keystroke.modifiers.shift {
                    index.checked_sub(1)
                } else {
                    (index + 1 < handles.len()).then_some(index + 1)
                };
                if let Some(next) = next {
                    handles[next].focus(window);
                    window.prevent_default();
                    cx.notify();
                    return true;
                }
            }
        }
        if let Some(side) = self
            .history_state
            .time_grids
            .iter()
            .position(|h| h.is_focused(window))
            .filter(|_| self.history_state.time_expanded)
        {
            self.history_state.time_side = side;
            let unit = self.history_state.time_unit;
            let delta = match key {
                "left" => -1,
                "right" => 1,
                "up" => {
                    if unit == TimeUnit::Day {
                        -7
                    } else {
                        -4
                    }
                }
                "down" => {
                    if unit == TimeUnit::Day {
                        7
                    } else {
                        4
                    }
                }
                _ => return false,
            };
            let current = self.history_state.time_cursors[side];
            let next = if unit == TimeUnit::Day {
                current.checked_add_signed(chrono::Duration::days(delta))
            } else {
                let n = delta.unsigned_abs() as u32 * if unit == TimeUnit::Year { 12 } else { 1 };
                if delta < 0 {
                    current.checked_sub_months(chrono::Months::new(n))
                } else {
                    current.checked_add_months(chrono::Months::new(n))
                }
            };
            if let Some(date) = next.filter(|d| {
                d.year() >= 1
                    && *d <= chrono::Local::now().date_naive()
                    && (unit != TimeUnit::Day
                        || self.can_show_time_month(side, d.with_day(1).unwrap()))
            }) {
                self.history_state.time_cursors[self.history_state.time_side] = date;
                if unit == TimeUnit::Day {
                    let shown = self.history_state.time_months[side];
                    if date.with_day(1).unwrap() != shown {
                        self.history_state.time_months[side] = date.with_day(1).unwrap();
                        self.sync_time_browse(window, cx);
                    }
                } else {
                    self.history_state.time_year = date.year();
                    self.sync_time_browse(window, cx);
                }
                if event.keystroke.modifiers.shift {
                    self.pick_history_time(date, window, cx);
                }
                cx.notify();
            }
            return true;
        }
        false
    }
    pub(super) fn visible_time_part(&self, index: usize) -> bool {
        index < 10
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn quick_ranges_follow_calendar_weeks_months_and_leap_years() {
        let date = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap();
        for (today, quick, from, to) in [
            (
                date(2024, 4, 1),
                TimeQuickRange::Yesterday,
                date(2024, 3, 31),
                date(2024, 3, 31),
            ),
            (
                date(2024, 4, 1),
                TimeQuickRange::ThisWeek,
                date(2024, 4, 1),
                date(2024, 4, 1),
            ),
            (
                date(2024, 4, 1),
                TimeQuickRange::LastWeek,
                date(2024, 3, 25),
                date(2024, 3, 31),
            ),
            (
                date(2024, 3, 1),
                TimeQuickRange::LastMonth,
                date(2024, 2, 1),
                date(2024, 2, 29),
            ),
            (
                date(2023, 3, 1),
                TimeQuickRange::LastMonth,
                date(2023, 2, 1),
                date(2023, 2, 28),
            ),
            (
                date(2025, 1, 1),
                TimeQuickRange::LastMonth,
                date(2024, 12, 1),
                date(2024, 12, 31),
            ),
            (
                date(2025, 1, 1),
                TimeQuickRange::ThisWeek,
                date(2024, 12, 30),
                date(2025, 1, 1),
            ),
            (
                date(2024, 12, 31),
                TimeQuickRange::ThisMonth,
                date(2024, 12, 1),
                date(2024, 12, 31),
            ),
        ] {
            assert_eq!(quick.dates(today), Some((from, to)));
        }
        let monday = date(2024, 4, 1);
        assert_eq!(
            TimeQuickRange::ThisWeek.dates(monday),
            TimeQuickRange::ThisMonth.dates(monday)
        );
    }
    #[::core::prelude::v1::test]
    fn incomplete_and_non_numeric_fields_have_actionable_errors() {
        let parts = parts_for("2024-02-29 09:12", "2024-02-29 18:00").unwrap();
        for index in 0..10 {
            let endpoint = if index < 5 { "开始" } else { "结束" };
            let field = ["年份", "月份", "日期", "小时", "分钟"][index % 5];
            for value in ["", " ", "abc", "99999999999999999999"] {
                let mut values = parts.clone();
                values[index] = value.into();
                let error = parse_parts(&values, TimeUnit::Day, true)
                    .unwrap_err()
                    .to_string();
                assert_eq!(
                    error,
                    if value.trim().is_empty() {
                        format!("请填写{endpoint}{field}")
                    } else {
                        format!("请输入有效的{endpoint}{field}")
                    }
                );
            }
        }
    }
    #[::core::prelude::v1::test]
    fn inclusive_minutes_invalid_dates_and_whole_periods_share_bounds() {
        let v = parts_for("2024-02-29 09:12", "2024-02-29 09:12").unwrap();
        let (a, b) = parse_parts(&v, TimeUnit::Day, true).unwrap();
        assert_eq!(b - a, 60_000);
        let (a, b) = parse_parts(&v, TimeUnit::Month, false).unwrap();
        assert_eq!(
            crate::history::local_date(a),
            NaiveDate::from_ymd_opt(2024, 2, 1).unwrap()
        );
        assert_eq!(
            crate::history::local_date(b - 1),
            NaiveDate::from_ymd_opt(2024, 2, 29).unwrap()
        );
        let mut invalid = v.clone();
        invalid[7] = "30".into();
        assert!(parse_parts(&invalid, TimeUnit::Day, false).is_err());
        let mut reverse = v;
        reverse[9] = "11".into();
        assert!(parse_parts(&reverse, TimeUnit::Day, true).is_err());
    }
    #[::core::prelude::v1::test]
    fn cross_year_ranges_and_future_dates_are_validated() {
        let v = parts_for("2023/11/01 00:00", "2024/02/29 23:59").unwrap();
        let (a, b) = parse_parts(&v, TimeUnit::Month, false).unwrap();
        assert_eq!(
            crate::history::local_date(a),
            NaiveDate::from_ymd_opt(2023, 11, 1).unwrap()
        );
        assert_eq!(
            crate::history::local_date(b - 1),
            NaiveDate::from_ymd_opt(2024, 2, 29).unwrap()
        );
        let future = parts_for("9999-01-01 00:00", "9999-01-01 23:59").unwrap();
        assert!(parse_parts(&future, TimeUnit::Day, false).is_err());
    }
}
