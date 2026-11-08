//! Live GPUI input/subscription regression with temporary data.
use super::*;
use crate::taskbar_content::UnitStyle;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

pub static RUN: AtomicBool = AtomicBool::new(false);
static OUTCOME: Mutex<Option<Result<(), String>>> = Mutex::new(None);

/// Exercise actual input subscriptions and GPUI event dispatch for both targets.
pub(super) async fn exercise_font_controls(
    this: &WeakEntity<TrafficApp>,
    cx: &mut AsyncWindowContext,
) -> anyhow::Result<()> {
    let (target, original, other) = this.update_in(cx, |app, _, cx| {
        app.editor.settings_scroll.set_offset(point(px(0.), px(0.)));
        cx.notify();
        (
            app.display_tab,
            app.display_content().clone(),
            app.inactive_editor.weight.entity_id(),
        )
    })?;
    Timer::after(Duration::from_millis(100)).await;
    this.update_in(cx, |app, window, cx| {
        app.editor.weight.read(cx).focus_handle(cx).focus(window);
        let choices = app
            .taskbar_fonts
            .iter()
            .map(|name| {
                (
                    name.clone(),
                    Action::Display(DisplayAction::TaskbarFont(name.clone())),
                    *name == app.display_content().font_family,
                )
            })
            .collect();
        app.toggle_picker("taskbar-font", choices, window, cx);
        anyhow::ensure!(
            app.font_search.read(cx).focus_handle(cx).is_focused(window),
            "font search did not receive focus"
        );
        app.font_search
            .update(cx, |input, cx| input.set_value(" sEgOe ", window, cx));
        Ok(())
    })??;
    Timer::after(Duration::from_millis(100)).await;
    this.update_in(cx, |app, _, _| {
        let picker = app.picker.as_ref().unwrap();
        anyhow::ensure!(
            !picker.choices.is_empty()
                && picker
                    .choices
                    .iter()
                    .all(|(name, _, _)| name.to_lowercase().contains("segoe")),
            "font search did not filter case-insensitively"
        );
        Ok(())
    })??;
    cx.update(|window, cx| {
        window.dispatch_keystroke(Keystroke::parse("down").unwrap(), cx);
    })?;
    let selected = this.update_in(cx, |app, _, _| {
        let picker = app.picker.as_ref().unwrap();
        picker.choices[picker.cursor].0.clone()
    })?;
    cx.update(|window, cx| {
        window.dispatch_keystroke(Keystroke::parse("enter").unwrap(), cx);
    })?;
    this.update_in(cx, |app, window, cx| {
        anyhow::ensure!(
            app.picker.is_none() && app.display_content().font_family == selected,
            "filtered selection applied the wrong font"
        );
        anyhow::ensure!(
            app.editor
                .weight
                .read(cx)
                .focus_handle(cx)
                .is_focused(window),
            "font menu did not restore focus"
        );
        let choices = app
            .taskbar_fonts
            .iter()
            .map(|name| {
                (
                    name.clone(),
                    Action::Display(DisplayAction::TaskbarFont(name.clone())),
                    *name == selected,
                )
            })
            .collect();
        app.toggle_picker("taskbar-font", choices, window, cx);
        anyhow::ensure!(
            app.font_search.read(cx).value().is_empty(),
            "font query was not reset on reopening"
        );
        app.font_search.update(cx, |input, cx| {
            input.set_value("font-no-match-0e8f4", window, cx)
        });
        Ok(())
    })??;
    Timer::after(Duration::from_millis(100)).await;
    for key in ["up", "down", "enter", "home", "end", "space"] {
        cx.update(|window, cx| {
            window.dispatch_keystroke(Keystroke::parse(key).unwrap(), cx);
        })?;
    }
    this.update_in(cx, |app, window, cx| {
        anyhow::ensure!(
            app.picker
                .as_ref()
                .is_some_and(|picker| picker.choices.is_empty())
                && app.display_content().font_family == selected,
            "empty search applied a font or closed unexpectedly"
        );
        app.font_search
            .update(cx, |input, cx| input.set_value("", window, cx));
        Ok(())
    })??;
    Timer::after(Duration::from_millis(100)).await;
    this.update_in(cx, |app, window, cx| {
        anyhow::ensure!(
            app.picker.as_ref().unwrap().choices.len() == app.taskbar_fonts.len(),
            "clearing search did not restore fonts"
        );
        let next = if target == DisplayTab::Taskbar {
            DisplayTab::Floating
        } else {
            DisplayTab::Taskbar
        };
        app.act(Action::DisplayTab(next), window, cx);
        anyhow::ensure!(
            app.picker.is_none() && app.editor.weight.entity_id() == other,
            "switching targets retained the search menu"
        );
        app.act(Action::DisplayTab(target), window, cx);
        app.act(
            Action::Display(DisplayAction::TaskbarFont(original.font_family.clone())),
            window,
            cx,
        );
        app.act(
            Action::Display(DisplayAction::TaskbarFontWeight(
                TaskbarFontWeight::new(450).unwrap(),
            )),
            window,
            cx,
        );
        app.editor.weight.read(cx).focus_handle(cx).focus(window);
        Ok(())
    })??;
    Timer::after(Duration::from_millis(100)).await;
    for (key, expected) in [
        ("up", 451),
        ("down", 450),
        ("shift-up", 460),
        ("shift-down", 450),
    ] {
        cx.update(|window, cx| {
            window.dispatch_keystroke(Keystroke::parse(key).unwrap(), cx);
        })?;
        this.update_in(cx, |app, _, _| {
            anyhow::ensure!(
                app.display_content().font_weight.value() == expected,
                "weight key {key} did not apply {expected}"
            );
            Ok(())
        })??;
    }
    for (delta, shift, expected) in [
        (ScrollDelta::Lines(point(0., 3.)), false, 451),
        (ScrollDelta::Lines(point(0., -3.)), false, 450),
        (ScrollDelta::Lines(point(3., 0.)), true, 460),
        (ScrollDelta::Pixels(point(px(0.), px(-12.))), false, 460),
        (ScrollDelta::Pixels(point(px(0.), px(-12.))), false, 459),
    ] {
        this.update_in(cx, |app, window, cx| {
            let position = app.editor.weight_bounds.get().center();
            anyhow::ensure!(
                position.y > px(0.) && position.y < window.viewport_size().height,
                "weight field is outside viewport"
            );
            let scroll = app.editor.settings_scroll.offset();
            anyhow::ensure!(
                app.scroll_taskbar_weight(
                    &ScrollWheelEvent {
                        position,
                        delta,
                        modifiers: Modifiers {
                            shift,
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                    window,
                    cx
                ),
                "weight wheel was not handled"
            );
            anyhow::ensure!(
                app.editor.settings_scroll.offset() == scroll,
                "weight wheel scrolled settings"
            );
            Ok(())
        })??;
        this.update_in(cx, |app, _, _| {
            anyhow::ensure!(
                app.display_content().font_weight.value() == expected,
                "weight wheel did not apply {expected}: {:?}",
                app.display_content().font_weight
            );
            Ok(())
        })??;
    }
    // Post a wheel message only to this temporary test window. This verifies
    // the capture handler runs before TextInput consumes the event.
    this.update_in(cx, |app, window, _| {
        use windows_sys::Win32::{
            Foundation::POINT,
            Graphics::Gdi::ClientToScreen,
            UI::WindowsAndMessaging::{PostMessageW, WM_MOUSEWHEEL},
        };
        let center = app.editor.weight_bounds.get().center();
        let mut position = POINT {
            x: (f32::from(center.x) * window.scale_factor()) as i32,
            y: (f32::from(center.y) * window.scale_factor()) as i32,
        };
        unsafe {
            let hwnd = app.main_hwnd as _;
            anyhow::ensure!(
                ClientToScreen(hwnd, &mut position) != 0,
                "wheel coordinates unavailable"
            );
            let coordinates =
                (position.x as u16 as u32 | ((position.y as u16 as u32) << 16)) as isize;
            anyhow::ensure!(
                PostMessageW(hwnd, WM_MOUSEWHEEL, (120u32 << 16) as usize, coordinates) != 0,
                "wheel message failed"
            );
        }
        Ok(())
    })??;
    Timer::after(Duration::from_millis(100)).await;
    this.update_in(cx, |app, _, _| {
        anyhow::ensure!(
            app.display_content().font_weight.value() > 459,
            "native wheel was consumed by TextInput instead of weight capture"
        );
        Ok(())
    })??;
    for (start, key, expected) in [(900, "up", 900), (100, "down", 100), (100, "up", 101)] {
        this.update_in(cx, |app, window, cx| {
            app.act(
                Action::Display(DisplayAction::TaskbarFontWeight(
                    TaskbarFontWeight::new(start).unwrap(),
                )),
                window,
                cx,
            );
        })?;
        cx.update(|window, cx| {
            window.dispatch_keystroke(Keystroke::parse(key).unwrap(), cx);
        })?;
        this.update_in(cx, |app, _, _| {
            anyhow::ensure!(
                app.display_content().font_weight.value() == expected,
                "weight range clamp failed"
            );
            Ok(())
        })??;
    }
    this.update_in(cx, |app, window, cx| {
        app.editor
            .weight
            .update(cx, |input, cx| input.set_value("901", window, cx));
    })?;
    cx.update(|window, cx| {
        window.dispatch_keystroke(Keystroke::parse("up").unwrap(), cx);
    })?;
    this.update_in(cx, |app, window, cx| {
        anyhow::ensure!(
            app.display_content().font_weight.value() == 101
                && app.editor.weight.read(cx).value().as_ref() == "901"
                && app.editor.weight_error,
            "weight stepping accepted an invalid draft"
        );
        app.act(
            Action::Display(DisplayAction::TaskbarFontWeight(original.font_weight)),
            window,
            cx,
        );
        anyhow::ensure!(
            *app.display_content() == original,
            "font control exercise changed unrelated content"
        );
        Ok(())
    })??;
    for (text, expected) in [
        ("12.25", 12.25),
        ("12.5", 12.5),
        ("12.75", 12.75),
        ("13.125", 13.125),
    ] {
        this.update_in(cx, |app, window, cx| {
            app.editor.size.read(cx).focus_handle(cx).focus(window);
            app.editor
                .size
                .update(cx, |input, cx| input.set_value(text, window, cx));
        })?;
        Timer::after(Duration::from_millis(60)).await;
        this.update_in(cx, |app, _, _| {
            let frame = app.content_frame(app.display_content());
            anyhow::ensure!(
                app.display_content().font_size == expected && frame.font_size == expected,
                "fractional size input was rounded or did not reach rendering: {text}"
            );
            Ok(())
        })??;
    }
    for text in ["", "invalid", "7.99", "20.01", "NaN", "inf"] {
        this.update_in(cx, |app, window, cx| {
            app.editor
                .size
                .update(cx, |input, cx| input.set_value(text, window, cx));
        })?;
        Timer::after(Duration::from_millis(60)).await;
        this.update_in(cx, |app, _, cx| {
            app.update_taskbar_font_size_input(true, cx);
            anyhow::ensure!(
                app.display_content().font_size == 13.125
                    && app.editor.size_error
                    && app.editor.size.read(cx).value().as_ref() == text,
                "invalid size was applied or its draft was lost: {text}"
            );
            Ok(())
        })??;
    }
    this.update_in(cx, |app, window, cx| {
        let next = if target == DisplayTab::Taskbar {
            DisplayTab::Floating
        } else {
            DisplayTab::Taskbar
        };
        let size_id = app.editor.size.entity_id();
        app.act(Action::DisplayTab(next), window, cx);
        anyhow::ensure!(
            app.editor.size.entity_id() != size_id,
            "size input is shared between targets"
        );
        app.act(Action::DisplayTab(target), window, cx);
        anyhow::ensure!(
            app.editor.size_error
                && app.editor.size.read(cx).value().as_ref() == "inf"
                && app.display_content().font_size == 13.125,
            "switching tabs lost the invalid size draft"
        );
        app.act(
            Action::Display(DisplayAction::TaskbarFontSize(12.)),
            window,
            cx,
        );
        anyhow::ensure!(
            !app.editor.size_error && app.editor.size.read(cx).value().as_ref() == "12",
            "size preset did not replace the invalid draft"
        );
        app.act(
            Action::Display(DisplayAction::TaskbarFontSize(original.font_size)),
            window,
            cx,
        );
        anyhow::ensure!(
            *app.display_content() == original,
            "size controls changed unrelated content"
        );
        Ok(())
    })??;
    Ok(())
}

fn precision_choices() -> Vec<(String, Action, bool)> {
    [None, Some(0), Some(1), Some(2)]
        .into_iter()
        .map(|value| {
            (
                value.map_or_else(|| "自动".into(), |value| format!("{value} 位小数")),
                Action::Display(DisplayAction::TaskbarPrecision(value)),
                value.is_none(),
            )
        })
        .collect()
}

fn spacing_point(app: &TrafficApp, prefix: bool) -> Point<Pixels> {
    let frame = app.content_frame(&app.config.taskbar_content);
    let cell = &frame.rows[0][0];
    let x = if prefix {
        (cell.segments[0].x + cell.segments[0].width + cell.segments[1].x) / 2.
    } else {
        (cell.x + cell.width + frame.rows[0][1].x) / 2.
    };
    let bounds = app.editor.traffic_bounds.get();
    point(
        bounds.left() + px(x * crate::taskbar_text::PREVIEW_SCALE),
        (app.editor.preview_scroll.bounds().top()
            + px(56.)
            + app.editor.spacing.footer_bounds.get().bottom())
            / 2.,
    )
}

fn check_preview_layout(app: &TrafficApp) -> anyhow::Result<()> {
    let card = app.editor.spacing.card_bounds.get();
    let preview = app.editor.preview_scroll.bounds();
    let footer = app.editor.spacing.footer_bounds.get();
    let settings = app.editor.settings_scroll.bounds();
    anyhow::ensure!(
        card.size.height > px(140.),
        "the allocated preview card collapsed: {card:?}"
    );
    anyhow::ensure!(
        preview.top() >= card.top() && footer.bottom() <= card.bottom(),
        "preview content overflowed its allocated card: card={card:?}, preview={preview:?}, footer={footer:?}"
    );
    anyhow::ensure!(
        settings.top() >= card.bottom() - px(0.5),
        "settings overlap the preview: card={card:?}, settings={settings:?}"
    );
    Ok(())
}

fn check_toolbar_rows(app: &TrafficApp, window: &Window, compact: bool) -> anyhow::Result<()> {
    let [layout, fonts, spacing, reset] = app.editor.toolbar_groups.get();
    let center_y = |bounds: Bounds<Pixels>| bounds.top() + bounds.size.height / 2.;
    anyhow::ensure!(
        app.editor.toolbar_compact == compact,
        "toolbar did not change row count"
    );
    for bounds in [layout, fonts, spacing, reset] {
        anyhow::ensure!(
            bounds.size.width > px(0.)
                && bounds.left() >= px(0.)
                && bounds.right() <= window.viewport_size().width,
            "toolbar group overflowed window: {bounds:?}"
        );
    }
    anyhow::ensure!(
        (center_y(layout) - center_y(spacing)).abs() < px(1.)
            && (center_y(layout) - center_y(reset)).abs() < px(1.),
        "layout, spacing and reset must share the first row"
    );
    if compact {
        anyhow::ensure!(
            fonts.top() >= layout.bottom().max(spacing.bottom()).max(reset.bottom())
                && (fonts.left() - layout.left()).abs() < px(1.),
            "font controls must form a left-aligned second row: {layout:?}, {fonts:?}, {spacing:?}, {reset:?}"
        );
        anyhow::ensure!(
            layout.right() <= spacing.left() && spacing.right() <= reset.left(),
            "compact toolbar groups are out of order"
        );
    } else {
        anyhow::ensure!(
            (center_y(layout) - center_y(fonts)).abs() < px(1.)
                && layout.right() <= fonts.left()
                && fonts.right() <= spacing.left()
                && spacing.right() <= reset.left(),
            "wide toolbar must remain one ordered row"
        );
    }
    Ok(())
}

pub fn schedule(window: &mut Window, cx: &mut Context<TrafficApp>) {
    cx.spawn_in(window, async move |this, cx| {
        let outcome: anyhow::Result<()> = async {
            let baseline = this.update_in(cx, |app, window, cx| {
                app.act(Action::Nav(Page::Display), window, cx);
                anyhow::ensure!(!app.editor.editing, "editor must start collapsed");
                let baseline = app.config.clone();
                app.act(Action::Display(DisplayAction::TaskbarEditing), window, cx);
                app.act(Action::Display(DisplayAction::TaskbarSelect(Metric::Cpu)), window, cx);
                anyhow::ensure!(app.editor.selected.is_none() && app.config == baseline, "repeat selection must clear the inspector without removing the metric");
                app.sync_taskbar_editor(window, cx);
                anyhow::ensure!(app.editor.selected.is_none(), "automatic refresh reselected a deselected metric");
                app.select_taskbar_item(Metric::Vram, window, cx);
                anyhow::ensure!(app.taskbar_item(Metric::Vram).unit_style == UnitStyle::Short, "new metrics must default to short units");
                app.act(Action::Display(DisplayAction::TaskbarUnitStyle), window, cx);
                anyhow::ensure!(app.taskbar_item(Metric::Vram).unit_style == UnitStyle::Full, "unit style did not toggle to full");
                app.act(Action::Display(DisplayAction::TaskbarUnitStyle), window, cx);
                app.act(Action::Display(DisplayAction::TaskbarUnit(ByteUnit::G)), window, cx);
                app.act(Action::Display(DisplayAction::TaskbarSelect(Metric::Gpu)), window, cx);
                app.act(Action::Display(DisplayAction::TaskbarSelect(Metric::Vram)), window, cx);
                anyhow::ensure!(app.taskbar_item(Metric::Vram).unit_style == UnitStyle::Short && app.taskbar_item(Metric::Vram).unit == Some(ByteUnit::G) && app.config == baseline, "unit style drafts changed config or were lost when switching selection");
                app.act(Action::Display(DisplayAction::TaskbarSelect(Metric::Gpu)), window, cx);
                app.act(Action::Display(DisplayAction::TaskbarDevice("temporarily-missing-device".into())), window, cx);
                app.act(Action::Display(DisplayAction::TaskbarPrecision(Some(2))), window, cx);
                app.editor.label.update(cx, |input, cx| input.set_value("图形", window, cx));
                anyhow::ensure!(app.config == baseline, "selecting or editing an unadded metric changed the layout");
                Ok(baseline)
            })??;
            Timer::after(Duration::from_millis(150)).await;
            for (width, compact) in [(1050., true), (900., true), (1240., false), (1500., false), (1050., true), (1240., false)] {
                this.update_in(cx, |_, window, cx| {
                    window.resize(size(px(width), px(870.)));
                    cx.notify();
                })?;
                Timer::after(Duration::from_millis(150)).await;
                this.update_in(cx, |app, window, _| check_toolbar_rows(app, window, compact))??;
            }
            this.update_in(cx, |app, window, cx| {
                check_preview_layout(app)?;
                anyhow::ensure!(app.config == baseline, "draft input subscription wrote to config");
                anyhow::ensure!(app.taskbar_item(Metric::Gpu).label == "图形", "draft prefix subscription failed");
                app.act(Action::Display(DisplayAction::TaskbarSelect(Metric::Cpu)), window, cx);
                app.act(Action::Display(DisplayAction::TaskbarSelect(Metric::Gpu)), window, cx);
                anyhow::ensure!(app.taskbar_item(Metric::Gpu).precision == Some(2), "switching selection lost a draft");
                app.act(Action::Display(DisplayAction::TaskbarPrecision(None)), window, cx);
                app.toggle_picker("taskbar-precision", precision_choices(), window, cx);
                Ok(())
            })??;
            Timer::after(Duration::from_millis(150)).await;
            this.update_in(cx, |app, window, cx| {
                anyhow::ensure!(app.picker_focus.is_focused(window), "dropdown did not receive keyboard focus");
                let bounds = app.picker_bounds["taskbar-precision"].get();
                anyhow::ensure!(bounds.size.width >= px(110.) && (bounds.size.height - px(34.)).abs() < px(1.), "dropdown trigger dimensions are inconsistent: {bounds:?}");
                anyhow::ensure!(app.editor.settings_scroll.bounds().size.height > px(0.), "settings scroll viewport collapsed");
                cx.notify();
                Ok(())
            })??;
            cx.update(|window, cx| { window.dispatch_keystroke(Keystroke::parse("down").unwrap(), cx); })?;
            cx.update(|window, cx| { window.dispatch_keystroke(Keystroke::parse("enter").unwrap(), cx); })?;
            Timer::after(Duration::from_millis(80)).await;
            this.update_in(cx, |app, window, cx| {
                anyhow::ensure!(app.picker.is_none() && app.taskbar_item(Metric::Gpu).precision == Some(0), "keyboard dropdown selection did not apply and close");
                anyhow::ensure!(app.config == baseline, "dropdown selection persisted an unadded metric");
                app.toggle_picker("taskbar-precision", precision_choices(), window, cx);
                app.toggle_picker("taskbar-precision", precision_choices(), window, cx);
                anyhow::ensure!(app.picker.is_none(), "a second trigger click did not close the dropdown");
                app.toggle_picker("taskbar-precision", precision_choices(), window, cx);
                Ok(())
            })??;
            Timer::after(Duration::from_millis(80)).await;
            cx.update(|window, cx| { window.dispatch_keystroke(Keystroke::parse("escape").unwrap(), cx); })?;
            this.update_in(cx, |app, window, cx| {
                anyhow::ensure!(app.picker.is_none(), "Escape did not dismiss the dropdown");
                app.toggle_picker("taskbar-precision", precision_choices(), window, cx);
                app.act(Action::Display(DisplayAction::TaskbarSelect(Metric::Cpu)), window, cx);
                anyhow::ensure!(app.picker.is_none(), "selecting a metric left stale dropdown choices open");
                app.act(Action::Display(DisplayAction::TaskbarSelect(Metric::Gpu)), window, cx);
                app.act(Action::Display(DisplayAction::TaskbarAdd(Metric::Gpu)), window, cx);
                let item = app.config.taskbar_content.item(Metric::Gpu).unwrap();
                anyhow::ensure!(item.label == "图形" && item.precision == Some(0) && item.device == "temporarily-missing-device", "adding did not preserve draft settings");
                app.act(Action::Display(DisplayAction::TaskbarRemove), window, cx);
                anyhow::ensure!(app.editor.selected == Some(Metric::Gpu) && app.config.taskbar_content.item(Metric::Gpu).is_none(), "removal must retain the inspected metric");
                app.act(Action::Display(DisplayAction::TaskbarAdd(Metric::Gpu)), window, cx);
                anyhow::ensure!(app.config.taskbar_content.item(Metric::Gpu).unwrap().label == "图形", "re-adding lost the removed item's draft");
                app.drop_taskbar_item(Metric::Download, 1, 0, window, cx);
                anyhow::ensure!(app.config.taskbar_content.rows[1][0].metric == Metric::Download, "cross-row insertion failed");
                app.drop_taskbar_item(Metric::Download, 1, 2, window, cx);
                anyhow::ensure!(app.config.taskbar_content.rows[1][1].metric == Metric::Download, "insertion after the target failed");
                app.select_taskbar_item(Metric::Cpu, window, cx);
                app.editor.label.read(cx).focus_handle(cx).focus(window);
                app.editor.label.update(cx, |input, cx| input.set_value("处理器", window, cx));
                Ok(())
            })??;
            Timer::after(Duration::from_millis(150)).await;
            let offset = this.update_in(cx, |app, _, _| app.config.taskbar_offset)?;
            cx.update(|window, cx| { window.dispatch_keystroke(Keystroke::parse("left").unwrap(), cx); })?;
            Timer::after(Duration::from_millis(80)).await;
            this.update_in(cx, |app, window, cx| {
                anyhow::ensure!(app.config.taskbar_offset == offset, "prefix input's arrow key changed taskbar position");
                anyhow::ensure!(app.config.taskbar_content.item(Metric::Cpu).unwrap().label == "处理器", "prefix change subscription did not run");
                app.act(Action::Display(DisplayAction::TaskbarAdd(Metric::Today)), window, cx);
                app.act(Action::Display(DisplayAction::TaskbarUnit(ByteUnit::T)), window, cx);
                app.act(Action::Display(DisplayAction::TaskbarSelect(Metric::Memory)), window, cx);
                app.act(Action::Display(DisplayAction::TaskbarValueFormat(ValueFormat::UsedTotal)), window, cx);
                app.act(Action::Display(DisplayAction::TaskbarUnit(ByteUnit::M)), window, cx);
                app.act(Action::Display(DisplayAction::TaskbarUnitStyle), window, cx);
                app.act(Action::Display(DisplayAction::TaskbarPrecision(Some(2))), window, cx);
                app.act(Action::Display(DisplayAction::TaskbarGap(18)), window, cx);
                app.editor.label.update(cx, |input, cx| input.set_value("", window, cx));
                Ok(())
            })??;
            Timer::after(Duration::from_millis(1200)).await;
            let (backend, expected) = this.update_in(cx, |app, _, _| (app.backend.clone(), app.config.clone()))?;
            anyhow::ensure!(expected.taskbar_content.item(Metric::Memory).unwrap().label.is_empty(), "empty prefix was not retained");
            anyhow::ensure!(expected.taskbar_content.item(Metric::Memory).unwrap().unit == Some(ByteUnit::M), "capacity unit was not saved independently of the memory display mode");
            anyhow::ensure!(expected.taskbar_content.item(Metric::Memory).unwrap().unit_style == UnitStyle::Full, "explicit full unit style was not persisted");
            anyhow::ensure!(expected.taskbar_content.item(Metric::Today).unwrap().unit == Some(ByteUnit::T), "fixed T unit was not persisted for traffic");
            anyhow::ensure!(Config::load(&backend.dir)? == expected, "saved content did not roundtrip after the debounce interval");
            let system = backend.shared.lock().unwrap().system.clone();
            anyhow::ensure!(system.cpu_percent.is_some() && system.memory.is_some(), "system sampler did not refresh independently");
            this.update_in(cx, |app, window, cx| {
                app.act(Action::Display(DisplayAction::TaskbarRows(1)), window, cx);
                app.act(Action::Display(DisplayAction::TaskbarRows(2)), window, cx);
                app.toggle_picker("taskbar-precision", precision_choices(), window, cx);
                let content = app.config.taskbar_content.clone();
                app.act(Action::Display(DisplayAction::TaskbarEditing), window, cx);
                anyhow::ensure!(app.picker.is_none() && !app.editor.editing && content == app.config.taskbar_content, "closing editor must close menus and retain edits");
                app.drop_taskbar_item(Metric::Memory, 0, 0, window, cx);
                anyhow::ensure!(content == app.config.taskbar_content, "read-only preview allowed a drop");
                app.act(Action::Display(DisplayAction::TaskbarEditing), window, cx);
                for metric in Metric::ALL { app.act(Action::Display(DisplayAction::TaskbarAdd(metric)), window, cx); }
                anyhow::ensure!(app.config.taskbar_content.rows.iter().flatten().count() == 13, "metric library is incomplete");
                Ok(())
            })??;
            Timer::after(Duration::from_millis(150)).await;
            this.update_in(cx, |app, window, cx| {
                anyhow::ensure!(app.editor.preview_scroll.max_offset().width > px(0.), "the long preview did not expose horizontal scrolling");
                app.editor.settings_scroll.set_offset(point(px(0.), px(0.)));
                anyhow::ensure!(controls::has_more_below(&app.editor.settings_scroll), "scrollable content has no bottom fade");
                app.editor.settings_scroll.set_offset(point(px(0.), -app.editor.settings_scroll.max_offset().height));
                anyhow::ensure!(!controls::has_more_below(&app.editor.settings_scroll), "bottom fade remained at the end");
                app.act(Action::Display(DisplayAction::TaskbarReset), window, cx);
                anyhow::ensure!(app.config.taskbar_content == TaskbarContent::default() && app.editor.selected == Some(Metric::Cpu), "reset did not restore the four-item default");
                Ok(())
            })??;
            Timer::after(Duration::from_millis(150)).await;
            let unchanged = this.update_in(cx, |app, window, cx| {
                let unchanged = app.config.taskbar_content.clone();
                app.start_taskbar_drag(Metric::Cpu, window, cx);
                let frame = &app.editor.drag.as_ref().unwrap().frame;
                let cell = &frame.rows[1][0];
                let bounds = app.editor.traffic_bounds.get();
                let point = point(bounds.origin.x + px((cell.x + cell.width / 2.) * crate::taskbar_text::PREVIEW_SCALE), bounds.origin.y + px(crate::taskbar_text::LINE_HEIGHT * crate::taskbar_text::PREVIEW_SCALE * 1.5));
                app.update_taskbar_drag(point, cx);
                anyhow::ensure!(app.editor.drop == Some(DropIntent::Swap(Metric::Memory)), "center hit-test did not advertise a swap");
                let candidate = app.taskbar_drop_candidate().unwrap();
                anyhow::ensure!(candidate.position(Metric::Cpu) == Some((1, 0)) && app.config.taskbar_content == unchanged, "hover committed a provisional move");
                Ok(unchanged)
            })??;
            cx.update(|window, cx| { window.dispatch_keystroke(Keystroke::parse("escape").unwrap(), cx); })?;
            this.update_in(cx, |app, window, cx| {
                anyhow::ensure!(app.editor.drag.is_none() && app.editor.drop.is_none() && app.config.taskbar_content == unchanged, "Escape did not cancel the provisional move");
                app.act(Action::Display(DisplayAction::TaskbarMove(MoveDirection::Down)), window, cx);
                anyhow::ensure!(app.config.taskbar_content.position(Metric::Cpu) == Some((1, 0)), "the down button disagreed with the provisional preview");
                app.act(Action::Display(DisplayAction::TaskbarReset), window, cx);
                Ok(())
            })??;
            // Exercise the same spacing handlers used by the pointer listeners.
            // Actual native pointer hit-testing remains a separate manual check.
            Timer::after(Duration::from_millis(100)).await;
            let editing_height = this.update_in(cx, |app, window, cx| {
                let height = app.editor.spacing.card_bounds.get().size.height;
                app.act(Action::Display(DisplayAction::TaskbarEditing), window, cx);
                height
            })?;
            Timer::after(Duration::from_millis(80)).await;
            let original_height = this.update_in(cx, |app, window, cx| {
                check_preview_layout(app)?;
                let height = app.editor.spacing.card_bounds.get().size.height;
                anyhow::ensure!(height > px(140.) && height < px(180.), "preview card bounds did not capture the actual compact card: {height:?}");
                anyhow::ensure!((height - editing_height).abs() < px(0.5), "editing spacing increased the preview card height: {editing_height:?} vs {height:?}");
                app.act(Action::Display(DisplayAction::TaskbarEditing), window, cx);
                app.editor.spacing.set_preview_hovered(true);
                cx.notify();
                Ok(height)
            })??;
            Timer::after(Duration::from_millis(80)).await;
            for prefix in [false, true] {
                let (start, baseline, selection) = this.update_in(cx, |app, _, _| {
                    let footer = app.editor.spacing.footer_bounds.get();
                    anyhow::ensure!((app.editor.spacing.card_bounds.get().size.height - original_height).abs() < px(0.5) && footer.size.height < px(19.), "hovering spacing handles grew the card or wrapped the footer");
                    anyhow::ensure!(footer.top() - app.editor.preview_scroll.bounds().bottom() <= px(3.) && app.editor.spacing.card_bounds.get().bottom() - footer.bottom() <= px(9.), "sorting footer still reserves the old tall spacing");
                    anyhow::ensure!(app.editor.spacing.occluded_bounds().is_none(), "inactive spacing handles left text hidden");
                    Ok((spacing_point(app, prefix), app.config.taskbar_content.clone(), app.editor.selected))
                })??;
                this.update_in(cx, |app, window, cx| {
                    app.editor.spacing.set_preview_hovered(true);
                    app.start_taskbar_spacing(taskbar_spacing::GuideKey {
                        kind: if prefix { taskbar_spacing::SpacingKind::Prefix } else { taskbar_spacing::SpacingKind::Segment }, column: 0,
                    }, start.x, window, cx);
                })?;
                this.update_in(cx, |app, _, _| {
                    anyhow::ensure!(app.editor.spacing.is_dragging() && app.editor.drag.is_none(), "spacing guide did not receive mouse down, or started sorting");
                    Ok(())
                })??;
                let end = point(start.x + px(8. * crate::taskbar_text::PREVIEW_SCALE), start.y);
                this.update_in(cx, |app, _, cx| app.update_taskbar_spacing(end.x, cx))?;
                Timer::after(Duration::from_millis(80)).await;
                this.update_in(cx, |app, _, _| {
                    let preview = app.editor.spacing.preview_content().unwrap();
                    let mask = app.editor.spacing.occluded_bounds().ok_or_else(|| anyhow::anyhow!("active spacing grip did not hide text behind it"))?;
                    let metrics = app.editor.traffic_bounds.get();
                    let expected_width = px(f32::from(if prefix { preview.prefix_gap } else { preview.gap }) * crate::taskbar_text::PREVIEW_SCALE).max(px(1.));
                    anyhow::ensure!(mask.top() <= metrics.top() && mask.bottom() > metrics.bottom() && (mask.size.width - expected_width).abs() < px(0.01) && (app.editor.spacing.card_bounds.get().size.height - original_height).abs() < px(0.5), "spacing grip width did not follow the expanded gap, or preview height changed");
                    anyhow::ensure!(app.config.taskbar_content == baseline, "spacing drag persisted before mouse up");
                    anyhow::ensure!(if prefix { preview.prefix_gap == baseline.prefix_gap + 8 && preview.gap == baseline.gap } else { preview.gap == baseline.gap + 8 && preview.prefix_gap == baseline.prefix_gap }, "spacing preview changed the wrong interval or did not follow pointer movement");
                    Ok(())
                })??;
                this.update_in(cx, |app, _, cx| app.update_taskbar_spacing(start.x + px(4. * crate::taskbar_text::PREVIEW_SCALE), cx))?;
                Timer::after(Duration::from_millis(80)).await;
                this.update_in(cx, |app, _, _| {
                    let value = if prefix { baseline.prefix_gap } else { baseline.gap };
                    let expected_width = px(f32::from(value + 4) * crate::taskbar_text::PREVIEW_SCALE);
                    let mask = app.editor.spacing.occluded_bounds().ok_or_else(|| anyhow::anyhow!("shrinking the spacing lost the active grip"))?;
                    anyhow::ensure!((mask.size.width - expected_width).abs() < px(0.01) && app.config.taskbar_content == baseline, "spacing grip did not shrink with the provisional gap");
                    Ok(())
                })??;
                this.update_in(cx, |app, _, cx| app.finish_taskbar_spacing(end.x, cx))?;
                this.update_in(cx, |app, _, _| {
                    anyhow::ensure!(!app.editor.spacing.is_dragging() && app.editor.selected == selection, "spacing release selected a metric or left a drag active");
                    anyhow::ensure!(if prefix { app.config.taskbar_content.prefix_gap == baseline.prefix_gap + 8 } else { app.config.taskbar_content.gap == baseline.gap + 8 }, "mouse up did not apply spacing");
                    Ok(())
                })??;
                Timer::after(Duration::from_millis(100)).await;
            }
            Timer::after(Duration::from_millis(500)).await;
            this.update_in(cx, |app, _, _| {
                let saved = Config::load(&app.backend.dir)?;
                let saved_fields = toml::Value::try_from(&saved)?;
                let expected_fields = toml::Value::try_from(&app.config)?;
                let differences = expected_fields.as_table().unwrap().iter().filter_map(|(key,value)|
                    (saved_fields.get(key) != Some(value)).then_some(key)).collect::<Vec<_>>();
                anyhow::ensure!(saved == app.config, "spacing changes were not saved; differing fields: {differences:?}");
                Ok(())
            })??;
            let (start, baseline) = this.update_in(cx, |app, _, _| (spacing_point(app, true), app.config.taskbar_content.clone()))?;
            this.update_in(cx, |app, window, cx| {
                app.start_taskbar_spacing(taskbar_spacing::GuideKey { kind: taskbar_spacing::SpacingKind::Prefix, column: 0 }, start.x, window, cx);
                app.update_taskbar_spacing(start.x + px(200.), cx);
                anyhow::ensure!(app.editor.spacing.preview_content().unwrap().prefix_gap == 32, "spacing preview exceeded its upper bound");
                Ok(())
            })??;
            Timer::after(Duration::from_millis(80)).await;
            this.update_in(cx, |app, _, _| {
                let mask = app.editor.spacing.occluded_bounds().ok_or_else(|| anyhow::anyhow!("maximum spacing lost the grip"))?;
                anyhow::ensure!((mask.size.width - px(32. * crate::taskbar_text::PREVIEW_SCALE)).abs() < px(0.01), "maximum spacing still uses a fixed grip width");
                Ok(())
            })??;
            cx.update(|window, cx| { window.dispatch_keystroke(Keystroke::parse("escape").unwrap(), cx); })?;
            this.update_in(cx, |app, window, cx| {
                anyhow::ensure!(!app.editor.spacing.is_dragging() && app.config.taskbar_content == baseline, "Escape wrote or retained a spacing preview");
                app.act(Action::Display(DisplayAction::TaskbarGap(0)), window, cx);
                Ok(())
            })??;
            Timer::after(Duration::from_millis(100)).await;
            let start = this.update_in(cx, |app, _, _| spacing_point(app, false))?;
            this.update_in(cx, |app, window, cx| {
                app.start_taskbar_spacing(taskbar_spacing::GuideKey { kind: taskbar_spacing::SpacingKind::Segment, column: 0 }, start.x, window, cx);
                app.update_taskbar_spacing(start.x - px(200.), cx);
                anyhow::ensure!(app.editor.spacing.preview_content().unwrap().gap == 0, "spacing preview exceeded its lower bound");
                Ok(())
            })??;
            Timer::after(Duration::from_millis(80)).await;
            this.update_in(cx, |app, _, cx| {
                let mask = app.editor.spacing.occluded_bounds().ok_or_else(|| anyhow::anyhow!("zero spacing lost its visible marker"))?;
                anyhow::ensure!((mask.size.width - px(1.)).abs() < px(0.01), "zero spacing did not shrink to a hairline marker");
                app.finish_taskbar_spacing(start.x + px(5. * crate::taskbar_text::PREVIEW_SCALE), cx);
                Ok(())
            })??;
            this.update_in(cx, |app, window, cx| {
                anyhow::ensure!(app.config.taskbar_content.gap == 5, "a zero-width gap cannot be expanded");
                app.act(Action::Display(DisplayAction::TaskbarReset), window, cx);
                Ok(())
            })??;
            // Exercise shared spacing handlers against actual rendered bounds.
            // Native pointer/wheel dispatch remains a separate desktop check.
            for prefix in [false, true] {
                this.update_in(cx, |app, window, cx| {
                    app.act(Action::Display(DisplayAction::TaskbarReset), window, cx);
                    app.config.taskbar_content.gap = 32;
                    app.config.taskbar_content.prefix_gap = 32;
                    app.save_display_content(cx);
                    app.editor.spacing.set_preview_hovered(true);
                    window.blur();
                    cx.notify();
                })?;
                Timer::after(Duration::from_millis(80)).await;
                let (start, selection, scroll) = this.update_in(cx, |app, _, _| (spacing_point(app, prefix), app.editor.selected, app.editor.preview_scroll.offset()))?;
                this.update_in(cx, |app, window, cx| { app.exercise_taskbar_spacing(prefix, start, None, window, cx); })?;
                Timer::after(Duration::from_millis(80)).await;
                let consumed = this.update_in(cx, |app, window, cx| app.exercise_taskbar_spacing(prefix, start, Some(ScrollDelta::Lines(point(0., -96.))), window, cx))?;
                anyhow::ensure!(consumed, "spacing wheel did not claim its adjustment");
                Timer::after(Duration::from_millis(80)).await;
                this.update_in(cx, |app, _, _| {
                    let mask = app.editor.spacing.occluded_bounds().ok_or_else(|| anyhow::anyhow!("wheel shrink lost the highlighted guide"))?;
                    anyhow::ensure!(mask.size.width == px(1.) && !mask.dilate(px(7.)).contains(&start), "test did not move the zero-width guide away from the stationary pointer");
                    anyhow::ensure!(app.editor.preview_scroll.offset() == scroll && app.editor.selected == selection, "spacing wheel scrolled the preview or selected a metric");
                    Ok(())
                })??;
                // A tiny mouse movement inside the frozen region also retains it.
                this.update_in(cx, |app, window, cx| { app.exercise_taskbar_spacing(prefix, start + point(px(0.2), px(0.)), None, window, cx); })?;
                this.update_in(cx, |app, window, cx| { app.exercise_taskbar_spacing(prefix, start, Some(ScrollDelta::Lines(point(0., 3.))), window, cx); })?;
                cx.update(|window, cx| {
                    window.dispatch_keystroke(Keystroke::parse("up").unwrap(), cx);
                    window.dispatch_keystroke(Keystroke::parse("down").unwrap(), cx);
                })?;
                this.update_in(cx, |app, _, _| {
                    let content = &app.config.taskbar_content;
                    anyhow::ensure!((if prefix { content.prefix_gap } else { content.gap }) == 1 && (if prefix { content.gap } else { content.prefix_gap }) == 32, "wheel/keys lost the retained guide or changed the other interval");
                    Ok(())
                })??;
                this.update_in(cx, |app, window, cx| { app.exercise_taskbar_spacing(prefix, start, Some(ScrollDelta::Pixels(point(px(0.), px(-12.)))), window, cx); })?;
                this.update_in(cx, |app, _, _| {
                    anyhow::ensure!((if prefix { app.config.taskbar_content.prefix_gap } else { app.config.taskbar_content.gap }) == 1, "partial precision wheel delta changed an entire step");
                    Ok(())
                })??;
                this.update_in(cx, |app, window, cx| { app.exercise_taskbar_spacing(prefix, start, Some(ScrollDelta::Pixels(point(px(0.), px(-12.)))), window, cx); })?;
                cx.update(|window, cx| {
                    for _ in 0..35 { window.dispatch_keystroke(Keystroke::parse("up").unwrap(), cx); }
                })?;
                this.update_in(cx, |app, _, _| {
                    anyhow::ensure!((if prefix { app.config.taskbar_content.prefix_gap } else { app.config.taskbar_content.gap }) == 32, "keyboard upper boundary was not clamped");
                    Ok(())
                })??;
                cx.update(|window, cx| { for _ in 0..35 { window.dispatch_keystroke(Keystroke::parse("down").unwrap(), cx); } })?;
                this.update_in(cx, |app, _, _| {
                    anyhow::ensure!((if prefix { app.config.taskbar_content.prefix_gap } else { app.config.taskbar_content.gap }) == 0, "keyboard lower boundary was not clamped");
                    Ok(())
                })??;
                cx.update(|window, cx| {
                    window.dispatch_keystroke(Keystroke::parse("escape").unwrap(), cx);
                    window.dispatch_keystroke(Keystroke::parse("up").unwrap(), cx);
                })?;
                this.update_in(cx, |app, _, _| {
                    anyhow::ensure!((if prefix { app.config.taskbar_content.prefix_gap } else { app.config.taskbar_content.gap }) == 0, "Escape left spacing keys active");
                    Ok(())
                })??;
                // Drag from 32 down to 0 through shared handlers, then wheel
                // at the release point without returning to the moved shadow.
                this.update_in(cx, |app, window, cx| {
                    app.act(Action::Display(DisplayAction::TaskbarReset), window, cx);
                    app.config.taskbar_content.gap = 32;
                    app.config.taskbar_content.prefix_gap = 32;
                    app.save_display_content(cx);
                    app.editor.spacing.set_preview_hovered(true);
                    cx.notify();
                })?;
                Timer::after(Duration::from_millis(80)).await;
                let start = this.update_in(cx, |app, _, _| spacing_point(app, prefix))?;
                let end = start - point(px(32. * crate::taskbar_text::PREVIEW_SCALE), px(0.));
                this.update_in(cx, |app, window, cx| {
                    app.exercise_taskbar_spacing(prefix, start, None, window, cx);
                    app.start_taskbar_spacing(taskbar_spacing::GuideKey { kind: if prefix { taskbar_spacing::SpacingKind::Prefix } else { taskbar_spacing::SpacingKind::Segment }, column: 0 }, start.x, window, cx);
                    app.update_taskbar_spacing(end.x, cx);
                })?;
                Timer::after(Duration::from_millis(80)).await;
                this.update_in(cx, |app, _, cx| {
                    anyhow::ensure!(app.editor.spacing.is_dragging(), "spacing drag ended before release (prefix={prefix})");
                    app.release_taskbar_spacing(end, cx);
                    anyhow::ensure!((if prefix { app.config.taskbar_content.prefix_gap } else { app.config.taskbar_content.gap }) == 0, "spacing release did not commit zero (prefix={prefix})");
                    Ok(())
                })??;
                Timer::after(Duration::from_millis(80)).await;
                let consumed = this.update_in(cx, |app, window, cx| app.exercise_taskbar_spacing(prefix, end, Some(ScrollDelta::Lines(point(0., 3.))), window, cx))?;
                this.update_in(cx, |app, _, _| {
                    anyhow::ensure!(!app.editor.spacing.is_dragging() && (if prefix { app.config.taskbar_content.prefix_gap } else { app.config.taskbar_content.gap }) == 1, "drag release did not retain wheel control at the pointer (prefix={prefix}, consumed={consumed}, content={:?}, pointer={end:?}, card={:?})", app.config.taskbar_content, app.editor.spacing.card_bounds.get());
                    Ok(())
                })??;
                this.update_in(cx, |app, window, cx| app.editor.label.read(cx).focus_handle(cx).focus(window))?;
                cx.update(|window, cx| { window.dispatch_keystroke(Keystroke::parse("up").unwrap(), cx); })?;
                this.update_in(cx, |app, window, _| {
                    anyhow::ensure!((if prefix { app.config.taskbar_content.prefix_gap } else { app.config.taskbar_content.gap }) == 1, "spacing key intercepted an input arrow");
                    window.blur();
                    Ok(())
                })??;
                let blank = this.update_in(cx, |app, _, _| { let card = app.editor.spacing.card_bounds.get(); point(card.left() + px(6.), card.bottom() - px(10.)) })?;
                this.update_in(cx, |app, window, cx| { app.exercise_taskbar_spacing(prefix, blank, None, window, cx); })?;
                cx.update(|window, cx| {
                    window.dispatch_keystroke(Keystroke::parse("up").unwrap(), cx);
                })?;
                this.update_in(cx, |app, _, _| {
                    anyhow::ensure!((if prefix { app.config.taskbar_content.prefix_gap } else { app.config.taskbar_content.gap }) == 1, "moving outside retained spacing keyboard control");
                    Ok(())
                })??;
            }
            Timer::after(Duration::from_millis(500)).await;
            this.update_in(cx, |app, window, cx| {
                anyhow::ensure!(Config::load(&app.backend.dir)? == app.config, "wheel/key spacing changes were not saved");
                app.act(Action::Display(DisplayAction::TaskbarReset), window, cx);
                Ok(())
            })??;
            // Render every palette and mode in a narrow viewport; check real
            // layout bounds, not just the style builder's declared constants.
            this.update_in(cx, |app, window, cx| {
                window.resize(size(px(900.), px(700.)));
                app.select_taskbar_item(Metric::Upload, window, cx);
            })?;
            for color in ColorTheme::ALL {
                for mode in ["light", "dark"] {
                    this.update_in(cx, |app, window, cx| {
                        app.act(Action::Theme(mode), window, cx);
                        app.act(Action::ColorTheme(color), window, cx);
                        app.editor.settings_scroll.set_offset(point(px(0.), -app.editor.settings_scroll.max_offset().height));
                        app.toggle_picker("taskbar-precision", precision_choices(), window, cx);
                    })?;
                    Timer::after(Duration::from_millis(100)).await;
                    this.update_in(cx, |app, window, cx| {
                        let bounds = app.picker_bounds["taskbar-precision"].get();
                        let unit_bounds = app.picker_bounds["taskbar-unit"].get();
                        check_preview_layout(app)?;
                        anyhow::ensure!(bounds.size.width >= px(110.) && (bounds.size.height - px(34.)).abs() < px(1.), "palette or narrow viewport changed control dimensions");
                        anyhow::ensure!((unit_bounds.origin.y - bounds.origin.y).abs() < px(1.) && unit_bounds.size.width >= px(70.) && bounds.left() - unit_bounds.right() >= px(55.), "unit, style button and precision did not fit on the same row: {unit_bounds:?}, {bounds:?}");
                        anyhow::ensure!(app.p.dark == (mode == "dark") && app.config.color_theme == color, "theme did not apply");
                        app.close_picker(window, cx);
                        Ok(())
                    })??;
                }
            }
            this.update_in(cx, |app, window, cx| {
                let choices = (0..30).map(|index| (format!("选项 {index}"), Action::Display(DisplayAction::TaskbarPrecision(Some((index % 3) as u8))), index == 0)).collect();
                app.toggle_picker("taskbar-precision", choices, window, cx);
            })?;
            Timer::after(Duration::from_millis(100)).await;
            this.update_in(cx, |app, _, _| {
                anyhow::ensure!(app.picker_scroll.max_offset().height > px(0.), "long dropdown has no scroll range");
                Ok(())
            })??;
            cx.update(|window, cx| { window.dispatch_keystroke(Keystroke::parse("end").unwrap(), cx); })?;
            Timer::after(Duration::from_millis(100)).await;
            this.update_in(cx, |app, window, cx| {
                anyhow::ensure!(app.picker.as_ref().is_some_and(|picker| picker.cursor == 29) && app.picker_scroll.offset().y < px(0.), "keyboard selection did not reveal the last option");
                app.close_picker(window, cx);
                app.select_taskbar_item(Metric::Upload, window, cx);
                app.editor.settings_scroll.set_offset(point(px(0.), px(0.)));
                cx.notify();
                Ok(())
            })??;
            Timer::after(Duration::from_millis(100)).await;
            let destination = this.update_in(cx, |app, window, cx| {
                let maximum = app.editor.settings_scroll.max_offset().height;
                anyhow::ensure!(maximum > px(0.), "wheel regression needs overflowing settings");
                for _ in 0..2 {
                    app.editor.settings_scrollbar.scroll(&app.editor.settings_scroll, true,
                        ScrollDelta::Lines(point(0., -3.)), window.line_height());
                    cx.notify();
                }
                anyhow::ensure!(app.editor.settings_scroll.offset().y == px(0.), "line wheel still jumps immediately instead of animating");
                Ok((-window.line_height() * 6.).max(-maximum))
            })??;
            Timer::after(Duration::from_millis(300)).await;
            this.update_in(cx, |app, window, cx| {
                let offset = app.editor.settings_scroll.offset().y;
                anyhow::ensure!((offset - destination).abs() < px(0.5), "wheel animation did not reach its accumulated target: {offset:?} vs {destination:?}");
                app.editor.settings_scrollbar.scroll(&app.editor.settings_scroll, true,
                    ScrollDelta::Pixels(point(px(0.), px(10.))), window.line_height());
                cx.notify();
                anyhow::ensure!((app.editor.settings_scroll.offset().y - (offset + px(10.))).abs() < px(0.5), "precision scrolling was delayed or applied twice");
                let choices = Metric::Upload.unit_choices().iter().map(|&(unit, text)| (text.into(), Action::Display(DisplayAction::TaskbarUnit(unit)), unit == ByteUnit::Auto)).collect();
                app.toggle_picker("taskbar-unit", choices, window, cx);
                anyhow::ensure!(app.picker.as_ref().unwrap().choices.iter().map(|(text, _, _)| text.as_str()).collect::<Vec<_>>() == ["自动", "K", "M"], "unit options are inconsistent");
                Ok(())
            })??;
            Timer::after(Duration::from_millis(100)).await;
            cx.update(|window, cx| { window.dispatch_keystroke(Keystroke::parse("down").unwrap(), cx); })?;
            cx.update(|window, cx| { window.dispatch_keystroke(Keystroke::parse("enter").unwrap(), cx); })?;
            this.update_in(cx, |app, window, cx| {
                anyhow::ensure!(app.picker.is_none() && app.taskbar_item(Metric::Upload).unit == Some(ByteUnit::K), "unit keyboard selection failed");
                anyhow::ensure!(app.taskbar_item(Metric::Upload).unit_style == UnitStyle::Short, "reset must restore short units");
                app.act(Action::Display(DisplayAction::TaskbarUnitStyle), window, cx);
                let full = app.taskbar_item(Metric::Upload).value(&app.data);
                app.act(Action::Display(DisplayAction::TaskbarUnitStyle), window, cx);
                let short = app.taskbar_item(Metric::Upload).value(&app.data);
                anyhow::ensure!(app.taskbar_item(Metric::Upload).unit_style == UnitStyle::Short && short == full.replace("KB/s", "K/s"), "unit style did not update the live value");
                app.act(Action::Display(DisplayAction::TaskbarUnit(ByteUnit::M)), window, cx);
                anyhow::ensure!(app.taskbar_item(Metric::Upload).unit_style == UnitStyle::Short, "selecting a unit changed the unit style");
                app.act(Action::Display(DisplayAction::TaskbarUnitStyle), window, cx);
                anyhow::ensure!(app.taskbar_item(Metric::Upload).unit_style == UnitStyle::Full, "unit style did not toggle back");
                Ok(())
            })??;
            this.update_in(cx, |app, window, cx| {
                app.close_picker(window, cx);
                app.select_taskbar_item(Metric::Cpu, window, cx);
                app.editor.colors[1].read(cx).focus_handle(cx).focus(window);
                app.editor.colors[0].update(cx, |input, cx| input.set_value("#EF4444", window, cx));
                app.editor.colors[1].update(cx, |input, cx| input.set_value("#invalid", window, cx));
                app.act(Action::Display(DisplayAction::TaskbarFont("Consolas".into())), window, cx);
                app.act(Action::Display(DisplayAction::TaskbarFontSize(20.)), window, cx);
                app.act(Action::Display(DisplayAction::TaskbarHidePercent(true)), window, cx);
                cx.notify();
            })?;
            Timer::after(Duration::from_millis(150)).await;
            this.update_in(cx, |app, window, cx| {
                let item = app.taskbar_item(Metric::Cpu);
                anyhow::ensure!(item.text_color == Some(0xEF4444) && item.background_color.is_none(), "color subscriptions lost a valid color or accepted invalid hex");
                app.update_taskbar_color_input(TaskbarColorTarget::Background, true, cx);
                anyhow::ensure!(!app.editor.color_errors[1].is_empty(), "invalid hex has no validation message");
                app.act(Action::Display(DisplayAction::TaskbarColor(TaskbarColorTarget::Background, Some(0x111827))), window, cx);
                anyhow::ensure!(app.editor.color_errors[1].is_empty(), "choosing a valid color did not clear the error");
                let frame = app.content_frame(&app.config.taskbar_content);
                let cpu = frame.rows.iter().flatten().find(|cell| cell.metric == Metric::Cpu).unwrap();
                anyhow::ensure!(cpu.text_color == Some(0xEF4444) && cpu.background_color == Some(0x111827) && cpu.segments[2].text.is_empty(), "appearance is missing from the preview");
                anyhow::ensure!(frame.font_family == "Consolas" && frame.font_size == 20., "font changes did not reach the preview");
                for weight in TaskbarFontWeight::ALL {
                    app.act(Action::Display(DisplayAction::TaskbarFontWeight(weight)), window, cx);
                    anyhow::ensure!(app.editor.weight.read(cx).value().as_ref() == weight.value().to_string(), "preset did not update numeric weight input");
                    let frame = app.content_frame(&app.config.taskbar_content);
                    anyhow::ensure!(frame.font_weight == weight && frame.metric_frame(Metric::Cpu).unwrap().font_weight == weight, "font weight did not reach the preview and example");
                }
                check_preview_layout(app)?;
                app.select_taskbar_item(Metric::Vram, window, cx);
                anyhow::ensure!(app.editor.colors[0].read(cx).value().is_empty() && app.editor.colors[1].read(cx).value().is_empty(), "switching metrics leaked colors into another inspector");
                app.act(Action::Display(DisplayAction::TaskbarColor(TaskbarColorTarget::Text, Some(0x22C55E))), window, cx);
                app.select_taskbar_item(Metric::Cpu, window, cx);
                anyhow::ensure!(app.editor.colors[0].read(cx).value().as_ref() == "#EF4444", "switching selection lost the CPU color");
                app.editor.colors[0].update(cx, |input, cx| input.set_value("", window, cx));
                Ok(())
            })??;
            exercise_font_controls(&this, cx).await?;
            this.update_in(cx, |app, window, cx| {
                app.editor.size.update(cx, |input, cx| input.set_value("12.5", window, cx));
                app.editor.weight.read(cx).focus_handle(cx).focus(window);
                app.editor.weight.update(cx, |input, cx| input.set_value("450", window, cx));
            })?;
            Timer::after(Duration::from_millis(150)).await;
            this.update_in(cx, |app, window, cx| {
                anyhow::ensure!(app.config.taskbar_content.font_weight.value() == 450, "custom numeric weight subscription failed");
                let frame = app.content_frame(&app.config.taskbar_content);
                anyhow::ensure!(frame.font_weight.value() == 450, "numeric weight did not reach preview");
                app.editor.weight.update(cx, |input, cx| input.set_value("901", window, cx));
                Ok(())
            })??;
            Timer::after(Duration::from_millis(150)).await;
            this.update_in(cx, |app, window, cx| {
                anyhow::ensure!(app.config.taskbar_content.font_weight.value() == 450, "out-of-range numeric weight was applied");
                app.update_taskbar_weight_input(true, cx);
                anyhow::ensure!(app.editor.weight_error, "invalid numeric weight has no validation feedback");
                app.act(Action::Display(DisplayAction::TaskbarFontWeight(TaskbarFontWeight::NORMAL)), window, cx);
                anyhow::ensure!(!app.editor.weight_error && app.editor.weight.read(cx).value().as_ref() == "400", "preset did not replace invalid weight draft");
                app.editor.weight.update(cx, |input, cx| input.set_value("550", window, cx));
                Ok(())
            })??;
            Timer::after(Duration::from_millis(700)).await;
            this.update_in(cx, |app, _, _| {
                anyhow::ensure!(app.config.taskbar_content.font_weight.value() == 550, "custom weight was not retained for persistence");
                anyhow::ensure!(app.config.taskbar_content.font_size == 12.5, "fractional size did not persist");
                anyhow::ensure!(app.taskbar_item(Metric::Cpu).text_color.is_none(), "empty color input did not restore the theme default");
                anyhow::ensure!(app.taskbar_item(Metric::Vram).text_color == Some(0x22C55E), "switching selection lost draft colors");
                anyhow::ensure!(Config::load(&app.backend.dir)? == app.config, "font, size, colors and hidden percent did not persist");
                Ok(())
            })??;
            floating_tests::exercise(&this, cx).await?;
            println!("GPUI editor checks passed, including appearance input, invalid colors, per-item colors, font/size, hidden percent, draft selection and persistence; native pointer drag is a separate check");
            Ok(())
        }.await;
        *OUTCOME.lock().unwrap() = Some(outcome.map_err(|error| error.to_string()));
        let _ = cx.update(|_, cx| cx.quit());
    }).detach();
}

#[::core::prelude::v1::test]
#[ignore = "opens this application's GPUI editor with temporary data for input and persistence checks"]
fn gpui_taskbar_editor_input_and_persistence() {
    let dir = tempfile::tempdir().unwrap();
    Config {
        theme: "light".into(),
        color_theme: ColorTheme::CloudBlue,
        floating: false,
        taskbar: false,
        taskbar_position_mode: TaskbarPositionMode::Manual,
        taskbar_left: true,
        taskbar_offset: 800,
        ..Config::default()
    }
    .save(dir.path())
    .unwrap();
    let backend = Backend::start(dir.path().to_path_buf()).unwrap();
    RUN.store(true, Ordering::Release);
    let result = run(backend.clone());
    let shutdown = backend.shutdown();
    RUN.store(false, Ordering::Release);
    result.unwrap();
    shutdown.unwrap();
    OUTCOME
        .lock()
        .unwrap()
        .take()
        .expect("GPUI editor regression did not finish")
        .unwrap();
}
