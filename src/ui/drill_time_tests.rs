use super::*;

async fn pointer(
    this: &WeakEntity<TrafficApp>,
    cx: &mut AsyncWindowContext,
    position: Point<Pixels>,
    wheel: bool,
) -> anyhow::Result<()> {
    this.update_in(cx, |app, window, _| {
        use windows_sys::Win32::{
            Foundation::POINT,
            Graphics::Gdi::ClientToScreen,
            UI::WindowsAndMessaging::{PostMessageW, WM_MOUSEMOVE, WM_MOUSEWHEEL},
        };
        let mut position = POINT {
            x: (f32::from(position.x) * window.scale_factor()) as i32,
            y: (f32::from(position.y) * window.scale_factor()) as i32,
        };
        let coords = |p: &POINT| (p.x as u16 as u32 | ((p.y as u16 as u32) << 16)) as isize;
        unsafe {
            anyhow::ensure!(
                PostMessageW(app.main_hwnd as _, WM_MOUSEMOVE, 0, coords(&position)) != 0,
                "locator mouse event failed"
            );
            if wheel {
                anyhow::ensure!(
                    ClientToScreen(app.main_hwnd as _, &mut position) != 0,
                    "locator screen coordinates unavailable"
                );
                anyhow::ensure!(
                    PostMessageW(
                        app.main_hwnd as _,
                        WM_MOUSEWHEEL,
                        120 << 16,
                        coords(&position)
                    ) != 0,
                    "locator wheel event failed"
                );
            }
        }
        Ok::<_, anyhow::Error>(())
    })??;
    Timer::after(Duration::from_millis(65)).await;
    settle_layout(this, cx).await
}

pub(super) async fn exercise(
    this: &WeakEntity<TrafficApp>,
    cx: &mut AsyncWindowContext,
) -> anyhow::Result<()> {
    let (view, pending, position, history) = this.update_in(cx, |app, window, cx| {
        app.seconds.pinned = false;
        app.seconds.locator.reset();
        app.follow_drill_time(window, cx);
        let plot = app.seconds.second_bounds.get();
        let view = app.seconds.view.unwrap();
        anyhow::ensure!(
            app.drill_locator_time() == Some(view.from + (view.to - view.from) / 2),
            "initial locator did not use range midpoint"
        );
        Ok::<_, anyhow::Error>((
            view,
            app.seconds.pending.clone(),
            point(
                plot.left() + plot.size.width * (110. / 600.),
                plot.bottom() - px(5.),
            ),
            app.seconds.inline_history,
        ))
    })??;
    pointer(this, cx, position, false).await?;
    let expected = this.update_in(cx, |app, window, _| {
        let expected = drill_time::cursor_time(
            view.from,
            view.to,
            app.seconds.second_bounds.get(),
            position,
        )
        .unwrap()
        .div_euclid(1000)
            * 1000;
        let actual = app
            .seconds
            .locator
            .preview_at
            .ok_or_else(|| anyhow::anyhow!("chart mouse event did not update locator"))?;
        let pixel_ms = ((view.to - view.from) as f64
            / f64::from(
                f32::from(app.seconds.second_bounds.get().size.width) * window.scale_factor(),
            ))
        .ceil() as i64;
        anyhow::ensure!(
            (actual - expected).abs() <= pixel_ms + 1000,
            "locator did not follow actual chart mouse event: {actual} vs {expected}"
        );
        anyhow::ensure!(
            app.seconds.pending == pending && app.seconds.view == Some(view),
            "mouse preview submitted a window query"
        );
        Ok::<_, anyhow::Error>(actual)
    })??;
    this.update_in(cx, |app, window, cx| {
        app.seconds.hover = None;
        app.seconds.hover_data = None;
        if history {
            app.act(Action::Drill(DrillAction::DrillLocateToggle), window, cx);
        }
        cx.notify();
    })?;
    settle_layout(this, cx).await?;
    let (field, scroll) = this.update_in(cx, |app, window, _| {
        let fields = app.seconds.locator.bounds.each_ref().map(|b| b.get());
        anyhow::ensure!(
            fields.iter().all(|b| b.size.width >= px(21.)
                && b.left() >= px(0.)
                && b.right() <= window.viewport_size().width),
            "locator fields overflow: {fields:?}"
        );
        Ok::<_, anyhow::Error>((fields[5].center(), app.seconds.scroll.offset()))
    })??;
    pointer(this, cx, field, true).await?;
    this.update_in(cx, |app, window, cx| {
        anyhow::ensure!(
            !app.seconds.locator.following && app.drill_time_result(cx) == Some(expected + 1000),
            "wheel did not increment the hovered second: {:?}, expected {}, following {}",
            app.drill_time_result(cx),
            expected + 1000,
            app.seconds.locator.following
        );
        anyhow::ensure!(
            app.seconds.pending == pending
                && app.seconds.view == Some(view)
                && app.seconds.scroll.offset() == scroll,
            "draft wheel queried or scrolled the modal"
        );
        app.seconds.locator.parts[5].focus_handle(cx).focus(window);
        Ok::<_, anyhow::Error>(())
    })??;
    cx.update(|window, cx| window.dispatch_keystroke(Keystroke::parse("up").unwrap(), cx))?;
    this.update_in(cx, |app, _, cx| {
        anyhow::ensure!(
            app.drill_time_result(cx) == Some(expected + 2000) && app.seconds.pending == pending,
            "keyboard step queried or failed to carry time"
        );
        Ok::<_, anyhow::Error>(())
    })??;
    pointer(this, cx, position, false).await?;
    this.update_in(cx, |app, window, cx| {
        anyhow::ensure!(
            app.drill_time_result(cx) == Some(expected + 2000),
            "chart mouse movement overwrote manual draft"
        );
        app.seconds.locator.parts[0].update(cx, |s, cx| {
            s.set_value(seconds_page::date(expected + 5000), window, cx)
        });
        Ok::<_, anyhow::Error>(())
    })??;
    settle_layout(this, cx).await?;
    this.update_in(cx, |app, window, cx| {
        anyhow::ensure!(
            app.drill_time_result(cx) == Some(expected + 5000),
            "full datetime paste did not populate segments"
        );
        app.follow_drill_time(window, cx);
        app.edit_drill_time(window, cx);
        app.seconds.locator.parts[0].update(cx, |s, cx| s.set_value("0", window, cx));
        Ok::<_, anyhow::Error>(())
    })??;
    settle_layout(this, cx).await?;
    this.update_in(cx, |app, window, cx| {
        app.locate_seconds(window, cx);
        anyhow::ensure!(
            app.seconds.pending == pending && !app.seconds.locator.error.is_empty(),
            "invalid time queried or missed inline error"
        );
        app.follow_drill_time(window, cx);
        app.edit_drill_time(window, cx);
        app.seconds.locator.parts[5].focus_handle(cx).focus(window);
        Ok::<_, anyhow::Error>(())
    })??;
    if !history {
        let original = this.update_in(cx, |app, _, _| {
            (
                app.seconds.data.clone(),
                app.seconds.loaded_request.clone(),
                app.seconds.path.clone(),
                app.seconds.serial,
            )
        })?;
        cx.update(|window, cx| window.dispatch_keystroke(Keystroke::parse("enter").unwrap(), cx))?;
        this.update_in(cx, |app, _, cx| {
            anyhow::ensure!(
                app.seconds.serial == original.3 + 1
                    && app.seconds.path.len() == original.2.len() + 1,
                "Enter submitted more than one locate query"
            );
            app.seconds.data = original.0;
            app.seconds.loaded_request = original.1;
            app.seconds.path = original.2;
            app.seconds.view = Some(view);
            app.seconds.pending = pending;
            cx.notify();
            Ok::<_, anyhow::Error>(())
        })??;
    }
    this.update_in(cx, |app, window, cx| {
        app.seconds.locate_open = false;
        app.seconds.hover = None;
        app.seconds.hover_data = None;
        app.seconds.hover_pending = None;
        app.seconds.locator.reset();
        app.follow_drill_time(window, cx);
    })?;
    settle_layout(this, cx).await?;
    let saved = this.update_in(cx, |app, _, cx| {
        let saved = app.seconds.data.clone().unwrap();
        let sample = saved.samples.first().unwrap();
        app.seconds.pinned = true;
        app.seconds.hover = Some(crate::traffic_drill::Target::Second(sample.seq));
        anyhow::ensure!(
            app.drill_locator_time() == Some(sample.from),
            "fixed locator did not use the selected sample"
        );
        app.seconds.pinned = false;
        app.seconds.hover = None;
        app.seconds.data.as_mut().unwrap().samples.clear();
        app.seconds.data.as_mut().unwrap().first = None;
        app.seconds.data.as_mut().unwrap().last = None;
        cx.notify();
        Ok::<_, anyhow::Error>(saved)
    })??;
    settle_layout(this, cx).await?;
    let empty_position =
        this.update_in(cx, |app, _, _| app.seconds.second_bounds.get().center())?;
    pointer(this, cx, empty_position, false).await?;
    this.update_in(cx, |app, window, cx| {
        anyhow::ensure!(
            app.seconds.locator.preview_at.is_some()
                && app.seconds.data.as_ref().unwrap().samples.is_empty(),
            "empty seconds chart did not update time preview"
        );
        app.seconds.data = Some(saved);
        app.seconds.hover = None;
        app.seconds.hover_data = None;
        app.seconds.hover_pending = None;
        app.seconds.locator.reset();
        app.follow_drill_time(window, cx);
        Ok::<_, anyhow::Error>(())
    })??;
    settle_layout(this, cx).await?;
    Ok(())
}
