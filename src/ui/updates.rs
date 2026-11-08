use super::*;
use crate::release_update::{self, Cache, CheckError, CheckFrequency, Release};
use chrono::{Local, Utc};
use gpui_component::text::TextView;

#[derive(Clone, Copy)]
pub(super) enum UpdateAction {
    Check,
    DismissNotice,
    Frequency(CheckFrequency),
    OpenNotes,
    CloseNotes,
    RefreshNotes,
    OpenReleasePage,
    Download,
}

#[derive(Clone, Copy)]
enum Fetch {
    Idle,
    Pending,
    Failed(CheckError),
}

enum Notice<'a> {
    Update(&'a Release),
    Result(&'static str),
    Failed(CheckError),
}

pub(super) struct State {
    cache: Cache,
    latest: Fetch,
    manual_feedback: bool,
    history: Fetch,
    notes_open: bool,
    notes_opener: FocusHandle,
    notes_controls: [FocusHandle; 4],
    notes_scroll: ScrollHandle,
    portable: bool,
}

impl State {
    pub(super) fn new(dir: &std::path::Path, cx: &mut App) -> Self {
        let cache = Cache::load(dir);
        Self {
            latest: cache.last_error.map_or(Fetch::Idle, Fetch::Failed),
            cache,
            manual_feedback: false,
            history: Fetch::Idle,
            notes_open: false,
            notes_opener: cx.focus_handle(),
            notes_controls: std::array::from_fn(|_| cx.focus_handle()),
            notes_scroll: ScrollHandle::new(),
            portable: crate::config::portable_data_dir().is_some(),
        }
    }

    pub(super) fn has_unread(&self) -> bool {
        self.cache.unread_update().is_some()
    }

    fn newer_release(&self) -> Option<&Release> {
        self.cache
            .latest
            .as_ref()
            .filter(|release| release.is_newer())
    }

    fn download_url(&self) -> Option<String> {
        self.newer_release()?.download_url(self.portable)
    }

    fn notice(&self) -> Option<Notice<'_>> {
        match self.latest {
            Fetch::Pending => None,
            Fetch::Failed(error) if self.manual_feedback => Some(Notice::Failed(error)),
            _ => self.cache.update_notice().map(Notice::Update).or_else(|| {
                self.manual_feedback.then(|| {
                    Notice::Result(if self.newer_release().is_some() {
                        "检查完成"
                    } else {
                        "已是最新版"
                    })
                })
            }),
        }
    }

    fn dismiss_notice(&mut self) -> bool {
        let dismissed = match self.notice() {
            Some(Notice::Update(release)) => Some(release.tag().to_string()),
            _ => None,
        };
        self.manual_feedback = false;
        if let Some(tag) = dismissed {
            self.cache.dismissed_tag = Some(tag);
            true
        } else {
            false
        }
    }

    fn store(&self, dir: &std::path::Path) {
        if let Err(error) = self.cache.save(dir) {
            tracing::warn!(%error, "Unable to cache the release check");
        }
    }

    fn mark_seen(&mut self) -> bool {
        if let Some(release) = self.cache.unread_update() {
            self.cache.seen_tag = Some(release.tag().to_string());
            true
        } else {
            false
        }
    }
}

impl TrafficApp {
    pub(super) fn poll_update_schedule(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.page == Page::Settings
            && self.settings_section == SettingsSection::About
            && window.is_window_active()
            && self.updates.mark_seen()
        {
            self.updates.store(&self.backend.dir);
            cx.notify();
        }
        if self.tick == 1 || self.tick.is_multiple_of(10) {
            self.check_update_if_due(cx);
        }
    }

    fn check_update_if_due(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.updates.latest, Fetch::Pending)
            && self.config.update_check_frequency.is_due(
                Local::now(),
                self.updates
                    .cache
                    .last_attempt
                    .map(|time| time.with_timezone(&Local)),
            )
        {
            self.check_update(false, cx);
        }
    }

    fn check_update(&mut self, manual: bool, cx: &mut Context<Self>) {
        if matches!(self.updates.latest, Fetch::Pending) {
            return;
        }
        self.updates.manual_feedback = manual;
        self.updates.latest = Fetch::Pending;
        self.updates.cache.last_attempt = Some(Utc::now());
        self.updates.store(&self.backend.dir);
        let fetch = cx
            .background_executor()
            .spawn(async { release_update::fetch_latest() });
        cx.spawn(async move |this, cx| {
            let result = fetch.await;
            let _ = this.update(cx, |app, cx| {
                let cache = &mut app.updates.cache;
                cache.last_error = result.as_ref().err().copied();
                app.updates.latest = match result {
                    Ok(release) => {
                        cache.last_success = Some(Utc::now());
                        let previous = cache.latest.replace(release);
                        if cache.current.is_none() {
                            cache.current = previous.filter(Release::matches_current);
                        }
                        Fetch::Idle
                    }
                    Err(error) => Fetch::Failed(error),
                };
                app.updates.store(&app.backend.dir);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn refresh_update_history(&mut self, cx: &mut Context<Self>) {
        if matches!(self.updates.history, Fetch::Pending) {
            return;
        }
        self.updates.history = Fetch::Pending;
        let fetch = cx
            .background_executor()
            .spawn(async { release_update::fetch_history() });
        cx.spawn(async move |this, cx| {
            let result = fetch.await;
            let _ = this.update(cx, |app, cx| {
                app.updates.history = match result {
                    Ok(releases) => {
                        app.updates.cache.history = Some(releases);
                        app.updates.store(&app.backend.dir);
                        Fetch::Idle
                    }
                    Err(error) => Fetch::Failed(error),
                };
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn update_notes_is_open(&self) -> bool {
        self.updates.notes_open
    }

    pub(super) fn dismiss_update_notes(&mut self) {
        self.updates.notes_open = false;
    }

    pub(super) fn handle_update_action(
        &mut self,
        action: UpdateAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            UpdateAction::Check => self.check_update(true, cx),
            UpdateAction::DismissNotice => {
                if self.updates.dismiss_notice() {
                    self.updates.store(&self.backend.dir);
                }
                self.updates.notes_opener.focus(window);
            }
            UpdateAction::Frequency(frequency) => {
                if self.config.update_check_frequency != frequency {
                    self.config.update_check_frequency = frequency;
                    self.save_config();
                    self.check_update_if_due(cx);
                }
            }
            UpdateAction::OpenNotes => {
                self.updates.notes_open = true;
                self.updates.notes_scroll.set_offset(Point::default());
                if self.updates.cache.history.is_none()
                    && matches!(self.updates.history, Fetch::Idle)
                {
                    self.refresh_update_history(cx);
                }
                self.updates.notes_controls[0].focus(window);
            }
            UpdateAction::CloseNotes => {
                self.dismiss_update_notes();
                self.updates.notes_opener.focus(window);
            }
            UpdateAction::RefreshNotes => self.refresh_update_history(cx),
            UpdateAction::OpenReleasePage => {
                let url = self
                    .updates
                    .newer_release()
                    .filter(|_| self.updates.download_url().is_none())
                    .map_or_else(
                        || release_update::RELEASES_PAGE.to_string(),
                        release_update::Release::page_url,
                    );
                cx.open_url(&url);
            }
            UpdateAction::Download => {
                if let Some(url) = self.updates.download_url() {
                    cx.open_url(&url);
                }
            }
        }
    }

    fn update_note_actions(&self) -> [Option<UpdateAction>; 4] {
        [
            Some(UpdateAction::CloseNotes),
            Some(UpdateAction::RefreshNotes),
            Some(UpdateAction::OpenReleasePage),
            self.updates
                .download_url()
                .is_some()
                .then_some(UpdateAction::Download),
        ]
    }

    pub(super) fn update_notes_keystroke(
        &mut self,
        event: &KeystrokeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.updates.notes_open {
            return false;
        }
        // GPUI activates the focused click target on key-up.
        match event.keystroke.key.as_str() {
            "escape" => self.act(Action::Update(UpdateAction::CloseNotes), window, cx),
            "tab" => {
                let visible: Vec<_> = self
                    .update_note_actions()
                    .iter()
                    .zip(&self.updates.notes_controls)
                    .filter_map(|(action, handle)| action.map(|_| handle))
                    .collect();
                let count = visible.len();
                let focused = visible.iter().position(|handle| handle.is_focused(window));
                let next = if event.keystroke.modifiers.shift {
                    focused.map_or(count - 1, |i| (i + count - 1) % count)
                } else {
                    focused.map_or(0, |i| (i + 1) % count)
                };
                visible[next].focus(window);
            }
            _ => return false,
        }
        true
    }

    fn update_button_style(&self, foreground: u32, cx: &Context<Self>) -> ButtonCustomVariant {
        let p = self.p;
        ButtonCustomVariant::new(cx)
            .color(control_surface(p).into())
            .border(control_border(p).into())
            .foreground(rgb(foreground).into())
            .hover(state_surface(p).into())
            .active(state_surface(p).into())
    }

    pub(super) fn update_header_actions(&self, cx: &Context<Self>) -> Div {
        let p = self.p;
        let checking = matches!(self.updates.latest, Fetch::Pending);
        let check = Button::new("about-check-update")
            .label(if checking {
                "正在检查…"
            } else {
                "检查更新"
            })
            .custom(self.update_button_style(p.text, cx))
            .disabled(checking)
            .on_click(cx.listener(|app, _, window, cx| {
                app.act(Action::Update(UpdateAction::Check), window, cx)
            }));
        let notes = div()
            .id("about-update-notes")
            .flex()
            .items_center()
            .h(px(28.))
            .px(px(8.))
            .cursor_pointer()
            .text_size(px(13.))
            .text_color(rgb(p.muted))
            .hover(move |d| d.text_color(rgb(p.accent_text)))
            .child("更新内容")
            .track_focus(&self.updates.notes_opener)
            .tab_index(0)
            .when(self.keyboard_navigation, |d| {
                d.focus(move |d| d.text_color(rgb(p.accent_text)).underline())
            })
            .on_click(cx.listener(|app, _, window, cx| {
                app.act(Action::Update(UpdateAction::OpenNotes), window, cx);
            }));
        let notice = self.updates.notice().map(|notice| {
            let (text, color) = match notice {
                Notice::Update(release) => (format!("发现新版本 {}", release.tag()), p.accent_text),
                Notice::Result(text) => (text.to_string(), p.muted),
                Notice::Failed(error) => (error.message().to_string(), p.amber),
            };
            row()
                .gap(px(4.))
                .min_w_0()
                .child(label(text, color).text_size(px(12.)).whitespace_normal())
                .child(
                    Button::new("dismiss-update-notice")
                        .label("×")
                        .ghost()
                        .tooltip("关闭更新提示")
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.act(Action::Update(UpdateAction::DismissNotice), window, cx);
                        })),
                )
        });
        column()
            .gap(px(4.))
            .items_end()
            .child(check)
            .child(row().gap(px(8.)).children(notice).child(notes.flex_none()))
    }

    pub(super) fn update_frequency_row(&self, cx: &Context<Self>) -> Div {
        let frequency = self.config.update_check_frequency;
        row()
            .justify_between()
            .child(label("检查频率", self.p.text))
            .child(
                div().w(px(132.)).child(
                    self.taskbar_picker(
                        "update-check-frequency",
                        frequency.label().into(),
                        CheckFrequency::ALL
                            .into_iter()
                            .map(|choice| {
                                (
                                    choice.label().into(),
                                    Action::Update(UpdateAction::Frequency(choice)),
                                    choice == frequency,
                                )
                            })
                            .collect(),
                        cx,
                    ),
                ),
            )
    }

    pub(super) fn update_notes_overlay(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Stateful<Div>> {
        let state = &self.updates;
        if !state.notes_open {
            return None;
        }
        let p = self.p;
        let actions = self.update_note_actions();
        let button =
            |index: usize, id: &'static str, title: &'static str, active, cx: &Context<Self>| {
                actions[index].map(|action| {
                    self.button(id, title, active, Action::Update(action), cx)
                        .track_focus(&state.notes_controls[index])
                        .focus(move |d| d.bg(state_surface(p)))
                })
            };
        let releases: Vec<_> = state.cache.releases().collect();
        let message = match state.history {
            Fetch::Pending => Some("正在获取发行记录…"),
            Fetch::Failed(error) => Some(error.message()),
            Fetch::Idle if releases.is_empty() => Some("暂无更新说明。"),
            Fetch::Idle => None,
        };
        let body = column()
            .gap(px(16.))
            .min_w_0()
            .when_some(message, |d, message| {
                d.child(label(message, p.muted).whitespace_normal())
            })
            .children(releases.into_iter().enumerate().map(|(index, release)| {
                let notes = release.notes_html();
                column()
                    .min_w_0()
                    .gap(px(8.))
                    .when(index != 0, |d| d.child(settings_separator(p)))
                    .child(
                        row()
                            .gap(px(8.))
                            .child(heading(release.tag().to_string(), p))
                            .when(release.matches_current(), |d| {
                                d.child(label("当前版本", p.muted).text_size(px(12.)))
                            }),
                    )
                    .when(notes.trim().is_empty(), |d| {
                        d.child(label("该版本暂无更新说明。", p.muted))
                    })
                    .when(!notes.trim().is_empty(), |d| {
                        d.child(
                            TextView::html(
                                SharedString::from(format!(
                                    "about-release-notes-{}",
                                    release.tag()
                                )),
                                notes.to_string(),
                                window,
                                cx,
                            )
                            .text_color(rgb(p.text)),
                        )
                    })
            }));
        let viewport = window.viewport_size();
        Some(
            div()
                .id("update-notes-overlay")
                .absolute()
                .inset_0()
                .occlude()
                .bg(rgba(if p.dark { 0x00000070 } else { 0x00000038 }))
                .flex()
                .items_center()
                .justify_center()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                .child(
                    self.card()
                        .id("update-notes-dialog")
                        .bg(popup_surface(p))
                        .shadow_md()
                        .w((viewport.width - px(48.)).min(px(660.)))
                        .gap(px(12.))
                        .child(
                            row()
                                .justify_between()
                                .child(heading("更新内容", p))
                                .children(button(0, "about-notes-close", "关闭", false, cx)),
                        )
                        .child(settings_separator(p))
                        .child(
                            div()
                                .id("about-notes-scroll")
                                .w_full()
                                .min_w_0()
                                .h((viewport.height - px(250.)).clamp(px(80.), px(430.)))
                                .overflow_x_hidden()
                                .overflow_y_scroll()
                                .track_scroll(&state.notes_scroll)
                                .child(body),
                        )
                        .child(settings_separator(p))
                        .child(
                            row()
                                .justify_end()
                                .flex_wrap()
                                .children(button(1, "about-refresh-notes", "刷新", false, cx))
                                .children(button(
                                    2,
                                    "about-release-page",
                                    "在 GitHub 查看",
                                    false,
                                    cx,
                                ))
                                .children(button(3, "about-notes-download", "下载更新", true, cx)),
                        ),
                ),
        )
    }

    #[cfg(test)]
    pub(super) fn exercise_update_settings(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        let frequency = self.config.update_check_frequency;
        anyhow::ensure!(
            frequency == CheckFrequency::Weekly,
            "fresh update setting did not default to weekly"
        );
        self.handle_update_action(UpdateAction::Frequency(CheckFrequency::Monthly), window, cx);
        anyhow::ensure!(
            self.config.update_check_frequency == CheckFrequency::Monthly
                && !matches!(self.updates.latest, Fetch::Pending),
            "frequency setting did not respect the latest check"
        );
        self.handle_update_action(UpdateAction::Frequency(CheckFrequency::Never), window, cx);
        let state = &mut self.updates;
        let latest = state.cache.latest.take();
        anyhow::ensure!(
            state.notice().is_none(),
            "an unchecked version showed a notice"
        );
        state.manual_feedback = true;
        state.latest = Fetch::Pending;
        anyhow::ensure!(
            state.notice().is_none(),
            "pending check retained a result notice"
        );
        state.latest = Fetch::Idle;
        state.cache.latest = state.cache.current.take();
        anyhow::ensure!(
            matches!(state.notice(), Some(Notice::Result("已是最新版")))
                && state.download_url().is_none(),
            "current release incorrectly offered a download"
        );
        state.cache.current = state.cache.latest.take();
        state.cache.latest = latest;
        anyhow::ensure!(
            matches!(state.notice(), Some(Notice::Update(_))),
            "new version was not surfaced"
        );
        state.latest = Fetch::Failed(CheckError::Network);
        anyhow::ensure!(
            matches!(state.notice(), Some(Notice::Failed(CheckError::Network))),
            "cached result was not distinguished from a failed check"
        );
        state.latest = Fetch::Idle;
        anyhow::ensure!(
            state.dismiss_notice() && state.notice().is_none() && !state.has_unread(),
            "dismissed update remained visible"
        );
        state.cache.dismissed_tag = None;
        let portable = state.portable;
        for (mode, suffix) in [(false, "-setup.exe"), (true, "-portable.zip")] {
            state.portable = mode;
            anyhow::ensure!(
                state
                    .download_url()
                    .is_some_and(|url| url.ends_with(suffix)),
                "download did not match the package mode"
            );
        }
        state.portable = portable;
        self.handle_update_action(UpdateAction::OpenNotes, window, cx);
        anyhow::ensure!(
            self.updates.notes_open && self.update_note_actions()[3].is_some(),
            "new release did not offer latest notes and a download"
        );
        anyhow::ensure!(
            self.updates.cache.releases().any(Release::matches_current)
                && self.updates.cache.releases().count() == 3
                && matches!(self.updates.history, Fetch::Idle),
            "cached history or current notes were not available offline"
        );
        self.handle_update_action(UpdateAction::CloseNotes, window, cx);
        anyhow::ensure!(
            !self.updates.notes_open && self.updates.notes_opener.is_focused(window),
            "closing notes did not restore focus"
        );
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn verify_update_dialog_layout(&self, window: &Window) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.updates.notes_controls[0].is_focused(window),
            "dialog did not focus its close control"
        );
        anyhow::ensure!(
            self.updates.notes_scroll.max_offset().height > px(0.),
            "long update notes did not scroll"
        );
        anyhow::ensure!(
            self.updates.notes_scroll.max_offset().width <= px(1.),
            "notes overflowed horizontally"
        );
        Ok(())
    }
}
