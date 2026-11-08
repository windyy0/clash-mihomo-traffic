pub(super) use super::history_state::{CalendarAction, HistoryState, HistoryTab, stamp};
use super::*;
impl TrafficApp {
    pub(super) fn history_record_table(&self, window: &Window, cx: &Context<Self>) -> Div {
        let p = self.p;
        let page = self.displayed_page();
        let loading = page.is_none();
        let offset = page.unwrap_or_default() * self.query.page_size;
        let pane = self.usage_detail_docked(window);
        let narrow = pane || window.viewport_size().width - self.sidebar_width() < px(850.);
        let number = if pane {
            70.
        } else if narrow {
            74.
        } else {
            95.
        };
        let speed_width = if pane {
            76.
        } else if narrow {
            100.
        } else {
            124.
        };
        let gap = if pane { 6. } else { 8. };
        let application = self.query.dimension == "app";
        let mut table = column().gap(px(0.)).child(
            row()
                .h(px(36.))
                .gap(px(gap))
                .px(px(8.))
                .child(label("#", p.muted).w(px(26.)))
                .child(
                    label(if application { "应用" } else { "项目" }, p.muted)
                        .flex_1()
                        .min_w_0(),
                )
                .when(application, |d| {
                    d.children(["峰值", "均速"].map(|title| {
                        column()
                            .w(px(speed_width))
                            .flex_none()
                            .items_end()
                            .gap(px(1.))
                            .child(label(title, p.muted).text_size(px(12.)))
                            .child(label("下载／上传", p.muted).text_size(px(10.5)))
                    }))
                })
                .children(
                    [
                        TrafficSort::Download,
                        TrafficSort::Upload,
                        TrafficSort::Total,
                    ]
                    .into_iter()
                    .map(|sort| {
                        self.button(
                            format!("history-sort-{sort:?}"),
                            format!(
                                "{}{}",
                                if sort == TrafficSort::Total {
                                    "合计"
                                } else {
                                    sort.label()
                                },
                                if self.query.sort == sort {
                                    if self.query.ascending { " ↑" } else { " ↓" }
                                } else {
                                    ""
                                }
                            ),
                            false,
                            Action::History(HistoryAction::Sort(sort)),
                            cx,
                        )
                        .w(px(number))
                        .h(px(32.))
                        .px(px(0.))
                        .justify_end()
                    }),
                ),
        );
        if !loading {
            for (index, item) in self.report.rows.iter().enumerate() {
                let key = item.key.clone();
                let app = application;
                let name = if app {
                    app_name(&key).to_string()
                } else {
                    key.clone()
                };
                let tooltip = key.clone();
                let selected = app && self.usage_detail.key.as_deref() == Some(key.as_str());
                let header = row()
                    .id(SharedString::from(format!("history-row-header-{index}")))
                    .h(px(40.))
                    .gap(px(gap))
                    .child(
                        label(format!("{}", offset + index + 1), p.muted)
                            .w(px(26.))
                            .text_size(px(11.)),
                    )
                    .child(
                        row()
                            .flex_1()
                            .min_w_0()
                            .gap(px(6.))
                            .when(app, |d| d.child(self.application_icon(&key, true)))
                            .child(label(name, p.text).flex_1().min_w_0().truncate()),
                    )
                    .when(app, |d| {
                        d.child(self.history_speed_cell(
                            index,
                            self.report.row_speeds.get(&key),
                            false,
                            speed_width,
                        ))
                        .child(self.history_speed_cell(
                            index,
                            self.report.row_speeds.get(&key),
                            true,
                            speed_width,
                        ))
                    })
                    .child(
                        label(human_bytes(item.bytes.down), p.download_text)
                            .w(px(number))
                            .text_right(),
                    )
                    .child(
                        label(human_bytes(item.bytes.up), p.upload_text)
                            .w(px(number))
                            .text_right(),
                    )
                    .child(
                        label(human_bytes(item.bytes.total()), p.text)
                            .w(px(number))
                            .text_right(),
                    )
                    .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx));
                table = table.child(
                    column()
                        .id(SharedString::from(format!("history-row-{index}")))
                        .relative()
                        .gap(px(5.))
                        .px(px(8.))
                        .py(px(8.))
                        .min_w_0()
                        .min_h(px(56.))
                        .rounded(px(6.))
                        .when(selected, |d| d.bg(state_surface(p)))
                        .hover(move |d| d.bg(state_surface(p)))
                        .child(header)
                        .when(app, |d| {
                            d.cursor_pointer()
                                .on_click(cx.listener(move |app, _, window, cx| {
                                    app.act(
                                        Action::Application(ApplicationAction::OpenUsageDetail(
                                            key.clone(),
                                        )),
                                        window,
                                        cx,
                                    )
                                }))
                        }),
                );
            }
        }
        if loading || self.report.rows.is_empty() {
            table = table.child(self.empty(
                if loading {
                    "正在读取记录"
                } else {
                    "没有匹配的记录"
                },
                "可调整时间范围或搜索条件",
            ));
        }
        let bounds = self.app_usage_bounds.clone();
        let records = self
            .card()
            .p(px(16.))
            .relative()
            .flex_1()
            .min_w_0()
            .gap(px(10.))
            .child(
                canvas(move |rect, _, _| bounds.set(rect), |_, _, _, _| {})
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
            )
            .child(
                row()
                    .justify_between()
                    .gap(px(8.))
                    .flex_wrap()
                    .child(
                        row().gap(px(2.)).children(
                            [
                                ("应用", "app"),
                                ("域名", "domain"),
                                ("出站", "route"),
                                ("来源", "source"),
                            ]
                            .into_iter()
                            .map(|(title, dimension)| {
                                self.button(
                                    format!("history-dim-{dimension}"),
                                    title,
                                    self.query.dimension == dimension,
                                    Action::Dimension(dimension),
                                    cx,
                                )
                                .px(px(8.))
                            }),
                        ),
                    )
                    .child(
                        row()
                            .gap(px(12.))
                            .flex_wrap()
                            .when(self.query.dimension == "domain", |d| {
                                d.child(
                                    row()
                                        .gap(px(6.))
                                        .flex_none()
                                        .child(label("合并子域名", p.muted))
                                        .child(self.switch_control(
                                            "history-group-domains",
                                            "合并子域名",
                                            self.query.group_domains,
                                            Action::History(HistoryAction::ToggleDomainGrouping),
                                            cx,
                                        )),
                                )
                            })
                            .child(self.button(
                                "history-export",
                                "导出 CSV",
                                false,
                                Action::Export,
                                cx,
                            )),
                    ),
            )
            .child(self.text_input(&self.search))
            .when(self.report.archive_daily, |d| {
                d.child(
                    label(
                        "此范围包含已归档明细：域名／出站／来源按整日查询，应用保留分钟精度。",
                        p.amber,
                    )
                    .text_size(px(12.))
                    .whitespace_normal(),
                )
            })
            .when(!self.report.error.is_empty(), |d| {
                d.child(label(self.report.error.clone(), p.amber).whitespace_normal())
            })
            .child(table)
            .when(!loading, |d| d.child(self.pagination(cx)));
        row()
            .items_start()
            .gap(px(16.))
            .child(records)
            .when(pane, |d| d.child(self.usage_detail_pane(window, cx)))
    }
    pub(super) fn history_quality(&self, window: &Window, cx: &Context<Self>) -> Div {
        let p = self.p;
        let h = &self.history_state;
        let loading = self.displayed_page().is_none();
        let delta = self.report.total.total() as i128 - self.report.observed.total() as i128;
        let difference = human_bytes(delta.unsigned_abs().min(u64::MAX as u128) as u64);
        let gap_count = self.report.gaps + self.report.crossing_gaps;
        let toggle = |id, title, open, action| {
            self.button(id, title, open, action, cx)
                .h(px(32.))
                .px(px(10.))
                .gap(px(6.))
                .child(
                    svg()
                        .path(if open {
                            "icons/chevron-up.svg"
                        } else {
                            "icons/chevron-down.svg"
                        })
                        .size(px(12.))
                        .text_color(rgb(if open { p.accent_text } else { p.muted })),
                )
        };
        let mut quality = self.card().p(px(20.)).gap(px(18.)).child(
            row()
                .justify_between()
                .gap(px(12.))
                .flex_wrap()
                .child(
                    row()
                        .gap(px(10.))
                        .child(heading("采集情况", p))
                        .child(label("所选时段", p.muted).text_size(px(13.))),
                )
                .child(
                    row()
                        .gap(px(6.))
                        .child(toggle(
                            "history-quality-toggle",
                            "采集说明",
                            h.quality_open,
                            Action::History(HistoryAction::Quality),
                        ))
                        .child(toggle(
                            "history-gaps-toggle",
                            "空档记录",
                            h.gaps_open,
                            Action::History(HistoryAction::Gaps),
                        )),
                ),
        );
        if loading {
            return quality.child(label("正在读取所选时段的采集情况…", p.muted));
        }
        if !h.quality_open {
            quality = quality.child(
                row().gap(px(20.)).flex_wrap().children(
                    [
                        format!(
                            "{} {difference}",
                            if delta >= 0 { "未归属" } else { "待对账" }
                        ),
                        format!("未知进程 {}", human_bytes(self.report.unknown.total())),
                        format!("记录空档 {gap_count} 段"),
                    ]
                    .map(|text| label(text, p.text).text_size(px(13.))),
                ),
            );
        }
        if h.quality_open {
            let (title, detail) = match self.report.history.as_deref() {
                Some(trend) if !trend.observed() => (
                    "所选时段没有采集记录",
                    "空白表示没有采集数据，不代表流量为零。",
                ),
                Some(trend) if trend.complete() => (
                    "已结束时段的记录完整",
                    "当前用量来自已有采样，尚未结束的时段会继续更新。",
                ),
                Some(_) => (
                    "所选时段的记录有空档",
                    "当前用量只包含已记录的部分，不能代表这一时段的完整用量。",
                ),
                None => (
                    "以已记录用量为准",
                    "暂时无法判断此时段是否记录完整；没有记录的部分不按零补算。",
                ),
            };
            let compact = window.viewport_size().width - self.sidebar_width() < px(810.);
            let tile = |title, value, takeaway, detail| {
                column()
                    .flex_1()
                    .min_w_0()
                    .gap(px(12.))
                    .p(px(14.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(control_border(p))
                    .when(compact, |d| d.flex_row().items_start().gap(px(18.)))
                    .child(
                        column()
                            .gap(px(6.))
                            .when(compact, |d| d.w(px(126.)).flex_none())
                            .child(label(title, p.muted).text_size(px(13.)))
                            .child(
                                label(value, p.text)
                                    .text_size(px(22.))
                                    .font_weight(FontWeight::SEMIBOLD),
                            ),
                    )
                    .child(
                        column()
                            .gap(px(6.))
                            .min_w_0()
                            .when(compact, |d| d.flex_1())
                            .child(
                                label(takeaway, p.text)
                                    .text_size(px(14.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .whitespace_normal(),
                            )
                            .child(
                                label(detail, p.muted)
                                    .text_size(px(13.))
                                    .whitespace_normal(),
                            ),
                    )
            };
            quality = quality
                .child(
                    column()
                        .gap(px(8.))
                        .p(px(16.))
                        .rounded(px(8.))
                        .bg(state_surface(p))
                        .child(
                            label(title, p.text)
                                .text_size(px(16.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .whitespace_normal(),
                        )
                        .child(label(detail, p.text).text_size(px(13.)).whitespace_normal()),
                )
                .child(
                    stretch()
                        .gap(px(12.))
                        .when(compact, |d| d.flex_col())
                        .child(tile(
                            if delta >= 0 { "未归属" } else { "待对账" },
                            difference,
                            if delta >= 0 {
                                "暂时无法对应到应用"
                            } else {
                                "应用汇总暂高于总量"
                            },
                            if delta >= 0 {
                                "总量与应用用量的差额，不能还原成具体应用名单。"
                            } else {
                                "应用用量与连续采样总量不一致，保留差额供核对。"
                            },
                        ))
                        .child(tile(
                            "未知进程",
                            human_bytes(self.report.unknown.total()),
                            "有流量，程序未识别",
                            "这部分已计入应用用量，但没有确认程序身份。",
                        ))
                        .child(tile(
                            "范围内恢复",
                            human_bytes(self.report.recovered.total()),
                            "找回用量，归属未知",
                            "仅汇总完整落在所选时段内的空档；无法分配到具体分钟或应用。",
                        )),
                );
        }
        if h.gaps_open {
            quality = quality.child(self.history_gap_records(window, cx));
        }
        quality
    }

    fn history_gap_records(&self, window: &Window, cx: &Context<Self>) -> Div {
        use super::history_state::GAP_PAGE_SIZE;
        let p = self.p;
        let gap_count = self.report.gaps + self.report.crossing_gaps;
        let rows = self.report.gap_rows.len();
        let page = self.history_state.gap_page(&self.query, rows);
        let pages = rows.div_ceil(GAP_PAGE_SIZE).max(1);
        let start = page * GAP_PAGE_SIZE;
        let end = (start + GAP_PAGE_SIZE).min(rows);
        let compact = window.viewport_size().width - self.sidebar_width() < px(810.);
        let duration_width = if compact { 80. } else { 96. };
        let recovery_width = if compact { 96. } else { 116. };
        let result_width = if compact { 136. } else { 168. };
        let mut records = column()
            .gap(px(16.))
            .pt(px(18.))
            .border_t_1()
            .border_color(control_border(p))
            .child(
                row()
                    .justify_between()
                    .flex_wrap()
                    .gap(px(10.))
                    .child(heading("空档记录", p))
                    .child(
                        label(
                            format!("所选时段共 {gap_count} 段 · 最近 {rows} 段可查看"),
                            p.muted,
                        )
                        .text_size(px(13.)),
                    ),
            )
            .child(
                label(
                    "先看持续时间，再看恢复量。短暂的采样延迟也会留下空档，条数不代表故障次数。",
                    p.text,
                )
                .text_size(px(14.))
                .whitespace_normal(),
            );
        if rows == 0 {
            return column()
                .gap(px(10.))
                .pt(px(18.))
                .border_t_1()
                .border_color(control_border(p))
                .child(heading("没有空档记录", p))
                .child(
                    label(
                        "此时段没有保存空档事件；是否记录完整，请结合上方采集说明查看。",
                        p.muted,
                    )
                    .whitespace_normal(),
                );
        }
        records = records.child(
            row()
                .gap(px(12.))
                .pb(px(4.))
                .child(label("空档时段", p.muted).flex_1().min_w_0())
                .child(label("持续时间", p.muted).w(px(duration_width)).flex_none())
                .child(
                    label("恢复量", p.muted)
                        .w(px(recovery_width))
                        .flex_none()
                        .text_right(),
                )
                .child(label("记录结果", p.muted).w(px(result_width)).flex_none()),
        );
        let mut list = column().gap(px(0.));
        for (index, gap) in self
            .report
            .gap_rows
            .iter()
            .enumerate()
            .take(end)
            .skip(start)
        {
            let recovered = gap.bytes.total();
            let crossing = gap.from < self.query.from || gap.to > self.query.to;
            let (date, time) = gap_interval(gap);
            let restored = gap.reason == "跨缺口恢复：时间和应用归属不可确定";
            let (result, note) = if restored || recovered > 0 {
                ("已恢复计数", "时刻和应用未知")
            } else if gap.reason == "内核变化，缺口不可恢复" {
                ("内核计数变化", "无法恢复此段用量")
            } else {
                ("未取得恢复量", "具体情况见原始记录")
            };
            let tooltip = format!(
                "{}\n{}",
                gap.reason,
                if crossing {
                    "此段跨越所选时段边界，展示整段恢复量，不计入“范围内恢复”。"
                } else {
                    "恢复量是从内核找回的计数，不代表丢失量，也不会均摊到分钟或应用。"
                }
            );
            list = list.child(
                row()
                    .id(SharedString::from(format!("history-gap-row-{index}")))
                    .gap(px(12.))
                    .py(px(14.))
                    .min_h(px(76.))
                    .border_t_1()
                    .border_color(control_border(p))
                    .child(
                        column()
                            .gap(px(5.))
                            .flex_1()
                            .min_w_0()
                            .child(label(date, p.muted).text_size(px(12.)))
                            .child(label(time, p.text).text_size(px(14.)).whitespace_normal()),
                    )
                    .child(
                        label(gap_duration(gap), p.text)
                            .text_size(px(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .w(px(duration_width))
                            .flex_none(),
                    )
                    .child(
                        label(
                            human_bytes(recovered),
                            if recovered > 0 { p.amber } else { p.muted },
                        )
                        .text_size(px(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .w(px(recovery_width))
                        .flex_none()
                        .text_right(),
                    )
                    .child(
                        column()
                            .gap(px(5.))
                            .w(px(result_width))
                            .flex_none()
                            .child(label(result, p.text).text_size(px(13.)))
                            .child(
                                label(
                                    if crossing {
                                        "跨所选时段边界"
                                    } else {
                                        note
                                    },
                                    p.muted,
                                )
                                .text_size(px(12.)),
                            ),
                    )
                    .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx)),
            );
        }
        records.child(list)
            .child(label("恢复量是找回的计数，无法还原到具体分钟或应用。跨所选时段边界的记录显示整段恢复量，不计入“范围内恢复”。", p.muted).text_size(px(13.)).whitespace_normal())
            .child(row().justify_between().gap(px(12.)).flex_wrap()
                .child(label(format!("显示 {start_display}–{end} 段 · 第 {page_display} / {pages} 页", start_display = start + 1, page_display = page + 1), p.muted).text_size(px(13.)))
                .child(row().gap(px(8.))
                    .when(page > 0, |d| d.child(self.button("history-gaps-prev", "上一页", false, Action::History(HistoryAction::GapPage(-1)), cx)))
                    .when(page + 1 < pages, |d| d.child(self.button("history-gaps-next", "下一页", false, Action::History(HistoryAction::GapPage(1)), cx)))))
    }
}

fn gap_interval(gap: &crate::model::Gap) -> (String, String) {
    let local = |time| {
        chrono::DateTime::from_timestamp_millis(time).map(|date| date.with_timezone(&chrono::Local))
    };
    if let (Some(from), Some(to)) = (local(gap.from), local(gap.to)) {
        let date = if from.date_naive() == to.date_naive() {
            from.format("%Y-%m-%d").to_string()
        } else {
            format!("{} → {}", from.format("%Y-%m-%d"), to.format("%Y-%m-%d"))
        };
        (
            date,
            format!("{} – {}", from.format("%H:%M:%S"), to.format("%H:%M:%S")),
        )
    } else {
        (stamp(gap.from), stamp(gap.to))
    }
}

fn gap_duration(gap: &crate::model::Gap) -> String {
    let seconds = (gap.to.saturating_sub(gap.from).max(0) as u64).div_ceil(1000);
    match seconds {
        0..60 => format!("{seconds} 秒"),
        60..3600 => format!("{}分{}秒", seconds / 60, seconds % 60),
        3600..86400 => format!("{}小时{}分", seconds / 3600, seconds % 3600 / 60),
        _ => format!("{}天{}小时", seconds / 86400, seconds % 86400 / 3600),
    }
}
