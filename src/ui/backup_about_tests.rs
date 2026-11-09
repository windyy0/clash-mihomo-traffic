//! Isolated form and layout checks; no user's backup is restored.
use super::*;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
use windows_sys::Win32::UI::{
    Input::KeyboardAndMouse::{VK_ESCAPE, VK_RETURN, VK_SPACE},
    WindowsAndMessaging::{PostMessageW, WM_KEYDOWN, WM_KEYUP},
};

pub static RUN: AtomicBool = AtomicBool::new(false);
static OUTCOME: Mutex<Option<Result<(), String>>> = Mutex::new(None);

async fn wait_directory(
    this: &WeakEntity<TrafficApp>,
    cx: &mut AsyncWindowContext,
) -> anyhow::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(6);
    while this.update_in(cx, |app, _, _| app.backup_directory_busy)? {
        anyhow::ensure!(
            Instant::now() < deadline,
            "directory preference did not finish saving"
        );
        Timer::after(Duration::from_millis(40)).await;
    }
    Ok(())
}

pub fn schedule(window: &mut Window, cx: &mut Context<TrafficApp>) {
    cx.spawn_in(window, async move |this, cx| {
        let result: anyhow::Result<()> = async {
            this.update_in(cx, |app, window, cx| {
                app.act(Action::Nav(Page::Settings), window, cx);
                app.act(Action::SettingsSection(SettingsSection::Backup), window, cx);
                anyhow::ensure!(!app.can_restore_backup(cx), "empty restore form was enabled");
                app.restore_path.update(cx, |s, cx| s.set_value(r"C:\备份 data\first.sqlite3", window, cx));
                Ok(())
            })??;
            Timer::after(Duration::from_millis(80)).await;
            this.update_in(cx, |app, window, cx| {
                app.act(Action::Backup(BackupAction::RestoreArm), window, cx);
                anyhow::ensure!(app.can_restore_backup(cx), "confirmed nonempty restore form was not enabled");
                app.restore_path.update(cx, |s, cx| s.set_value(r"C:\备份 data\second.sqlite3", window, cx));
                Ok(())
            })??;
            Timer::after(Duration::from_millis(80)).await;
            this.update_in(cx, |app, window, cx| {
                anyhow::ensure!(!app.restore_armed && !app.can_restore_backup(cx), "changing the backup file retained confirmation");
                app.act(Action::Backup(BackupAction::RestoreArm), window, cx);
                app.act(Action::Nav(Page::Overview), window, cx);
                anyhow::ensure!(!app.restore_armed && !app.can_restore_backup(cx), "leaving settings retained confirmation");
                app.act(Action::Nav(Page::Settings), window, cx);
                app.act(Action::Backup(BackupAction::RestoreArm), window, cx);
                app.act(Action::SettingsSection(SettingsSection::About), window, cx);
                anyhow::ensure!(!app.restore_armed && !app.can_restore_backup(cx), "leaving backup page retained confirmation");
                anyhow::ensure!(SettingsSection::About.label() == "关于" && !app.about_details_open, "about should start with concise summary");
                app.exercise_update_settings(window, cx)?;
                app.act(Action::SettingsFold(SettingsFold::AboutDetails), window, cx);
                anyhow::ensure!(app.about_details_open, "about details did not expand");
                app.act(Action::SettingsFold(SettingsFold::AboutDetails), window, cx);
                app.restore_path.update(cx, |s, cx| s.set_value("", window, cx));
                Ok(())
            })??;
            let custom = this.update_in(cx, |app, _, cx| {
                let custom = app.backend.dir.join("自定义备份 data");
                std::fs::create_dir(&custom)?;
                std::fs::write(custom.join("existing.sqlite3"), b"keep original")?;
                app.set_backup_directory(Some(custom.clone()), cx);
                anyhow::ensure!(app.backup_directory_busy && !app.can_restore_backup(cx), "directory validation should disable restore");
                Ok(custom)
            })??;
            wait_directory(&this, cx).await?;
            this.update_in(cx, |app, _, cx| {
                anyhow::ensure!(app.backup_directory_error.is_empty() && app.config.backup_directory()? == custom, "custom directory was not selected");
                let saved = Config::load(&app.backend.dir)?;
                anyhow::ensure!(saved.backup_directory()? == custom, "custom directory was not persisted");
                anyhow::ensure!(saved.update_check_frequency == crate::release_update::CheckFrequency::Never, "update frequency was not persisted");
                // An unrelated UI save may still have an older local snapshot.
                app.config.backup_dir = None;
                app.config.theme = "dark".into();
                app.save_config();
                anyhow::ensure!(app.config.backup_directory()? == custom, "unrelated settings overwrote the directory preference");
                let blocked = app.backend.dir.join("not-a-directory");
                std::fs::write(&blocked, b"blocked")?;
                app.set_backup_directory(Some(blocked), cx);
                Ok(())
            })??;
            wait_directory(&this, cx).await?;
            this.update_in(cx, |app, window, cx| {
                anyhow::ensure!(!app.backup_directory_error.is_empty() && app.config.backup_directory()? == custom, "invalid directory replaced the previous choice");
                app.act(Action::Backup(BackupAction::ResetBackupDirectory), window, cx);
                Ok(())
            })??;
            wait_directory(&this, cx).await?;
            this.update_in(cx, |app, _, _| {
                anyhow::ensure!(app.config.backup_dir.is_none() && Config::load(&app.backend.dir)?.backup_dir.is_none(), "default directory did not persist");
                anyhow::ensure!(app.config.backup_directory()? == crate::config::default_backup_directory()?, "reset did not select document default");
                anyhow::ensure!(app.backup_directory_error.is_empty(), "reset kept the old directory error");
                anyhow::ensure!(std::fs::read(custom.join("existing.sqlite3"))? == b"keep original", "changing the directory moved existing backups");
                Ok(())
            })??;
            let latest = this.update_in(cx, |app, window, cx| {
                window.resize(size(px(900.), px(700.)));
                let latest = app.set_fixture_latest_release(None, cx);
                app.act(Action::Update(updates::UpdateAction::OpenNotes), window, cx);
                latest
            })?;
            Timer::after(Duration::from_millis(180)).await;
            this.update_in(cx, |app, window, cx| {
                app.verify_update_dialog_layout(window, false)?;
                app.verify_update_controls_alignment()?;
                app.act(Action::Update(updates::UpdateAction::CloseNotes), window, cx);
                app.set_fixture_latest_release(latest, cx);
                Ok::<_, anyhow::Error>(())
            })??;
            for (color_theme, theme) in [
                (ColorTheme::CloudBlue, "light"),
                (ColorTheme::CloudBlue, "dark"),
                (ColorTheme::Mica, "light"),
                (ColorTheme::Mica, "dark"),
            ] {
                for width in [1240., 900.] {
                    for section in [SettingsSection::Backup, SettingsSection::About] {
                        for expanded in [false, true] {
                            this.update_in(cx, |app, window, cx| {
                                window.resize(size(px(width), px(700.)));
                                app.act(Action::ColorTheme(color_theme), window, cx);
                                app.act(Action::Theme(theme), window, cx);
                                app.act(Action::SettingsSection(section), window, cx);
                                app.about_details_open = expanded;
                                app.set_fixture_update_progress((section == SettingsSection::About && width == 900.).then(|| {
                                    use crate::release_update::UpdateProgress;
                                    if expanded {
                                        UpdateProgress::Downloading { received: 4_194_304, total: Some(8_388_608) }
                                    } else {
                                        UpdateProgress::Connecting
                                    }
                                }));
                                app.config.backup_dir = expanded.then(|| format!(r"D:\{}\备份目录", "long-backup-directory-".repeat(14)));
                                if section == SettingsSection::Backup {
                                    app.restore_path.update(cx, |s, cx| s.set_value(if expanded { format!(r"D:\{}\流量备份.sqlite3", "long-backup-path-".repeat(12)) } else { String::new() }, window, cx));
                                } else if expanded {
                                    app.act(Action::Update(updates::UpdateAction::OpenNotes), window, cx);
                                }
                                cx.notify();
                            })?;
                            Timer::after(Duration::from_millis(180)).await;
                            this.update_in(cx, |app, window, cx| {
                                anyhow::ensure!((window.viewport_size().width - px(width)).abs() <= px(1.), "unexpected viewport width");
                                anyhow::ensure!(app.connection_test_scroll.bounds().size.height > px(0.), "settings viewport collapsed");
                                anyhow::ensure!(app.connection_test_scroll.max_offset().width <= px(1.), "horizontal overflow: {theme} {width} {} expanded={expanded}", section.label());
                                anyhow::ensure!(!app.can_restore_backup(cx), "layout fixture must never arm restore");
                                if section == SettingsSection::About {
                                    app.verify_update_controls_alignment()?;
                                    let frequency = app.picker_bounds["update-check-frequency"].get();
                                    anyhow::ensure!(
                                        (px(55.)..px(90.)).contains(&frequency.size.width),
                                        "update frequency picker did not fit its label: {frequency:?}"
                                    );
                                    if expanded {
                                        app.verify_update_dialog_layout(window, true)?;
                                    }
                                }
                                Ok(())
                            })??;
                            if section == SettingsSection::About && expanded {
                                for (key, open) in [(VK_ESCAPE, false), (VK_RETURN, true), (VK_SPACE, false)] {
                                    this.update_in(cx, |app, _, _| {
                                        let hwnd = app.main_hwnd as _;
                                        anyhow::ensure!(
                                            unsafe {
                                                PostMessageW(hwnd, WM_KEYDOWN, key as usize, 1) != 0
                                                    && PostMessageW(hwnd, WM_KEYUP, key as usize, 1 | (1 << 30) | (1 << 31)) != 0
                                            },
                                            "could not post update notes key {key}"
                                        );
                                        Ok(())
                                    })??;
                                    Timer::after(Duration::from_millis(40)).await;
                                    this.update_in(cx, |app, _, _| {
                                        anyhow::ensure!(app.update_notes_is_open() == open, "unexpected update notes state after virtual key {key}, expected open={open}");
                                        Ok(())
                                    })??;
                                }
                            }
                        }
                    }
                    println!("backup/about layouts passed: {color_theme:?} {theme} {width}x700");
                }
            }
            this.update_in(cx, |app, _, cx| {
                app.set_fixture_update_progress(None);
                cx.notify();
            })?;
            println!("backup/about checks passed: confirmation reset, update notes scroll and keyboard, 32 layouts");
            let hold = std::env::var("MIHOMO_BACKUP_ABOUT_QA_HOLD_MS").ok().and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
            if hold > 0 {
                for section in [SettingsSection::Backup, SettingsSection::About] {
                    this.update_in(cx, |app, window, cx| {
                        window.resize(size(px(1240.), px(870.)));
                        app.act(Action::Theme("light"), window, cx);
                        app.act(Action::SettingsSection(section), window, cx);
                        app.about_details_open = false;
                        app.config.backup_dir = app.backend.config.lock().unwrap().backup_dir.clone();
                        app.restore_path.update(cx, |s, cx| s.set_value("", window, cx));
                        cx.notify();
                    })?;
                    println!("visual review: {}", section.label());
                    Timer::after(Duration::from_millis(hold)).await;
                }
            }
            Ok(())
        }.await;
        *OUTCOME.lock().unwrap() = Some(result.map_err(|e| e.to_string()));
        let _ = cx.update(|_, cx| cx.quit());
    }).detach();
}

#[::core::prelude::v1::test]
#[ignore = "opens isolated backup/about forms for input and layout verification"]
fn gpui_backup_about_layout_and_restore_confirmation() {
    let dir = tempfile::tempdir().unwrap();
    let mut newer = semver::Version::parse(env!("CARGO_PKG_VERSION")).unwrap();
    newer.patch += 1;
    let cached = serde_json::json!({
        "latest": {"tag": format!("v{newer}"), "notes": format!("## 更新内容\n\n{}", "- 查看发行说明\n".repeat(48)), "installer": true, "portable": true},
        "current": {"tag": concat!("v", env!("CARGO_PKG_VERSION")), "notes": "## 当前版本\n\n缓存的发行说明", "installer": true, "portable": true},
        "history": [{"tag": "v0.0.0", "notes": "缓存的历史发行说明", "installer": false, "portable": false}],
        "last_attempt": chrono::Utc::now(),
        "last_success": chrono::Utc::now()
    });
    std::fs::write(
        dir.path().join(crate::release_update::CACHE_FILE),
        cached.to_string(),
    )
    .unwrap();
    Config {
        endpoint: "http://127.0.0.1:9".into(),
        floating: false,
        taskbar: false,
        tray_visible: false,
        open_hotkey: String::new(),
        ..Default::default()
    }
    .save(dir.path())
    .unwrap();
    let backend = Backend::start(dir.path().to_path_buf()).unwrap();
    backend.stop.store(true, Ordering::Relaxed);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !backend.done.load(Ordering::Acquire) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(backend.done.load(Ordering::Acquire));
    RUN.store(true, Ordering::Release);
    let result = run(backend.clone());
    RUN.store(false, Ordering::Release);
    backend.writer.close(Duration::from_secs(5)).unwrap();
    result.unwrap();
    OUTCOME
        .lock()
        .unwrap()
        .take()
        .expect("backup/about check did not finish")
        .unwrap();
}
