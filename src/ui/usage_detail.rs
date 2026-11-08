use super::*;
use apps_page::DetailContext;

const PANE_WIDTH: f32 = 420.;
const DOCK_WIDTH: f32 = 960.;

impl TrafficApp {
    pub(super) fn usage_detail_open(&self) -> bool {
        (self.page == Page::Apps
            || (self.page == Page::History && self.history_state.tab == HistoryTab::Records))
            && self.selected.is_empty()
            && !self.activity.details
            && !self.analysis.active
            && self.usage_detail.key.is_some()
    }

    pub(super) fn usage_detail_docked(&self, window: &Window) -> bool {
        self.usage_detail_open()
            && window.viewport_size().width - self.sidebar_width() - px(48.) >= px(DOCK_WIDTH)
    }

    pub(super) fn active_detail_context(&self) -> Option<DetailContext> {
        if !self.selected.is_empty() {
            return Some(DetailContext {
                key: self.selected.clone(),
                group_apps: !self.activity.selected_process,
            });
        }
        self.usage_detail_open().then(|| DetailContext {
            key: self.usage_detail.key.clone().unwrap(),
            group_apps: self.page != Page::History,
        })
    }

    pub(super) fn open_usage_detail(&mut self, key: String) {
        self.history_state.restore_offset = None;
        if self.usage_detail.key.as_deref() == Some(key.as_str()) && self.usage_detail_open() {
            return;
        }
        let first = !self.usage_detail_open();
        self.expanded_application_path = None;
        self.usage_detail.key = Some(key);
        self.usage_detail.processes_expanded = false;
        self.usage_detail.scroll.set_offset(point(px(0.), px(0.)));
        self.usage_detail.scrollbar = ScrollbarState::default();
        if first {
            self.app_usage_anchor_pending = true;
        }
        self.refresh_detail_query();
    }

    pub(super) fn close_usage_detail(&mut self) {
        if self.usage_detail.key.take().is_some() {
            self.expanded_application_path = None;
            self.pending_detail_query = None;
            self.unsent_detail_query = None;
            self.detail_requested_at = None;
            self.usage_detail.processes_expanded = false;
        }
    }

    pub(super) fn usage_detail_pane(&self, window: &Window, cx: &Context<Self>) -> Stateful<Div> {
        let p = self.p;
        let context = self.active_detail_context().expect("open usage detail");
        let viewport_height = self.page_scroll.bounds().size.height;
        let height = if viewport_height > px(0.) {
            viewport_height - px(32.)
        } else {
            window.viewport_size().height - px(100.)
        }
        .max(px(320.));
        let bounds = self.usage_detail.bounds.clone();
        let content = div()
            .id("usage-detail-scroll")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.usage_detail.scroll)
            .child(
                self.application_detail_content(&context, true, true, cx)
                    .px(px(12.))
                    .pb(px(12.)),
            )
            .on_scroll_wheel(cx.listener(|app, event: &ScrollWheelEvent, _, cx| {
                if app
                    .usage_detail
                    .scrollbar
                    .note_native_scroll(&app.usage_detail.scroll, event.delta)
                {
                    cx.notify();
                }
                cx.stop_propagation();
            }));
        column()
            .id("usage-detail-pane")
            .relative()
            .w(px(PANE_WIDTH))
            .h(height)
            .flex_none()
            .min_h_0()
            .gap(px(0.))
            .overflow_hidden()
            .on_scroll_wheel(cx.listener(|_, _, _, cx| cx.stop_propagation()))
            .when(self.usage_detail_docked(window), |pane| {
                pane.bg(self.card_surface())
            })
            .when(!self.usage_detail_docked(window), |pane| pane.bg(rgb(p.bg)))
            .child(
                canvas(move |area, _, _| bounds.set(area), |_, _, _, _| {})
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(content)
                    .child(controls::scroll_overlay(
                        Scrollbar::vertical(
                            &self.usage_detail.scrollbar,
                            &self.usage_detail.scroll,
                        )
                        .native_wheel()
                        .scrolling_only(true),
                    )),
            )
    }
}
