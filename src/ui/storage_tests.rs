//! Exercise real input subscriptions and layout using an isolated ledger.
use super::*;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
pub static RUN: AtomicBool = AtomicBool::new(false);
static OUTCOME: Mutex<Option<Result<(), String>>> = Mutex::new(None);

#[::core::prelude::v1::test]
fn unit_conversion_preserves_day_and_megabyte_precision() {
    use super::storage_settings::{format_amount, parse_amount};
    assert_eq!(format_amount(90, 30), "3");
    assert_eq!(format_amount(123, 30), "4.1");
    assert_eq!(format_amount(1024, 1024), "1");
    assert_eq!(format_amount(500, 1024), "0.48828125");
    assert_eq!(parse_amount("0.1", 30, 3650), Some(3));
    assert_eq!(parse_amount("3.1", 30, 3650), Some(93));
    assert_eq!(
        parse_amount("1.5", 1024, u64::MAX / (1024 * 1024)),
        Some(1536)
    );
    assert_eq!(
        parse_amount("0.0009765625", 1024, u64::MAX / (1024 * 1024)),
        Some(1)
    );
    for text in ["0", "-1", "NaN", "inf", "1.5", "3651"] {
        assert_eq!(parse_amount(text, 1, 3650), None);
    }
    assert_eq!(parse_amount("122", 30, 3650), None);
    assert_eq!(parse_amount("0.05", 30, 3650), None);
    assert_eq!(
        parse_amount("18446744073709551615", 1024, u64::MAX / (1024 * 1024)),
        None
    );
}

pub fn schedule(window: &mut Window, cx: &mut Context<TrafficApp>) {
    cx.spawn_in(window, async move |this, cx| {
        let result: anyhow::Result<()> = async {
            this.update_in(cx, |app, window, cx| {
                app.act(Action::Nav(Page::Settings), window, cx);
                app.act(Action::SettingsSection(SettingsSection::Storage), window, cx);
                app.act(Action::Storage(StorageAction::RetentionTimeUnit(RetentionTimeUnit::Days)),window,cx);
                if app.config.storage_limit_enabled { app.act(Action::Storage(StorageAction::RetentionToggle(true)),window,cx); }
                app.retention.read(cx).focus_handle(cx).focus(window);
                app.retention.update(cx, |input, cx| input.set_value("123", window, cx));
                Ok::<_,anyhow::Error>(())
            })??;
            cx.update(|window, cx| { window.dispatch_keystroke(Keystroke::parse("enter").unwrap(), cx); })?;
            Timer::after(Duration::from_millis(500)).await;
            this.update_in(cx, |app, window, cx| {
                anyhow::ensure!(app.config.retention_days == 123, "Enter did not save days");
                anyhow::ensure!(Config::load(&app.backend.dir)?.retention_days == 123, "days not persisted");
                app.retention.update(cx, |input, cx| input.set_value("0", window, cx));
                app.save_retention(window, cx);
                anyhow::ensure!(!app.retention_error.is_empty() && app.config.retention_days == 123, "invalid days saved");
                app.act(Action::Storage(StorageAction::RetentionToggle(false)), window, cx);
                anyhow::ensure!(!app.config.retention_days_enabled && app.retention_error.is_empty(), "invalid draft prevented disabling days");
                anyhow::ensure!(app.config.retention_policy().days.is_none() && crate::storage_management::Policy::from(&app.config).max_bytes.is_none(), "unlimited still has constraints");
                app.act(Action::Storage(StorageAction::RetentionSizeUnit(RetentionSizeUnit::Mb)), window, cx);
                app.act(Action::Storage(StorageAction::RetentionToggle(true)), window, cx);
                app.retention_size.update(cx, |input, cx| input.set_value("2048", window, cx));
                app.retention_size.read(cx).focus_handle(cx).focus(window);
                Ok(())
            })??;
            cx.update(|window, cx| { window.dispatch_keystroke(Keystroke::parse("enter").unwrap(), cx); })?;
            Timer::after(Duration::from_millis(500)).await;
            this.update_in(cx, |app, window, cx| {
                anyhow::ensure!(app.config.storage_limit_mb == 2048, "capacity Enter did not save");
                anyhow::ensure!(Config::load(&app.backend.dir)?.storage_limit_mb == 2048, "capacity not persisted");
                for invalid in ["", "0", "-1", "1.5", "18446744073709551615"] {
                    app.retention_size.update(cx, |input, cx| input.set_value(invalid, window, cx));
                    app.save_retention_size(window, cx);
                    anyhow::ensure!(!app.retention_size_error.is_empty() && app.config.storage_limit_mb == 2048, "invalid capacity saved: {invalid}");
                }
                app.act(Action::Storage(StorageAction::RetentionToggle(true)), window, cx);
                app.act(Action::Storage(StorageAction::RetentionToggle(true)), window, cx);
                app.act(Action::Storage(StorageAction::RetentionToggle(false)), window, cx);
                anyhow::ensure!(app.config.retention_days == 123 && app.config.storage_limit_mb == 2048, "toggle lost saved values");
                anyhow::ensure!(app.config.retention_policy().days == Some(123) && crate::storage_management::Policy::from(&app.config).max_bytes.is_some(), "combined policy failed");
                app.act(Action::Storage(StorageAction::RetentionTimeUnit(RetentionTimeUnit::Months)), window, cx);
                anyhow::ensure!(app.retention.read(cx).value().as_ref() == "4.1" && app.config.retention_days == 123, "month conversion changed the policy");
                app.retention.update(cx, |input, cx| input.set_value("3", window, cx));
                app.save_retention(window, cx);
                anyhow::ensure!(app.config.retention_days == 90, "three months must equal ninety days");
                app.act(Action::Storage(StorageAction::RetentionSizeUnit(RetentionSizeUnit::Gb)), window, cx);
                anyhow::ensure!(app.retention_size.read(cx).value().as_ref() == "2", "GB conversion failed");
                app.retention_size.update(cx, |input, cx| input.set_value("1.5", window, cx));
                app.save_retention_size(window, cx);
                anyhow::ensure!(app.config.storage_limit_mb == 1536, "fractional GB failed");
                app.act(Action::Storage(StorageAction::RetentionSizeUnit(RetentionSizeUnit::Mb)), window, cx);
                anyhow::ensure!(app.retention_size.read(cx).value().as_ref() == "1536", "MB conversion failed");
                app.config.retention_days = 1;
                app.retention.update(cx, |input, cx| input.set_value("0.033333", window, cx));
                app.save_retention(window, cx);
                anyhow::ensure!(app.config.retention_days == 1 && app.retention_error.is_empty(), "recurring month display changed saved days");
                app.retention.update(cx, |input, cx| input.set_value("invalid", window, cx));
                app.retention_size.update(cx, |input, cx| input.set_value("invalid", window, cx));
                app.data_directory_error = "old directory error".into();
                app.act(Action::Storage(StorageAction::ResetStorageSettings), window, cx);
                anyhow::ensure!(app.config.retention_days == 90 && app.config.retention_time_unit == RetentionTimeUnit::Months && app.retention.read(cx).value().as_ref() == "3", "default three months failed");
                anyhow::ensure!(app.config.storage_limit_mb == 2048 && app.config.storage_limit_unit == RetentionSizeUnit::Gb && app.retention_size.read(cx).value().as_ref() == "2", "default two GB failed");
                anyhow::ensure!(app.retention_error.is_empty() && app.retention_size_error.is_empty() && app.data_directory_error.is_empty() && app.config.retention_days_enabled && !app.config.storage_limit_enabled && app.config.seconds_enabled && app.config.seconds_days==3, "page restore must clear errors and restore default switches");
                anyhow::ensure!(app.data_directory_draft == Some(crate::config::default_data_dir()) && !app.migration_busy, "default directory must be staged without starting migration");
                anyhow::ensure!(app.config.endpoint == "http://127.0.0.1:9" && !app.config.floating && !app.config.taskbar && !app.config.tray_visible, "storage restore changed unrelated preferences");
                // Unit menus use the existing keyboard picker and persist their choices.
                app.act(Action::Storage(StorageAction::RetentionTimeUnit(RetentionTimeUnit::Days)),window,cx);
                app.toggle_picker("retention-time-unit", RetentionTimeUnit::ALL.into_iter().map(|unit| (unit.label().into(), Action::Storage(StorageAction::RetentionTimeUnit(unit)), unit == RetentionTimeUnit::Days)).collect(), window, cx);
                Ok(())
            })??;
            cx.update(|window, cx| { window.dispatch_keystroke(Keystroke::parse("down").unwrap(), cx); window.dispatch_keystroke(Keystroke::parse("enter").unwrap(), cx); })?;
            Timer::after(Duration::from_millis(500)).await;
            this.update_in(cx, |app, window, cx| {
                anyhow::ensure!(app.config.retention_time_unit == RetentionTimeUnit::Months && app.retention.read(cx).value().as_ref() == "3", "keyboard unit selection failed");
                let saved = Config::load(&app.backend.dir)?;
                anyhow::ensure!(saved.retention_time_unit == RetentionTimeUnit::Months && saved.storage_limit_unit == RetentionSizeUnit::Gb, "unit preferences not saved");
                app.act(Action::Storage(StorageAction::ResetStorageSettings), window, cx);
                anyhow::ensure!(app.data_directory_draft == Some(crate::config::default_data_dir()), "restore default target is wrong");
                app.act(Action::Storage(StorageAction::CancelDataDirectory), window, cx);
                anyhow::ensure!(app.data_directory_draft.is_none(), "cancel retained target");
                let live_directory = app.backend.dir.clone();
                app.backend.dir = crate::config::default_data_dir();
                app.data_directory_draft = Some(live_directory.clone());
                app.act(Action::Storage(StorageAction::ResetStorageSettings), window, cx);
                anyhow::ensure!(app.data_directory_draft.is_none() && !app.migration_busy, "already-default directory must clear pending target without migrating");
                app.backend.dir = live_directory;
                window.blur();
                Ok(())
            })??;
            this.update_in(cx,|app,window,cx| {
                if !app.config.storage_limit_enabled { app.act(Action::Storage(StorageAction::RetentionToggle(true)),window,cx); }
                app.retention_size.read(cx).focus_handle(cx).focus(window);
                app.retention_size.update(cx,|s,cx|s.set_value("0",window,cx));
            })?;
            cx.update(|window,cx|window.dispatch_keystroke(Keystroke::parse("enter").unwrap(),cx))?;
            Timer::after(Duration::from_millis(100)).await;
            this.update_in(cx,|app,window,cx| {
                anyhow::ensure!(!app.retention_size_error.is_empty() && !app.retention_size.read(cx).value().contains('\n'),"invalid Enter inserted newline");
                app.act(Action::Storage(StorageAction::ResetStorageSettings),window,cx);app.act(Action::Storage(StorageAction::CancelDataDirectory),window,cx);
                app.act(Action::Storage(StorageAction::StorageOpen(storage_management_page::Dialog::Clean(crate::storage_management::Target::Seconds))),window,cx);
                app.act(Action::Storage(StorageAction::StoragePreset(0)),window,cx);
                Ok::<_,anyhow::Error>(())
            })??;
            Timer::after(Duration::from_millis(250)).await;
            this.update_in(cx,|app,window,cx| {
                anyhow::ensure!(app.storage_manager.preview.as_ref().and_then(|p|p.cutoff)==Some(crate::storage_management::days_before(chrono::Utc::now().timestamp_millis(),7)?),"7-day boundary incorrect");
                app.act(Action::Storage(StorageAction::StoragePreset(3)),window,cx);Ok::<_,anyhow::Error>(())
            })??;
            Timer::after(Duration::from_millis(250)).await;
            this.update_in(cx,|app,window,cx| {
                anyhow::ensure!(app.storage_manager.preview.as_ref().is_some_and(|p|p.cutoff.is_none()),"all raw boundary incorrect");
                app.act(Action::Storage(StorageAction::StorageClose),window,cx);
                app.act(Action::Storage(StorageAction::StorageOpen(storage_management_page::Dialog::Clean(crate::storage_management::Target::Details))),window,cx);
                app.act(Action::Storage(StorageAction::StoragePreset(3)),window,cx);
                Ok::<_,anyhow::Error>(())
            })??;
            Timer::after(Duration::from_millis(250)).await;
            this.update_in(cx,|app,window,cx| {
                anyhow::ensure!(app.storage_manager.preview.as_ref().and_then(|p|p.cutoff)==Some(crate::storage_management::days_before(chrono::Utc::now().timestamp_millis(),0)?),"all ledger failed today protection");
                app.act(Action::Storage(StorageAction::StorageClose),window,cx);app.storage_manager.expanded=true;
                app.act(Action::Storage(StorageAction::StorageOpen(storage_management_page::Dialog::Reclaim)),window,cx);
                Ok::<_,anyhow::Error>(())
            })??;
            cx.update(|window,cx|window.dispatch_keystroke(Keystroke::parse("escape").unwrap(),cx))?;
            this.update_in(cx,|app,_,_| {anyhow::ensure!(app.storage_manager.dialog.is_none() && app.storage_manager.expanded,"Esc changed storage context");Ok::<_,anyhow::Error>(())})??;
            let preview_hold=std::env::var("MIHOMO_STORAGE_REDESIGN_HOLD_MS").ok().and_then(|v|v.parse::<u64>().ok()).unwrap_or(0);
            for theme in ["light","dark"] {
                for width in [1240.,900.] {
                    this.update_in(cx,|app,window,cx| {
                        app.act(Action::Theme(theme),window,cx);window.resize(size(px(width),px(700.)));
                        app.storage_manager.expanded=false;app.storage_manager.help_expanded=false;
                        app.data_directory_draft=None;app.data_directory_error.clear();app.retention_error.clear();app.retention_size_error.clear();
                        app.connection_test_scroll.set_offset(point(px(0.),px(0.)));cx.notify();
                    })?;
                    Timer::after(Duration::from_millis(600)).await;
                    this.update_in(cx,|app,_,_| {
                        let cards=app.storage_manager.cards.each_ref().map(|b|b.get());let viewport=app.connection_test_scroll.bounds();
                        anyhow::ensure!(cards[0].top()>=viewport.top()-px(1.) && cards[1].bottom()<=viewport.bottom()+px(1.),"common settings left first viewport: {cards:?} / {viewport:?}");
                        anyhow::ensure!(cards[0].bottom()+px(12.)<=cards[1].top() && cards[1].bottom()+px(12.)<=cards[2].top(),"storage cards overlap");
                        anyhow::ensure!(app.connection_test_scroll.max_offset().width<=px(1.),"compact storage horizontal overflow");
                        Ok::<_,anyhow::Error>(())
                    })??;
                    println!("storage compact layout {theme} {width}: usage and collection settings both visible");
                    if preview_hold>0 {Timer::after(Duration::from_millis(preview_hold)).await;}
                }
            }
            let hold = std::env::var("MIHOMO_STORAGE_QA_HOLD_MS").ok().and_then(|v|v.parse::<u64>().ok()).unwrap_or(0);
            for theme in ["light", "dark"] {
                for width in [1240., 900.] {
                    this.update_in(cx, |app, window, cx| {
                        app.act(Action::Theme(theme), window, cx);
                        window.resize(size(px(width), px(700.)));
                        app.data_directory_draft = Some(std::path::PathBuf::from(format!(r"D:\{}\目标 data", "long-directory-segment-".repeat(12))));
                        app.data_directory_error = "无法迁移：目标目录正被另一实例使用，请关闭该实例后重试。".into();
                        app.retention_error = "请输入 1–3650 之间的整数天数".into();
                        app.retention_size_error = "请输入有效的正整数 MB".into();
                        app.data.storage_over_limit = true;
                        app.storage_manager.expanded = true;
                        app.storage_manager.help_expanded=true;
                        cx.notify();
                    })?;
                    Timer::after(Duration::from_millis(700)).await;
                    this.update_in(cx, |app, window, _| {
                        anyhow::ensure!((window.viewport_size().width - px(width)).abs() <= px(1.), "unexpected viewport");
                        anyhow::ensure!(app.connection_test_scroll.bounds().size.height > px(0.), "storage viewport collapsed");
                        anyhow::ensure!(app.connection_test_scroll.max_offset().width <= px(1.), "storage horizontal overflow at {theme} / {width}: {:?}", app.connection_test_scroll.max_offset());
                        for col in 0..3 {
                            let a=app.storage_manager.policy_bounds[0][col].get();let b=app.storage_manager.policy_bounds[1][col].get();
                            anyhow::ensure!((a.left()-b.left()).abs()<px(1.) && (a.size.width-b.size.width).abs()<px(1.),"policy controls misaligned at column {col}: {a:?} {b:?}");
                        }
                        for id in ["retention-time-unit", "retention-size-unit", "seconds-time-unit"] {
                            let unit_width = app.picker_bounds[id].get().size.width;
                            anyhow::ensure!(unit_width>=px(56.) && unit_width<=px(66.),"unit did not fit longest option: {id} {unit_width:?}");
                            anyhow::ensure!((unit_width-app.picker_bounds["seconds-time-unit"].get().size.width).abs()<px(1.),"storage units differ in width");
                        }
                        Ok(())
                    })??;
                    let parent_offset=this.update_in(cx, |app,_,_|app.page_scroll.offset())?;
                    this.update_in(cx, |app,window,cx|app.act(Action::Storage(StorageAction::StorageOpen(storage_management_page::Dialog::Clean(crate::storage_management::Target::Seconds))),window,cx))?;
                    Timer::after(Duration::from_millis(500)).await;
                    this.update_in(cx, |app,window,cx| {
                        anyhow::ensure!(app.storage_manager.bounds.get().size.width<=px(width-48.) && app.storage_manager.bounds.get().size.height<=window.viewport_size().height-px(70.),"storage modal overflow");
                        anyhow::ensure!(!app.storage_manager.pending && app.storage_manager.preview.as_ref().is_some_and(|v|v.rows==0),"empty preview failed");
                        app.act(Action::Storage(StorageAction::StorageConfirm),window,cx);
                        anyhow::ensure!(!app.backend.shared.lock().unwrap().storage_management.busy,"empty preview was submitted");
                        app.act(Action::Storage(StorageAction::StoragePreset(2)),window,cx);
                        app.storage_manager.date.update(cx,|s,cx|s.set_value("invalid",window,cx));app.preview_storage(cx);
                        anyhow::ensure!(!app.storage_manager.error.is_empty() && app.storage_manager.preview.is_none(),"invalid custom date accepted");
                        app.act(Action::Storage(StorageAction::StorageClose),window,cx);
                        anyhow::ensure!(app.storage_manager.expanded && app.page_scroll.offset()==parent_offset,"closing modal changed parent state");
                        Ok::<_,anyhow::Error>(())
                    })??;
                    println!("storage layout {theme} {width}x700: categories and cleanup modal valid, empty confirmation blocked");
                    if hold > 0 { Timer::after(Duration::from_millis(hold)).await; }
                }
            }
            println!("storage GPUI checks passed: Enter, validation, unlimited, combined limits, saved values and four layouts");
            Ok(())
        }.await;
        *OUTCOME.lock().unwrap() = Some(result.map_err(|error| error.to_string()));
        let _ = cx.update(|_, cx| cx.quit());
    }).detach();
}

#[::core::prelude::v1::test]
#[ignore = "opens a temporary GPUI window for storage form/layout checks"]
fn gpui_storage_settings_validation_and_layout() {
    let dir = tempfile::tempdir().unwrap();
    Config {
        endpoint: "http://127.0.0.1:9".into(),
        theme: "light".into(),
        floating: false,
        taskbar: false,
        tray_visible: false,
        ..Default::default()
    }
    .save(dir.path())
    .unwrap();
    let backend = Backend::start(dir.path().to_path_buf()).unwrap();
    RUN.store(true, Ordering::Release);
    let result = run(backend.clone());
    RUN.store(false, Ordering::Release);
    backend.shutdown().unwrap();
    result.unwrap();
    OUTCOME
        .lock()
        .unwrap()
        .take()
        .expect("storage regression did not finish")
        .unwrap();
}
