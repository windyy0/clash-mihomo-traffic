use super::*;
use crate::release_update::{self, Cache, CheckError, CheckFrequency, Release};
use chrono::{Local, Utc};
use gpui_component::text::TextView;

const FEEDBACK_DURATION: Duration = Duration::from_secs(30);

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

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
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

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) enum CheckAction {
    Check,
    Dismiss { tag: Option<String> },
    Seen,
    Frequency(CheckFrequency),
    Notes { force: bool },
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct CheckSnapshot {
    cache: Cache,
    latest: Fetch,
    manual_feedback: bool,
    current_notes: Fetch,
    revision: u64,
}

pub(super) struct ReleaseChecks {
    dir: std::path::PathBuf,
    cache: Cache,
    latest: Fetch,
    pending_latest: Option<std::sync::mpsc::Receiver<Result<Release, CheckError>>>,
    manual_feedback: bool,
    feedback_deadline: Option<Instant>,
    current_notes: Fetch,
    pending_current_notes: Option<std::sync::mpsc::Receiver<Result<Release, CheckError>>>,
    revision: u64,
}

pub(super) struct State {
    checks: Entity<ReleaseChecks>,
    notes_open: bool,
    notes_opener: FocusHandle,
    notes_controls: [FocusHandle; 4],
    current_notes_scroll: ScrollHandle,
    notes_scroll: ScrollHandle,
    portable: bool,
    install_dir: Option<std::path::PathBuf>,
    installing: bool,
    install_error: Option<String>,
}

impl State {
    pub(super) fn new(checks: Entity<ReleaseChecks>, cx: &mut App) -> Self {
        let portable = crate::config::portable_data_dir().is_some();
        Self {
            checks,
            notes_open: false,
            notes_opener: cx.focus_handle(),
            notes_controls: std::array::from_fn(|_| cx.focus_handle()),
            current_notes_scroll: ScrollHandle::new(),
            notes_scroll: ScrollHandle::new(),
            portable,
            install_dir: if portable {
                None
            } else {
                crate::native::installed_app_dir()
            },
            installing: false,
            install_error: None,
        }
    }

    pub(super) fn has_unread(&self, cx: &App) -> bool {
        self.checks.read(cx).cache.unread_update().is_some()
    }

    fn download_url(&self, cx: &App) -> Option<String> {
        self.checks
            .read(cx)
            .newer_release()?
            .download_url(self.portable)
    }

    fn download_label(&self) -> &'static str {
        if self.installing {
            "正在下载或更新…"
        } else if self.portable {
            "下载并更新"
        } else if self.install_dir.is_some() {
            "下载并安装"
        } else {
            "下载更新"
        }
    }
}

impl ReleaseChecks {
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

    fn dismiss_notice(&mut self, tag: Option<String>) -> bool {
        self.feedback_deadline = None;
        let feedback = std::mem::take(&mut self.manual_feedback);
        if let Some(tag) = tag.filter(|tag| {
            self.cache
                .update_notice()
                .is_some_and(|release| release.tag() == tag)
        }) {
            self.cache.dismissed_tag = Some(tag);
            self.store();
            true
        } else {
            feedback
        }
    }

    fn expire_feedback(&mut self, now: Instant) -> bool {
        if self
            .feedback_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            self.feedback_deadline = None;
            self.manual_feedback = false;
            return true;
        }
        false
    }

    pub(super) fn new(dir: std::path::PathBuf) -> Self {
        let mut checks = Self::mirror(dir);
        checks.cache = Cache::load(&checks.dir);
        checks.latest = checks.cache.last_error.map_or(Fetch::Idle, Fetch::Failed);
        checks.revision = 1;
        checks
    }

    pub(super) fn mirror(dir: std::path::PathBuf) -> Self {
        Self {
            dir,
            cache: Cache::default(),
            latest: Fetch::Idle,
            pending_latest: None,
            manual_feedback: false,
            feedback_deadline: None,
            current_notes: Fetch::Idle,
            pending_current_notes: None,
            revision: 0,
        }
    }

    pub(super) fn revision(&self) -> u64 {
        self.revision
    }

    pub(super) fn tick(&mut self, frequency: Option<CheckFrequency>) -> bool {
        let latest = self.poll_latest();
        let notes = self.poll_current_notes();
        let expired = self.expire_feedback(Instant::now());
        let started = frequency.is_some_and(|frequency| self.check_if_due(frequency));
        let changed = latest || notes || expired || started;
        if changed {
            self.revision += 1;
        }
        changed
    }

    pub(super) fn request(&mut self, action: CheckAction, known: u64) -> Option<CheckSnapshot> {
        self.perform(action);
        self.snapshot(known)
    }

    pub(super) fn snapshot(&self, known: u64) -> Option<CheckSnapshot> {
        (self.revision != known).then(|| CheckSnapshot {
            cache: self.cache.clone(),
            latest: self.latest,
            manual_feedback: self.manual_feedback,
            current_notes: self.current_notes,
            revision: self.revision,
        })
    }

    fn perform(&mut self, action: CheckAction) -> bool {
        let changed = match action {
            CheckAction::Check => self.check_latest(true),
            CheckAction::Dismiss { tag } => self.dismiss_notice(tag),
            CheckAction::Seen => {
                let changed = self.mark_seen();
                if changed {
                    self.store();
                }
                changed
            }
            CheckAction::Frequency(frequency) => self.check_if_due(frequency),
            CheckAction::Notes { force } => {
                let current = (force || self.cache.current_release().is_none())
                    && self.refresh_current_notes();
                let latest = force && self.check_latest(false);
                current || latest
            }
        };
        if changed {
            self.revision += 1;
        }
        changed
    }

    pub(super) fn sync(&mut self, backend: &UiBackend, action: CheckAction) -> bool {
        if !backend.is_remote() {
            return self.perform(action);
        }
        let Some(snapshot) = backend.check_updates(action, self.revision) else {
            return false;
        };
        self.apply_snapshot(snapshot)
    }

    pub(super) fn apply_snapshot(&mut self, snapshot: CheckSnapshot) -> bool {
        if snapshot.revision <= self.revision {
            return false;
        }
        self.cache = snapshot.cache;
        self.latest = snapshot.latest;
        self.manual_feedback = snapshot.manual_feedback;
        self.current_notes = snapshot.current_notes;
        self.revision = snapshot.revision;
        true
    }

    fn newer_release(&self) -> Option<&Release> {
        self.cache
            .latest
            .as_ref()
            .filter(|release| release.is_newer())
    }

    fn store(&self) {
        if let Err(error) = self.cache.save(&self.dir) {
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
    pub(super) fn check_if_due(&mut self, frequency: CheckFrequency) -> bool {
        frequency.is_due(
            Local::now(),
            self.cache
                .last_attempt
                .map(|time| time.with_timezone(&Local)),
        ) && self.check_latest(false)
    }

    fn check_latest(&mut self, manual: bool) -> bool {
        if matches!(self.latest, Fetch::Pending) {
            return false;
        }
        self.manual_feedback = manual;
        self.feedback_deadline = None;
        self.cache.last_attempt = Some(Utc::now());
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        match std::thread::Builder::new()
            .name("release-check".into())
            .spawn(move || {
                let _ = send.send(release_update::fetch_latest());
            }) {
            Ok(_) => {
                self.pending_latest = Some(receive);
                self.latest = Fetch::Pending;
            }
            Err(error) => {
                tracing::warn!(%error, "Unable to start the release check");
                self.cache.last_error = Some(CheckError::Network);
                self.latest = Fetch::Failed(CheckError::Network);
                self.feedback_deadline = manual.then(|| Instant::now() + FEEDBACK_DURATION);
            }
        }
        self.store();
        true
    }

    pub(super) fn poll_latest(&mut self) -> bool {
        let Some(receive) = &self.pending_latest else {
            return false;
        };
        let result = match receive.try_recv() {
            Ok(result) => result,
            Err(std::sync::mpsc::TryRecvError::Empty) => return false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Err(CheckError::Network),
        };
        self.pending_latest = None;
        self.cache.last_error = result.as_ref().err().copied();
        self.latest = match result {
            Ok(release) => {
                self.cache.last_success = Some(Utc::now());
                if release.matches_current() {
                    self.cache.current = Some(release.clone());
                } else if self.cache.current.is_none() {
                    self.cache.current = self.cache.latest.take().filter(Release::matches_current);
                }
                self.cache.latest = Some(release);
                Fetch::Idle
            }
            Err(error) => Fetch::Failed(error),
        };
        self.feedback_deadline = self
            .manual_feedback
            .then(|| Instant::now() + FEEDBACK_DURATION);
        self.store();
        true
    }

    fn refresh_current_notes(&mut self) -> bool {
        if matches!(self.current_notes, Fetch::Pending) {
            return false;
        }
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        match std::thread::Builder::new()
            .name("current-release-notes".into())
            .spawn(move || {
                let _ = send.send(release_update::fetch_current());
            }) {
            Ok(_) => {
                self.pending_current_notes = Some(receive);
                self.current_notes = Fetch::Pending;
            }
            Err(error) => {
                tracing::warn!(%error, "Unable to start the current release notes request");
                self.current_notes = Fetch::Failed(CheckError::Network);
            }
        }
        true
    }

    fn poll_current_notes(&mut self) -> bool {
        let Some(receive) = &self.pending_current_notes else {
            return false;
        };
        let result = match receive.try_recv() {
            Ok(result) => result,
            Err(std::sync::mpsc::TryRecvError::Empty) => return false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Err(CheckError::Network),
        };
        self.pending_current_notes = None;
        self.current_notes = match result {
            Ok(release) => {
                self.cache.current = Some(release);
                self.store();
                Fetch::Idle
            }
            Err(error) => Fetch::Failed(error),
        };
        true
    }
}

impl TrafficApp {
    pub(super) fn mark_update_seen(&self, window: &Window, cx: &mut Context<Self>) {
        if self.page == Page::Settings
            && self.settings_section == SettingsSection::About
            && window.is_window_active()
            && self.updates.checks.read(cx).cache.unread_update().is_some()
        {
            self.apply_check_action(CheckAction::Seen, cx);
        }
    }

    fn apply_check_action(&self, action: CheckAction, cx: &mut Context<Self>) {
        self.updates.checks.update(cx, |checks, cx| {
            if checks.sync(&self.backend, action) {
                cx.notify();
            }
        });
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
            UpdateAction::Check => self.apply_check_action(CheckAction::Check, cx),
            UpdateAction::DismissNotice => {
                let tag = match self.updates.checks.read(cx).notice() {
                    Some(Notice::Update(release)) => Some(release.tag().to_string()),
                    _ => None,
                };
                self.apply_check_action(CheckAction::Dismiss { tag }, cx);
                self.updates.notes_opener.focus(window);
            }
            UpdateAction::Frequency(frequency) => {
                if self.config.update_check_frequency != frequency {
                    self.config.update_check_frequency = frequency;
                    self.save_config();
                    self.apply_check_action(CheckAction::Frequency(frequency), cx);
                }
            }
            UpdateAction::OpenNotes => {
                self.updates.notes_open = true;
                self.updates
                    .current_notes_scroll
                    .set_offset(Point::default());
                self.updates.notes_scroll.set_offset(Point::default());
                self.apply_check_action(CheckAction::Notes { force: false }, cx);
                self.updates.notes_controls[0].focus(window);
            }
            UpdateAction::CloseNotes => {
                self.dismiss_update_notes();
                self.updates.notes_opener.focus(window);
            }
            UpdateAction::RefreshNotes => {
                self.apply_check_action(CheckAction::Notes { force: true }, cx);
            }
            UpdateAction::OpenReleasePage => {
                let checks = self.updates.checks.read(cx);
                let url = checks
                    .newer_release()
                    .or_else(|| checks.cache.current_release())
                    .map_or_else(
                        || release_update::RELEASES_PAGE.to_string(),
                        release_update::Release::page_url,
                    );
                cx.open_url(&url);
            }
            UpdateAction::Download => self.download_update(cx),
        }
    }

    fn download_update(&mut self, cx: &mut Context<Self>) {
        if self.updates.installing {
            return;
        }
        let Some(release) = self.updates.checks.read(cx).newer_release() else {
            return;
        };
        let Some(url) = release.download_url(self.updates.portable) else {
            return;
        };
        let portable = self.updates.portable;
        let install_dir = self.updates.install_dir.clone();
        if !portable && install_dir.is_none() {
            cx.open_url(&url);
            return;
        }
        let release = release.clone();
        let data_dir = self.backend.dir.clone();
        self.updates.install_error = None;
        let (send, receive) = tokio::sync::oneshot::channel();
        if let Err(error) = std::thread::Builder::new()
            .name("update-installer".into())
            .spawn(move || {
                let result = match install_dir {
                    Some(dir) => release_update::download_and_install(&release, &dir),
                    None => release_update::portable::download_and_update(&release, &data_dir),
                };
                let _ = send.send(result);
            })
        {
            tracing::warn!(%error, "Unable to start the installer download");
            self.updates.install_error = Some("无法启动下载，请重试。".into());
            return;
        }
        self.updates.installing = true;
        cx.spawn(async move |this, cx| {
            let result = receive
                .await
                .unwrap_or_else(|_| Err(anyhow::anyhow!("更新任务中断，请重试。")));
            let _ = this.update(cx, |app, cx| {
                app.updates.installing = false;
                if let Err(error) = result {
                    tracing::warn!(%error, "Unable to complete the update installation");
                    app.updates.install_error = Some(error.to_string());
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn update_note_actions(&self, cx: &App) -> [Option<UpdateAction>; 4] {
        [
            Some(UpdateAction::CloseNotes),
            Some(UpdateAction::RefreshNotes),
            Some(UpdateAction::OpenReleasePage),
            (!self.updates.installing && self.updates.download_url(cx).is_some())
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
                    .update_note_actions(cx)
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
        let checks = self.updates.checks.read(cx);
        let checking = matches!(checks.latest, Fetch::Pending);
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
        let notice = checks.notice().map(|notice| {
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
            .min_w_0()
            .max_w_full()
            .ml_auto()
            .gap(px(4.))
            .items_end()
            .child(row().gap(px(8.)).children(notice).child(check))
            .child(
                row()
                    .gap(px(8.))
                    .when(
                        self.updates.installing || self.updates.download_url(cx).is_some(),
                        |d| {
                            d.child(
                                Button::new("about-download-update")
                                    .label(self.updates.download_label())
                                    .custom(self.update_button_style(p.accent_text, cx))
                                    .disabled(self.updates.installing)
                                    .on_click(cx.listener(|app, _, window, cx| {
                                        app.act(Action::Update(UpdateAction::Download), window, cx);
                                    })),
                            )
                        },
                    )
                    .child(notes.flex_none()),
            )
            .when_some(self.updates.install_error.as_ref(), |d, error| {
                d.child(
                    label(error.clone(), p.amber)
                        .text_size(px(12.))
                        .whitespace_normal(),
                )
            })
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
        let actions = self.update_note_actions(cx);
        let button =
            |index: usize, id: &'static str, title: &'static str, active, cx: &Context<Self>| {
                actions[index].map(|action| {
                    self.button(id, title, active, Action::Update(action), cx)
                        .track_focus(&state.notes_controls[index])
                        .focus(move |d| d.bg(state_surface(p)))
                })
            };
        let checks = state.checks.read(cx);
        let current_notes = checks
            .cache
            .current_release()
            .map(|release| release.notes_html().to_string())
            .unwrap_or_default();
        let newer = checks
            .newer_release()
            .map(|release| (release.tag().to_string(), release.notes_html().to_string()));
        let message = match checks.current_notes {
            Fetch::Pending => Some("正在获取当前版本说明…"),
            Fetch::Failed(CheckError::Unavailable) if current_notes.is_empty() => {
                Some("当前版本暂无发行说明。")
            }
            Fetch::Failed(error) => Some(error.message()),
            Fetch::Idle => None,
        };
        let mut version_notes = |tag: &str, notes: String, message: Option<&str>| {
            let section = column()
                .min_w_0()
                .gap(px(8.))
                .child(heading(tag.to_string(), p))
                .when_some(message, |d, message| {
                    d.child(label(message.to_string(), p.muted).whitespace_normal())
                });
            if notes.trim().is_empty() {
                section.when(message.is_none(), |d| {
                    d.child(label("该版本暂无更新说明。", p.muted))
                })
            } else {
                section.child(
                    TextView::html(
                        SharedString::from(format!("about-release-notes-{tag}")),
                        notes,
                        window,
                        cx,
                    )
                    .text_color(rgb(p.text)),
                )
            }
        };
        let current = version_notes(
            concat!("v", env!("CARGO_PKG_VERSION")),
            current_notes,
            message,
        );
        let newer = newer.map(|(tag, notes)| version_notes(&tag, notes, None));
        let has_update = newer.is_some();
        let viewport = window.viewport_size();
        let panel = |title, id, scroll: &ScrollHandle, body| {
            column()
                .min_w_0()
                .gap(px(10.))
                .child(heading(title, p))
                .child(
                    div()
                        .id(id)
                        .min_w_0()
                        .h((viewport.height - px(280.)).clamp(px(80.), px(430.)))
                        .overflow_x_hidden()
                        .overflow_y_scroll()
                        .track_scroll(scroll)
                        .child(body),
                )
        };
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
                        .w(
                            (viewport.width - px(48.)).min(px(if has_update {
                                920.
                            } else {
                                660.
                            })),
                        )
                        .gap(px(12.))
                        .child(
                            row()
                                .justify_between()
                                .child(heading("版本说明", p))
                                .children(button(0, "about-notes-close", "关闭", false, cx)),
                        )
                        .child(settings_separator(p))
                        .child(
                            stretch()
                                .gap(px(16.))
                                .child(
                                    panel(
                                        "当前版本",
                                        "about-current-notes",
                                        &state.current_notes_scroll,
                                        current,
                                    )
                                    .when(has_update, |d| d.flex_none().w(relative(0.34)))
                                    .when(!has_update, |d| d.flex_1()),
                                )
                                .when_some(newer, |d, notes| {
                                    d.child(div().w(px(1.)).flex_none().bg(surface_border(p)))
                                        .child(
                                            panel(
                                                "更新内容",
                                                "about-notes-scroll",
                                                &state.notes_scroll,
                                                notes,
                                            )
                                            .flex_1(),
                                        )
                                }),
                        )
                        .child(settings_separator(p))
                        .when_some(state.install_error.as_ref(), |d, error| {
                            d.child(
                                label(error.clone(), p.amber)
                                    .text_size(px(12.))
                                    .whitespace_normal(),
                            )
                        })
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
                                .children(button(
                                    3,
                                    "about-notes-download",
                                    state.download_label(),
                                    true,
                                    cx,
                                ))
                                .when(state.installing, |d| {
                                    d.child(label(state.download_label(), p.muted))
                                }),
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
                && !matches!(self.updates.checks.read(cx).latest, Fetch::Pending),
            "frequency setting did not respect the latest check"
        );
        self.handle_update_action(UpdateAction::Frequency(CheckFrequency::Never), window, cx);
        let state = &mut self.updates;
        state
            .checks
            .clone()
            .update(cx, |checks, _| -> anyhow::Result<()> {
                let latest = checks.cache.latest.take();
                anyhow::ensure!(
                    checks.notice().is_none(),
                    "an unchecked version showed a notice"
                );
                checks.manual_feedback = true;
                checks.latest = Fetch::Pending;
                anyhow::ensure!(
                    checks.notice().is_none(),
                    "pending check retained a result notice"
                );
                checks.latest = Fetch::Idle;
                checks.cache.latest = checks.cache.current.take();
                anyhow::ensure!(
                    matches!(checks.notice(), Some(Notice::Result("已是最新版")))
                        && checks
                            .newer_release()
                            .and_then(|release| release.download_url(state.portable))
                            .is_none(),
                    "current release incorrectly offered a download"
                );
                checks.cache.current = checks.cache.latest.take();
                checks.cache.latest = latest;
                anyhow::ensure!(
                    matches!(checks.notice(), Some(Notice::Update(_))),
                    "new version was not surfaced"
                );
                checks.latest = Fetch::Failed(CheckError::Network);
                anyhow::ensure!(
                    matches!(checks.notice(), Some(Notice::Failed(CheckError::Network))),
                    "cached result was not distinguished from a failed check"
                );
                checks.latest = Fetch::Idle;
                let tag = checks.newer_release().unwrap().tag().to_string();
                anyhow::ensure!(
                    checks.dismiss_notice(Some(tag))
                        && checks.notice().is_none()
                        && checks.cache.unread_update().is_none(),
                    "dismissed update remained visible"
                );
                checks.cache.dismissed_tag = None;
                let portable = state.portable;
                for (mode, suffix) in [(false, "-setup.exe"), (true, "-portable.zip")] {
                    state.portable = mode;
                    anyhow::ensure!(
                        checks
                            .newer_release()
                            .and_then(|release| release.download_url(state.portable))
                            .is_some_and(|url| url.ends_with(suffix)),
                        "download did not match the package mode"
                    );
                }
                state.portable = portable;
                Ok(())
            })?;
        self.handle_update_action(UpdateAction::OpenNotes, window, cx);
        anyhow::ensure!(
            self.updates.notes_open && self.update_note_actions(cx)[3].is_some(),
            "new release did not offer latest notes and a download"
        );
        let checks = self.updates.checks.read(cx);
        anyhow::ensure!(
            checks
                .cache
                .current_release()
                .is_some_and(Release::matches_current)
                && matches!(checks.current_notes, Fetch::Idle),
            "cached current notes were not available offline"
        );
        self.handle_update_action(UpdateAction::CloseNotes, window, cx);
        anyhow::ensure!(
            !self.updates.notes_open && self.updates.notes_opener.is_focused(window),
            "closing notes did not restore focus"
        );
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn set_fixture_latest_release(
        &self,
        release: Option<Release>,
        cx: &mut Context<Self>,
    ) -> Option<Release> {
        self.updates.checks.update(cx, |checks, cx| {
            cx.notify();
            std::mem::replace(&mut checks.cache.latest, release)
        })
    }

    #[cfg(test)]
    pub(super) fn verify_update_dialog_layout(
        &self,
        window: &Window,
        has_update: bool,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.updates.notes_controls[0].is_focused(window),
            "dialog did not focus its close control"
        );
        anyhow::ensure!(
            self.updates.notes_scroll.max_offset().width <= px(1.)
                && self.updates.current_notes_scroll.max_offset().width <= px(1.),
            "notes overflowed horizontally"
        );
        if !has_update {
            anyhow::ensure!(
                self.updates.current_notes_scroll.bounds().size.width > px(500.)
                    && self.updates.notes_scroll.bounds().size.width == px(0.),
                "current-only notes still reserved a column for another release"
            );
            return Ok(());
        }
        anyhow::ensure!(
            self.updates.notes_scroll.max_offset().height > px(0.),
            "long update notes did not scroll"
        );
        anyhow::ensure!(
            self.updates.current_notes_scroll.bounds().right()
                < self.updates.notes_scroll.bounds().left(),
            "current and new release notes were not side by side"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn manual_check_feedback_expires_without_dismissing_a_known_update() {
        let dir = tempfile::tempdir().unwrap();
        let mut checks = ReleaseChecks::new(dir.path().to_path_buf());
        let current: Release = serde_json::from_value(serde_json::json!({
            "tag": concat!("v", env!("CARGO_PKG_VERSION")), "notes": "", "installer": true, "portable": true
        })).unwrap();
        let mut newer = semver::Version::parse(env!("CARGO_PKG_VERSION")).unwrap();
        newer.patch += 1;
        let newer: Release = serde_json::from_value(serde_json::json!({
            "tag": format!("v{newer}"), "notes": "", "installer": true, "portable": true
        }))
        .unwrap();
        for (cached, result) in [(None, Ok(current)), (Some(newer), Err(CheckError::Network))] {
            checks.cache.latest = cached;
            let (send, receive) = std::sync::mpsc::sync_channel(1);
            checks.manual_feedback = true;
            checks.latest = Fetch::Pending;
            checks.pending_latest = Some(receive);
            let started = Instant::now();
            assert!(!checks.tick(None) && checks.notice().is_none());
            assert!(checks.feedback_deadline.is_none());
            assert!(send.send(result).is_ok());
            assert!(checks.tick(None));
            let deadline = checks.feedback_deadline.unwrap();
            assert!(deadline >= started + Duration::from_secs(30));
            assert!(matches!(
                checks.notice(),
                Some(Notice::Result("已是最新版") | Notice::Failed(CheckError::Network))
            ));
            assert!(!checks.expire_feedback(deadline - Duration::from_millis(1)));
            let revision = checks.revision();
            checks.feedback_deadline = Some(Instant::now());
            assert!(checks.tick(None));
            assert_eq!(checks.revision(), revision + 1);
            assert!(!checks.snapshot(revision).unwrap().manual_feedback);
            if checks.newer_release().is_some() {
                assert!(matches!(checks.notice(), Some(Notice::Update(_))));
                assert!(checks.cache.dismissed_tag.is_none());
            } else {
                assert!(checks.notice().is_none());
            }
        }
    }
}
