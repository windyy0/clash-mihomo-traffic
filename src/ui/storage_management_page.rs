use super::*;
use crate::storage_management::{self as management, Preview, Request, Target};
use chrono::{Local, NaiveDate};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Dialog {
    Clean(Target),
    Reclaim,
}
pub(super) struct State {
    pub expanded: bool,
    pub help_expanded: bool,
    pub dialog: Option<Dialog>,
    pub preset: usize,
    pub date: Entity<InputState>,
    pub(super) serial: u64,
    pub(super) epoch: u64,
    pub(super) preview: Option<Preview>,
    pub(super) pending: bool,
    pub(super) submitted: bool,
    pub(super) error: String,
    last: Instant,
    focus: FocusHandle,
    previous: Option<FocusHandle>,
    controls: Vec<FocusHandle>,
    scroll: ScrollHandle,
    pub bounds: Rc<Cell<Bounds<Pixels>>>,
    #[cfg(test)]
    pub cards: [Rc<Cell<Bounds<Pixels>>>; 3],
    #[cfg(test)]
    pub policy_bounds: [[Rc<Cell<Bounds<Pixels>>>; 3]; 2],
}
impl State {
    pub fn new(window: &mut Window, cx: &mut Context<TrafficApp>) -> Self {
        Self {
            expanded: false,
            help_expanded: false,
            dialog: None,
            preset: 1,
            date: input("", "YYYY-MM-DD", false, window, cx),
            serial: 0,
            epoch: 0,
            preview: None,
            pending: false,
            submitted: false,
            error: String::new(),
            last: Instant::now(),
            focus: cx.focus_handle(),
            previous: None,
            controls: (0..4).map(|_| cx.focus_handle()).collect(),
            scroll: ScrollHandle::new(),
            bounds: Rc::new(Cell::new(Bounds::default())),
            #[cfg(test)]
            cards: std::array::from_fn(|_| Rc::new(Cell::new(Bounds::default()))),
            #[cfg(test)]
            policy_bounds: std::array::from_fn(|_| {
                std::array::from_fn(|_| Rc::new(Cell::new(Bounds::default())))
            }),
        }
    }
}
fn date(value: i64) -> String {
    chrono::DateTime::from_timestamp_millis(value)
        .map(|v| {
            v.with_timezone(&Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|| "—".into())
}
impl TrafficApp {
    pub(super) fn refresh_storage(&mut self, cx: &mut Context<Self>) {
        self.backend.refresh_storage();
        self.storage_manager.last = Instant::now();
        cx.notify();
    }
    pub(super) fn poll_storage(&mut self, cx: &mut Context<Self>) {
        if self.page == Page::Settings
            && self.settings_section == SettingsSection::Storage
            && self.storage_manager.last.elapsed() >= Duration::from_secs(60)
        {
            self.refresh_storage(cx);
        }
        let s = self.backend.shared.lock().unwrap();
        if self.storage_manager.dialog.is_some() && self.storage_manager.epoch != s.storage_epoch {
            self.storage_manager.preview = None;
            self.storage_manager.pending = false;
            self.storage_manager.error = "数据库已替换，请关闭后重新打开预览".into();
        } else if self.storage_manager.pending
            && s.storage_management.preview_id == self.storage_manager.serial
        {
            self.storage_manager.preview = s.storage_management.preview.clone();
            self.storage_manager.error = s.storage_management.preview_error.clone();
            self.storage_manager.pending = false;
            cx.notify();
        }
    }
    pub(super) fn open_storage_dialog(
        &mut self,
        dialog: Dialog,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.backend.shared.lock().unwrap().storage_management.busy {
            return;
        }
        self.close_picker(window, cx);
        self.storage_manager.previous = window.focused(cx);
        self.storage_manager.dialog = Some(dialog);
        self.storage_manager.preset = 1;
        self.storage_manager.epoch = self.backend.shared.lock().unwrap().storage_epoch;
        self.storage_manager.error.clear();
        self.storage_manager.submitted = false;
        self.storage_manager.preview = None;
        self.storage_manager
            .scroll
            .set_offset(point(px(0.), px(0.)));
        let text = (Local::now().date_naive() - chrono::Days::new(30)).to_string();
        self.storage_manager
            .date
            .update(cx, |s, cx| s.set_value(text, window, cx));
        self.storage_manager.focus.focus(window);
        if matches!(dialog, Dialog::Clean(_)) {
            self.preview_storage(cx);
        }
        cx.notify();
    }
    pub(super) fn close_storage_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_picker(window, cx);
        self.storage_manager.dialog = None;
        self.storage_manager.pending = false;
        if let Some(focus) = self.storage_manager.previous.take() {
            focus.focus(window);
        } else {
            window.blur();
        }
        cx.notify();
    }
    pub(super) fn preview_storage(&mut self, cx: &mut Context<Self>) {
        let Some(Dialog::Clean(target)) = self.storage_manager.dialog else {
            return;
        };
        self.storage_manager.preview = None;
        self.storage_manager.submitted = false;
        self.storage_manager.error.clear();
        let now = chrono::Utc::now().timestamp_millis();
        let cutoff = match self.storage_manager.preset {
            0 => management::days_before(now, 7).map(Some),
            1 => management::days_before(now, 30).map(Some),
            2 => NaiveDate::parse_from_str(
                self.storage_manager.date.read(cx).value().trim(),
                "%Y-%m-%d",
            )
            .map_err(anyhow::Error::from)
            .and_then(management::midnight)
            .map(Some),
            _ => Ok(None),
        };
        let cutoff = match cutoff {
            Ok(v) => v,
            Err(_) => {
                self.storage_manager.pending = false;
                self.storage_manager.error = "请输入有效的本地日期：YYYY-MM-DD".into();
                cx.notify();
                return;
            }
        };
        self.storage_manager.serial = self.storage_manager.serial.wrapping_add(1);
        self.storage_manager.pending = self.backend.preview_storage(Request::Preview {
            id: self.storage_manager.serial,
            epoch: self.storage_manager.epoch,
            target,
            cutoff,
        });
        if !self.storage_manager.pending {
            self.storage_manager.error = "任务队列忙，请稍后刷新预览".into();
        }
        cx.notify();
    }
    pub(super) fn confirm_storage(&mut self, cx: &mut Context<Self>) {
        if self.storage_manager.submitted
            || self.storage_manager.pending
            || !self.storage_manager.error.is_empty()
        {
            return;
        }
        let request = match self.storage_manager.dialog {
            Some(Dialog::Clean(_)) => {
                let Some(p) = self.storage_manager.preview.clone().filter(|v| v.rows > 0) else {
                    return;
                };
                Request::Clean(p)
            }
            Some(Dialog::Reclaim) => Request::Reclaim {
                epoch: self.storage_manager.epoch,
            },
            None => return,
        };
        match self.backend.submit_storage_job(request) {
            Ok(true) => self.storage_manager.submitted = true,
            Ok(false) => return,
            Err(error) => self.storage_manager.error = error.to_string(),
        }
        cx.notify();
    }
    pub(super) fn storage_keystroke(
        &mut self,
        event: &KeystrokeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.picker.is_some() {
            return self.picker_keystroke(event, window, cx);
        }
        match event.keystroke.key.as_str() {
            "escape" => {
                self.close_storage_dialog(window, cx);
                true
            }
            "tab" => {
                let mut handles = self.storage_manager.controls.clone();
                if self.storage_manager.preset == 2 {
                    handles.push(self.storage_manager.date.read(cx).focus_handle(cx));
                }
                let i = handles.iter().position(|h| h.is_focused(window));
                let n = handles.len();
                let next = if event.keystroke.modifiers.shift {
                    i.map_or(n - 1, |i| (i + n - 1) % n)
                } else {
                    i.map_or(0, |i| (i + 1) % n)
                };
                handles[next].focus(window);
                true
            }
            "enter" | "space" => {
                if event.keystroke.key == "enter"
                    && self
                        .storage_manager
                        .date
                        .read(cx)
                        .focus_handle(cx)
                        .is_focused(window)
                {
                    self.preview_storage(cx);
                    return true;
                }
                match self
                    .storage_manager
                    .controls
                    .iter()
                    .position(|h| h.is_focused(window))
                {
                    Some(0) => self.close_storage_dialog(window, cx),
                    Some(1) => self.act(
                        Action::Storage(StorageAction::StorageRangePicker),
                        window,
                        cx,
                    ),
                    Some(2) => self.preview_storage(cx),
                    Some(3) => self.confirm_storage(cx),
                    _ => return false,
                };
                true
            }
            _ => false,
        }
    }
    pub(super) fn storage_usage(&self, cx: &Context<Self>) -> Div {
        let p = self.p;
        let s = &self.data.storage_management;
        let used = s.snapshot.as_ref().map(|v| v.used());
        let limit = self
            .config
            .storage_limit_enabled
            .then_some(self.config.storage_limit_mb.saturating_mul(1024 * 1024));
        let status = match (used, limit) {
            (Some(used), Some(limit)) if used > limit => "已超出容量上限".into(),
            (Some(used), Some(limit)) => {
                format!("已用 {:.1}%", used as f64 / limit.max(1) as f64 * 100.)
            }
            (None, Some(limit)) => format!("上限 {}", human_bytes(limit)),
            _ => "未设容量上限".into(),
        };
        let mut body=column().gap(px(10.))
            .child(row().justify_between().gap(px(12.))
                .child(heading("存储空间",p))
                .child(self.button("storage-refresh","刷新",false,Action::Storage(StorageAction::StorageRefresh),cx)))
            .child(row().items_end().justify_between().gap(px(12.)).flex_wrap()
                .child(column().gap(px(3.))
                    .child(label(used.map(human_bytes).unwrap_or_else(||"—".into()),p.text).text_size(px(27.)).font_weight(FontWeight::SEMIBOLD))
                    .child(label("数据库有效数据",p.muted).id("storage-effective-help").text_size(px(12.))
                        .tooltip(|window,cx|Tooltip::new("四类数据及所属索引的合计。不含可复用空页、写入日志、备份、导出和普通日志；空表也占少量基础页面。").build(window,cx))))
                .child(label(status,if s.over_limit {p.amber} else {p.muted}).text_size(px(12.))));
        if let Some(v) = &s.snapshot {
            let colors = [p.blue, p.upload, p.accent, p.muted];
            let names = ["秒级明细", "历史详细记录", "长期汇总", "系统与归属数据"];
            let total = limit.unwrap_or(v.used()).max(v.used()).max(1);
            let mut bar = row()
                .gap(px(0.))
                .h(px(6.))
                .w_full()
                .rounded(px(3.))
                .overflow_hidden()
                .bg(state_surface(p));
            for (i, bytes) in v.categories.into_iter().enumerate() {
                if bytes > 0 {
                    bar = bar.child(
                        div()
                            .id(SharedString::from(format!("storage-usage-segment-{i}")))
                            .min_w_0()
                            .flex_none()
                            .w(relative(bytes as f32 / total as f32))
                            .h_full()
                            .bg(rgb(colors[i]))
                            .tooltip(move |window, cx| {
                                Tooltip::new(format!("{} · {}", names[i], human_bytes(bytes)))
                                    .build(window, cx)
                            }),
                    );
                }
            }
            if total > v.used() {
                bar = bar.child(
                    div()
                        .min_w_0()
                        .flex_none()
                        .w(relative((total - v.used()) as f32 / total as f32))
                        .h_full(),
                );
            }
            body = body.child(bar);
        }
        body
    }
    pub(super) fn storage_breakdown(&self, cx: &Context<Self>) -> Div {
        let p = self.p;
        let s = &self.data.storage_management;
        let mut body = column().gap(px(10.));
        if s.refreshing {
            body = body.child(label("正在更新存储占用…", p.muted).text_size(px(12.)));
        }
        if self.storage_manager.expanded {
            body = body.child(
                row()
                    .justify_between()
                    .gap(px(12.))
                    .flex_wrap()
                    .child(
                        label("超限时先清秒级，再清今天以前的历史明细", p.muted)
                            .text_size(px(12.))
                            .whitespace_normal(),
                    )
                    .child(
                        label(
                            s.snapshot
                                .as_ref()
                                .map(|v| {
                                    format!(
                                        "更新于 {}",
                                        chrono::DateTime::from_timestamp_millis(v.at)
                                            .map(|d| d
                                                .with_timezone(&Local)
                                                .format("%H:%M:%S")
                                                .to_string())
                                            .unwrap_or_default()
                                    )
                                })
                                .unwrap_or_else(|| "尚未统计".into()),
                            p.muted,
                        )
                        .text_size(px(12.)),
                    ),
            );
            let categories = [
                (
                    "秒级明细",
                    "全局、原始进程及宿主的原始采样和索引",
                    Some(Target::Seconds),
                ),
                (
                    "历史详细记录",
                    "域名、出站、来源共享的明细、对应事件及索引",
                    Some(Target::Details),
                ),
                (
                    "长期汇总",
                    "总量、应用分钟用量、每日汇总、峰值与均速摘要；保护保留",
                    None,
                ),
                (
                    "系统与归属数据",
                    "元数据、应用标识、采集缺口和其他必需结构；保护保留",
                    None,
                ),
            ];
            let colors = [p.blue, p.upload, p.accent, p.muted];
            let mut list = column().gap(px(0.));
            for (i, (name, hint, target)) in categories.into_iter().enumerate() {
                let value = s
                    .snapshot
                    .as_ref()
                    .map(|v| human_bytes(v.categories[i]))
                    .unwrap_or_else(|| "—".into());
                let hint = if i == 0 {
                    format!(
                        "{}\n最早可查：{}",
                        hint,
                        self.data
                            .seconds_status
                            .first
                            .map(date)
                            .unwrap_or_else(|| "暂无记录".into())
                    )
                } else {
                    hint.to_string()
                };
                let mut item = row()
                    .gap(px(12.))
                    .min_h(px(40.))
                    .child(
                        row()
                            .gap(px(8.))
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .size(px(6.))
                                    .rounded_full()
                                    .flex_none()
                                    .bg(rgb(colors[i])),
                            )
                            .child(
                                label(name, p.text)
                                    .id(SharedString::from(format!("storage-category-{i}")))
                                    .whitespace_normal()
                                    .tooltip(move |window, cx| {
                                        Tooltip::new(hint.clone()).build(window, cx)
                                    }),
                            ),
                    )
                    .child(
                        label(value, p.text)
                            .min_w(px(80.))
                            .text_right()
                            .whitespace_nowrap(),
                    );
                if let Some(target) = target {
                    item = item.child(
                        self.button(
                            if i == 0 {
                                "storage-clean-seconds"
                            } else {
                                "storage-clean-details"
                            },
                            "清理",
                            false,
                            Action::Storage(StorageAction::StorageOpen(Dialog::Clean(target))),
                            cx,
                        )
                        .w(px(64.))
                        .opacity(if s.busy { 0.45 } else { 1. }),
                    );
                } else {
                    item = item.child(
                        label("受保护", p.muted)
                            .w(px(64.))
                            .text_center()
                            .text_size(px(12.)),
                    );
                }
                list = list.child(item);
                if i < 3 {
                    list = list.child(settings_separator(p));
                }
            }
            body = body.child(list);
            if let Some(v) = &s.snapshot {
                let mut files = row()
                    .gap(px(20.))
                    .flex_wrap()
                    .pt(px(12.))
                    .border_t_1()
                    .border_color(surface_border(p));
                for (name, value, hint) in [
                    (
                        "实际文件",
                        v.physical,
                        "数据库主文件与WAL写入日志的合计，与上方有效数据分开计量",
                    ),
                    (
                        "写入日志",
                        v.wal,
                        "待合并到主文件的WAL数据，已包含在实际文件占用中",
                    ),
                    (
                        "可复用空页",
                        v.reusable,
                        "数据库内已释放、可供后续写入复用的页面，不计入有效数据限额",
                    ),
                ] {
                    files = files.child(
                        column()
                            .gap(px(4.))
                            .flex_1()
                            .min_w(px(105.))
                            .child(
                                label(name, p.muted)
                                    .id(SharedString::from(format!("storage-file-{name}")))
                                    .text_size(px(12.))
                                    .tooltip(move |window, cx| {
                                        Tooltip::new(hint).build(window, cx)
                                    }),
                            )
                            .child(label(human_bytes(value), p.text).text_size(px(14.))),
                    );
                }
                body = body.child(files);
            }
            body = body.child(
                row()
                    .justify_between()
                    .gap(px(8.))
                    .flex_wrap()
                    .child(label("清理后文件不一定立即缩小", p.muted).text_size(px(12.)))
                    .child(
                        self.button(
                            "storage-reclaim",
                            "回收文件空间…",
                            false,
                            Action::Storage(StorageAction::StorageOpen(Dialog::Reclaim)),
                            cx,
                        )
                        .opacity(if s.busy { 0.45 } else { 1. }),
                    ),
            );
        }
        if s.over_limit {
            body = body.child(
                label(
                    format!(
                        "有效数据仍超限：{}",
                        if s.limit_note.is_empty() {
                            "正在检查可清理记录…"
                        } else {
                            &s.limit_note
                        }
                    ),
                    p.amber,
                )
                .text_size(px(12.))
                .whitespace_normal(),
            );
        }
        for (text, color) in [
            (&s.job, p.muted),
            (&s.error, p.amber),
            (&s.job_error, p.amber),
        ] {
            if !text.is_empty() {
                body = body.child(
                    label(text.clone(), color)
                        .text_size(px(12.))
                        .whitespace_normal(),
                );
            }
        }
        body
    }
    pub(super) fn storage_overlay(
        &self,
        window: &Window,
        cx: &Context<Self>,
    ) -> Option<Stateful<Div>> {
        let dialog = self.storage_manager.dialog?;
        let measured = self.storage_manager.bounds.clone();
        let p = self.p;
        let st = &self.storage_manager;
        let s = &self.data.storage_management;
        let title = match dialog {
            Dialog::Clean(t) => format!("清理{}", t.label()),
            Dialog::Reclaim => "回收文件空间".into(),
        };
        let mut body = column().gap(px(14.)).w_full().child(
            row()
                .justify_between()
                .gap(px(10.))
                .child(heading(title, p))
                .child(
                    self.button(
                        "storage-dialog-close",
                        "关闭 Esc",
                        false,
                        Action::Storage(StorageAction::StorageClose),
                        cx,
                    )
                    .track_focus(&st.controls[0]),
                ),
        );
        match dialog {
            Dialog::Clean(target) => {
                let titles = ["7天前", "30天前", "自定义日期之前", "全部可清理明细"];
                body = body.child(
                    row()
                        .gap(px(8.))
                        .flex_wrap()
                        .child(
                            div().track_focus(&st.controls[1]).child(
                                self.taskbar_picker(
                                    "storage-clean-range",
                                    titles[st.preset].into(),
                                    titles
                                        .into_iter()
                                        .enumerate()
                                        .map(|(i, t)| {
                                            (
                                                t.into(),
                                                Action::Storage(StorageAction::StoragePreset(i)),
                                                st.preset == i,
                                            )
                                        })
                                        .collect(),
                                    cx,
                                ),
                            ),
                        )
                        .when(st.preset == 2, |d| {
                            d.child(div().w(px(158.)).child(self.text_input(&st.date)))
                        })
                        .child(
                            self.button(
                                "storage-preview",
                                "刷新预览",
                                false,
                                Action::Storage(StorageAction::StoragePreview),
                                cx,
                            )
                            .track_focus(&st.controls[2]),
                        ),
                );
                if st.pending {
                    body = body.child(label("正在统计准确范围与数量…", p.muted));
                }
                if let Some(v) = &st.preview {
                    let range = match (v.first, v.last) {
                        (Some(a), Some(b)) => format!("实际记录范围：{} — {}", date(a), date(b)),
                        _ => "此范围没有可清理的记录".into(),
                    };
                    body = body
                        .child(
                            label(
                                v.cutoff
                                    .map(|t| {
                                        format!("截止：{} 之前（本地时间，不含截止时刻）", date(t))
                                    })
                                    .unwrap_or_else(|| "范围：预览时已保存的全部秒级记录".into()),
                                p.text,
                            )
                            .whitespace_normal(),
                        )
                        .child(label(range, p.muted).whitespace_normal())
                        .child(label(
                            format!(
                                "{} 条记录{}",
                                v.rows,
                                if target == Target::Seconds {
                                    format!(" · {} 个完整采样批次", v.batches)
                                } else {
                                    String::new()
                                }
                            ),
                            p.text,
                        ));
                }
                body=body.child(label(if target==Target::Seconds {"只删除秒级原始采样及应用行；分钟用量、峰值与均速摘要全部保留。采集开启时，清理后仍会产生新记录。"} else {"域名、出站、来源共享明细一并清理；最多清到今天零点，保留当天明细、全部长期汇总及采集缺口。"},p.muted).whitespace_normal());
            }
            Dialog::Reclaim => {
                body=body.child(label("将暂停采集，等待正在执行的数据库任务结束，再整理数据库文件并恢复采集。不会删除有效记录。",p.text).whitespace_normal())
                .child(label("耗时取决于数据量，整理需要额外磁盘空间。暂停期间的明细不能补录；恢复后按既有采集空档逻辑对账，不计入速度统计。失败也会恢复采集。",p.amber).whitespace_normal());
            }
        }
        let disabled = st.pending
            || st.submitted
            || !st.error.is_empty()
            || s.busy
            || matches!(dialog, Dialog::Clean(_))
                && st.preview.as_ref().is_none_or(|v| v.rows == 0);
        if !st.error.is_empty() {
            body = body.child(label(st.error.clone(), p.amber).whitespace_normal());
        }
        if st.submitted {
            body = body
                .child(label(s.job.clone(), p.muted).whitespace_normal())
                .when(!s.job_error.is_empty(), |d| {
                    d.child(label(s.job_error.clone(), p.amber).whitespace_normal())
                });
        }
        body = body.child(
            row().justify_end().child(
                self.button(
                    "storage-confirm",
                    if matches!(dialog, Dialog::Reclaim) {
                        "确认暂停并回收"
                    } else {
                        "确认清理"
                    },
                    true,
                    Action::Storage(StorageAction::StorageConfirm),
                    cx,
                )
                .track_focus(&st.controls[3])
                .opacity(if disabled { 0.4 } else { 1. }),
            ),
        );
        Some(
            div()
                .id("storage-modal-overlay")
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
                        .id("storage-modal")
                        .bg(popup_surface(p))
                        .shadow_md()
                        .relative()
                        .child(
                            canvas(move |bounds, _, _| measured.set(bounds), |_, _, _, _| {})
                                .absolute()
                                .size_full(),
                        )
                        .track_focus(&st.focus)
                        .w((window.viewport_size().width - px(48.)).min(px(760.)))
                        .max_h(window.viewport_size().height - px(72.))
                        .overflow_y_scroll()
                        .track_scroll(&st.scroll)
                        .child(body),
                ),
        )
    }
}
