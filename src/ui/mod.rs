use crate::{
    app_activity::ActivityOrder,
    backend::{Backend, Command},
    config::{
        Config, ProcessMode, RecentUsageRefreshMode, RetentionSizeUnit, RetentionTimeUnit,
        TaskbarPositionMode, TrayIconStyle,
    },
    date_range::TimePeriod,
    model::*,
    native::{self, UiEvent},
    taskbar_content::{
        ByteUnit, DropIntent, Metric, MoveDirection, TaskbarContent, TaskbarFontWeight,
        TaskbarItem, ValueFormat,
    },
    theme::{ColorTheme, Palette},
    wallpaper::WallpaperWatcher,
};
use gpui::{prelude::*, *};
use gpui_component::{
    Disableable, Icon, Selectable, Theme,
    button::{Button, ButtonCustomVariant, ButtonVariants},
    checkbox::Checkbox,
    input::{InputEvent, InputState, TextInput},
    popover::{Popover, PopoverContent},
    slider::{Slider, SliderEvent, SliderState},
    tooltip::Tooltip,
};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    time::{Duration, Instant},
};

mod activity_page;
mod analysis_page;
mod annual_calendar;
mod assets;
mod calendar;
mod calendar_controls;
mod calendar_outline;
mod calendar_year;
mod drill;
mod drill_time;
mod history_chart;
mod history_chart_hover;
mod history_chart_motion;
mod history_page;
mod history_range;
mod history_state;
mod history_time;
mod time_editor_state;
use history_page::HistoryTab;
mod app_histogram;
mod apps_page;
#[cfg(test)]
mod apps_tests;
mod backup_about;
mod bridge;
pub(crate) use bridge::UiBackend;
#[cfg(test)]
mod backup_about_tests;
mod chart;
mod connection;
#[cfg(test)]
mod connection_tests;
mod connections_page;
#[cfg(test)]
mod connections_page_tests;
mod controls;
mod general;
mod overview_hover;
mod overview_page;
#[cfg(test)]
mod overview_tests;
mod scroll_fade;
mod seconds_page;
mod storage_management_page;
mod storage_settings;
#[cfg(test)]
mod storage_tests;
mod thin_scrollbar;
mod updates;
mod usage_detail;
use thin_scrollbar::{Scrollbar, ScrollbarState};
mod taskbar_appearance;
mod taskbar_drag;
use taskbar_drag::{TaskbarDrag, TaskbarDragSession};
mod taskbar_editor;
mod taskbar_position;
use taskbar_appearance::TaskbarColorTarget;
mod taskbar_spacing;
use controls::{PickerState, control_border, control_surface, popup_surface};
mod display_editor;
use display_editor::DisplayEditor;
use floating::Floating;
mod desktop_host;
mod floating;
mod navigation;
mod window;
use analysis_page::AnalysisAction;
use apps_page::ApplicationAction;
use backup_about::BackupAction;
use connection::ConnectionAction;
use connections_page::ConnectionsAction;
pub use desktop_host::{run_desktop, run_interface_process};
use display_editor::DisplayAction;
use history_state::HistoryAction;
use navigation::{DisplayTab, Page, SettingsSection};
use overview_page::OverviewAction;
use seconds_page::DrillAction;
use storage_settings::StorageAction;
use window::apply_theme;
#[cfg(test)]
use window::run;
#[cfg(test)]
mod floating_tests;

#[derive(Clone)]
enum Action {
    Application(ApplicationAction),
    Overview(OverviewAction),
    Connections(ConnectionsAction),
    History(HistoryAction),
    Analysis(AnalysisAction),
    Display(DisplayAction),
    Connection(ConnectionAction),
    Storage(StorageAction),
    Drill(DrillAction),
    Backup(BackupAction),
    Update(updates::UpdateAction),
    OpenProcessSettings,
    Nav(Page),
    DisplayTab(DisplayTab),
    SettingsSection(SettingsSection),
    Collapse,
    Period(TimePeriod),
    ApplyRange,
    Dimension(&'static str),
    Next(i32),
    JumpToPage,
    Sort,
    AppConnections,
    ActivityPreview(activity_page::ActivityTab),
    ActivityDetails(activity_page::ActivityTab),
    CloseActivityDetails,
    ActivityPage(i32),
    RecentRefresh,
    RecentUsageRefreshMode(RecentUsageRefreshMode),
    Config(&'static str),
    Theme(&'static str),
    ColorTheme(ColorTheme),
    TrayIconStyle(TrayIconStyle),
    ApplyOpenHotkey,
    RecordOpenHotkey,
    CancelOpenHotkeyRecording,
    ResetOpenHotkey,
    SettingsFold(SettingsFold),
    Export,
    OpenDir(&'static str),
    OpenExportDirectory,
    Reconnect,
    Notice,
}

#[derive(Clone, Copy)]
enum SettingsFold {
    Diagnostics,
    AboutDetails,
}

pub struct TrafficApp {
    #[cfg(test)]
    connection_test_scroll: ScrollHandle,
    page_scroll: ScrollHandle,
    page_scrollbar: ScrollbarState,
    app_usage_bounds: Rc<Cell<Bounds<Pixels>>>,
    app_usage_height: Rc<Cell<Pixels>>,
    app_usage_anchor_pending: bool,
    app_list_offset: Point<Pixels>,
    usage_detail: apps_page::UsageDetailState,
    connections_view: connections_page::State,
    overview_state: overview_page::State,
    seconds: seconds_page::State,
    expanded_application_path: Option<String>,
    backend: UiBackend,
    page: Page,
    display_tab: DisplayTab,
    settings_section: SettingsSection,
    p: Palette,
    mica_active: bool,
    wallpaper: WallpaperWatcher,
    config: Config,
    editor: DisplayEditor,
    inactive_editor: DisplayEditor,
    taskbar_fonts: Vec<String>,
    floating_background: Entity<InputState>,
    floating_background_error: bool,
    floating_transparency: Entity<SliderState>,
    floating_row_gap: Entity<SliderState>,
    picker: Option<PickerState>,
    font_search: Entity<InputState>,
    picker_focus: FocusHandle,
    picker_bounds: HashMap<&'static str, Rc<Cell<Bounds<Pixels>>>>,
    picker_scroll: ScrollHandle,
    picker_scrollbar: ScrollbarState,
    taskbar_position: taskbar_position::State,
    query: Query,
    query_id: u64,
    request_sequence: u64,
    detail_id: u64,
    report: Report,
    detail: Report,
    detail_cache: crate::detail_cache::DetailCache,
    received_report_id: u64,
    received_detail_id: u64,
    detail_requested_at: Option<Instant>,
    unsent_query: Option<(u64, Query)>,
    pending_query: Option<Query>,
    unsent_detail_query: Option<(u64, Query)>,
    pending_detail_query: Option<Query>,
    data: Shared,
    live_chart_view: chart::LiveChart,
    search: Entity<InputState>,
    page_number: Entity<InputState>,
    connection_form: connection::ConnectionForm,
    retention: Entity<InputState>,
    retention_size: Entity<InputState>,
    retention_size_error: String,
    storage_manager: storage_management_page::State,
    data_directory_draft: Option<std::path::PathBuf>,
    data_directory_error: String,
    migration_busy: bool,
    hotkey_draft: String,
    hotkey_recording: bool,
    hotkey_focus: FocusHandle,
    hotkey_pending: Option<(u32, crate::hotkey::Hotkey)>,
    hotkey_request: u32,
    open_hotkey_error: String,
    restore_path: Entity<InputState>,
    period: TimePeriod,
    app_custom_range: Option<(i64, i64)>,
    selected: String,
    detail_dimension: &'static str,
    history_state: history_page::HistoryState,
    analysis: analysis_page::AnalysisState,
    app_icons: crate::app_icons::AppIcons,
    icon_scale: f32,
    activity: activity_page::ActivityPageState,
    live_app_order: RefCell<ActivityOrder>,
    restore_armed: bool,
    about_details_open: bool,
    updates: updates::State,
    backup_directory_busy: bool,
    backup_directory_error: String,
    retention_error: String,
    keyboard_navigation: bool,
    tick: u64,
    last_search: String,
    main_hwnd: isize,
    deferred_close: Cell<bool>,
    floating: Rc<window::FloatingWindow>,
    #[cfg(test)]
    _window_queries: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

fn input(
    value: &str,
    hint: &str,
    masked: bool,
    window: &mut Window,
    cx: &mut App,
) -> Entity<InputState> {
    cx.new(|cx| {
        let mut s = InputState::new(window, cx)
            .placeholder(hint.to_string())
            .masked(masked);
        s.set_value(value.to_string(), window, cx);
        s
    })
}
fn row() -> Div {
    div().flex().items_center().gap(px(12.))
}
fn stretch() -> Div {
    let mut d = row();
    d.style().align_items = Some(AlignItems::Stretch);
    d
}
fn column() -> Div {
    div().flex().flex_col().gap(px(16.))
}
fn label(text: impl Into<SharedString>, color: u32) -> Div {
    div()
        .text_size(px(13.))
        .line_height(relative(1.45))
        .text_color(rgb(color))
        .child(text.into())
}
fn heading(text: impl Into<SharedString>, p: Palette) -> Div {
    div()
        .text_size(px(15.))
        .line_height(relative(1.4))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(p.text))
        .child(text.into())
}
fn card_surface(p: Palette, mica_active: bool) -> Rgba {
    if mica_active { rgba(0) } else { rgb(p.bg) }
}
fn state_surface(p: Palette) -> Rgba {
    tint_surface(p, p.accent)
}
fn switch_off_surface(p: Palette) -> Rgba {
    let mut off = rgb(p.text);
    off.a = if p.dark { 0.12 } else { 0.06 };
    off
}
fn position_choice_group() -> Div {
    row().flex_none().gap(px(2.))
}
fn position_action_separator(p: Palette) -> Div {
    div()
        .w(px(1.))
        .h(px(18.))
        .mx(px(10.))
        .flex_none()
        .bg(surface_border(p))
}
fn tint_surface(p: Palette, color: u32) -> Rgba {
    let mut color = rgb(color);
    color.a = if p.dark { 0.12 } else { 0.08 };
    color
}
fn surface_border(p: Palette) -> Rgba {
    let mut color = rgb(p.text);
    color.a = if p.dark { 0.10 } else { 0.06 };
    color
}
fn card_border(p: Palette) -> Rgba {
    let mut color = rgb(p.text);
    color.a = if p.dark { 0.20 } else { 0.16 };
    color
}

#[derive(IntoElement)]
struct InfoHelpTrigger {
    id: &'static str,
    palette: Palette,
    selected: bool,
}

impl Selectable for InfoHelpTrigger {
    fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    fn is_selected(&self) -> bool {
        self.selected
    }
}

impl RenderOnce for InfoHelpTrigger {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let p = self.palette;
        let id = self.id;
        div()
            .id(id)
            .group(id)
            .focusable()
            .tab_stop(true)
            .size(px(28.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(8.))
            .cursor_pointer()
            .text_color(rgb(if self.selected { p.accent_text } else { p.text }))
            .bg(if self.selected {
                state_surface(p)
            } else {
                rgba(0)
            })
            .hover(move |s| s.bg(state_surface(p)).text_color(rgb(p.accent_text)))
            .active(move |s| s.bg(switch_off_surface(p)).text_color(rgb(p.accent_text)))
            .child(
                svg()
                    .path("icons/position-info.svg")
                    .size(px(18.))
                    .flex_none()
                    // Svg only paints when its own style has an explicit color.
                    .text_color(rgb(if self.selected { p.accent_text } else { p.text }))
                    .group_hover(id, move |s| s.text_color(rgb(p.accent_text))),
            )
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
    }
}

fn settings_separator(p: Palette) -> Div {
    div().h(px(1.)).w_full().flex_none().bg(surface_border(p))
}
fn settings_row(title: &str, description: &str, control: impl IntoElement, p: Palette) -> Div {
    row()
        .min_h(px(62.))
        .gap(px(16.))
        .py(px(11.))
        .child(
            column()
                .gap(px(5.))
                .flex_1()
                .min_w_0()
                .child(heading(title.to_string(), p))
                .when(!description.is_empty(), |d| {
                    d.child(label(description.to_string(), p.muted).whitespace_normal())
                }),
        )
        .child(div().flex_none().child(control))
}
fn settings_info(title: &str, value: impl Into<SharedString>, p: Palette) -> Div {
    column()
        .flex_1()
        .min_w_0()
        .gap(px(7.))
        .child(label(title.to_string(), p.muted))
        .child(
            div()
                .text_color(rgb(p.text))
                .whitespace_normal()
                .child(value.into()),
        )
}
fn short(s: &str, max: usize) -> String {
    let mut chars = s.chars();
    let out: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        format!("{out}…")
    } else {
        out
    }
}

impl TrafficApp {
    fn card_surface(&self) -> Rgba {
        card_surface(self.p, self.mica_active)
    }
    fn card(&self) -> Div {
        let p = self.p;
        column()
            .p(px(19.))
            .bg(self.card_surface())
            .border_1()
            .border_color(card_border(p))
            .rounded(px(p.radius as f32))
            .min_w_0()
    }
    fn refresh(&mut self) {
        let revision = self.backend.storage_revision();
        if self.detail_cache.set_revision(revision) {
            self.detail = Report::default();
            self.pending_query = None;
            self.pending_detail_query = None;
        }
        if self.selected.is_empty() && !self.activity.details {
            self.refresh_main_query();
        }
        self.refresh_detail_query();
    }
    fn next_query_id(&mut self) -> u64 {
        self.request_sequence = self.request_sequence.max(self.query_id).max(self.detail_id) + 1;
        self.request_sequence
    }
    fn refresh_main_query(&mut self) {
        if self.pending_query.as_ref() == Some(&self.query) {
            return;
        }
        self.query_id = self.next_query_id();
        self.pending_query = Some(self.query.clone());
        self.unsent_query = (!self.backend.query(self.query_id, self.query.clone()))
            .then_some((self.query_id, self.query.clone()));
    }
    fn refresh_detail_query(&mut self) {
        let revision = self.backend.storage_revision();
        if self.detail_cache.set_revision(revision) {
            self.detail = Report::default();
            self.pending_detail_query = None;
            self.pending_query = None;
            if self.selected.is_empty() {
                self.refresh_main_query();
            }
        }
        let Some(q) = self.application_detail_query() else {
            return;
        };
        if self.pending_detail_query.as_ref() == Some(&q) {
            return;
        }
        if self.usage_detail_open()
            && self
                .detail
                .query
                .as_ref()
                .is_some_and(|old| old.from != q.from || old.to != q.to)
        {
            self.usage_detail.scroll.set_offset(point(px(0.), px(0.)));
            self.usage_detail.scrollbar = ScrollbarState::default();
        }
        self.unsent_detail_query = None;
        self.pending_detail_query = Some(q.clone());
        self.detail_id = self.next_query_id();
        if let Some(cached) = self.detail_cache.get(&q) {
            self.detail = cached.report;
            if cached.fresh {
                self.received_detail_id = self.detail_id;
                self.detail_requested_at = None;
                self.pending_detail_query = None;
                return;
            }
        } else if !crate::detail_cache::matches(&self.detail, &q) {
            self.detail = Report::default();
        }
        self.detail_requested_at = Some(Instant::now());
        self.unsent_detail_query =
            (!self.backend.query(self.detail_id, q.clone())).then_some((self.detail_id, q));
    }
    fn application_detail_query(&self) -> Option<Query> {
        self.active_detail_context()
            .map(|context| self.detail_query_for(&context))
    }
    fn detail_query_for(&self, context: &apps_page::DetailContext) -> Query {
        Query {
            app: context.key.clone(),
            history: self.page == Page::History,
            history_context: None,
            group_apps: context.group_apps,
            page: 0,
            page_size: 50,
            search: String::new(),
            dimension: "domain".into(),
            sort: TrafficSort::Total,
            ascending: false,
            ..self.query.clone()
        }
    }
    fn poll_query_results(&mut self, cx: &mut Context<Self>) {
        self.poll_overview(cx);
        self.poll_seconds(cx);
        self.poll_storage(cx);
        if let Some((id, query)) = self.unsent_query.take()
            && !self.backend.query(id, query.clone())
        {
            self.unsent_query = Some((id, query));
        }
        if let Some((id, query)) = self.unsent_detail_query.take()
            && !self.backend.query(id, query.clone())
        {
            self.unsent_detail_query = Some((id, query));
        }
        let report_id = (self.selected.is_empty() && self.received_report_id != self.query_id)
            .then_some(self.query_id);
        let detail_id = (self.active_detail_context().is_some()
            && self.received_detail_id != self.detail_id)
            .then_some(self.detail_id);
        let (revision, report, detail) = self.backend.query_results(report_id, detail_id);
        if self.detail_cache.set_revision(revision) {
            self.detail = Report::default();
            self.pending_query = None;
            self.pending_detail_query = None;
            self.refresh();
            cx.notify();
            return;
        }
        if let Some(report) = report {
            if self.page == Page::History
                && !report.error.is_empty()
                && self.report.query == report.query
            {
                self.report.error = report.error;
            } else {
                self.report = report;
            }
            self.received_report_id = self.query_id;
            self.pending_query = None;
            if matches!(self.page, Page::Apps | Page::History)
                && self.report.error.is_empty()
                && self.report.query.as_ref() == Some(&self.query)
            {
                let last_page = self.page_count().saturating_sub(1);
                if self.query.page > last_page {
                    self.query.page = last_page;
                    self.refresh_main_query();
                }
            }
            cx.notify();
        }
        if let Some(mut detail) = detail
            && let Some(query) = self.application_detail_query()
            && crate::detail_cache::matches(&detail, &query)
        {
            if detail.error.is_empty() {
                self.detail_cache.remember(&detail);
            } else if let Some(cached) = self.detail_cache.get(&query) {
                let error = detail.error;
                detail = cached.report;
                detail.error = error;
            }
            self.detail = detail;
            self.received_detail_id = self.detail_id;
            self.detail_requested_at = None;
            self.pending_detail_query = None;
            cx.notify();
        }
    }
    fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.migration_busy {
            cx.notify();
            return;
        }
        self.tick += 1;
        if self.hotkey_recording
            && (!window.is_window_active() || !self.hotkey_focus.is_focused(window))
        {
            self.cancel_hotkey_recording(cx);
        }
        let updated = self.backend.config_snapshot();
        let updated_content = match self.display_tab {
            DisplayTab::Taskbar => &updated.taskbar_content,
            DisplayTab::Floating => &updated.floating_content,
        };
        if self.editor.spacing.conflicts_with(updated_content) {
            self.cancel_taskbar_spacing(cx);
        }
        if self
            .editor
            .drag
            .as_ref()
            .is_some_and(|drag| &drag.original != updated_content)
        {
            self.cancel_taskbar_drag(window, cx);
        }
        let accent_changed = self.wallpaper.poll();
        let dark = updated.theme == "dark" || (updated.theme == "system" && native::dark_system());
        if self.config.theme != updated.theme
            || self.config.color_theme != updated.color_theme
            || self.p.dark != dark
            || updated.color_theme == ColorTheme::Mica && accent_changed
        {
            self.close_picker(window, cx);
            self.cancel_taskbar_drag(window, cx);
            self.cancel_taskbar_spacing(cx);
            self.p = Palette::new(updated.color_theme, dark)
                .with_wallpaper_accent(self.wallpaper.color())
                .main_window();
            self.mica_active = apply_theme(
                self.p,
                dark,
                updated.color_theme == ColorTheme::Mica,
                window,
                cx,
            );
        }
        self.config = updated;
        self.mark_update_seen(window, cx);
        self.sync_taskbar_editor(window, cx);
        self.sync_floating_controls(window, cx);
        native::titlebar_theme(
            self.main_hwnd,
            self.config.theme == "dark" || (self.config.theme == "system" && native::dark_system()),
        );

        let visible = self.config.floating
            && !(self.config.fullscreen_hide && native::fullscreen_foreground());
        if visible {
            self.refresh_floating(cx);
        } else {
            native::show(self.floating.hwnd(), false);
        }
        if self.tick.is_multiple_of(10) && visible {
            let (x, y) = native::position(self.floating.hwnd());
            if (Some(x), Some(y)) != (self.config.floating_x, self.config.floating_y) {
                self.config.floating_x = Some(x);
                self.config.floating_y = Some(y);
                self.save_config();
            }
        }
        if !native::visible(self.main_hwnd) {
            self.reset_overview_exit_hover();
            self.tick_overview_exit(false, cx);
            return;
        }
        self.tick_overview_exit(self.page == Page::Overview && window.is_window_active(), cx);
        let search = self.search.read(cx).value().to_string();
        if search != self.last_search {
            self.app_usage_anchor_pending = false;
            self.last_search = search.clone();
            self.query.page = 0;
            if self.page == Page::Connections {
                self.connections_view.page = 0;
            }
            self.query.search = if matches!(self.page, Page::Apps | Page::History) {
                search
            } else {
                String::new()
            };
            self.refresh();
        }
        if self.tick.is_multiple_of(10)
            && !(self.history_state.time_open && matches!(self.page, Page::Apps | Page::History))
            && !(self.page == Page::Apps && self.activity.details && self.selected.is_empty())
            && !(self.page == Page::History
                && (self.analysis.active
                    || self.history_state.drag.is_some()
                    || self.history_state.time_open))
        {
            let period = if self.page == Page::History {
                self.history_state.record_period
            } else {
                self.period
            };
            if let Ok(Some((from, to))) = period.bounds(chrono::Local::now())
                && (self.query.from != from || self.query.to != to)
            {
                if self.page == Page::History {
                    use chrono::Datelike;
                    let previous = crate::history::local_date(self.query.from);
                    let next = crate::history::local_date(from);
                    if self.history_state.month == previous.with_day(1).unwrap() {
                        self.history_state.month = next.with_day(1).unwrap();
                    }
                    self.history_state.anchor = next;
                    self.history_state.cursor = next;
                }
                self.query.from = from;
                self.query.to = to;
                self.query.page = 0;
            }
            self.refresh();
        }
        let next_data = self.backend.snapshot();
        if self.data.storage_revision != next_data.storage_revision {
            self.refresh();
        }
        self.data = next_data;
        if self.page == Page::Connections {
            self.sync_connections_view();
        }
        self.poll_analysis(cx);
        self.poll_history_calendar(self.tick.is_multiple_of(10), cx);
        self.poll_recent_usage(cx);
        self.sync_retention_inputs(window, cx);
        self.sync_seconds_inputs(window, cx);
        self.sync_connection_form();
        self.sync_position_controls(window, cx);
        self.poll_query_results(cx);
        self.icon_scale = window.scale_factor();
        let generation = self.backend.analysis_generation();
        let mut keys: Vec<_> = self
            .data
            .connections
            .iter()
            .map(Connection::app)
            .chain(self.data.app_rates.keys().cloned())
            .chain(
                self.report
                    .apps
                    .iter()
                    .chain(&self.report.rows)
                    .map(|row| row.key.clone()),
            )
            .collect();
        for snapshot in [&self.analysis.snapshot, &self.activity.recent_snapshot]
            .into_iter()
            .flatten()
        {
            if snapshot.request.generation == generation {
                keys.extend(snapshot.apps.keys().cloned());
            }
        }
        keys.push(self.selected.clone());
        keys.extend(
            self.data
                .identities
                .values()
                .map(|owner| owner.icon.clone()),
        );
        self.app_icons.update_aliases(generation, keys);
        cx.notify();
    }
    fn save_config(&mut self) {
        if let Err(error) = self.backend.save_ui_config(self.config.clone()) {
            self.backend.notice(format!("设置无效：{error}"));
        }
        self.config = self.backend.config_snapshot();
    }
    fn change_config(&mut self, key: &str) {
        match key {
            "floating" => self.config.floating = !self.config.floating,
            "taskbar" => self.config.taskbar = !self.config.taskbar,
            "locked" => self.config.floating_locked = !self.config.floating_locked,
            "today" => self.config.floating_today = !self.config.floating_today,
            "fullscreen" => self.config.fullscreen_hide = !self.config.fullscreen_hide,
            "horizontal" => {
                self.config.taskbar_horizontal = !self.config.taskbar_horizontal;
                self.config
                    .taskbar_content
                    .set_rows(if self.config.taskbar_horizontal { 1 } else { 2 });
            }
            "left" => self.config.taskbar_left = !self.config.taskbar_left,
            "autorun" => self.config.autorun = !self.config.autorun,
            "close" => self.config.close_to_tray = !self.config.close_to_tray,
            "tray" => self.config.tray_visible = !self.config.tray_visible,
            _ => {}
        }
        self.save_config();
    }
    fn act(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if self.migration_busy {
            return;
        }
        self.cancel_hotkey_recording(cx);
        self.close_picker(window, cx);
        self.cancel_taskbar_drag(window, cx);
        self.cancel_taskbar_spacing(cx);
        match action {
            Action::Application(action) => self.handle_application_action(action, window, cx),
            Action::Nav(page) => self.navigate(page, window, cx),
            Action::DisplayTab(tab) => self.select_display_tab(tab, window, cx),
            Action::SettingsSection(section) => self.select_settings_section(section, window, cx),
            Action::Collapse => {
                self.config.collapsed = !self.config.collapsed;
                self.save_config();
            }
            Action::Period(n) => {
                self.app_usage_anchor_pending = false;
                if n == TimePeriod::Custom {
                    if self.page == Page::Apps {
                        self.toggle_time_range(window, cx);
                    }
                } else if let Ok(Some((from, to))) = n.bounds(chrono::Local::now()) {
                    self.period = n;
                    if self.page == Page::Apps {
                        self.app_custom_range = None;
                    }
                    self.history_state.time_open = false;
                    self.history_state.time_expanded = false;
                    self.history_state.time_initialized = false;
                    self.history_state.time_quick = None;
                    self.query.page = 0;
                    self.query.from = from;
                    self.query.to = to;
                    self.refresh();
                }
            }
            Action::ApplyRange => self.apply_history_dates(cx),
            Action::Dimension(d) => {
                self.close_usage_detail();
                self.query.dimension = d.into();
                self.query.page = 0;
                self.refresh();
            }
            Action::History(action) => self.handle_history_action(action, window, cx),
            Action::Next(n) => {
                let (current, pages) = self.pagination_position();
                if let Some(page) = current.checked_add_signed(n as isize)
                    && page < pages
                {
                    self.go_to_page(page, window, cx);
                }
            }
            Action::JumpToPage => {
                let (_, pages) = self.pagination_position();
                if let Some(page) = self.page_input_target(pages, cx) {
                    self.go_to_page(page, window, cx);
                }
            }
            Action::Sort => {
                self.app_usage_anchor_pending = false;
                self.query.ascending = !self.query.ascending;
                self.query.page = 0;
                self.refresh();
            }
            Action::ActivityPreview(tab) => {
                window.blur();
                self.activity.preview = tab;
                if tab == activity_page::ActivityTab::Recent {
                    self.refresh_recent_usage_on_entry();
                }
            }
            Action::ActivityDetails(tab) => self.show_activity_details(tab, window, cx),
            Action::CloseActivityDetails => self.close_activity_details(window, cx),
            Action::ActivityPage(step) => self.activity_page(step),
            Action::RecentRefresh => {
                if self.activity.recent_pending.is_none() {
                    self.request_recent_usage();
                }
            }
            Action::RecentUsageRefreshMode(mode) => {
                self.close_picker(window, cx);
                self.config.recent_usage_refresh_mode = mode;
                self.save_config();
            }
            Action::Analysis(action) => self.handle_analysis_action(action, window, cx),
            Action::AppConnections => {
                let Some(context) = self.active_detail_context() else {
                    return;
                };
                self.act(Action::Nav(Page::Connections), window, cx);
                self.activity.connection_group = Some(context.key.clone());
                self.connections_view = connections_page::State::default();
                self.connections_view.raw_process = !context.group_apps;
                self.query.page = 0;
                self.query.page_size = 50;
                self.search.update(cx, |s, cx| {
                    s.set_value("", window, cx);
                });
                self.selected.clear();
            }
            Action::OpenProcessSettings => {
                self.act(
                    Action::SettingsSection(SettingsSection::Connection),
                    window,
                    cx,
                );
                self.act(Action::Nav(Page::Settings), window, cx);
            }
            Action::Connections(action) => self.handle_connections_action(action),
            Action::Overview(action) => self.handle_overview_action(action, window, cx),
            Action::Config(key) => self.change_config(key),
            Action::Theme(theme) => {
                self.config.theme = theme.into();
                self.save_config();
                let dark = theme == "dark" || (theme == "system" && native::dark_system());
                self.p = Palette::new(self.config.color_theme, dark)
                    .with_wallpaper_accent(self.wallpaper.color())
                    .main_window();
                self.mica_active = apply_theme(
                    self.p,
                    dark,
                    self.config.color_theme == ColorTheme::Mica,
                    window,
                    cx,
                );
            }
            Action::ColorTheme(color_theme) => {
                self.config.color_theme = color_theme;
                self.save_config();
                if color_theme == ColorTheme::Mica {
                    self.wallpaper.request(self.main_hwnd);
                }
                let dark = self.config.theme == "dark"
                    || (self.config.theme == "system" && native::dark_system());
                self.p = Palette::new(color_theme, dark)
                    .with_wallpaper_accent(self.wallpaper.color())
                    .main_window();
                self.mica_active =
                    apply_theme(self.p, dark, color_theme == ColorTheme::Mica, window, cx);
            }
            Action::TrayIconStyle(style) => {
                self.config.tray_icon_style = style;
                self.save_config();
            }
            Action::ApplyOpenHotkey => self.apply_open_hotkey(cx),
            Action::RecordOpenHotkey => self.start_hotkey_recording(window, cx),
            Action::CancelOpenHotkeyRecording => {}
            Action::ResetOpenHotkey => {
                if self.hotkey_pending.is_none() {
                    self.hotkey_draft = crate::hotkey::DEFAULT.into();
                    self.apply_open_hotkey(cx);
                }
            }
            Action::Display(action) => {
                if self.handle_display_action(action, window, cx).is_break() {
                    return;
                }
            }
            Action::Connection(action) => self.handle_connection_action(action, cx),
            Action::SettingsFold(fold) => match fold {
                SettingsFold::Diagnostics => {
                    self.connection_form.diagnostics_open = !self.connection_form.diagnostics_open
                }
                SettingsFold::AboutDetails => self.about_details_open = !self.about_details_open,
            },
            Action::Export => {
                let mut query = self.query.clone();
                query.search = self.search.read(cx).value().to_string();
                self.backend.send(Command::Export(query));
            }
            Action::Backup(action) => self.handle_backup_action(action, window, cx),
            Action::Update(action) => self.handle_update_action(action, window, cx),
            Action::OpenDir(name) => self.open_directory(self.backend.dir.join(name)),
            Action::OpenExportDirectory => {
                self.open_directory_with(crate::csv_export::ensure_directory)
            }
            Action::Storage(action) => self.handle_storage_action(action, window, cx),
            Action::Drill(action) => self.handle_drill_action(action, window, cx),
            Action::Reconnect => self.backend.send(Command::Reconnect),
            Action::Notice => {
                self.backend.notice(String::new());
                self.data.notice.clear();
            }
        }
        if self.usage_detail.key.is_some() && !self.usage_detail_open() {
            self.close_usage_detail();
        }
        self.refresh_floating(cx);
        cx.notify();
    }
    fn open_directory(&self, path: std::path::PathBuf) {
        self.open_directory_with(move || Ok(path));
    }
    fn open_directory_with(
        &self,
        path: impl FnOnce() -> anyhow::Result<std::path::PathBuf> + Send + 'static,
    ) {
        let backend = self.backend.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("open-directory".into())
            .spawn(move || {
                if let Err(error) = path().and_then(|path| native::open_folder(&path)) {
                    backend.notice(format!("打开目录失败：{error:#}"));
                }
            })
        {
            self.backend.notice(format!("打开目录失败：{error:#}"));
        }
    }
    fn button(
        &self,
        id: impl Into<SharedString>,
        text: impl Into<SharedString>,
        active: bool,
        action: Action,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let p = self.p;
        div()
            .id(id.into())
            .flex()
            .items_center()
            .justify_center()
            .px(px(12.))
            .h(px(if self.page == Page::History { 32. } else { 34. }))
            .rounded(px(9.))
            .text_size(px(13.))
            .cursor_pointer()
            .bg(if active { state_surface(p) } else { rgba(0) })
            .text_color(rgb(if active { p.accent_text } else { p.muted }))
            .hover(move |s| s.bg(state_surface(p)).text_color(rgb(p.accent_text)))
            .child(scroll_fade::FadingText(text.into()))
            .on_click(cx.listener(move |s, _, w, cx| s.act(action.clone(), w, cx)))
    }
    fn switch_control(
        &self,
        id: &'static str,
        title: &str,
        on: bool,
        action: Action,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let tooltip = format!("{title}：{}", if on { "已开启" } else { "已关闭" });
        self.switch_control_with_tooltip(id, &tooltip, on, action, cx)
    }
    fn switch_control_with_tooltip(
        &self,
        id: &'static str,
        tooltip: &str,
        on: bool,
        action: Action,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let p = self.p;
        let off = switch_off_surface(p);
        let mut off_hover = off;
        off_hover.a = if p.dark { 0.18 } else { 0.10 };
        let mut outline = rgb(p.text);
        outline.a = if p.dark { 0.40 } else { 0.32 };
        let track = if on { rgb(p.accent) } else { off };
        let hover = if on { rgb(p.accent_hover()) } else { off_hover };
        let thumb = if on { p.switch_thumb() } else { p.muted };
        let tooltip = tooltip.to_owned();
        let key_action = action.clone();
        div()
            .id(id)
            .group(id)
            .focusable()
            .tab_stop(true)
            .flex()
            .items_center()
            .justify_center()
            .w(px(52.))
            .h(px(32.))
            .flex_none()
            .rounded_full()
            .border_2()
            .border_color(rgba(0))
            .when(self.keyboard_navigation, |d| {
                d.focus(move |s| s.border_color(rgb(p.accent)))
            })
            .cursor_pointer()
            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .when(on, |d| d.justify_end())
                    .w(px(44.))
                    .h(px(24.))
                    .flex_none()
                    .p(px(2.))
                    .rounded_full()
                    .border_1()
                    .border_color(if on { rgb(p.accent) } else { outline })
                    .bg(track)
                    .group_hover(id, move |s| s.bg(hover))
                    .child(
                        div()
                            .size(px(18.))
                            .flex_none()
                            .rounded_full()
                            .bg(rgb(thumb)),
                    ),
            )
            .on_click(cx.listener(move |s, _, window, cx| {
                cx.stop_propagation();
                s.act(action.clone(), window, cx);
            }))
            .on_key_down(cx.listener(move |s, event: &KeyDownEvent, window, cx| {
                if !event.is_held
                    && !event.keystroke.modifiers.modified()
                    && matches!(event.keystroke.key.as_str(), "space" | "enter")
                {
                    cx.stop_propagation();
                    window.prevent_default();
                    s.act(key_action.clone(), window, cx);
                }
            }))
    }
    fn toggle(
        &self,
        id: &'static str,
        title: &str,
        description: &str,
        on: bool,
        action: Action,
        cx: &Context<Self>,
    ) -> Div {
        let p = self.p;
        row()
            .justify_between()
            .gap(px(16.))
            .py(px(10.))
            .child(
                column()
                    .gap(px(4.))
                    .flex_1()
                    .min_w_0()
                    .child(heading(title.to_string(), p))
                    .when(!description.is_empty(), |d| {
                        d.child(label(description.to_string(), p.muted))
                    }),
            )
            .child(self.switch_control(id, title, on, action, cx))
    }
    fn settings_segment(
        &self,
        id: &'static str,
        title: &'static str,
        selected: bool,
        action: Action,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let p = self.p;
        div()
            .id(id)
            .flex()
            .items_center()
            .justify_center()
            .min_w(px(76.))
            .h(px(32.))
            .px(px(12.))
            .rounded(px(7.))
            .cursor_pointer()
            .text_size(px(13.))
            .bg(if selected { state_surface(p) } else { rgba(0) })
            .text_color(rgb(if selected { p.accent_text } else { p.muted }))
            .hover(move |s| s.bg(state_surface(p)).text_color(rgb(p.accent_text)))
            .child(title)
            .on_click(cx.listener(move |s, _, window, cx| s.act(action.clone(), window, cx)))
    }
    fn settings_disclosure(
        &self,
        id: &'static str,
        title: &'static str,
        expanded: bool,
        fold: SettingsFold,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let p = self.p;
        row()
            .id(id)
            .w_full()
            .justify_between()
            .min_h(px(36.))
            .cursor_pointer()
            .text_color(rgb(p.text))
            .hover(move |d| d.text_color(rgb(p.accent_text)))
            .child(title)
            .child(
                div()
                    .font_family("Segoe MDL2 Assets")
                    .text_size(px(11.))
                    .text_color(rgb(p.muted))
                    .child(if expanded { "\u{e70e}" } else { "\u{e70d}" }),
            )
            .on_click(
                cx.listener(move |s, _, window, cx| s.act(Action::SettingsFold(fold), window, cx)),
            )
    }
    fn period_tools(&self, cx: &Context<Self>) -> Div {
        row()
            .gap(px(3.))
            .p(px(4.))
            .flex_none()
            .rounded(px(10.))
            .bg(rgba(0))
            .children(
                TimePeriod::ALL
                    .into_iter()
                    .filter(|period| self.page != Page::Overview || *period != TimePeriod::Custom)
                    .enumerate()
                    .map(|(i, period)| {
                        self.button(
                            format!("period-{i}"),
                            period.label(),
                            if self.page == Page::History {
                                self.history_state.record_period == period
                            } else {
                                self.period == period
                            },
                            if self.page == Page::History {
                                Action::History(HistoryAction::QuickPeriod(period))
                            } else {
                                Action::Period(period)
                            },
                            cx,
                        )
                        .h(px(32.))
                        .rounded(px(7.))
                        .bg(
                            if (if self.page == Page::History {
                                self.history_state.record_period
                            } else {
                                self.period
                            }) == period
                            {
                                state_surface(self.p)
                            } else {
                                rgba(0)
                            },
                        )
                    }),
            )
    }
    fn empty(&self, title: &str, description: &str) -> Div {
        column()
            .items_center()
            .justify_center()
            .py(px(30.))
            .gap(px(9.))
            .child(
                div()
                    .text_size(px(25.))
                    .text_color(rgb(if self.p.mica {
                        self.p.text
                    } else {
                        self.p.blue
                    }))
                    .child("≈"),
            )
            .child(heading(title.to_string(), self.p))
            .child(label(description.to_string(), self.p.muted))
    }
    fn page_count(&self) -> usize {
        self.report.row_count.div_ceil(self.query.page_size)
    }
    fn displayed_page(&self) -> Option<usize> {
        self.report.query.as_ref().and_then(|query| {
            (query == &self.query || query.differs_only_by_page(&self.query)).then_some(query.page)
        })
    }
    fn pagination_position(&self) -> (usize, usize) {
        if self.page == Page::Connections {
            (
                self.connections_view.page,
                self.filtered_connections()
                    .len()
                    .div_ceil(connections_page::PAGE_SIZE),
            )
        } else {
            (self.query.page, self.page_count())
        }
    }
    fn go_to_page(&mut self, page: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.page_number.update(cx, |input, cx| {
            if input.focus_handle(cx).is_focused(window) {
                window.blur();
            }
            input.set_value("", window, cx);
        });
        if self.page == Page::Connections {
            if page != self.connections_view.page {
                self.connections_view.page = page;
                self.page_scroll.set_offset(point(px(0.), px(0.)));
            }
        } else if self.displayed_page().is_some() && page != self.query.page {
            self.query.page = page;
            self.app_usage_anchor_pending =
                matches!(self.page, Page::Apps | Page::History) && self.selected.is_empty();
            self.refresh();
        }
    }
    fn page_input_target(&self, pages: usize, cx: &App) -> Option<usize> {
        self.page_number
            .read(cx)
            .value()
            .trim()
            .parse::<usize>()
            .ok()?
            .checked_sub(1)
            .filter(|page| *page < pages)
    }
    fn pagination(&self, cx: &Context<Self>) -> Div {
        self.pagination_footer(
            self.report.row_count,
            self.query.page_size,
            self.displayed_page().unwrap_or(0),
            self.query.page,
            cx,
        )
    }
    fn pagination_footer(
        &self,
        count: usize,
        page_size: usize,
        displayed: usize,
        requested: usize,
        cx: &Context<Self>,
    ) -> Div {
        let p = self.p;
        let pages = count.div_ceil(page_size);
        let page = (displayed + 1).min(pages);
        let mut input = self.text_input(&self.page_number);
        input.height = 24.;
        input.search_scroll = Some(self.page_scroll.clone());
        let empty = self.page_number.read(cx).value().is_empty();
        input.invalid = !empty && self.page_input_target(pages, cx).is_none();
        let step_button = |id, title, step, disabled| {
            Button::new(id)
                .label(title)
                .custom(
                    ButtonCustomVariant::new(cx)
                        .foreground(rgb(p.muted).into())
                        .hover(state_surface(p).into())
                        .active(control_surface(p).into()),
                )
                .compact()
                .disabled(disabled)
                .on_click(
                    cx.listener(move |app, _, window, cx| app.act(Action::Next(step), window, cx)),
                )
        };
        let page_label = row()
            .gap(px(4.))
            .flex_none()
            .text_size(px(12.))
            .child(label("第", p.muted))
            .when(pages <= 1, |d| d.child(label(page.to_string(), p.text)))
            .when(pages > 1, |d| {
                d.child(
                    div()
                        .id("pagination-page")
                        .relative()
                        .w(px(
                            44. + pages.to_string().len().saturating_sub(3) as f32 * 7.
                        ))
                        .child(input)
                        .tooltip(|window, cx| {
                            Tooltip::new("输入页码后按 Enter 跳转").build(window, cx)
                        })
                        .when(empty, |d| {
                            d.child(
                                div()
                                    .absolute()
                                    .inset_0()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(label(page.to_string(), p.muted)),
                            )
                        }),
                )
            })
            .child(label(format!("/ {pages} 页"), p.muted));
        row()
            .justify_between()
            .gap(px(8.))
            .flex_wrap()
            .pt(px(6.))
            .border_t_1()
            .border_color(surface_border(p))
            .child(label(format!("共 {count} 条"), p.muted).text_size(px(12.)))
            .child(
                row()
                    .gap(px(4.))
                    .flex_wrap()
                    .child(step_button(
                        "prev",
                        "上一页",
                        -1,
                        requested == 0 || pages == 0,
                    ))
                    .child(page_label)
                    .child(step_button(
                        "next",
                        "下一页",
                        1,
                        requested >= pages.saturating_sub(1),
                    )),
            )
    }
}

impl Render for TrafficApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.animate_overview_exit(window);
        if self.history_state.time_open && matches!(self.page, Page::Apps | Page::History) {
            let range = if self.page == Page::History
                && self.history_state.time_target == HistoryTab::Analysis
            {
                self.history_state.analysis_range
            } else {
                (self.query.from, self.query.to)
            };
            if range != self.history_state.time_applied {
                self.history_state.time_applied = range;
                self.reset_time_range(window, cx);
            }
        }
        self.schedule_app_usage_anchor(window, cx);
        self.schedule_analysis_anchor(window, cx);
        self.schedule_activity_anchor(window, cx);
        self.schedule_history_restore(window, cx);
        let p = self.p;
        let timed = self.page.has_time_range()
            && !(self.page == Page::Apps && self.activity.details && self.selected.is_empty());
        let fixed_content = matches!(self.page, Page::Display | Page::Settings);
        let content = match self.page {
            Page::Overview => self.overview(window, cx),
            Page::Apps => self.apps(window, cx),
            Page::History if self.history_state.tab == HistoryTab::Analysis => {
                self.history_comparison(window, cx)
            }
            Page::History => self.history(window, cx),
            Page::Connections => self.connections(window, cx),
            Page::Display => self.display(cx),
            Page::Settings => self.settings(cx),
        };
        let mut body = column().gap(px(16.)).px(px(24.)).py(px(16.));
        if self.history_state.time_open
            && ((self.page == Page::Apps && timed)
                || (self.page == Page::History && self.history_state.tab == HistoryTab::Records))
        {
            body = body.child(self.time_range_editor(window, cx));
        }
        if !self.data.connected && self.page != Page::Settings {
            body = body.child(
                row()
                    .p(px(12.))
                    .rounded(px(10.))
                    .bg(rgb(p.tint))
                    .child(label(short(&self.data.status, 140), p.amber).flex_1())
                    .child(self.button("status-reconnect", "重试", false, Action::Reconnect, cx)),
            );
        }
        if !self.data.notice.is_empty()
            && !(self.page == Page::Settings && self.data.notice == "正在重新连接…")
        {
            body = body.child(
                row()
                    .p(px(12.))
                    .rounded(px(10.))
                    .bg(rgb(p.tint))
                    .child(
                        label(self.data.notice.clone(), p.accent_text)
                            .flex_1()
                            .min_w_0()
                            .whitespace_normal(),
                    )
                    .child(self.button("notice-close", "×", false, Action::Notice, cx)),
            );
        }
        if !self.report.error.is_empty()
            && !(self.page == Page::History && self.history_state.tab == HistoryTab::Analysis)
        {
            body = body.child(label(self.report.error.clone(), p.amber));
        }
        body = body
            .child(content)
            .when(fixed_content, |d| d.h_full().min_h_0());
        div()
            .id("traffic-window-root")
            .size_full()
            .flex()
            .flex_col()
            .capture_any_mouse_down(cx.listener(|app, event: &MouseDownEvent, window, cx| {
                if app.page == Page::Overview
                    && app.overview_state.hover.is_some()
                    && !app
                        .overview_state
                        .hover_bounds
                        .get()
                        .contains(&event.position)
                    && !app
                        .overview_state
                        .bar_bounds
                        .borrow()
                        .values()
                        .any(|rect| rect.contains(&event.position))
                {
                    app.close_overview_hover(cx);
                }
                if app.history_chart_card_right_click(event, window, cx) {
                    return;
                }
                if app.page == Page::History
                    && app.history_state.chart_hover.is_some_and(|h| h.visible)
                    && !app
                        .history_state
                        .chart_plot_bounds
                        .get()
                        .contains(&event.position)
                    && !app
                        .history_state
                        .chart_hover_bounds
                        .get()
                        .contains(&event.position)
                {
                    app.close_history_chart_hover(cx);
                }
                if app.keyboard_navigation {
                    app.keyboard_navigation = false;
                    cx.notify();
                }
            }))
            .on_mouse_move(cx.listener(|app, event: &MouseMoveEvent, window, cx| {
                app.move_time_drag(event.position, window, cx);
                if app.page == Page::History && app.history_state.time_drag.is_none() {
                    app.track_history_chart_pointer(event.position, cx);
                    app.move_history_drag(event.position, cx);
                }
            }))
            .on_hover(cx.listener(|app, entered: &bool, _, cx| {
                if !*entered && app.valid_history_chart_hover().is_some_and(|h| !h.pinned) {
                    app.close_history_chart_hover(cx);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|app, _, window, cx| {
                    app.finish_history_drag(cx);
                    app.finish_time_drag(window, cx);
                    app.taskbar_position.hold_generation += 1;
                    app.cancel_taskbar_drag(window, cx);
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|app, _, window, cx| {
                    app.finish_history_drag(cx);
                    app.finish_time_drag(window, cx);
                    app.cancel_taskbar_drag(window, cx)
                }),
            )
            .font_family("Microsoft YaHei UI")
            .text_size(px(13.))
            .line_height(relative(1.45))
            .text_color(rgb(p.text))
            .bg(if self.mica_active { rgba(0) } else { rgb(p.bg) })
            .child(self.titlebar(window, cx))
            .child(
                row()
                    .gap(px(0.))
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(self.sidebar(cx))
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .h_full()
                            .min_w_0()
                            .child(
                                div()
                                    .id("page-scroll")
                                    .size_full()
                                    .when(fixed_content, |d| d.overflow_hidden())
                                    .when(!fixed_content, |d| d.overflow_y_scroll())
                                    .track_scroll(&self.page_scroll)
                                    .when(self.page == Page::Apps, |page| {
                                        page.on_scroll_wheel(cx.listener(
                                            |app, event: &ScrollWheelEvent, _, cx| {
                                                if app.page_scrollbar.note_native_scroll(
                                                    &app.page_scroll,
                                                    event.delta,
                                                ) {
                                                    cx.notify();
                                                }
                                            },
                                        ))
                                    })
                                    .child(body),
                            )
                            .when(!fixed_content, |d| {
                                d.child(
                                    controls::scroll_overlay(
                                        Scrollbar::vertical(
                                            &self.page_scrollbar,
                                            &self.page_scroll,
                                        )
                                        .native_wheel()
                                        .scrolling_only(self.page == Page::Apps),
                                    )
                                    .w_auto()
                                    .right(px(8.)),
                                )
                            })
                            .when(
                                self.usage_detail_open() && !self.usage_detail_docked(window),
                                |d| {
                                    d.child(
                                        self.usage_detail_pane(window, cx)
                                            .absolute()
                                            .top(px(16.))
                                            .right(px(24.))
                                            .occlude(),
                                    )
                                },
                            ),
                    ),
            )
            .children(self.history_overlays(window, cx))
            .children(self.overview_usage_overlay(window, cx))
            .children(self.storage_overlay(window, cx))
            .children(self.picker_overlay(window, cx))
            .children(self.seconds_overlay(window, cx))
            .children(self.update_notes_overlay(window, cx))
            .when(self.migration_busy, |d| {
                d.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(rgb(p.bg))
                        .child(label("正在保存设置并准备迁移，完成后自动重启…", p.text))
                        .on_mouse_down(MouseButton::Left, |_, window, cx| {
                            window.prevent_default();
                            cx.stop_propagation();
                        }),
                )
            })
    }
}

#[cfg(test)]
mod editor_tests;

#[cfg(test)]
mod position_key_tests;
