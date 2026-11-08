use super::*;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

pub static RUN: AtomicBool = AtomicBool::new(false);
static OUTCOME: Mutex<Option<Result<(), String>>> = Mutex::new(None);

// Exercise GPUI's actual dispatcher, including the Input key bindings. A
// direct call to step_position would miss the reported focus/dispatch bug.
async fn press(
    app: &WeakEntity<TrafficApp>,
    cx: &mut AsyncWindowContext,
    key: &str,
    expected: i32,
) -> anyhow::Result<()> {
    cx.update(|window, cx| window.dispatch_keystroke(Keystroke::parse(key).unwrap(), cx))?;
    Timer::after(Duration::from_millis(60)).await;
    let (position, native_position, manual) = app.update_in(cx, |app, _, _| {
        let geometry = app.backend.shared.lock().unwrap().taskbar_geometry;
        (
            app.manual_x(geometry),
            geometry.x,
            app.config.taskbar_position_mode == TaskbarPositionMode::Manual,
        )
    })?;
    anyhow::ensure!(
        position == expected,
        "{key}: expected {expected}px, got {position}px"
    );
    if manual {
        anyhow::ensure!(
            native_position == expected,
            "{key}: Shell layout did not apply {expected}px (got {native_position})"
        );
    }
    println!("GPUI dispatched {key}: {position}px");
    Ok(())
}

pub fn schedule(window: &mut Window, cx: &mut Context<TrafficApp>) {
    cx.spawn_in(window, async move |this, cx| {
        let outcome: anyhow::Result<()> = async {
            let mut ready = false;
            for _ in 0..100 {
                ready = this.update_in(cx, |app, _, _| {
                    app.backend.shared.lock().unwrap().taskbar_geometry.ready
                })?;
                if ready {
                    break;
                }
                Timer::after(Duration::from_millis(20)).await;
            }
            anyhow::ensure!(ready, "Explorer taskbar geometry not ready");
            let start = this.update_in(cx, |app, window, cx| {
                app.act(Action::Nav(Page::Display), window, cx);
                anyhow::ensure!(
                    !app.editor.editing,
                    "position controls must work with the editor closed"
                );
                window.blur(); // Reproduce clicking blank space and losing Div focus.
                Ok::<_, anyhow::Error>(
                    app.manual_x(app.backend.shared.lock().unwrap().taskbar_geometry),
                )
            })??;
            Timer::after(Duration::from_millis(100)).await;
            press(&this, cx, "right", start + 1).await?;
            press(&this, cx, "shift-right", start + 9).await?;
            press(&this, cx, "left", start + 8).await?;
            press(&this, cx, "shift-left", start).await?;

            this.update_in(cx, |app, window, cx| {
                app.taskbar_position
                    .input
                    .read(cx)
                    .focus_handle(cx)
                    .focus(window)
            })?;
            Timer::after(Duration::from_millis(80)).await;
            press(&this, cx, "right", start + 1).await?;
            this.update_in(cx, |app, window, cx| {
                app.taskbar_position
                    .input
                    .update(cx, |s, cx| s.set_value("1000", window, cx));
            })?;
            press(&this, cx, "right", 1001).await?;
            press(&this, cx, "shift-left", 993).await?;
            press(&this, cx, "ctrl-left", 993).await?;

            this.update_in(cx, |app, window, cx| {
                app.taskbar_position
                    .input
                    .update(cx, |s, cx| s.set_value("invalid", window, cx));
            })?;
            press(&this, cx, "right", 993).await?;
            this.update_in(cx, |app, _, _| {
                anyhow::ensure!(
                    !app.taskbar_position.error.is_empty(),
                    "invalid pending number must show an error"
                );
                Ok(())
            })??;
            this.update_in(cx, |app, window, cx| {
                app.taskbar_position
                    .input
                    .update(cx, |s, cx| s.set_value("0", window, cx));
            })?;
            press(&this, cx, "left", 0).await?;
            press(&this, cx, "left", 0).await?;
            let max = this.update_in(cx, |app, window, cx| {
                let max = app.backend.shared.lock().unwrap().taskbar_geometry.max_x;
                app.taskbar_position
                    .input
                    .update(cx, |s, cx| s.set_value(max.to_string(), window, cx));
                max
            })?;
            press(&this, cx, "right", max).await?;
            press(&this, cx, "right", max).await?;

            this.update_in(cx, |app, window, cx| {
                app.act(Action::Nav(Page::Overview), window, cx)
            })?;
            Timer::after(Duration::from_millis(80)).await;
            press(&this, cx, "left", max).await?;
            this.update_in(cx, |app, window, cx| {
                app.act(Action::Nav(Page::Display), window, cx);
                app.act(
                    Action::Display(DisplayAction::TaskbarMode(TaskbarPositionMode::AutoAvoid)),
                    window,
                    cx,
                );
            })?;
            Timer::after(Duration::from_millis(80)).await;
            press(&this, cx, "left", max).await?;
            Ok(())
        }
        .await;
        *OUTCOME.lock().unwrap() = Some(outcome.map_err(|error| error.to_string()));
        let _ = cx.update(|_, cx| cx.quit());
    })
    .detach();
}

#[::core::prelude::v1::test]
#[ignore = "opens this application's GPUI window with temporary data for a keyboard dispatch regression"]
fn gpui_position_keys_survive_blur_and_numeric_focus() {
    let dir = tempfile::tempdir().unwrap();
    Config {
        theme: "light".into(),
        color_theme: ColorTheme::CloudBlue,
        floating: false,
        taskbar: true,
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
    let _ = backend.request_shutdown();
    RUN.store(false, Ordering::Release);
    result.unwrap();
    OUTCOME
        .lock()
        .unwrap()
        .take()
        .expect("GPUI keyboard regression did not finish")
        .unwrap();
}
