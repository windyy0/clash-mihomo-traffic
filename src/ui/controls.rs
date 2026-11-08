use super::*;

pub(super) fn control_surface(p: Palette) -> Rgba {
    let mut tint = rgb(p.accent);
    if p.mica {
        // Let the wallpaper-tinted Mica remain visible through the control.
        tint.a = if p.dark { 0.075 } else { 0.035 };
        return tint;
    }
    tint.a = if p.dark { 0.08 } else { 0.04 };
    rgb(p.bg).blend(tint)
}

pub(super) fn popup_surface(p: Palette) -> Rgba {
    if p.mica {
        // Resolve a bright pastel from the theme hue instead of mixing the
        // contrast-adjusted accent into gray, which makes large menus muddy.
        let mut surface: Hsla = rgb(p.accent).into();
        surface.s =
            (surface.s * if p.dark { 0.35 } else { 0.55 }).min(if p.dark { 0.13 } else { 0.30 });
        surface.l = if p.dark { 0.205 } else { 0.945 };
        return surface.into();
    }
    let mut tint = rgb(p.accent);
    tint.a = if p.dark { 0.04 } else { 0.02 };
    rgb(p.bg).blend(tint)
}

pub(super) fn control_border(p: Palette) -> Rgba {
    if p.mica {
        let mut tint = rgb(p.accent);
        tint.a = 0.2;
        let mut border = rgb(p.text).blend(tint);
        border.a = if p.dark { 0.18 } else { 0.12 };
        return border;
    }
    let mut border = rgb(p.text);
    border.a = if p.dark { 0.20 } else { 0.14 };
    border
}

fn picker_item_surface(p: Palette) -> Rgba {
    if p.mica {
        let mut surface = rgb(p.accent);
        surface.a = if p.dark { 0.08 } else { 0.05 };
        return surface;
    }
    state_surface(p)
}

fn content_sized_picker(id: &str) -> bool {
    matches!(
        id,
        "history-range"
            | "analysis-sort"
            | "connections-sort"
            | "analysis-bucket"
            | "seconds-time-unit"
            | "storage-clean-range"
    )
}

pub(super) fn storage_unit_width(cx: &Context<TrafficApp>) -> Pixels {
    let system = cx.text_system();
    let font = system.resolve_font(&font("Microsoft YaHei UI"));
    let width = ["天", "月", "长期", "MB", "GB"]
        .into_iter()
        .map(|text| {
            text.chars()
                .map(|ch| {
                    system
                        .advance(font, px(13.), ch)
                        .map_or(px(13.), |size| size.width)
                })
                .fold(px(0.), |sum, width| sum + width)
        })
        .fold(px(0.), Pixels::max);
    // Horizontal padding, border, label/arrow gap, and the arrow.
    px((f32::from(width) + 34.).ceil())
}

fn picker_text_width(text: &str, window: &Window) -> Pixels {
    let mut style = window.text_style();
    style.font_family = "Microsoft YaHei UI".into();
    style.font_weight = FontWeight::NORMAL;
    window
        .text_system()
        .shape_line(
            text.to_owned().into(),
            px(13.),
            &[style.to_run(text.len())],
            None,
        )
        .width
}

pub(super) fn project_surface(p: Palette, added: bool, selected: bool) -> Rgba {
    let mut tint = rgb(p.accent);
    tint.a = if selected {
        if p.dark { 0.18 } else { 0.10 }
    } else if added {
        if p.dark { 0.08 } else { 0.035 }
    } else {
        0.
    };
    tint
}

pub(super) fn has_more_below(handle: &ScrollHandle) -> bool {
    handle.max_offset().height + handle.offset().y > px(1.)
}

pub(super) fn scroll_overlay(scrollbar: Scrollbar) -> Div {
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .child(scrollbar)
}

#[derive(IntoElement)]
pub(super) struct ThemedInput {
    pub height: f32,
    pub state: Entity<InputState>,
    pub palette: Palette,
    pub keyboard_navigation: bool,
    pub invalid: bool,
    pub disabled: bool,
    pub search_scroll: Option<ScrollHandle>,
}

impl RenderOnce for ThemedInput {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = self.palette;
        let focus = self.state.read(cx).focus_handle(cx);
        let mut field = TextInput::new(&self.state)
            .disabled(self.disabled)
            .appearance(false)
            .focus_bordered(false)
            .px(px(10.))
            .py(px(0.))
            .text_size(px(13.))
            .text_color(rgb(p.text));
        field.style().size.height = Some(px(self.height - 2.).into());
        div()
            .id(("themed-input", self.state.entity_id()))
            .w_full()
            .min_w_0()
            .relative()
            .h(px(self.height))
            .flex_none()
            .track_focus(&focus)
            .rounded(px(8.))
            .border_1()
            .border_color(if self.invalid {
                rgb(p.amber)
            } else {
                control_border(p)
            })
            .bg(control_surface(p))
            .when(self.keyboard_navigation && !self.invalid, |d| {
                d.focus(move |s| s.border_color(rgb(p.accent)))
            })
            .child(field)
            .when_some(self.search_scroll, |field, scroll| {
                field
                    .on_mouse_down_out(move |event, window, _| {
                        if event.button == MouseButton::Left && focus.is_focused(window) {
                            window.blur();
                        }
                    })
                    .child(
                        canvas(
                            |bounds, window, _| {
                                window.insert_hitbox(bounds, HitboxBehavior::Normal)
                            },
                            move |_, hitbox, window, _| {
                                let hitbox = hitbox.clone();
                                let scroll = scroll.clone();
                                let view = window.current_view();
                                // TextInput consumes every wheel event, including vertical
                                // scrolling in a single-line field. Route that axis to the
                                // page before the input's bubble handler receives it.
                                window.on_mouse_event(
                                    move |event: &ScrollWheelEvent, phase, window, cx| {
                                        if phase != DispatchPhase::Capture
                                            || !hitbox.should_handle_scroll(window)
                                            || event.modifiers.shift
                                        {
                                            return;
                                        }
                                        let delta = event.delta.pixel_delta(window.line_height());
                                        if delta.y == px(0.) || delta.x.abs() > delta.y.abs() {
                                            return;
                                        }
                                        let mut offset = scroll.offset();
                                        offset.y = (offset.y + delta.y)
                                            .clamp(-scroll.max_offset().height, px(0.));
                                        scroll.set_offset(offset);
                                        cx.notify(view);
                                        cx.stop_propagation();
                                    },
                                );
                            },
                        )
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full(),
                    )
            })
    }
}

pub(super) struct PickerState {
    pub id: &'static str,
    pub all_choices: Vec<(String, Action, bool)>,
    pub choices: Vec<(String, Action, bool)>,
    pub cursor: usize,
    pub trigger_bounds: Rc<Cell<Bounds<Pixels>>>,
    pub previous_focus: Option<FocusHandle>,
}

impl TrafficApp {
    pub(super) fn text_input(&self, state: &Entity<InputState>) -> ThemedInput {
        ThemedInput {
            height: if self.page == Page::History { 32. } else { 34. },
            state: state.clone(),
            palette: self.p,
            keyboard_navigation: self.keyboard_navigation,
            invalid: false,
            disabled: false,
            search_scroll: (state.entity_id() == self.search.entity_id()
                || state.entity_id() == self.analysis.search.entity_id()
                || state.entity_id() == self.activity.live_search.entity_id()
                || state.entity_id() == self.activity.recent_search.entity_id())
            .then(|| self.page_scroll.clone()),
        }
    }

    pub(super) fn close_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(picker) = self.picker.take() {
            if self.picker_focus.contains_focused(window, cx)
                || (picker.id == "taskbar-font"
                    && self
                        .font_search
                        .read(cx)
                        .focus_handle(cx)
                        .is_focused(window))
            {
                if let Some(previous) = picker.previous_focus {
                    window.focus(&previous);
                } else {
                    window.blur();
                }
            }
            cx.notify();
        }
    }

    pub(super) fn toggle_picker(
        &mut self,
        id: &'static str,
        choices: Vec<(String, Action, bool)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let was_open = self.picker.as_ref().is_some_and(|picker| picker.id == id);
        self.close_picker(window, cx);
        if !was_open && !choices.is_empty() {
            if id == "taskbar-font" {
                self.font_search
                    .update(cx, |state, cx| state.set_value("", window, cx));
            }
            let cursor = choices
                .iter()
                .position(|(_, _, selected)| *selected)
                .unwrap_or(0);
            self.picker = Some(PickerState {
                id,
                all_choices: choices.clone(),
                choices,
                cursor,
                trigger_bounds: self.picker_bounds[id].clone(),
                previous_focus: window.focused(cx),
            });
            self.picker_scroll.set_offset(point(px(0.), px(0.)));
            self.picker_scroll.scroll_to_item(cursor);
            if id == "taskbar-font" {
                self.font_search.read(cx).focus_handle(cx).focus(window);
            } else {
                window.focus(&self.picker_focus);
            }
            cx.notify();
        }
    }

    pub(super) fn filter_font_picker(&mut self, cx: &mut Context<Self>) {
        let Some(picker) = self
            .picker
            .as_mut()
            .filter(|picker| picker.id == "taskbar-font")
        else {
            return;
        };
        let query = self.font_search.read(cx).value().trim().to_lowercase();
        picker.choices = picker
            .all_choices
            .iter()
            .filter(|(name, _, _)| name.to_lowercase().contains(&query))
            .cloned()
            .collect();
        picker.cursor = picker
            .choices
            .iter()
            .position(|(_, _, selected)| *selected)
            .unwrap_or(0);
        self.picker_scroll.set_offset(point(px(0.), px(0.)));
        self.picker_scroll.scroll_to_item(picker.cursor);
        cx.notify();
    }

    fn choose_picker(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let action = self
            .picker
            .as_ref()
            .and_then(|picker| picker.choices.get(index))
            .map(|(_, action, _)| action.clone());
        if let Some(action) = action {
            self.close_picker(window, cx);
            self.act(action, window, cx);
        }
    }

    pub(super) fn picker_keystroke(
        &mut self,
        event: &KeystrokeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self
            .picker
            .as_ref()
            .is_some_and(|picker| picker.id == "taskbar-font")
            && self.font_search.update(cx, |state, cx| {
                state.marked_text_range(window, cx).is_some()
            })
        {
            return false;
        }
        let Some(picker) = self.picker.as_mut() else {
            return false;
        };
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;
        if modifiers.control || modifiers.alt || modifiers.platform || modifiers.function {
            return false;
        }
        // Keep normal caret movement and spaces available in the search field.
        if picker.id == "taskbar-font" && matches!(key, "home" | "end" | "space") {
            return false;
        }
        match key {
            "up" | "down" | "home" | "end" => {
                let len = picker.choices.len();
                if len == 0 {
                    return true;
                }
                picker.cursor = match key {
                    "up" => (picker.cursor + len - 1) % len,
                    "down" => (picker.cursor + 1) % len,
                    "home" => 0,
                    _ => len - 1,
                };
                self.picker_scroll.scroll_to_item(picker.cursor);
                cx.notify();
            }
            "enter" | "space" => {
                let index = picker.cursor;
                self.choose_picker(index, window, cx);
            }
            "escape" | "tab" => self.close_picker(window, cx),
            _ => return false,
        }
        true
    }

    pub(super) fn taskbar_picker(
        &self,
        id: &'static str,
        current: String,
        choices: Vec<(String, Action, bool)>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let p = self.p;
        let bounds = self.picker_bounds[id].clone();
        let open = self.picker.as_ref().is_some_and(|picker| picker.id == id);
        let keys = choices.clone();
        let tooltip = current.clone();
        let icon_only = current.is_empty();
        let fitted = content_sized_picker(id) && id != "seconds-time-unit";
        let compact = matches!(
            id,
            "retention-time-unit"
                | "seconds-time-unit"
                | "retention-size-unit"
                | "analysis-bucket"
                | "analysis-sort"
                | "connections-sort"
                | "history-range"
                | "history-annual-scope"
        );
        div()
            .id(id)
            .relative()
            .key_context("TaskbarEditor")
            .focusable()
            .tab_stop(true)
            .when(!compact, |d| d.w_full().min_w_0())
            .when(compact, |d| d.flex_none())
            .when(id == "history-annual-scope", |d| d.min_w(px(104.)))
            .when(
                matches!(
                    id,
                    "retention-time-unit" | "seconds-time-unit" | "retention-size-unit"
                ),
                |d| d.w(storage_unit_width(cx)).justify_between(),
            )
            .h(px(if fitted || id == "history-annual-scope" {
                32.
            } else {
                34.
            }))
            .px(px(if fitted {
                10.
            } else if icon_only || compact {
                8.
            } else {
                10.
            }))
            .flex()
            .items_center()
            .gap(px(if icon_only {
                0.
            } else if compact && !fitted {
                4.
            } else {
                8.
            }))
            .rounded(px(8.))
            .bg(control_surface(p))
            .border_1()
            .border_color(if open {
                rgb(p.accent)
            } else {
                control_border(p)
            })
            .text_size(px(13.))
            .text_color(rgb(p.text))
            .hover(move |s| s.border_color(rgb(p.accent)))
            .when(self.keyboard_navigation, |d| {
                d.focus(move |s| s.border_color(rgb(p.accent)))
            })
            .when(
                matches!(id, "taskbar-device" | "taskbar-format" | "taskbar-font"),
                |d| d.tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx)),
            )
            .child(
                canvas(
                    move |rect, _, _| bounds.set(rect.dilate(px(1.))),
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .when(!icon_only, |d| {
                d.child(
                    label(current, p.text)
                        .when(!compact, |d| d.flex_1().min_w_0().truncate())
                        .when(compact, |d| d.flex_none()),
                )
            })
            .child(
                Icon::default()
                    .path("icons/chevron-down.svg")
                    .size(px(if compact { 12. } else { 14. }))
                    .text_color(rgb(p.muted)),
            )
            .on_click(cx.listener(move |app, _, window, cx| {
                app.toggle_picker(id, choices.clone(), window, cx)
            }))
            .on_key_down(cx.listener(move |app, event: &KeyDownEvent, window, cx| {
                if matches!(
                    event.keystroke.key.as_str(),
                    "enter" | "space" | "down" | "up"
                ) {
                    app.toggle_picker(id, keys.clone(), window, cx);
                    window.prevent_default();
                    cx.stop_propagation();
                }
            }))
    }

    pub(super) fn picker_overlay(&self, window: &Window, cx: &Context<Self>) -> Option<AnyElement> {
        let picker = self.picker.as_ref()?;
        let searchable = picker.id == "taskbar-font";
        let unit_menu = matches!(
            picker.id,
            "retention-time-unit" | "retention-size-unit" | "analysis-bucket"
        );
        let show_tooltip = matches!(
            picker.id,
            "taskbar-device" | "taskbar-format" | "taskbar-font"
        );
        let p = self.p;
        let bounds = picker.trigger_bounds.get();
        let fitted = content_sized_picker(picker.id);
        let width = if fitted {
            let text_width = picker
                .choices
                .iter()
                .map(|(text, _, _)| picker_text_width(text, window))
                .fold(px(0.), Pixels::max);
            // Border, list padding, item padding, and the separate checkmark slot.
            let chrome = px(24. + if unit_menu { 0. } else { 22. });
            (text_width + chrome)
                .max(bounds.size.width)
                .min((window.viewport_size().width - px(16.)).max(px(0.)))
        } else if matches!(picker.id, "taskbar-device" | "taskbar-font") {
            bounds.size.width.max(px(240.)).min(px(360.))
        } else if picker.id == "taskbar-font-size" {
            // Include the checkmark, text, padding and scrollbar gutter.
            bounds.size.width.max(px(140.))
        } else {
            bounds.size.width
        };
        let height =
            (picker.choices.len().max(1) as f32 * 32. + 10. + if searchable { 46. } else { 0. })
                .min(300.);
        let above = bounds.bottom() + px(height + 12.) > window.bounds().size.height
            && bounds.origin.y > window.bounds().size.height - bounds.bottom();
        let trigger_bounds = picker.trigger_bounds.clone();
        let menu = div()
            .id("taskbar-picker-menu")
            .relative()
            .flex()
            .flex_col()
            .track_focus(&self.picker_focus)
            .key_context("TaskbarEditor")
            .w(width)
            .h(px(height))
            .max_w_full()
            .rounded(px(8.))
            .border_1()
            .border_color(control_border(p))
            .bg(popup_surface(p))
            .shadow_sm()
            .occlude()
            .on_mouse_down_out(cx.listener(move |app, event: &MouseDownEvent, window, cx| {
                // Leave a click on the trigger to its toggle handler.
                if !trigger_bounds.get().contains(&event.position) {
                    app.close_picker(window, cx);
                }
            }))
            .when(searchable, |menu| {
                menu.child(
                    div()
                        .id("font-picker-search")
                        .p(px(6.))
                        .pb(px(4.))
                        .flex_none()
                        .child(self.text_input(&self.font_search)),
                )
            })
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .id("taskbar-picker-options")
                            .size_full()
                            .py(px(4.))
                            .pl(px(4.))
                            .pr(px(if picker.choices.len() > 9 { 14. } else { 4. }))
                            .overflow_y_scroll()
                            .track_scroll(&self.picker_scroll)
                            .when(picker.choices.is_empty(), |list| {
                                list.child(
                                    div().h(px(32.)).px(px(8.)).flex().items_center().child(
                                        label("未找到匹配的字体", p.muted).text_size(px(13.)),
                                    ),
                                )
                            })
                            .children(picker.choices.iter().enumerate().map(
                                |(index, (text, _, selected))| {
                                    let selected = *selected;
                                    let cursor = picker.cursor == index;
                                    div()
                                        .id(("taskbar-picker-option", index))
                                        .h(px(32.))
                                        .w_full()
                                        .flex_none()
                                        .flex()
                                        .items_center()
                                        .gap(px(6.))
                                        .px(px(6.))
                                        .rounded(px(5.))
                                        .bg(if cursor || selected {
                                            picker_item_surface(p)
                                        } else {
                                            rgba(0)
                                        })
                                        .hover(move |s| s.bg(picker_item_surface(p)))
                                        .when(!unit_menu, |d| {
                                            d.child(div().w(px(16.)).flex_none().when(
                                                selected,
                                                |d| {
                                                    d.child(
                                                        Icon::default()
                                                            .path("icons/check.svg")
                                                            .size(px(14.))
                                                            .text_color(rgb(p.accent_text)),
                                                    )
                                                },
                                            ))
                                        })
                                        .child(
                                            label(
                                                text.clone(),
                                                if selected { p.accent_text } else { p.text },
                                            )
                                            .text_size(px(13.))
                                            .flex_1()
                                            .min_w_0()
                                            .truncate(),
                                        )
                                        .when(show_tooltip, |d| {
                                            d.tooltip({
                                                let text = text.clone();
                                                move |window, cx| {
                                                    Tooltip::new(text.clone()).build(window, cx)
                                                }
                                            })
                                        })
                                        .on_click(cx.listener(move |app, _, window, cx| {
                                            app.choose_picker(index, window, cx)
                                        }))
                                },
                            )),
                    )
                    .child(controls::scroll_overlay(Scrollbar::vertical(
                        &self.picker_scrollbar,
                        &self.picker_scroll,
                    ))),
            );
        Some(
            deferred(
                anchored()
                    .anchor(if fitted && above {
                        Corner::BottomRight
                    } else if fitted {
                        Corner::TopRight
                    } else if above {
                        Corner::BottomLeft
                    } else {
                        Corner::TopLeft
                    })
                    .position(point(
                        if fitted {
                            bounds.right()
                        } else {
                            bounds.origin.x
                        },
                        if above {
                            bounds.origin.y - px(4.)
                        } else {
                            bounds.bottom() + px(4.)
                        },
                    ))
                    .snap_to_window_with_margin(px(8.))
                    .child(menu),
            )
            .with_priority(3)
            .into_any_element(),
        )
    }
}
