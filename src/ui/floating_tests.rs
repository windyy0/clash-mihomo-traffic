use super::*;

const COMPOSITION_BACKGROUND: u32 = 0x204080;

struct CompositionUnderlay;

impl Render for CompositionUnderlay {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().bg(rgb(COMPOSITION_BACKGROUND))
    }
}

pub(super) async fn exercise(
    this: &WeakEntity<TrafficApp>,
    cx: &mut AsyncWindowContext,
) -> anyhow::Result<()> {
    let baseline = this.update_in(cx, |app, window, cx| {
        app.editor
            .weight
            .update(cx, |input, cx| input.set_value("901", window, cx));
        app.update_taskbar_weight_input(true, cx);
        let baseline = app.config.taskbar_content.clone();
        let label_id = app.editor.label.entity_id();
        app.act(Action::DisplayTab(DisplayTab::Floating), window, cx);
        let surface = (
            app.config.floating_row_gap,
            app.config.floating_background_color,
            app.config.floating_background_transparency,
        );
        app.act(Action::Display(DisplayAction::TaskbarReset), window, cx);
        anyhow::ensure!(
            app.config.floating_content == TaskbarContent::floating()
                && app.config.taskbar_content == baseline
                && (
                    app.config.floating_row_gap,
                    app.config.floating_background_color,
                    app.config.floating_background_transparency
                ) == surface,
            "floating reset did not use the new preset independently of surface settings"
        );
        anyhow::ensure!(
            app.editor.label.entity_id() != label_id && !app.editor.editing,
            "display editor state was reused"
        );
        app.act(Action::Display(DisplayAction::TaskbarEditing), window, cx);
        for metric in Metric::ALL {
            app.act(
                Action::Display(DisplayAction::TaskbarAdd(metric)),
                window,
                cx,
            );
        }
        app.act(Action::Display(DisplayAction::TaskbarRows(13)), window, cx);
        app.act(
            Action::Display(DisplayAction::TaskbarFontSize(20.)),
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
        app.select_taskbar_item(Metric::Cpu, window, cx);
        app.act(
            Action::Display(DisplayAction::TaskbarHidePercent(true)),
            window,
            cx,
        );
        app.act(
            Action::Display(DisplayAction::TaskbarColor(
                TaskbarColorTarget::Text,
                Some(0xEF4444),
            )),
            window,
            cx,
        );
        app.act(
            Action::Display(DisplayAction::TaskbarColor(
                TaskbarColorTarget::Background,
                Some(0x111827),
            )),
            window,
            cx,
        );
        app.config.floating = true;
        app.config.fullscreen_hide = false;
        app.save_display_content(cx);
        app.floating_background
            .update(cx, |input, cx| input.set_value("invalid", window, cx));
        window.resize(size(px(900.), px(700.)));
        cx.notify();
        Ok(baseline)
    })??;
    super::editor_tests::exercise_font_controls(this, cx).await?;
    Timer::after(Duration::from_millis(250)).await;
    this.update_in(cx,|app,window,cx| {
        anyhow::ensure!(app.config.taskbar_content==baseline,"floating edits changed taskbar configuration");
        anyhow::ensure!(app.config.floating_content.rows.len()==13,"multiline editor did not update configuration");
        anyhow::ensure!(app.editor.preview_scroll.bounds().size.height<=px(180.5)&&app.editor.preview_scroll.max_offset().height>px(0.),"multiline preview is not bounded and scrollable");
        let viewport = app.editor.preview_scroll.bounds();
        let preview = app.editor.traffic_bounds.get();
        anyhow::ensure!((preview.center().x - viewport.center().x).abs() <= px(1.), "floating preview is not centered: {preview:?} in {viewport:?}");
        anyhow::ensure!(app.editor.settings_scroll.bounds().size.height>px(0.),"floating settings viewport collapsed");
        for id in ["floating-rows","taskbar-font","taskbar-font-size","taskbar-font-weight"] {
            let bounds=app.picker_bounds[id].get();
            anyhow::ensure!(bounds.size.width>px(0.)&&bounds.right()<=window.viewport_size().width,"floating toolbar control overflowed: {id}: {bounds:?}");
        }
        let (_,_,width,height,dpi)=native::floating_bounds(app.float_hwnd);
        let expected=(13.*24.+12.*8.+28.+2.*crate::floating_backdrop::SHADOW_MARGIN)*dpi as f32/96.;
        anyhow::ensure!(width>0 && (height as f32-expected.ceil()).abs()<=1.,"native floating window did not fit multiline content: {width}x{height}, expected height {expected}");
        anyhow::ensure!(native::floating_client_size(app.float_hwnd) == (width, height), "floating client area still reserves a system border");
        let original_position = native::position(app.float_hwnd);
        if let Some(saved) = app.config.floating_x.zip(app.config.floating_y) {
            anyhow::ensure!(original_position == saved, "floating startup did not restore the saved screen position");
        }
        let moved = (original_position.0 - 37, original_position.1 - 21);
        native::setup_float(app.float_hwnd, moved.0, moved.1);
        native::show(app.float_hwnd, true);
        anyhow::ensure!(native::position(app.float_hwnd) == moved, "visible refresh reset floating position");
        native::show(app.float_hwnd, false);
        native::show(app.float_hwnd, true);
        anyhow::ensure!(native::position(app.float_hwnd) == moved, "show after hide reset floating position");
        native::setup_float(app.float_hwnd, original_position.0, original_position.1);
        let floating = app.float_view.read(cx);
        anyhow::ensure!(floating.scroll.max_offset().width <= px(1.) && floating.scroll.max_offset().height <= px(1.), "floating content is clipped at its natural window size: {:?}", floating.scroll.max_offset());
        app.update_floating_background(true,cx);
        anyhow::ensure!(app.floating_background_error&&app.config.floating_background_color.is_none(),"invalid background input was applied");
        app.floating_background.update(cx,|input,cx|input.set_value("#123456",window,cx));
        app.floating_transparency.update(cx,|slider,cx|{slider.set_value(50.,window,cx);cx.emit(SliderEvent::Change(slider.value()));});
        app.floating_row_gap.update(cx,|slider,cx|{slider.set_value(17.,window,cx);cx.emit(SliderEvent::Change(slider.value()));});
        app.act(Action::Display(DisplayAction::TaskbarRows(1)), window, cx);
        cx.notify();
        Ok(())
    })??;
    Timer::after(Duration::from_millis(200)).await;
    this.update_in(cx, |app, window, cx| {
        anyhow::ensure!(
            app.config.floating_background_color == Some(0x123456)
                && app.config.floating_background_transparency == 50
                && app.config.floating_row_gap == 17,
            "floating appearance subscriptions failed"
        );
        anyhow::ensure!(!app.float_view.read(cx).mica_active, "custom translucent background was overridden by Mica");
        let viewport = app.editor.preview_scroll.bounds();
        let preview = app.editor.traffic_bounds.get();
        anyhow::ensure!(
            app.editor.preview_scroll.max_offset().width > px(0.)
                && (preview.left() - viewport.left() - px(32.)).abs() <= px(1.),
            "wide floating preview is not scrollable from its left edge: {preview:?} in {viewport:?}"
        );
        app.act(Action::Display(DisplayAction::TaskbarRows(13)), window, cx);
        cx.notify();
        Ok(())
    })??;
    Timer::after(Duration::from_millis(150)).await;
    this.update_in(cx, |app, _, cx| {
        let maximum = app.editor.preview_scroll.max_offset().height;
        app.editor
            .preview_scroll
            .set_offset(point(px(0.), -maximum));
        cx.notify();
    })?;
    Timer::after(Duration::from_millis(150)).await;
    this.update_in(cx, |app, window, cx| {
        // Hit testing uses the scrolled multirow bounds, rather than fixed taskbar height.
        app.start_taskbar_drag(Metric::Download, window, cx);
        let bounds = app.editor.traffic_bounds.get();
        let frame = &app.editor.drag.as_ref().unwrap().frame;
        let position = point(
            bounds.left() + px(5.),
            bounds.top() + px(12. * (frame.line_height + 17.) + frame.line_height / 2.),
        );
        app.update_taskbar_drag(position, cx);
        anyhow::ensure!(
            app.editor.drop.is_some(),
            "scrolled floating preview has no drop target"
        );
        app.cancel_taskbar_drag(window, cx);
        app.act(Action::Display(DisplayAction::CopyTaskbar), window, cx);
        anyhow::ensure!(
            app.editor.size.read(cx).value().as_ref() == baseline.font_size.to_string(),
            "copy did not synchronize the fractional size input"
        );
        anyhow::ensure!(
            app.config.floating_content == baseline,
            "copy did not transfer the complete taskbar content"
        );
        anyhow::ensure!(
            app.config.floating_background_color == Some(0x123456)
                && app.config.floating_background_transparency == 50
                && app.config.floating_row_gap == 17,
            "copy overwrote floating surface settings"
        );
        app.act(Action::Display(DisplayAction::TaskbarRows(3)), window, cx);
        app.act(
            Action::Display(DisplayAction::TaskbarFontWeight(TaskbarFontWeight::NORMAL)),
            window,
            cx,
        );
        app.select_taskbar_item(Metric::Gpu, window, cx);
        app.act(
            Action::Display(DisplayAction::TaskbarColor(
                TaskbarColorTarget::Text,
                Some(0x22C55E),
            )),
            window,
            cx,
        );
        app.editor
            .weight
            .update(cx, |input, cx| input.set_value("999", window, cx));
        app.update_taskbar_weight_input(true, cx);
        app.act(Action::DisplayTab(DisplayTab::Taskbar), window, cx);
        anyhow::ensure!(
            app.editor.weight_error && app.editor.weight.read(cx).value().as_ref() == "901",
            "taskbar invalid draft was lost when switching tabs"
        );
        anyhow::ensure!(
            app.taskbar_item(Metric::Vram).text_color == Some(0x22C55E),
            "taskbar item draft was lost"
        );
        anyhow::ensure!(
            app.config.taskbar_content == baseline,
            "copy or later floating edits changed taskbar content"
        );
        app.act(Action::DisplayTab(DisplayTab::Floating), window, cx);
        anyhow::ensure!(
            app.editor.weight_error && app.editor.weight.read(cx).value().as_ref() == "999",
            "floating invalid draft was replaced by taskbar input"
        );
        anyhow::ensure!(
            app.taskbar_item(Metric::Gpu).text_color == Some(0x22C55E)
                && app.config.floating_content.item(Metric::Gpu).is_none(),
            "floating unadded item did not remain a draft"
        );
        app.act(
            Action::Display(DisplayAction::TaskbarFontWeight(
                TaskbarFontWeight::new(450).unwrap(),
            )),
            window,
            cx,
        );
        Ok(())
    })??;
    for width in [900., 1240.] {
        for theme in ColorTheme::ALL {
            for mode in ["light", "dark"] {
                this.update_in(cx, |app, window, cx| {
                    window.resize(size(px(width), px(700.)));
                    app.config.floating_background_color = None;
                    app.config.floating_background_transparency = 0;
                    app.act(Action::ColorTheme(theme), window, cx);
                    app.act(Action::Theme(mode), window, cx);
                    app.save_display_content(cx);
                    cx.notify();
                })?;
                Timer::after(Duration::from_millis(100)).await;
                this.update_in(cx, |app, _, cx| {
                    anyhow::ensure!(
                        app.editor.settings_scroll.bounds().size.height > px(0.),
                        "floating settings collapsed while changing theme"
                    );
                    let floating = app.float_view.read(cx);
                    let (corner, region_size) = native::floating_corner_state(app.float_hwnd);
                    anyhow::ensure!(
                        region_size.is_none() && corner == 1,
                        "floating window still combines system and custom corner clipping"
                    );
                    if let Some(backdrop) = native::floating_backdrop(app.float_hwnd) {
                        anyhow::ensure!(
                            backdrop == 1,
                            "native floating backdrop differs from rendered material: {backdrop}"
                        );
                        anyhow::ensure!(
                            floating.mica_active == (theme == ColorTheme::Mica),
                            "supported Mica material was not applied for the floating theme"
                        );
                    }
                    Ok(())
                })??;
            }
        }
    }
    for rows in [1, 13, 2, 3] {
        this.update_in(cx, |app, window, cx| {
            app.act(Action::ColorTheme(ColorTheme::Mica), window, cx);
            app.act(
                Action::Display(DisplayAction::TaskbarRows(rows)),
                window,
                cx,
            );
        })?;
        Timer::after(Duration::from_millis(100)).await;
        this.update_in(cx, |app, _, _| {
            let (corner, region) = native::floating_corner_state(app.float_hwnd);
            let (width, height) = native::floating_client_size(app.float_hwnd);
            anyhow::ensure!(
                region.is_none() && corner == 1 && width > 0 && height > 0,
                "floating material clipping changed while resizing: {region:?}, {width}x{height}"
            );
            Ok(())
        })??;
    }
    this.update_in(cx, |app, window, cx| {
        app.act(Action::Display(DisplayAction::TaskbarRows(1)), window, cx);
        app.act(Action::Display(DisplayAction::TaskbarRows(2)), window, cx);
        app.act(Action::Display(DisplayAction::TaskbarRows(3)), window, cx);
        app.act(
            Action::Display(DisplayAction::TaskbarAdd(Metric::Gpu)),
            window,
            cx,
        );
        app.act(
            Action::Display(DisplayAction::TaskbarPrecision(Some(2))),
            window,
            cx,
        );
        app.act(
            Action::Display(DisplayAction::TaskbarDevice(
                "temporarily-missing-device".into(),
            )),
            window,
            cx,
        );
        app.config.floating_background_transparency = 100;
        app.save_display_content(cx);
    })?;
    Timer::after(Duration::from_millis(700)).await;
    this.update_in(cx, |app, _, cx| {
        anyhow::ensure!(
            !app.float_view.read(cx).mica_active,
            "fully transparent background still has a Mica backdrop"
        );
        anyhow::ensure!(
            Config::load(&app.backend.dir)? == app.config,
            "floating content, appearance or independent settings were not persisted"
        );
        anyhow::ensure!(
            app.config.taskbar_content == baseline,
            "taskbar configuration changed during floating regression"
        );
        Ok(())
    })??;
    // Sample the actual desktop composition over a uniform, contrasting surface.
    // Controls on the main window can otherwise change the pixels under the halo.
    let underlay = cx.update(|_, cx| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(640.), px(360.)), cx)),
                titlebar: None,
                focus: false,
                kind: WindowKind::PopUp,
                ..Default::default()
            },
            |_, cx| cx.new(|_| CompositionUnderlay),
        )
    })??;
    let underlay_hwnd = underlay.update(cx, |_, window, _| native::hwnd(window))?;
    for mode in ["light", "dark"] {
        this.update_in(cx, |app, window, cx| {
            app.act(Action::Theme(mode), window, cx);
            app.config.floating_background_transparency = 0;
            app.config.floating_background_color = None;
            app.config.floating = true;
            app.config.fullscreen_hide = false;
            app.save_display_content(cx);
            cx.notify();
        })?;
        Timer::after(Duration::from_millis(250)).await;
        this.update_in(cx, |app, _, _| {
            let (x, y, underlay_width, underlay_height, _) = native::floating_bounds(underlay_hwnd);
            let (_, _, width, height, dpi) = native::floating_bounds(app.float_hwnd);
            let scale = dpi as f32 / 96.;
            let margin = (crate::floating_backdrop::SHADOW_MARGIN * scale).round() as i32;
            native::fit_float(
                app.float_hwnd,
                width as f32 / scale,
                height as f32 / scale,
                native::FloatPosition::Saved(
                    x + (underlay_width - width) / 2 + margin,
                    y + (underlay_height - height) / 2 + margin,
                ),
            );
            native::show_composition_fixture(underlay_hwnd, app.float_hwnd);
        })?;
        Timer::after(Duration::from_millis(150)).await;
        this.update_in(cx, |app, _, _| {
            let native::FloatingComposition {outside, corners, inside, halo, far_shadow: shadow} = native::floating_composited_colors(app.float_hwnd)?;
            let distance = |a: u32, b: u32| [0, 8, 16].into_iter()
                .map(|shift| (((a >> shift) & 255) as i32 - ((b >> shift) & 255) as i32).abs())
                .max().unwrap();
            anyhow::ensure!(distance(outside, COMPOSITION_BACKGROUND) <= 3,
                "composition fixture is occluded or not rendered: {outside:06x}");
            // A transparent corner may contain a faint black system shadow;
            // it must preserve the underlay's hue rather than show Mica or a frame.
            let clear_corner = |corner: u32| {
                let alpha = (corner & 255) as f32 / (outside & 255).max(1) as f32;
                (0.8..=1.03).contains(&alpha) && [8, 16].into_iter().all(|shift| {
                    let expected = ((outside >> shift) & 255) as f32 * alpha;
                    ((((corner >> shift) & 255) as f32) - expected).abs() <= 3.
                })
            };
            anyhow::ensure!(corners.into_iter().all(clear_corner),
                "floating {mode} corners still draw an outer rectangle: {corners:x?}, outside={outside:06x}, shadow={shadow:06x}");
            anyhow::ensure!(distance(inside, outside) > 24,
                "floating {mode} Mica background became transparent: {inside:06x}");
            anyhow::ensure!(distance(shadow, outside) <= 3,
                "floating {mode} still casts a long shadow: {shadow:06x}, outside={outside:06x}");
            anyhow::ensure!(halo.into_iter().all(|color| (1..=12).contains(&distance(color, outside)))
                && halo.into_iter().all(|color| distance(color, halo[0]) <= 2),
                "floating {mode} halo is missing, too strong or directional: {halo:x?}, outside={outside:06x}");
            let (_, _, _, _, dpi) = native::floating_bounds(app.float_hwnd);
            println!("Floating {mode} halo at {dpi} DPI: {halo:06x?}");
            Ok(())
        })??;
        let mut material_colors = Vec::new();
        for source in [0xA05090, 0x5090A0] {
            this.update_in(cx, |app, _, cx| {
                app.float_view
                    .read(cx)
                    .test_mica_source(source, mode == "dark")
            })??;
            Timer::after(Duration::from_millis(250)).await;
            material_colors.push(this.update_in(cx, |app, _, _| {
                native::floating_composited_colors(app.float_hwnd).map(|sample| sample.inside)
            })??);
        }
        let difference = [0, 8, 16]
            .into_iter()
            .map(|shift| {
                (((material_colors[0] >> shift) & 255) as i32
                    - ((material_colors[1] >> shift) & 255) as i32)
                    .abs()
            })
            .max()
            .unwrap();
        anyhow::ensure!(
            difference >= if mode == "light" { 6 } else { 3 },
            "floating {mode} material hides wallpaper colors: {material_colors:x?}"
        );
        for color in &material_colors {
            let channels = [0, 8, 16].map(|shift| (color >> shift) & 255);
            anyhow::ensure!(
                channels.into_iter().all(|channel| if mode == "light" {
                    channel >= 190
                } else {
                    channel <= 96
                }),
                "floating {mode} material has incorrect luminosity: {color:06x}"
            );
        }
        println!("Floating {mode} Mica effect colors: {material_colors:x?}");
    }
    underlay.update(cx, |_, window, _| window.remove_window())?;
    println!(
        "Floating editor checks passed: independent drafts/validation, copy, 13 rows, bounded preview, scrolled drag hit testing, native auto sizing, background input/transparency and persistence"
    );
    Ok(())
}
