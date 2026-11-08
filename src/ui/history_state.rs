use super::*;
use crate::history::{CalendarMonth, CalendarRequest, Grain};
use chrono::{Datelike, NaiveDate};
use std::{collections::VecDeque, sync::Arc};

pub(super) const GAP_PAGE_SIZE: usize = 8;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum HistoryTab {
    #[default]
    Records,
    Analysis,
}

#[derive(Default)]
pub(super) struct NavigationReports(Vec<(u64, Report)>);

impl NavigationReports {
    pub(super) fn remember(
        &mut self,
        report: &Report,
        query: &Query,
        ancestors: &[(i64, i64, TimePeriod)],
        revision: u64,
    ) {
        self.0.retain(|(source, report)| {
            *source == revision
                && report.query.as_ref().is_some_and(|stored| {
                    stored != query
                        && ancestors
                            .iter()
                            .any(|&(from, to, _)| (stored.from, stored.to) == (from, to))
                })
        });
        if report.query.as_ref() == Some(query) && report.error.is_empty() {
            self.0.push((revision, report.clone()));
        }
    }

    fn get(&self, query: &Query, revision: u64) -> Option<Report> {
        self.0
            .iter()
            .rev()
            .find(|(source, report)| *source == revision && report.query.as_ref() == Some(query))
            .map(|(_, report)| report.clone())
    }

    fn clear(&mut self) {
        self.0.clear();
    }
}

#[cfg(test)]
mod navigation_tests {
    use super::{AnnualScope, CalendarDrag, NavigationReports, TimeUnit};
    use crate::{
        date_range::TimePeriod,
        model::{Bytes, Query, Report},
    };
    use chrono::NaiveDate;
    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }
    #[test]
    fn navigation_reports_restore_the_entire_parent_and_require_exact_query_and_source() {
        let query = Query {
            history: true,
            from: 0,
            to: 7,
            ..Default::default()
        };
        let report = Report {
            query: Some(query.clone()),
            total: Bytes {
                down: 1234,
                up: 567,
            },
            rows: vec![crate::model::Row {
                key: "parent.exe".into(),
                bytes: Bytes::default(),
            }],
            ..Default::default()
        };
        let mut cache = NavigationReports::default();
        cache.remember(&report, &query, &[], 1);
        let restored = cache.get(&query, 1).unwrap();
        assert_eq!(restored.total, report.total);
        assert_eq!(restored.rows[0].key, "parent.exe");
        assert!(cache.get(&query, 2).is_none());
        for changed in [
            Query {
                search: "different".into(),
                ..query.clone()
            },
            Query {
                dimension: "domain".into(),
                ..query.clone()
            },
            Query {
                page: 1,
                ..query.clone()
            },
            Query {
                ascending: true,
                ..query.clone()
            },
            Query {
                to: 8,
                ..query.clone()
            },
        ] {
            assert!(cache.get(&changed, 1).is_none());
        }
        cache.clear();
        assert!(cache.get(&query, 1).is_none());
    }
    #[test]
    fn navigation_reports_keep_only_current_ancestors_and_never_cache_failures() {
        let query = Query {
            history: true,
            from: 0,
            to: 100,
            ..Default::default()
        };
        let report = Report {
            query: Some(query.clone()),
            ..Default::default()
        };
        let mut cache = NavigationReports::default();
        cache.remember(&report, &query, &[], 1);
        let ancestors = [(0, 100, TimePeriod::Custom)];
        for from in 1..50 {
            let child_query = Query {
                from,
                to: from + 1,
                ..query.clone()
            };
            let child = Report {
                query: Some(child_query.clone()),
                ..Default::default()
            };
            cache.remember(&child, &child_query, &ancestors, 1);
            assert_eq!(cache.0.len(), 2);
            assert!(cache.get(&query, 1).is_some());
        }
        let failed = Report {
            error: "failed".into(),
            ..report.clone()
        };
        cache.remember(&failed, &query, &[], 2);
        assert!(cache.0.is_empty());
        cache.remember(&report, &Query { from: 1, ..query }, &[], 2);
        assert!(cache.0.is_empty());
    }
    #[test]
    fn annual_scopes_cover_calendar_months_and_leap_years() {
        assert_eq!(
            AnnualScope::Recent.bounds(date(2024, 2, 29)),
            (date(2023, 3, 1), date(2024, 2, 29))
        );
        assert_eq!(
            AnnualScope::Recent.bounds(date(2026, 1, 7)),
            (date(2025, 2, 1), date(2026, 1, 31))
        );
        let (a, b) = AnnualScope::Year(2024).bounds(date(2026, 10, 29));
        assert_eq!((b - a).num_days() + 1, 366);
    }
    #[test]
    fn whole_period_selection_clips_only_unfinished_periods() {
        let today = date(2026, 10, 29);
        assert_eq!(
            TimeUnit::Month.span(date(2024, 2, 20), today),
            Some((date(2024, 2, 1), date(2024, 2, 29)))
        );
        assert_eq!(
            TimeUnit::Year.span(date(2026, 1, 1), today),
            Some((date(2026, 1, 1), today))
        );
        assert_eq!(TimeUnit::Month.span(date(2026, 11, 1), today), None);
        assert_eq!(TimeUnit::Day.span(today, today), Some((today, today)));
    }
    #[test]
    fn month_drag_expands_both_endpoints_and_clips_the_current_month() {
        let drag = CalendarDrag {
            anchor: date(2024, 3, 1),
            over: date(2024, 2, 1),
            months: true,
        };
        assert_eq!(
            drag.range(date(2024, 4, 15)),
            (date(2024, 2, 1), date(2024, 3, 31))
        );
        assert_eq!(
            CalendarDrag {
                anchor: date(2023, 12, 1),
                over: date(2024, 2, 1),
                months: true
            }
            .range(date(2024, 2, 20)),
            (date(2023, 12, 1), date(2024, 2, 20))
        );
        assert_eq!(
            CalendarDrag {
                months: false,
                ..drag
            }
            .range(date(2024, 4, 15)),
            (date(2024, 2, 1), date(2024, 3, 1))
        );
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum CalendarView {
    #[default]
    Month,
    Year,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum AnnualScope {
    #[default]
    Recent,
    Year(i32),
}
impl AnnualScope {
    pub fn bounds(self, today: NaiveDate) -> (NaiveDate, NaiveDate) {
        match self {
            Self::Recent => {
                let end = today
                    .with_day(1)
                    .unwrap()
                    .checked_add_months(chrono::Months::new(1))
                    .unwrap();
                (
                    end.checked_sub_months(chrono::Months::new(12)).unwrap(),
                    end.pred_opt().unwrap(),
                )
            }
            Self::Year(year) => (
                NaiveDate::from_ymd_opt(year, 1, 1).unwrap(),
                NaiveDate::from_ymd_opt(year, 12, 31).unwrap(),
            ),
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum TimeUnit {
    #[default]
    Day,
    Month,
    Year,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TimeWheelTarget {
    Part(usize),
    DrillPart(usize),
    Browse(usize),
    Calendar(usize),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TimeQuickRange {
    Today,
    Yesterday,
    ThisWeek,
    LastWeek,
    ThisMonth,
    LastMonth,
}
impl TimeQuickRange {
    pub const ALL: [Self; 6] = [
        Self::Today,
        Self::Yesterday,
        Self::ThisWeek,
        Self::LastWeek,
        Self::ThisMonth,
        Self::LastMonth,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Today => "今天",
            Self::Yesterday => "昨天",
            Self::ThisWeek => "本周",
            Self::LastWeek => "上周",
            Self::ThisMonth => "本月",
            Self::LastMonth => "上月",
        }
    }
    pub fn dates(self, today: NaiveDate) -> Option<(NaiveDate, NaiveDate)> {
        let monday = today.checked_sub_signed(chrono::Duration::days(i64::from(
            today.weekday().num_days_from_monday(),
        )))?;
        let month = today.with_day(1)?;
        Some(match self {
            Self::Today => (today, today),
            Self::Yesterday => (today.pred_opt()?, today.pred_opt()?),
            Self::ThisWeek => (monday, today),
            Self::LastWeek => (
                monday.checked_sub_signed(chrono::Duration::days(7))?,
                monday.pred_opt()?,
            ),
            Self::ThisMonth => (month, today),
            Self::LastMonth => (
                month.checked_sub_months(chrono::Months::new(1))?,
                month.pred_opt()?,
            ),
        })
    }
}
impl TimeUnit {
    pub fn span(self, date: NaiveDate, today: NaiveDate) -> Option<(NaiveDate, NaiveDate)> {
        let (from, to) = match self {
            Self::Day => (date, date),
            Self::Month => {
                let from = date.with_day(1)?;
                (
                    from,
                    from.checked_add_months(chrono::Months::new(1))?
                        .pred_opt()?,
                )
            }
            Self::Year => (
                NaiveDate::from_ymd_opt(date.year(), 1, 1)?,
                NaiveDate::from_ymd_opt(date.year(), 12, 31)?,
            ),
        };
        (from <= today).then_some((from, to.min(today)))
    }
}
#[derive(Clone)]
pub(super) enum CalendarAction {
    Help(bool),
    View(CalendarView),
    Annual(AnnualScope),
    YearGrid,
    EditYear,
    YearStep(i32),
    TimeUnit(TimeUnit),
    TimePrecision,
    TimeShift(usize, i32),
    TimePick(NaiveDate),
    TimeCalendar,
    TimeReset,
    TimeBrowse(usize),
    TimeQuick(TimeQuickRange),
    TimeAllDay,
    JumpYear,
    Shift(i32),
    ToggleMonth,
    Year(i32),
    Month(u32),
    Week(NaiveDate),
    Locate,
    OpenTime,
    Back,
}
#[derive(Clone, Copy)]
pub(super) struct CalendarDrag {
    pub anchor: NaiveDate,
    pub over: NaiveDate,
    pub months: bool,
}
impl CalendarDrag {
    fn range(self, today: NaiveDate) -> (NaiveDate, NaiveDate) {
        let from = self.anchor.min(self.over);
        let to = self.anchor.max(self.over);
        if self.months {
            (
                from.with_day(1).unwrap(),
                to.with_day(1)
                    .unwrap()
                    .checked_add_months(chrono::Months::new(1))
                    .unwrap()
                    .pred_opt()
                    .unwrap()
                    .min(today),
            )
        } else {
            (from, to)
        }
    }
}
type TimeCellBounds = std::collections::BTreeMap<(usize, NaiveDate), Bounds<Pixels>>;

pub(super) struct HistoryState {
    pub view: CalendarView,
    pub annual_scope: AnnualScope,
    pub time_unit: TimeUnit,
    pub time_precision: bool,
    pub time_months: [NaiveDate; 2],
    pub time_year: i32,
    pub time_cursors: [NaiveDate; 2],
    pub time_side: usize,
    pub time_nav: [[FocusHandle; 2]; 2],
    pub time_grids: [FocusHandle; 2],
    pub time_all_day: FocusHandle,
    pub time_anchor: Option<(NaiveDate, NaiveDate)>,
    pub year_input: Entity<InputState>,
    pub time_controls: [FocusHandle; 8],
    pub tab: HistoryTab,
    pub record_period: TimePeriod,
    pub record_query: Query,
    pub analysis_period: Option<TimePeriod>,
    pub analysis_range: (i64, i64),
    pub dates: [[Entity<InputState>; 2]; 2],
    pub time_parts: [Entity<InputState>; 10],
    pub time_browse: [Entity<InputState>; 3],
    pub time_values: [String; 10],
    pub time_applied: (i64, i64),
    pub time_apps: bool,
    pub time_initialized: bool,
    pub time_expanded: bool,
    pub time_pick_target: Option<usize>,
    pub time_drag: Option<(NaiveDate, NaiveDate)>,
    pub time_dragged: bool,
    pub time_bounds: Rc<RefCell<TimeCellBounds>>,
    pub time_panel_bounds: [Rc<Cell<Bounds<Pixels>>>; 3],
    pub time_editor_bounds: Rc<Cell<Bounds<Pixels>>>,
    pub time_action_bounds: Rc<Cell<Bounds<Pixels>>>,
    pub time_part_bounds: [Rc<Cell<Bounds<Pixels>>>; 10],
    pub time_browse_bounds: [Rc<Cell<Bounds<Pixels>>>; 4],
    pub time_calendar_bounds: Rc<Cell<Bounds<Pixels>>>,
    pub time_wheel: f32,
    pub time_wheel_at: Option<Instant>,
    pub time_wheel_target: Option<TimeWheelTarget>,
    pub time_open: bool,
    pub time_target: HistoryTab,
    pub time_focus: FocusHandle,
    pub time_buttons: [FocusHandle; 3],
    pub time_quick: Option<TimeQuickRange>,
    pub time_quick_buttons: [FocusHandle; 6],
    pub time_quick_bounds: [Rc<Cell<Bounds<Pixels>>>; 6],
    pub offsets: [Point<Pixels>; 2],
    pub restore_offset: Option<Point<Pixels>>,
    pub outside_range: Option<(i64, i64)>,
    pub quality_open: bool,
    pub gaps_open: bool,
    pub gaps_page: Option<(i64, i64, usize)>,
    pub error: String,
    pub month: NaiveDate,
    pub anchor: NaiveDate,
    pub cursor: NaiveDate,
    pub drag: Option<CalendarDrag>,
    pub help_open: bool,
    pub help_trigger: Rc<Cell<Bounds<Pixels>>>,
    pub year_trigger: Rc<Cell<Bounds<Pixels>>>,
    pub year_grid: bool,
    pub year_edit: bool,
    pub year_wheel: f32,
    pub year_wheel_at: Option<Instant>,
    pub annual_hover: Option<NaiveDate>,
    pub chart_hover: Option<history_chart_hover::TrendHover>,
    pub chart_hover_sequence: u64,
    pub chart_motion: Rc<RefCell<history_chart_motion::ChartMotion>>,
    pub chart_hover_metrics: RefCell<Option<history_chart_hover::HoverMetrics>>,
    pub chart_plot_bounds: Rc<Cell<Bounds<Pixels>>>,
    pub chart_hover_bounds: Rc<Cell<Bounds<Pixels>>>,
    pub chart_back_bounds: Rc<Cell<Bounds<Pixels>>>,
    pub month_wheel: f32,
    pub month_wheel_at: Option<Instant>,
    pub month_bounds: Rc<RefCell<std::collections::BTreeMap<NaiveDate, Bounds<Pixels>>>>,
    pub calendar_focus: FocusHandle,
    pub month_open: bool,
    pub picker_year: i32,
    pub day_bounds: Rc<RefCell<std::collections::BTreeMap<NaiveDate, Bounds<Pixels>>>>,
    pub cache: VecDeque<Arc<CalendarMonth>>,
    pub month_pending: Option<CalendarRequest>,
    pub month_sequence: u64,
    pub month_error: String,
    pub month_retry: Option<Instant>,
    pub source_revision: u64,
    pub range_stack: Vec<(i64, i64, TimePeriod)>,
    pub seconds_level: bool,
    pub navigation_reports: NavigationReports,
    pub calendar_bounds: Rc<Cell<Bounds<Pixels>>>,
    pub hero_bounds: Rc<Cell<Bounds<Pixels>>>,
}
pub(super) fn stamp(time: i64) -> String {
    chrono::DateTime::from_timestamp_millis(time)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}
impl HistoryState {
    pub fn new(window: &mut Window, cx: &mut Context<TrafficApp>) -> Self {
        let query = Query {
            page_size: 10,
            history: true,
            ..Default::default()
        };
        let today = chrono::Local::now().date_naive();
        let dates = std::array::from_fn(|_| {
            [
                input(&stamp(query.from), "YYYY-MM-DD HH:mm", false, window, cx),
                input(
                    &stamp(query.to - 60_000),
                    "YYYY-MM-DD HH:mm",
                    false,
                    window,
                    cx,
                ),
            ]
        });
        Self {
            view: CalendarView::Month,
            annual_scope: AnnualScope::Recent,
            time_unit: TimeUnit::Day,
            time_precision: false,
            time_months: [today.with_day(1).unwrap(); 2],
            time_year: today.year(),
            time_cursors: [today; 2],
            time_side: 0,
            time_nav: std::array::from_fn(|_| std::array::from_fn(|_| cx.focus_handle())),
            time_grids: std::array::from_fn(|_| cx.focus_handle()),
            time_all_day: cx.focus_handle(),
            time_anchor: None,
            year_input: input(&today.year().to_string(), "年份", false, window, cx),
            time_controls: std::array::from_fn(|_| cx.focus_handle()),
            tab: HistoryTab::Records,
            record_period: TimePeriod::Today,
            record_query: query.clone(),
            analysis_period: Some(TimePeriod::Today),
            analysis_range: (query.from, query.to),
            dates,
            time_parts: std::array::from_fn(|_| input("", "", false, window, cx)),
            time_browse: std::array::from_fn(|_| input("", "", false, window, cx)),
            time_values: std::array::from_fn(|_| String::new()),
            time_applied: (query.from, query.to),
            time_apps: false,
            time_initialized: false,
            time_expanded: false,
            time_pick_target: None,
            time_drag: None,
            time_dragged: false,
            time_bounds: Rc::new(RefCell::new(Default::default())),
            time_panel_bounds: std::array::from_fn(|_| Rc::new(Cell::new(Bounds::default()))),
            time_editor_bounds: Rc::new(Cell::new(Bounds::default())),
            time_action_bounds: Rc::new(Cell::new(Bounds::default())),
            time_part_bounds: std::array::from_fn(|_| Rc::new(Cell::new(Bounds::default()))),
            time_browse_bounds: std::array::from_fn(|_| Rc::new(Cell::new(Bounds::default()))),
            time_calendar_bounds: Rc::new(Cell::new(Bounds::default())),
            time_wheel: 0.,
            time_wheel_at: None,
            time_wheel_target: None,
            time_open: false,
            time_target: HistoryTab::Records,
            time_focus: cx.focus_handle(),
            time_buttons: std::array::from_fn(|_| cx.focus_handle()),
            time_quick: None,
            time_quick_buttons: std::array::from_fn(|_| cx.focus_handle()),
            time_quick_bounds: std::array::from_fn(|_| Rc::new(Cell::new(Bounds::default()))),
            offsets: [point(px(0.), px(0.)); 2],
            restore_offset: None,
            outside_range: None,
            quality_open: false,
            gaps_open: false,
            gaps_page: None,
            error: String::new(),
            month: today.with_day(1).unwrap(),
            anchor: today,
            cursor: today,
            drag: None,
            help_open: false,
            help_trigger: Rc::new(Cell::new(Bounds::default())),
            year_trigger: Rc::new(Cell::new(Bounds::default())),
            year_grid: false,
            year_edit: false,
            year_wheel: 0.,
            year_wheel_at: None,
            annual_hover: None,
            chart_hover: None,
            chart_hover_sequence: 0,
            chart_motion: Rc::new(RefCell::new(Default::default())),
            chart_hover_metrics: RefCell::new(None),
            chart_plot_bounds: Rc::new(Cell::new(Bounds::default())),
            chart_hover_bounds: Rc::new(Cell::new(Bounds::default())),
            chart_back_bounds: Rc::new(Cell::new(Bounds::default())),
            month_wheel: 0.,
            month_wheel_at: None,
            month_bounds: Rc::new(RefCell::new(Default::default())),
            calendar_focus: cx.focus_handle(),
            month_open: false,
            picker_year: today.year(),
            day_bounds: Rc::new(RefCell::new(Default::default())),
            cache: VecDeque::new(),
            month_pending: None,
            month_sequence: 0,
            month_error: String::new(),
            month_retry: None,
            source_revision: u64::MAX,
            range_stack: vec![],
            seconds_level: false,
            navigation_reports: NavigationReports::default(),
            calendar_bounds: Rc::new(Cell::new(Bounds::default())),
            hero_bounds: Rc::new(Cell::new(Bounds::default())),
        }
    }
    pub fn index(&self) -> usize {
        usize::from(self.tab == HistoryTab::Analysis)
    }
    pub fn gap_page(&self, query: &Query, rows: usize) -> usize {
        self.gaps_page
            .filter(|&(from, to, _)| (from, to) == (query.from, query.to))
            .map_or(0, |(_, _, page)| page)
            .min(rows.saturating_sub(1) / GAP_PAGE_SIZE)
    }
    pub fn selected_dates(&self, query: &Query) -> (NaiveDate, NaiveDate) {
        if let Some(drag) = self.drag {
            drag.range(chrono::Local::now().date_naive())
        } else {
            (
                crate::history::local_date(query.from),
                crate::history::local_date(query.to - 1),
            )
        }
    }
    pub fn visible_month(&self) -> Option<&Arc<CalendarMonth>> {
        self.cached_month(self.month)
    }
    pub fn cached_month(&self, date: NaiveDate) -> Option<&Arc<CalendarMonth>> {
        self.cache.iter().find(|m| {
            m.request.year == date.year()
                && m.request.month == date.month()
                && m.request.generation == self.source_revision
        })
    }
    pub fn calendar_months(&self, today: NaiveDate) -> Vec<NaiveDate> {
        if self.view == CalendarView::Month {
            return vec![self.month];
        }
        let (mut month, end) = self.annual_scope.bounds(today);
        let mut months = Vec::with_capacity(12);
        while month <= end && month <= today {
            months.push(month);
            month = month.checked_add_months(chrono::Months::new(1)).unwrap();
        }
        months
    }
}
#[derive(Clone)]
pub(super) enum HistoryAction {
    Tab(HistoryTab),
    Period(TimePeriod),
    QuickPeriod(TimePeriod),
    ApplyRange,
    Quality,
    Gaps,
    GapPage(i32),
    Sort(TrafficSort),
    Calendar(history_page::CalendarAction),
    Bucket(i64, i64, crate::history::Grain),
    BackTo(usize),
    ToggleDomainGrouping,
}

impl TrafficApp {
    pub(super) fn handle_history_action(
        &mut self,
        action: HistoryAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            HistoryAction::Tab(tab) => self.switch_history_tab(tab, window, cx),
            HistoryAction::Period(period) => self.set_history_period(period, window, cx),
            HistoryAction::QuickPeriod(period) => {
                if self.history_state.tab == HistoryTab::Records {
                    self.set_history_period(period, window, cx);
                }
            }
            HistoryAction::ApplyRange => self.apply_history_dates(cx),
            HistoryAction::Quality => {
                self.history_state.quality_open = !self.history_state.quality_open
            }
            HistoryAction::Gaps => {
                self.history_state.gaps_open = !self.history_state.gaps_open;
                self.history_state.gaps_page = None;
            }
            HistoryAction::GapPage(step) => {
                let rows = self.report.gap_rows.len();
                let page = self
                    .history_state
                    .gap_page(&self.query, rows)
                    .saturating_add_signed(step as isize)
                    .min(rows.saturating_sub(1) / history_state::GAP_PAGE_SIZE);
                self.history_state.gaps_page = Some((self.query.from, self.query.to, page));
            }
            HistoryAction::Sort(sort) => {
                self.query.ascending = if self.query.sort == sort {
                    !self.query.ascending
                } else {
                    false
                };
                self.query.sort = sort;
                self.query.page = 0;
                self.refresh();
            }
            HistoryAction::Calendar(action) => self.history_calendar_action(action, window, cx),
            HistoryAction::Bucket(from, to, grain) => {
                if grain == crate::history::Grain::Minute {
                    self.open_history_seconds(from, window, cx);
                } else {
                    self.history_bucket_selected(from, to, grain);
                }
            }
            HistoryAction::BackTo(index) => self.history_back_to(index),
            HistoryAction::ToggleDomainGrouping => {
                self.query.group_domains = !self.query.group_domains;
                self.query.page = 0;
                self.refresh();
            }
        }
    }

    pub(super) fn schedule_history_restore(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.page != Page::History {
            return;
        }
        let tab = self.history_state.tab;
        let trends = self.analysis.show_trends;
        if tab == HistoryTab::Records && self.report.query.as_ref() != Some(&self.query) {
            return;
        }
        let offset = if tab == HistoryTab::Analysis {
            self.analysis.restore_offset.take()
        } else {
            self.history_state.restore_offset.take()
        };
        let Some(offset) = offset else {
            return;
        };
        let query = self.query.clone();
        let weak = cx.weak_entity();
        window.on_next_frame(move |_, cx| {
            let _ = weak.update(cx, |app, cx| {
                if app.page == Page::History
                    && app.history_state.tab == tab
                    && app.query == query
                    && (tab == HistoryTab::Records || app.analysis.show_trends == trends)
                {
                    app.page_scroll.set_offset(offset);
                    cx.notify();
                }
            });
        });
    }
    pub(super) fn save_history_context(&mut self, cx: &Context<Self>) {
        self.history_state.chart_motion.borrow_mut().clear();
        self.history_state.navigation_reports.clear();
        self.history_state.chart_hover = None;
        if self.history_state.tab == HistoryTab::Records {
            self.history_state.offsets[0] = self.page_scroll.offset();
        } else {
            self.analysis.offsets[usize::from(self.analysis.show_trends)] =
                self.page_scroll.offset();
        }
        self.history_state.record_query = self.query.clone();
        self.history_state.record_query.search = self.search.read(cx).value().to_string();
    }
    pub(super) fn enter_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.query = self.history_state.record_query.clone();
        self.query.history = true;
        self.query.page_size = 10;
        if let Ok(Some((from, to))) = self
            .history_state
            .record_period
            .bounds(chrono::Local::now())
        {
            self.query.from = from;
            self.query.to = to;
        }
        self.last_search = self.query.search.clone();
        self.search.update(cx, |s, cx| {
            s.set_value(self.query.search.clone(), window, cx)
        });
        self.analysis.active = self.history_state.tab == HistoryTab::Analysis;
        let offset = if self.analysis.active {
            let offset = self.analysis.offsets[usize::from(self.analysis.show_trends)];
            self.analysis.restore_offset = Some(offset);
            offset
        } else {
            self.history_state.restore_offset = Some(self.history_state.offsets[0]);
            self.history_state.offsets[0]
        };
        self.page_scroll.set_offset(offset);
        if self.analysis.active
            && (self.analysis.snapshot.is_none() || self.analysis.provisional)
            && self.analysis.pending.is_none()
        {
            self.request_analysis(self.analysis.range);
        }
        self.poll_history_calendar(true, cx);
    }
    pub(super) fn switch_history_tab(
        &mut self,
        tab: HistoryTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if tab == self.history_state.tab {
            return;
        }
        self.history_state.chart_motion.borrow_mut().clear();
        self.history_state.chart_hover = None;
        self.close_usage_detail();
        self.close_picker(window, cx);
        self.history_state.drag = None;
        self.history_state.month_open = false;
        self.history_state.help_open = false;
        self.history_state.annual_hover = None;
        self.history_state.time_open = false;
        self.history_state.time_expanded = false;
        self.history_state.time_initialized = false;
        self.history_state.time_quick = None;
        self.history_state.time_drag = None;
        self.history_state.tab = tab;
        self.analysis.active = tab == HistoryTab::Analysis;
        self.app_usage_anchor_pending = false;
        window.blur();
        if tab == HistoryTab::Analysis {
            self.history_state.offsets[0] = self.page_scroll.offset();
            self.history_state.restore_offset = None;
            self.history_state.analysis_period = Some(self.history_state.record_period);
            self.history_state.analysis_range = (self.query.from, self.query.to);
            self.analysis.show_trends = false;
            self.analysis.offsets = [point(px(0.), px(0.)); 2];
            self.analysis.restore_offset = Some(point(px(0.), px(0.)));
            self.analysis.focus = None;
            self.analysis.highlight = None;
            self.analysis.pinned_bucket = None;
            self.analysis.app_page = 0;
            self.analysis.burst_page = 0;
            self.analysis.sort = analysis_page::Sort::Change;
            self.analysis.ascending = false;
            self.analysis.history_bucket = None;
            self.analysis.search_value.clear();
            self.analysis
                .search
                .update(cx, |s, cx| s.set_value("", window, cx));
            self.page_scroll.set_offset(point(px(0.), px(0.)));
            self.request_analysis(self.analysis.range);
        } else {
            self.analysis.offsets[usize::from(self.analysis.show_trends)] =
                self.page_scroll.offset();
            self.analysis.restore_offset = None;
            self.history_state.restore_offset = Some(self.history_state.offsets[0]);
            self.page_scroll.set_offset(self.history_state.offsets[0]);
        }
        self.analysis.anchor = None;
        cx.notify();
    }
    pub(super) fn set_history_period(
        &mut self,
        period: TimePeriod,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_picker(window, cx);
        self.history_state.month_open = false;
        if period == TimePeriod::Custom {
            self.toggle_time_range(window, cx);
            return;
        }
        if let Ok(Some(range)) = period.bounds(chrono::Local::now()) {
            self.history_state.time_open = false;
            self.history_state.time_expanded = false;
            self.history_state.time_initialized = false;
            self.history_state.time_quick = None;
            if self.history_state.tab == HistoryTab::Analysis {
                self.history_state.analysis_period = Some(period);
                self.history_state.analysis_range = range;
                self.analysis.history_bucket = None;
                self.request_analysis(self.analysis.range);
            } else {
                self.history_state.record_period = period;
                self.set_history_range(range, true);
                let d = crate::history::local_date(
                    range.1.min(chrono::Utc::now().timestamp_millis()) - 1,
                );
                self.history_state.month = d.with_day(1).unwrap();
                self.history_state.anchor = crate::history::local_date(range.0);
                self.history_state.cursor = self.history_state.anchor;
                self.poll_history_calendar(false, cx);
            }
        }
    }
    pub(super) fn set_history_range(&mut self, range: (i64, i64), clear_stack: bool) {
        self.clear_history_seconds();
        if clear_stack {
            self.history_state.chart_motion.borrow_mut().clear();
        }
        self.history_state.chart_hover = None;
        self.close_usage_detail();
        self.query.history = true;
        self.query.history_context = None;
        self.query.from = range.0;
        self.query.to = range.1;
        self.follow_history_selection();
        self.query.page = 0;
        self.query.page_size = 10;
        if clear_stack {
            self.history_state.range_stack.clear();
            self.history_state.navigation_reports.clear();
        } else {
            let revision = self.backend.storage_revision();
            if self.data.storage_revision == revision
                && let Some(report) = self
                    .history_state
                    .navigation_reports
                    .get(&self.query, revision)
            {
                self.report = report;
            }
        }
        self.history_state.error.clear();
        self.app_usage_anchor_pending = false;
        self.history_state.restore_offset = None;
        self.refresh();
    }
    fn follow_history_selection(&mut self) {
        if self.history_state.view != CalendarView::Year {
            return;
        }
        let today = chrono::Local::now().date_naive();
        let selected = crate::history::local_date(
            self.query.to.min(chrono::Utc::now().timestamp_millis()) - 1,
        );
        let (from, to) = self.history_state.annual_scope.bounds(today);
        if selected < from || selected > to {
            self.history_state.annual_scope = AnnualScope::Year(selected.year());
            self.history_state.month_pending = None;
            self.history_state.month_retry = None;
            self.history_state.day_bounds.borrow_mut().clear();
        }
    }
    pub(super) fn commit_history_dates(&mut self, a: NaiveDate, b: NaiveDate, anchor: NaiveDate) {
        let today = chrono::Local::now().date_naive();
        if a > today || b > today {
            return;
        }
        let range = crate::history::day_bounds(a.min(b))
            .and_then(|(from, _)| Ok((from, crate::history::day_bounds(a.max(b))?.1)));
        if let Ok(range) = range {
            if range.1 - range.0 > 3660 * crate::history::DAY {
                self.history_state.error = "时间范围不能超过十年".into();
                return;
            }
            self.history_state.record_period = if a == today && b == today {
                TimePeriod::Today
            } else {
                TimePeriod::Custom
            };
            self.history_state.anchor = anchor;
            self.history_state.cursor = b;
            self.set_history_range(range, true);
        }
    }
    pub(super) fn begin_history_drag(
        &mut self,
        date: NaiveDate,
        shift: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if date > chrono::Local::now().date_naive() {
            return;
        }
        self.history_state.drag = Some(CalendarDrag {
            anchor: if shift {
                self.history_state.anchor
            } else {
                date
            },
            over: date,
            months: false,
        });
        self.history_state.cursor = date;
        self.history_state.calendar_focus.focus(window);
        self.history_state.help_open = false;
        self.history_state.annual_hover = None;
        cx.notify();
    }
    pub(super) fn begin_history_month_drag(
        &mut self,
        month: NaiveDate,
        shift: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.begin_history_drag(month.with_day(1).unwrap(), shift, window, cx);
        if let Some(drag) = self.history_state.drag.as_mut() {
            drag.months = true;
            drag.anchor = drag.anchor.with_day(1).unwrap();
        }
    }
    pub(super) fn move_history_drag(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if self.history_state.drag.is_none() {
            return;
        }
        let months = self.history_state.drag.as_ref().is_some_and(|d| d.months);
        let bounds = if months {
            &self.history_state.month_bounds
        } else {
            &self.history_state.day_bounds
        };
        let scope = self
            .history_state
            .annual_scope
            .bounds(chrono::Local::now().date_naive());
        let target = bounds
            .borrow()
            .iter()
            .find(|(d, b)| {
                ((!months || (**d >= scope.0 && **d <= scope.1))
                    && (self.history_state.view == CalendarView::Year
                        || (d.year() == self.history_state.month.year()
                            && d.month() == self.history_state.month.month())))
                    && b.contains(&position)
            })
            .map(|(d, _)| *d);
        if let Some(date) = target
            && date <= chrono::Local::now().date_naive()
            && self
                .history_state
                .drag
                .as_ref()
                .is_some_and(|d| d.over != date)
        {
            self.history_state.drag.as_mut().unwrap().over = date;
            cx.notify();
        }
    }
    pub(super) fn finish_history_drag(&mut self, cx: &mut Context<Self>) {
        if let Some(drag) = self.history_state.drag.take() {
            if drag.months {
                let today = chrono::Local::now().date_naive();
                let (from, to) = drag.range(today);
                let reverse = drag.over < drag.anchor;
                let anchor = if reverse {
                    TimeUnit::Month.span(drag.anchor, today).unwrap().1
                } else {
                    drag.anchor
                };
                self.commit_history_dates(from, to, anchor);
                self.history_state.cursor = if reverse { from } else { to };
            } else {
                self.commit_history_dates(drag.anchor, drag.over, drag.anchor);
            }
            cx.notify();
        }
    }
    pub(super) fn poll_history_calendar(&mut self, force_current: bool, cx: &mut Context<Self>) {
        let revision = self.backend.storage_revision();
        if revision != self.history_state.source_revision {
            self.history_state.chart_motion.borrow_mut().clear();
            self.history_state.navigation_reports.clear();
            self.history_state.month_retry = None;
            self.history_state.source_revision = revision;
            self.history_state.cache.clear();
            self.history_state.month_pending = None;
        }
        if let Some(reply) = self.backend.calendar_result()
            && self.history_state.month_pending.as_ref() == Some(&reply.request)
        {
            self.history_state.month_pending = None;
            if reply.request.generation == revision {
                match reply.result {
                    Ok(month) => {
                        self.history_state.cache.retain(|old| {
                            old.request.year != month.request.year
                                || old.request.month != month.request.month
                        });
                        self.history_state.cache.push_front(month);
                        while self.history_state.cache.len() > 12 {
                            let needed = self
                                .history_state
                                .calendar_months(chrono::Local::now().date_naive());
                            let index = self
                                .history_state
                                .cache
                                .iter()
                                .rposition(|m| {
                                    !needed.iter().any(|d| {
                                        d.year() == m.request.year && d.month() == m.request.month
                                    })
                                })
                                .unwrap_or(self.history_state.cache.len() - 1);
                            self.history_state.cache.remove(index);
                        }
                        self.history_state.month_error.clear();
                        self.history_state.month_retry = None;
                    }
                    Err(error) => {
                        self.history_state.month_error = error;
                        self.history_state.month_retry =
                            Some(Instant::now() + Duration::from_secs(5));
                    }
                }
                cx.notify();
            }
        }
        if self.page != Page::History
            || self.history_state.month_pending.is_some()
            || self
                .history_state
                .month_retry
                .is_some_and(|until| until > Instant::now())
        {
            return;
        }
        let today = chrono::Local::now().date_naive();
        let now = chrono::Utc::now().timestamp_millis();
        let needed = self.history_state.calendar_months(today);
        let stale = needed
            .iter()
            .copied()
            .find(|date| self.history_state.cached_month(*date).is_none())
            .or_else(|| {
                needed.iter().copied().find(|date| {
                    *date == today.with_day(1).unwrap()
                        && force_current
                        && self
                            .history_state
                            .cached_month(*date)
                            .is_some_and(|m| now - m.request.as_of >= 10_000)
                })
            });
        if let Some(month) = stale {
            self.history_state.month_sequence += 1;
            let request = CalendarRequest {
                id: self.history_state.month_sequence,
                generation: revision,
                year: month.year(),
                month: month.month(),
                as_of: now,
            };
            if self.backend.query_calendar(request.clone()) {
                self.history_state.month_pending = Some(request);
            } else {
                self.history_state.month_error = "月历查询繁忙，稍后重试".into();
                self.history_state.month_retry = Some(Instant::now() + Duration::from_secs(2));
            }
        }
    }
    fn browse_history_month(&mut self, date: NaiveDate, cx: &mut Context<Self>) {
        let date = date.with_day(1).unwrap();
        if date > chrono::Local::now().date_naive().with_day(1).unwrap() {
            return;
        }
        self.history_state.month = date.with_day(1).unwrap();
        if let Some(index) = self
            .history_state
            .cache
            .iter()
            .position(|m| m.request.year == date.year() && m.request.month == date.month())
        {
            let cached = self.history_state.cache.remove(index).unwrap();
            self.history_state.cache.push_front(cached);
        }
        self.history_state.month_open = false;
        self.history_state.day_bounds.borrow_mut().clear();
        self.history_state.month_pending = None;
        self.history_state.month_retry = None;
        self.poll_history_calendar(false, cx);
        cx.notify();
    }
    pub(super) fn history_calendar_action(
        &mut self,
        action: CalendarAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            CalendarAction::Help(open) => {
                self.history_state.help_open = open;
                self.history_state.month_open = false;
            }
            CalendarAction::View(view) => {
                self.history_state.annual_hover = None;
                if view == CalendarView::Year {
                    self.history_state.annual_scope =
                        AnnualScope::Year(self.history_state.month.year());
                }
                self.history_state.help_open = false;
                self.history_state.view = view;
                self.history_state.drag = None;
                self.history_state.month_open = false;
                self.history_state.month_pending = None;
                self.history_state.month_retry = None;
                self.history_state.day_bounds.borrow_mut().clear();
                if view == CalendarView::Month {
                    self.history_state.month = crate::history::local_date(
                        self.query.to.min(chrono::Utc::now().timestamp_millis()) - 1,
                    )
                    .with_day(1)
                    .unwrap();
                }
                self.poll_history_calendar(false, cx);
            }
            CalendarAction::Annual(scope) => {
                self.history_state.annual_hover = None;
                self.history_state.annual_scope = scope;
                self.history_state.month_open = false;
                self.history_state.month_pending = None;
                self.history_state.month_retry = None;
                self.history_state.day_bounds.borrow_mut().clear();
                self.poll_history_calendar(false, cx);
            }
            CalendarAction::YearGrid => {
                self.history_state.year_grid = true;
                self.history_state.year_edit = false;
                window.blur();
            }
            CalendarAction::EditYear => {
                if !self.history_state.month_open {
                    self.history_calendar_action(CalendarAction::ToggleMonth, window, cx);
                }
                self.history_state.year_edit = true;
                self.history_state.year_input.focus_handle(cx).focus(window);
            }
            CalendarAction::YearStep(delta) => {
                let year = if self.history_state.month_open {
                    self.history_state.picker_year
                } else if self.history_state.view == CalendarView::Year {
                    match self.history_state.annual_scope {
                        AnnualScope::Year(year) => year,
                        AnnualScope::Recent => chrono::Local::now().year(),
                    }
                } else {
                    self.history_state.month.year()
                };
                let (open, grid) = (self.history_state.month_open, self.history_state.year_grid);
                self.history_calendar_action(
                    CalendarAction::Year((year + delta).clamp(1, chrono::Local::now().year())),
                    window,
                    cx,
                );
                self.history_state.month_open = open;
                self.history_state.year_grid = grid;
            }
            CalendarAction::TimeUnit(unit) => self.change_time_unit(unit, window, cx),
            CalendarAction::TimePrecision => {
                if self.history_state.time_unit == TimeUnit::Day {
                    self.history_state.time_precision = !self.history_state.time_precision;
                }
            }
            CalendarAction::TimeShift(side, delta) => {
                if self.history_state.time_unit != TimeUnit::Day
                    || self.can_shift_time_calendar(side, delta)
                {
                    self.shift_time_calendar(side, delta, window, cx);
                    self.history_state.time_nav[side][usize::from(delta > 0)].focus(window);
                }
            }
            CalendarAction::TimePick(date) => self.pick_history_time(date, window, cx),
            CalendarAction::TimeCalendar => {
                self.history_state.time_expanded = !self.history_state.time_expanded;
                self.history_state.time_drag = None;
                self.history_state.time_dragged = false;
                self.history_state.time_anchor = None;
                self.history_state.time_pick_target = None;
            }
            CalendarAction::TimeReset => {
                if self.time_range_dirty(cx) {
                    self.reset_time_range(window, cx);
                    self.history_state.time_buttons[1].focus(window);
                }
            }
            CalendarAction::TimeBrowse(index) => self.edit_time_browse(index, window, cx),
            CalendarAction::TimeQuick(range) => self.select_time_quick(range, window, cx),
            CalendarAction::TimeAllDay => self.set_time_all_day(window, cx),
            CalendarAction::JumpYear => {
                if self.history_state.time_open {
                    self.edit_time_browse(0, window, cx);
                } else if let Ok(year) = self
                    .history_state
                    .year_input
                    .read(cx)
                    .value()
                    .parse::<i32>()
                {
                    self.history_calendar_action(CalendarAction::Year(year), window, cx);
                } else {
                    self.history_state.error = "请输入有效年份".into();
                }
            }
            CalendarAction::Shift(delta) => {
                let month = if delta < 0 {
                    self.history_state
                        .month
                        .checked_sub_months(chrono::Months::new(delta.unsigned_abs()))
                } else {
                    self.history_state
                        .month
                        .checked_add_months(chrono::Months::new(delta as u32))
                };
                if let Some(month) = month {
                    self.browse_history_month(month, cx);
                }
            }
            CalendarAction::ToggleMonth => {
                self.history_state.year_grid = self.history_state.view == CalendarView::Year;
                self.history_state.year_edit = false;
                self.history_state.annual_hover = None;
                self.history_state.help_open = false;
                self.history_state.month_open = !self.history_state.month_open;
                self.history_state.picker_year =
                    match (self.history_state.view, self.history_state.annual_scope) {
                        (CalendarView::Year, AnnualScope::Year(year)) => year,
                        (CalendarView::Year, AnnualScope::Recent) => chrono::Local::now().year(),
                        _ => self.history_state.month.year(),
                    };
                self.history_state.year_input.update(cx, |s, cx| {
                    s.set_value(self.history_state.picker_year.to_string(), window, cx)
                });
            }
            CalendarAction::Year(year) => {
                if !(1..=chrono::Local::now().year()).contains(&year) {
                    return;
                }
                let open = self.history_state.month_open;
                self.history_state.picker_year = year;
                self.history_state
                    .year_input
                    .update(cx, |s, cx| s.set_value(year.to_string(), window, cx));
                let today = chrono::Local::now().date_naive();
                let month = NaiveDate::from_ymd_opt(year, self.history_state.month.month(), 1)
                    .unwrap()
                    .min(today.with_day(1).unwrap());
                self.history_state.annual_hover = None;
                self.history_state.year_edit = false;
                self.history_state.year_grid = false;
                if self.history_state.view == CalendarView::Year {
                    self.history_state.month = month;
                    self.history_calendar_action(
                        CalendarAction::Annual(AnnualScope::Year(year)),
                        window,
                        cx,
                    );
                } else {
                    self.browse_history_month(month, cx);
                    self.history_state.month_open = open;
                }
            }
            CalendarAction::Month(month) => {
                if let Some(date) =
                    NaiveDate::from_ymd_opt(self.history_state.picker_year, month, 1)
                {
                    self.browse_history_month(date, cx);
                }
            }
            CalendarAction::Week(from) => {
                let to = from
                    .checked_add_days(chrono::Days::new(6))
                    .unwrap()
                    .min(chrono::Local::now().date_naive());
                self.commit_history_dates(from, to, from);
            }
            CalendarAction::Locate => {
                let date = crate::history::local_date(
                    self.query.to.min(chrono::Utc::now().timestamp_millis()) - 1,
                );
                if self.history_state.view == CalendarView::Year {
                    self.history_state.annual_scope = AnnualScope::Year(date.year());
                    self.history_state.month_pending = None;
                    self.history_state.month_retry = None;
                    self.poll_history_calendar(false, cx);
                } else {
                    self.browse_history_month(date, cx);
                }
            }
            CalendarAction::OpenTime => self.open_history_time(window, cx),
            CalendarAction::Back => {
                if let Some((from, to, period)) = self.history_state.range_stack.pop() {
                    self.begin_history_chart_motion((from, to));
                    self.history_state.record_period = period;
                    self.set_history_range((from, to), false);
                }
            }
        }
        cx.notify();
    }
    pub(super) fn history_bucket_selected(&mut self, from: i64, to: i64, grain: Grain) {
        self.clear_history_seconds();
        self.history_state.chart_hover = None;
        if grain == Grain::Minute || (from, to) == (self.query.from, self.query.to) {
            return;
        }
        let revision = self.backend.storage_revision();
        if self.data.storage_revision == revision {
            self.history_state.navigation_reports.remember(
                &self.report,
                &self.query,
                &self.history_state.range_stack,
                revision,
            );
        }
        self.begin_history_chart_motion((from, to));
        self.history_state.range_stack.push((
            self.query.from,
            self.query.to,
            self.history_state.record_period,
        ));
        self.history_state.record_period = TimePeriod::Custom;
        self.close_usage_detail();
        self.query.from = from;
        self.query.to = to;
        self.query.page = 0;
        self.query.history = true;
        self.query.history_context = None;
        self.history_state.error.clear();
        self.history_state.restore_offset = None;
        self.app_usage_anchor_pending = false;
        self.refresh();
        self.history_state.anchor = crate::history::local_date(from);
        self.history_state.cursor = self.history_state.anchor;
        self.history_state.month = self.history_state.anchor.with_day(1).unwrap();
        self.follow_history_selection();
    }
    pub(super) fn history_back_to(&mut self, index: usize) {
        if let Some(&(from, to, period)) = self.history_state.range_stack.get(index) {
            self.begin_history_chart_motion((from, to));
            self.history_state.range_stack.truncate(index);
            self.history_state.record_period = period;
            self.set_history_range((from, to), false);
        }
    }
    pub(super) fn history_keystroke(
        &mut self,
        event: &KeystrokeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.page != Page::History {
            return false;
        }
        let key = event.keystroke.key.as_str();
        if key == "escape" {
            if self.history_state.year_edit && self.history_state.month_open {
                self.history_state.year_edit = false;
                self.history_state.error.clear();
                self.history_state.year_input.update(cx, |s, cx| {
                    s.set_value(self.history_state.picker_year.to_string(), window, cx)
                });
                window.blur();
            } else if self.history_state.help_open {
                self.history_state.help_open = false;
            } else if self.history_state.drag.take().is_some() {
            } else if self.history_state.month_open {
                self.history_state.month_open = false;
                self.history_state.error.clear();
            } else if self.picker.is_some() {
                return false;
            } else if self.history_state.tab == HistoryTab::Analysis {
                if self.analysis.pinned_bucket.take().is_some() {
                    self.analysis.highlight = None;
                } else {
                    self.switch_history_tab(HistoryTab::Records, window, cx);
                }
            } else if self.seconds.inline_history && self.seconds.locate_open {
                self.seconds.locate_open = false;
                window.blur();
            } else if self.seconds.inline_history && self.seconds.hover_help_open {
                self.seconds.hover_help_open = false;
            } else if self.history_state.seconds_level {
                if let Some(index) = self.history_state.range_stack.len().checked_sub(1) {
                    self.history_back_to(index);
                }
            } else if self.history_state.chart_hover.is_some_and(|h| h.visible) {
                self.close_history_chart_hover(cx);
            } else {
                return false;
            }
            cx.notify();
            return true;
        }
        if self.history_state.month_open
            && key == "enter"
            && self
                .history_state
                .year_input
                .focus_handle(cx)
                .is_focused(window)
        {
            self.history_calendar_action(CalendarAction::JumpYear, window, cx);
            return true;
        }
        if self.history_state.tab == HistoryTab::Analysis
            || !self.history_state.calendar_focus.is_focused(window)
        {
            return false;
        }
        let delta = match key {
            "left" => Some(-1),
            "right" => Some(1),
            "up" => Some(-7),
            "down" => Some(7),
            _ => None,
        };
        if let Some(delta) = delta {
            self.keyboard_navigation = true;
            if let Some(date) = self
                .history_state
                .cursor
                .checked_add_signed(chrono::Duration::days(delta))
                && date <= chrono::Local::now().date_naive()
            {
                self.history_state.cursor = date;
                if event.keystroke.modifiers.shift {
                    self.commit_history_dates(
                        self.history_state.anchor,
                        date,
                        self.history_state.anchor,
                    );
                }
                if self.history_state.view == CalendarView::Month {
                    self.browse_history_month(date, cx);
                } else {
                    let (from, to) = self
                        .history_state
                        .annual_scope
                        .bounds(chrono::Local::now().date_naive());
                    if date < from || date > to {
                        self.history_state.annual_scope = AnnualScope::Year(date.year());
                        self.history_state.month_pending = None;
                        self.history_state.day_bounds.borrow_mut().clear();
                        self.poll_history_calendar(false, cx);
                    }
                }
            }
            cx.notify();
            return true;
        }
        if matches!(key, "enter" | "space") {
            let date = self.history_state.cursor;
            self.commit_history_dates(
                if event.keystroke.modifiers.shift {
                    self.history_state.anchor
                } else {
                    date
                },
                date,
                if event.keystroke.modifiers.shift {
                    self.history_state.anchor
                } else {
                    date
                },
            );
            cx.notify();
            return true;
        }
        false
    }
}
