use super::*;
use crate::floating_backdrop::{FloatingBackdrop, RADIUS, SHADOW_MARGIN};

fn halo(alpha: f32, dark: bool) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let body = bounds.inset(px(SHADOW_MARGIN));
            let scale = window.scale_factor();
            // Separate outward rings leave the translucent material untouched.
            // Every edge uses the same distance; there is no vertical offset.
            for ring in 1..=(SHADOW_MARGIN * scale).ceil() as u32 {
                let distance = ring as f32 / scale;
                let mut color = rgb(0);
                color.a = alpha
                    * if dark { 0.065 } else { 0.05 }
                    * (-0.5 * ((distance - 0.5 / scale) / 2.5).powi(2)).exp();
                window.paint_quad(
                    outline(body.dilate(px(distance)), color, BorderStyle::default())
                        .border_widths(px(1. / scale))
                        .corner_radii(px(RADIUS + distance)),
                );
            }
        },
    )
    .absolute()
    .size_full()
}
use crate::taskbar_text::{ContentFrame, ContentMeasure};

fn readout(frame: &ContentFrame, row_gap: f32, palette: Palette) -> Div {
    column()
        .gap(px(row_gap))
        .w(px(frame.width))
        .flex_none()
        .children(frame.rows.iter().map(|cells| {
            div()
                .relative()
                .w_full()
                .h(px(frame.line_height))
                .flex_none()
                .children(cells.iter().map(|cell| {
                    div()
                        .absolute()
                        .left(px(cell.x))
                        .top_0()
                        .w(px(cell.width))
                        .h(px(frame.line_height))
                        .border_1()
                        .border_color(rgba(0))
                        .rounded(px(4.))
                        .bg(cell.background_color.map(rgb).unwrap_or(rgba(0)))
                        .children(display_editor::cell_segments(
                            cell, frame, 1., palette, None,
                        ))
                }))
        }))
}

pub(super) fn surface_colors(config: &Config, p: Palette, mica_active: bool) -> (Rgba, Rgba) {
    let p = p.floating_window();
    let alpha = 1. - f32::from(config.floating_background_transparency) / 100.;
    let mut bg = rgb(config.floating_background_color.unwrap_or(p.panel));
    bg.a = alpha;
    if mica_active && wants_mica(config) {
        bg = rgba(0);
    }
    let mut border = rgb(p.line);
    border.a = alpha
        * if mica_active && wants_mica(config) {
            0.65
        } else {
            1.
        };
    (bg, border)
}

fn wants_mica(config: &Config) -> bool {
    config.color_theme == ColorTheme::Mica
        && config.floating_background_color.is_none()
        && config.floating_background_transparency == 0
}

fn panel(content: impl IntoElement, config: &Config, p: Palette, mica_active: bool) -> Div {
    let (bg, border) = surface_colors(config, p, mica_active);
    column()
        .gap_0()
        .px(px(17.))
        .py(px(13.))
        .rounded(px(RADIUS))
        .border_1()
        .border_color(border)
        .bg(bg)
        .child(content)
}

fn natural_size(frame: &ContentFrame, gap: f32) -> (f32, f32) {
    (
        frame.width + 36.,
        frame.rows.len() as f32 * frame.line_height
            + frame.rows.len().saturating_sub(1) as f32 * gap
            + 28.,
    )
}

pub struct Floating {
    backend: Backend,
    main: std::sync::Arc<std::sync::atomic::AtomicIsize>,
    pub(super) palette: Palette,
    measure: ContentMeasure,
    pub(super) scroll: ScrollHandle,
    appearance: Option<(bool, bool, u32)>,
    backdrop: Option<FloatingBackdrop>,
    initial_position: Option<native::FloatPosition>,
    pub(super) mica_active: bool,
    vertical: ScrollbarState,
    horizontal: ScrollbarState,
}
impl Floating {
    pub fn new(backend: Backend, main: std::sync::Arc<std::sync::atomic::AtomicIsize>) -> Self {
        let config = backend.config_snapshot();
        let palette = Palette::new(
            config.color_theme,
            config.theme == "dark" || (config.theme == "system" && native::dark_system()),
        );
        Self {
            backend,
            main,
            palette,
            measure: Default::default(),
            scroll: ScrollHandle::new(),
            appearance: None,
            backdrop: None,
            initial_position: Some(
                config
                    .floating_x
                    .zip(config.floating_y)
                    .map_or(native::FloatPosition::Default, |(x, y)| {
                        native::FloatPosition::Saved(x, y)
                    }),
            ),
            mica_active: false,
            vertical: Default::default(),
            horizontal: Default::default(),
        }
    }

    pub(super) fn reset_position(&mut self) {
        self.initial_position = Some(native::FloatPosition::Default);
    }

    #[cfg(test)]
    pub(super) fn test_mica_source(&self, color: u32, dark: bool) -> anyhow::Result<()> {
        self.backdrop
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("floating backdrop is missing"))?
            .test_source_color(color, dark)?;
        Ok(())
    }
}
impl Render for Floating {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let config = self.backend.config_snapshot();
        let data = self.backend.snapshot();
        let frame = self.measure.frame(&config.floating_content, &data);
        let (width, height) = natural_size(&frame, f32::from(config.floating_row_gap));
        let h = native::hwnd(window);
        let appearance = (
            wants_mica(&config),
            self.palette.dark,
            window.scale_factor().to_bits(),
        );
        if self.appearance != Some(appearance) {
            native::titlebar_theme(h, self.palette.dark);
            window.set_background_appearance(WindowBackgroundAppearance::Transparent);
            native::set_mica(h, false);
            native::float_corners(h);
            if !appearance.0 {
                self.backdrop = None;
            } else if self.backdrop.is_none() {
                self.backdrop = FloatingBackdrop::new(h)
                    .map_err(|error| {
                        tracing::warn!(%error, "Rounded floating wallpaper backdrop unavailable");
                        #[cfg(test)]
                        eprintln!("Floating backdrop creation failed: {error:?}");
                    })
                    .ok();
            }
            self.mica_active = self.backdrop.is_some();
            self.appearance = Some(appearance);
        }
        if let Some(backdrop) = &mut self.backdrop {
            let size = window.viewport_size();
            if let Err(error) = backdrop.resize(
                f32::from(size.width),
                f32::from(size.height),
                window.scale_factor(),
                self.palette.dark,
            ) {
                tracing::warn!(%error, "Rounded floating wallpaper backdrop resize failed");
                #[cfg(test)]
                eprintln!("Floating backdrop resize failed: {error:?}");
                self.backdrop = None;
                self.mica_active = false;
            }
        }
        // Resizing while the window is borrowed for rendering loses GPUI's
        // synchronous WM_SIZE callback. Let it update the viewport between frames.
        let initial_position = self
            .initial_position
            .take()
            .unwrap_or(native::FloatPosition::Keep);
        cx.foreground_executor()
            .spawn(async move {
                native::fit_float(
                    h,
                    width + 2. * SHADOW_MARGIN,
                    height + 2. * SHADOW_MARGIN,
                    initial_position,
                );
            })
            .detach();
        let main = self.main.clone();
        let locked = config.floating_locked;
        let contents = div()
            .relative()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .child(
                div()
                    .id("floating-content-scroll")
                    .size_full()
                    .overflow_scroll()
                    .track_scroll(&self.scroll)
                    .child(readout(
                        &frame,
                        f32::from(config.floating_row_gap),
                        self.palette,
                    )),
            )
            .child(controls::scroll_overlay(Scrollbar::horizontal(
                &self.horizontal,
                &self.scroll,
            )))
            .child(controls::scroll_overlay(Scrollbar::vertical(
                &self.vertical,
                &self.scroll,
            )));
        let alpha = 1. - f32::from(config.floating_background_transparency) / 100.;
        let surface = panel(contents, &config, self.palette, self.mica_active)
            .border(px(1. / window.scale_factor()))
            .id("floating-surface")
            .size_full()
            .cursor(if locked {
                CursorStyle::Arrow
            } else {
                CursorStyle::ClosedHand
            })
            .on_mouse_down(MouseButton::Left, move |_, _, _| {
                if !locked {
                    native::drag(h);
                }
            })
            .on_mouse_down(MouseButton::Right, move |_, _, _| {
                native::foreground(main.load(std::sync::atomic::Ordering::Relaxed))
            });
        div()
            .relative()
            .size_full()
            .child(halo(alpha, self.palette.dark))
            .child(div().size_full().p(px(SHADOW_MARGIN)).child(surface))
    }
}

impl TrafficApp {
    pub(super) fn sync_floating_controls(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for (slider, value) in [
            (&self.floating_row_gap, self.config.floating_row_gap),
            (
                &self.floating_transparency,
                self.config.floating_background_transparency,
            ),
        ] {
            if slider.read(cx).value().end() != f32::from(value) {
                slider.update(cx, |state, cx| {
                    state.set_value(f32::from(value), window, cx);
                });
            }
        }
        let current = self.floating_background.read(cx).value();
        let hex = current.trim().strip_prefix('#').unwrap_or(current.trim());
        let valid = current.trim().is_empty()
            || (hex.len() == 6 && hex.bytes().all(|c| c.is_ascii_hexdigit()));
        if valid
            && !self.floating_background_error
            && !self
                .floating_background
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        {
            let value = self
                .config
                .floating_background_color
                .map(|c| format!("#{c:06X}"))
                .unwrap_or_default();
            if !current.eq_ignore_ascii_case(&value) {
                self.floating_background
                    .update(cx, |state, cx| state.set_value(value, window, cx));
            }
        }
    }

    pub(super) fn update_floating_background(&mut self, submitted: bool, cx: &mut Context<Self>) {
        let value = self.floating_background.read(cx).value();
        let value = value.trim();
        let hex = value.strip_prefix('#').unwrap_or(value);
        let color = if value.is_empty() {
            Some(None)
        } else if hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            u32::from_str_radix(hex, 16).ok().map(Some)
        } else {
            None
        };
        self.floating_background_error = submitted && color.is_none();
        if let Some(color) = color
            && color != self.config.floating_background_color
        {
            self.config.floating_background_color = color;
            self.save_display_content(cx);
        }
        cx.notify();
    }

    pub(super) fn floating_toolbar(&self, cx: &Context<Self>) -> Div {
        let content = self.display_content();
        let p = self.p;
        let rows = (1..=13)
            .map(|n| {
                (
                    format!("{n} 行"),
                    Action::Display(DisplayAction::TaskbarRows(n)),
                    content.rows.len() == n,
                )
            })
            .collect();
        column()
            .gap(px(10.))
            .py(px(12.))
            .min_w_0()
            .child(
                row()
                    .gap(px(8.))
                    .child(label("排列", p.muted).text_size(px(12.)))
                    .child(div().w(px(84.)).flex_none().child(self.taskbar_picker(
                        "floating-rows",
                        format!("{} 行", content.rows.len()),
                        rows,
                        cx,
                    )))
                    .child(label("段间距", p.muted).text_size(px(12.)))
                    .child(div().w(px(60.)).child(Slider::new(&self.editor.gap)))
                    .child(
                        label(format!("{} px", content.gap), p.muted)
                            .text_size(px(12.))
                            .w(px(34.)),
                    )
                    .child(label("行间距", p.muted).text_size(px(12.)))
                    .child(div().w(px(60.)).child(Slider::new(&self.floating_row_gap)))
                    .child(
                        label(format!("{} px", self.config.floating_row_gap), p.muted)
                            .text_size(px(12.))
                            .w(px(34.)),
                    )
                    .child(div().flex_1())
                    .child(
                        self.button(
                            "floating-reset-content",
                            "恢复默认",
                            false,
                            Action::Display(DisplayAction::TaskbarReset),
                            cx,
                        )
                        .min_w(px(64.))
                        .px(px(8.))
                        .flex_none(),
                    ),
            )
            .child(self.taskbar_font_controls(cx))
    }

    fn floating_preview(&self, cx: &Context<Self>) -> Stateful<Div> {
        let content = self.editor.spacing.preview_content();
        let content = content.as_ref().unwrap_or(self.display_content());
        let frame = self
            .editor
            .drag
            .as_ref()
            .map(|d| d.frame.clone())
            .unwrap_or_else(|| self.content_frame(content));
        let (width, height) = natural_size(&frame, self.display_row_gap());
        let footer = self.editor.spacing.footer_bounds.clone();
        let card = self.editor.spacing.card_bounds.clone();
        let hover_card = card.clone();
        let app = cx.weak_entity();
        let viewport_height = (height + 14.).min(180.);
        let preview_width = width + if self.editor.editing { 24. } else { 0. };
        let preview_readout = self.taskbar_readout(&frame, self.editor.editing, cx);
        // Transparent content reveals the main window's real Mica at this
        // position, just as the separate panel reveals Mica at its own position.
        let preview = panel(
            preview_readout,
            &self.config,
            self.floating_palette(),
            self.mica_active,
        )
        .w(px(preview_width))
        .flex_none();
        column()
            .id("floating-preview-card")
            .relative()
            .flex_none()
            .w_full()
            .gap(px(6.))
            .p(px(12.))
            .rounded(px(12.))
            .border_1()
            .border_color(surface_border(self.p))
            .child(
                row()
                    .justify_between()
                    .gap(px(8.))
                    .child(label("布局预览", self.p.text).font_weight(FontWeight::MEDIUM))
                    .child(
                        row()
                            .gap(px(8.))
                            .child(
                                self.button(
                                    "floating-copy-taskbar",
                                    "复制任务栏布局",
                                    false,
                                    Action::Display(DisplayAction::CopyTaskbar),
                                    cx,
                                )
                                .px(px(8.)),
                            )
                            .child(
                                self.button(
                                    "floating-edit-toggle",
                                    if self.editor.editing {
                                        "收起编辑"
                                    } else {
                                        "编辑布局"
                                    },
                                    false,
                                    Action::Display(DisplayAction::TaskbarEditing),
                                    cx,
                                )
                                .px(px(8.)),
                            ),
                    ),
            )
            .child(
                div()
                    .relative()
                    .w_full()
                    .h(px(viewport_height))
                    .flex_none()
                    .child(
                        div()
                            .id("floating-preview-scroll")
                            .size_full()
                            .pb(px(14.))
                            .px(px(14.))
                            .overflow_scroll()
                            .track_scroll(&self.editor.preview_scroll)
                            .child(
                                div()
                                    .flex()
                                    .justify_center()
                                    .min_w(relative(1.))
                                    .w(px(preview_width))
                                    .flex_none()
                                    .child(preview),
                            ),
                    )
                    .child(controls::scroll_overlay(Scrollbar::horizontal(
                        &self.editor.preview_scrollbar,
                        &self.editor.preview_scroll,
                    )))
                    .child(controls::scroll_overlay(Scrollbar::vertical(
                        &self.editor.preview_vertical_scrollbar,
                        &self.editor.preview_scroll,
                    ))),
            )
            .children(self.taskbar_drop_candidate().map(|candidate| {
                let frame = self.editor.candidate_measure.frame(&candidate, &self.data);
                column()
                    .gap(px(4.))
                    .child(label("落位预览 · 松手后生效", self.p.accent_text).text_size(px(12.)))
                    .child(
                        div()
                            .id("floating-candidate-scroll")
                            .max_h(px(100.))
                            .overflow_scroll()
                            .child(readout(&frame, self.display_row_gap(), self.p)),
                    )
            }))
            .child(
                row()
                    .relative()
                    .h(px(18.))
                    .justify_end()
                    .when(self.editor.editing, |d| {
                        d.child(self.taskbar_spacing_hint().flex_1().min_w_0())
                    })
                    .child(
                        label(
                            format!(
                                "{} 行 · {} 项",
                                content.rows.len(),
                                content.rows.iter().map(Vec::len).sum::<usize>()
                            ),
                            self.p.muted,
                        )
                        .text_size(px(12.)),
                    )
                    .child(
                        canvas(move |bounds, _, _| footer.set(bounds), |_, _, _, _| {})
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full(),
                    ),
            )
            .when(self.editor.editing && self.editor.drag.is_none(), |d| {
                d.child(taskbar_spacing::SpacingOverlay::new(self, &frame, cx))
            })
            .child(
                canvas(
                    move |bounds, _, _| card.set(bounds),
                    move |_, _, window, _| {
                        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                            if phase == DispatchPhase::Capture {
                                let _ = app.update(cx, |app, cx| {
                                    let hovered = hover_card.get().contains(&event.position);
                                    if hovered != app.editor.spacing.preview_is_hovered() {
                                        app.editor.spacing.set_preview_hovered(hovered);
                                        cx.notify();
                                    }
                                });
                            }
                        });
                    },
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
    }

    pub(super) fn floating_settings(&self, cx: &Context<Self>) -> Div {
        let mut background = self.text_input(&self.floating_background);
        background.invalid = self.floating_background_error;
        let p = self.p;
        let options = column()
            .gap_0()
            .child(settings_separator(p).mt(px(16.)))
            .child(settings_row(
                "整体背景",
                "留空跟随主题",
                row()
                    .gap(px(8.))
                    .child(div().w(px(185.)).child(background))
                    .child(self.button(
                        "floating-background-reset",
                        "跟随主题",
                        false,
                        Action::Display(DisplayAction::FloatingBackgroundReset),
                        cx,
                    )),
                p,
            ))
            .when(self.floating_background_error, |d| {
                d.child(label("请输入六位颜色，如 #3B82F6", p.amber).text_size(px(12.)))
            })
            .child(settings_row(
                "背景透明度",
                "仅调整面板底色和边框",
                row()
                    .gap(px(8.))
                    .child(
                        div()
                            .w(px(140.))
                            .child(Slider::new(&self.floating_transparency)),
                    )
                    .child(
                        label(
                            format!("{}%", self.config.floating_background_transparency),
                            p.muted,
                        )
                        .w(px(36.)),
                    ),
                p,
            ))
            .child(self.toggle(
                "float-lock",
                "锁定位置",
                "开启后避免意外拖动",
                self.config.floating_locked,
                Action::Config("locked"),
                cx,
            ))
            .child(self.button(
                "reset-float",
                "恢复悬浮窗位置",
                false,
                Action::Display(DisplayAction::ResetFloat),
                cx,
            ))
            .child(settings_separator(p))
            .child(self.toggle(
                "fullscreen-hide",
                "全屏时隐藏悬浮窗",
                "离开全屏后自动恢复",
                self.config.fullscreen_hide,
                Action::Config("fullscreen"),
                cx,
            ));
        column()
            .w_full()
            .flex_1()
            .min_h_0()
            .gap_0()
            .child(
                self.toggle(
                    "float-on",
                    "显示悬浮窗",
                    "置顶显示 · 右键打开主界面",
                    self.config.floating,
                    Action::Config("floating"),
                    cx,
                )
                .flex_none(),
            )
            .child(self.floating_preview(cx))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .id("floating-settings-scroll")
                            .h_full()
                            .pr(px(14.))
                            .overflow_y_scroll()
                            .track_scroll(&self.editor.settings_scroll)
                            .child(scroll_fade::FadingContent::new(
                                column()
                                    .gap_0()
                                    .when(self.editor.editing, |d| d.child(self.taskbar_editor(cx)))
                                    .child(options),
                                &self.editor.settings_scroll,
                            )),
                    )
                    .child(scroll_fade::bottom_shade(&self.editor.settings_scroll, p))
                    .child(controls::scroll_overlay(Scrollbar::vertical(
                        &self.editor.settings_scrollbar,
                        &self.editor.settings_scroll,
                    ))),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn floating_surface_alpha_does_not_change_text_or_item_backgrounds() {
        for theme in ColorTheme::ALL {
            for dark in [false, true] {
                let p = Palette::new(theme, dark).with_wallpaper_accent(Some(0x9980CE));
                let mut config = Config::default();
                assert_eq!(
                    surface_colors(&config, p, false).0,
                    rgb(p.floating_window().panel)
                );
                config.color_theme = theme;
                assert_eq!(wants_mica(&config), theme == ColorTheme::Mica);
                assert_eq!(
                    surface_colors(&config, p, true).0,
                    if theme == ColorTheme::Mica {
                        rgba(0)
                    } else {
                        rgb(p.floating_window().panel)
                    }
                );
                config.floating_background_transparency = 50;
                assert!(!wants_mica(&config));
                config.floating_background_transparency = 0;
                config.floating_background_color = Some(0x123456);
                assert!(!wants_mica(&config));
                let before = config.floating_content.clone();
                for transparency in [0, 50, 100] {
                    config.floating_background_transparency = transparency;
                    let (bg, border) = surface_colors(&config, p, true);
                    let mut expected = rgb(0x123456);
                    expected.a = bg.a;
                    assert_eq!(bg, expected);
                    assert_eq!(bg.a, 1. - f32::from(transparency) / 100.);
                    assert_eq!(bg.a, border.a);
                    assert_eq!(config.floating_content, before);
                }
                let frame = ContentMeasure::default().frame(&before, &Shared::default());
                assert_eq!(
                    natural_size(&frame, 8.).1,
                    frame.line_height * 2. + 8. + 28.
                );
            }
        }
    }
}
