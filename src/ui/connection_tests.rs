//! Optional real GPUI form/layout regression; never writes to a user's core.
use super::*;
use crate::config::ProcessControl;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
pub static RUN: AtomicBool = AtomicBool::new(false);
static OUTCOME: Mutex<Option<Result<(), String>>> = Mutex::new(None);

pub fn schedule(window: &mut Window, cx: &mut Context<TrafficApp>) {
    cx.spawn_in(window, async move |this, cx| {
        let result: anyhow::Result<()> = async {
            this.update_in(cx, |app, window, cx| {
                app.act(Action::Nav(Page::Settings), window, cx);
                app.act(Action::SettingsSection(SettingsSection::Connection), window, cx);
                if std::env::var_os("MIHOMO_CONNECTION_QA_OPEN_DIRECTORY").is_some() {
                    app.data.permanent.source = app.backend.dir.join("配置 文件.yaml").display().to_string();
                    app.act(Action::Connection(ConnectionAction::OpenProcessLocation(false)), window, cx);
                    println!("configuration directory open returned to the GPUI event handler");
                }
                app.connection_form.automatic = false;
                let before=app.config.endpoint.clone();
                app.connection_form.endpoint.update(cx, |input,cx| input.set_value("",window,cx));
                app.act(Action::Connection(ConnectionAction::Save),window,cx);
                anyhow::ensure!(!app.connection_form.error.is_empty() && app.config.endpoint==before,"empty manual address replaced saved connection");
                app.act(Action::Connection(ConnectionAction::ProcessControl(true)),window,cx);
                Ok(())
            })??;
            Timer::after(Duration::from_millis(650)).await;
            this.update_in(cx, |app,window,cx| {
                anyhow::ensure!(Config::load(&app.backend.dir)?.process_control==ProcessControl::KeepAlways,"offline enable intent was not saved");
                app.act(Action::Connection(ConnectionAction::ProcessControl(false)),window,cx);
                Ok(())
            })??;
            Timer::after(Duration::from_millis(650)).await;
            this.update_in(cx, |app,window,cx| {
                anyhow::ensure!(Config::load(&app.backend.dir)?.process_control==ProcessControl::RestoreStrict,"offline disable intent was not saved");
                app.connection_form.endpoint.update(cx,|input,cx| input.set_value("http://127.0.0.1:19",window,cx));
                app.act(Action::Connection(ConnectionAction::Save),window,cx);
                anyhow::ensure!(app.config.process_control==ProcessControl::Unmanaged,"switching endpoints retained the old process preference");
                let saved_secret = app.config.secret.clone();
                app.connection_form.endpoint.update(cx,|input,cx| input.set_value("invalid draft address",window,cx));
                app.connection_form.secret.update(cx,|input,cx| input.set_value("unsaved-password",window,cx));
                app.act(Action::Connection(ConnectionAction::Mode(true)),window,cx);
                anyhow::ensure!(app.connection_form.automatic && app.config.endpoint=="auto","choosing automatic mode did not immediately save the connection");
                anyhow::ensure!(app.config.secret==saved_secret,"automatic connection applied an unsaved manual password");
                app.act(Action::Connection(ConnectionAction::Mode(false)),window,cx);
                anyhow::ensure!(!app.connection_form.automatic && app.config.endpoint=="auto","choosing manual mode changed the saved connection before applying");
                app.act(Action::Connection(ConnectionAction::Save),window,cx);
                anyhow::ensure!(!app.connection_form.error.is_empty() && app.config.endpoint=="auto","invalid manual draft replaced the automatic connection");
                app.act(Action::Connection(ConnectionAction::Mode(true)),window,cx);
                anyhow::ensure!(app.connection_form.automatic && app.connection_form.error.is_empty() && app.config.secret==saved_secret,"returning to automatic mode kept a manual error or applied a draft");
                Ok(())
            })??;
            Timer::after(Duration::from_millis(650)).await;
            this.update_in(cx, |app,window,cx| {
                anyhow::ensure!(Config::load(&app.backend.dir)?.endpoint=="auto","automatic connection choice was not persisted");
                let config_before = std::fs::read(app.backend.dir.join("config.toml"))?;
                for saved in ProcessMode::ALL {
                    app.data.permanent.mode = crate::client_config::SavedMode::Explicit(saved);
                    app.data.permanent.status = crate::permanent_control::Status::Idle;
                    app.reset_permanent_draft();
                    anyhow::ensure!(app.connection_form.permanent_draft==Some(saved) && !app.connection_form.permanent_draft_dirty,"saved mode was not reflected in the permanent selection");
                    let other = if saved == ProcessMode::Always { ProcessMode::Off } else { ProcessMode::Always };
                    app.act(Action::Connection(ConnectionAction::PermanentDraft(other)),window,cx);
                    anyhow::ensure!(app.connection_form.permanent_draft_dirty,"choosing a different mode was not marked pending");
                    app.act(Action::Connection(ConnectionAction::PermanentDraft(saved)),window,cx);
                    anyhow::ensure!(!app.connection_form.permanent_draft_dirty,"returning to the saved mode kept a stale pending flag");
                    app.act(Action::Connection(ConnectionAction::PermanentDraft(other)),window,cx);
                    app.act(Action::Connection(ConnectionAction::CancelPermanentDraft),window,cx);
                    anyhow::ensure!(app.connection_form.permanent_draft==Some(saved) && !app.connection_form.permanent_draft_dirty,"discard did not restore the saved selection");
                }
                for saved in [crate::client_config::SavedMode::Inherited, crate::client_config::SavedMode::Unknown("future mode".into()), crate::client_config::SavedMode::Unavailable] {
                    app.data.permanent.mode = saved;
                    app.reset_permanent_draft();
                    anyhow::ensure!(app.connection_form.permanent_draft.is_none() && !app.connection_form.permanent_draft_dirty,"an unset or unknown save value was treated as strict");
                    app.act(Action::Connection(ConnectionAction::PermanentDraft(ProcessMode::Strict)),window,cx);
                    anyhow::ensure!(app.connection_form.permanent_draft_dirty,"explicit strict was incorrectly treated as an inherited default");
                    app.act(Action::Connection(ConnectionAction::CancelPermanentDraft),window,cx);
                    anyhow::ensure!(app.connection_form.permanent_draft.is_none() && !app.connection_form.permanent_draft_dirty,"discard did not restore the unset selection");
                }
                app.data.permanent.mode = crate::client_config::SavedMode::Explicit(ProcessMode::Strict);
                app.reset_permanent_draft();
                app.act(Action::Connection(ConnectionAction::PermanentDraft(ProcessMode::Always)),window,cx);
                app.data.permanent.mode = crate::client_config::SavedMode::Explicit(ProcessMode::Always);
                app.sync_permanent_draft();
                anyhow::ensure!(!app.connection_form.permanent_draft_dirty,"a refresh matching the draft did not clear pending in idle status");
                app.data.permanent.mode = crate::client_config::SavedMode::Explicit(ProcessMode::Off);
                app.sync_permanent_draft();
                anyhow::ensure!(app.connection_form.permanent_draft==Some(ProcessMode::Off),"a clean draft did not follow an external saved-value change");
                app.data.permanent.mode = crate::client_config::SavedMode::Inherited;
                app.data.permanent.status = crate::permanent_control::Status::Restored;
                app.connection_form.process_restore_notice_dismissed = false;
                app.act(Action::Connection(ConnectionAction::DismissRestoreNotice),window,cx);
                anyhow::ensure!(app.connection_form.process_restore_notice_dismissed && app.data.permanent.status==crate::permanent_control::Status::Restored && app.data.permanent.mode==crate::client_config::SavedMode::Inherited,"closing the restoration notice changed the saved result");
                app.sync_permanent_draft();
                anyhow::ensure!(app.connection_form.process_restore_notice_dismissed,"refresh made a dismissed restoration notice visible");
                app.act(Action::Connection(ConnectionAction::Permanent(crate::permanent_control::Request::Restore)),window,cx);
                anyhow::ensure!(!app.connection_form.process_restore_notice_dismissed,"a new restoration did not reset the dismissed notice");
                anyhow::ensure!(std::fs::read(app.backend.dir.join("config.toml"))?==config_before,"editing or discarding a draft wrote configuration");
                app.data.permanent = app.backend.shared.lock().unwrap().permanent.clone();
                app.reset_permanent_draft();
                println!("permanent draft checks passed: three modes, return to saved, discard, inherited/unknown/unavailable, refresh and no configuration writes");
                Ok(())
            })??;
            let hold = std::env::var("MIHOMO_CONNECTION_QA_HOLD_MS").ok().and_then(|v|v.parse::<u64>().ok()).unwrap_or(0);
            for theme in ["light","dark"] {
                for width in [1240.,900.] {
                    for automatic in [true,false] {
                        for diagnostics in [false,true] {
                            this.update_in(cx,|app,window,cx| {
                                window.resize(size(px(width),px(700.)));
                                app.act(Action::Theme(theme),window,cx);
                                app.connection_form.automatic=automatic;
                                app.connection_form.diagnostics_open=diagnostics;
                                app.connection_form.error="控制地址无效，请检查地址格式与端口。".into();
                                app.connection_form.endpoint.update(cx,|input,cx|input.set_value(format!("http://127.0.0.1:19/{}","long-control-address-".repeat(12)),window,cx));
                                let fixture = crate::permanent_control::View {
                                    available: !diagnostics,
                                    client: "Clash Nyanpasu".into(),
                                    mode: if diagnostics && automatic {
                                        crate::client_config::SavedMode::Unknown("已有全局处理链，暂时无法确定最终保存值。".into())
                                    } else if automatic {
                                        crate::client_config::SavedMode::Inherited
                                    } else {
                                        crate::client_config::SavedMode::Explicit(ProcessMode::Strict)
                                    },
                                    source: format!(r"C:\client\{}\{}.yaml", "long-config-path-".repeat(12), "long-config-file-".repeat(12)),
                                    note: "全局覆写可能被后续扩展或脚本覆盖。".into(),
                                    error: if diagnostics && automatic { "读取失败：无法核验客户端配置目录，请检查目录和权限。".into() } else { String::new() },
                                    status: if diagnostics && !automatic { crate::permanent_control::Status::WaitingExit } else if diagnostics { crate::permanent_control::Status::Failed } else if automatic { crate::permanent_control::Status::Restored } else { crate::permanent_control::Status::Applied },
                                    backup: format!(r"C:\backup\{}", "long-backup-path-".repeat(12)),
                                    launch_available: !diagnostics && automatic,
                                    restoring: automatic || diagnostics,
                                };
                                {
                                    let mut shared = app.backend.shared.lock().unwrap();
                                    shared.permanent = fixture.clone();
                                    shared.connected = !automatic;
                                    shared.process_mode = if automatic { "future-mode".into() } else { "always".into() };
                                    app.data.connected = shared.connected;
                                    app.data.process_mode = shared.process_mode.clone();
                                }
                                app.data.permanent = fixture;
                                app.reset_permanent_draft();
                                if !automatic && !diagnostics {
                                    app.act(Action::Connection(ConnectionAction::PermanentDraft(ProcessMode::Always)),window,cx);
                                }
                                cx.notify();
                            })?;
                            Timer::after(Duration::from_millis(650)).await;
                            this.update_in(cx,|app,window,_| {
                                anyhow::ensure!((window.viewport_size().width-px(width)).abs()<=px(1.),"unexpected viewport width");
                                anyhow::ensure!(app.connection_test_scroll.bounds().size.height>px(0.),"settings scroll viewport collapsed");
                                anyhow::ensure!(app.connection_test_scroll.max_offset().width<=px(1.),"connection content overflows horizontally at {width}, {theme}, auto={automatic}, diagnostics={diagnostics}: {:?}",app.connection_test_scroll.max_offset());
                                Ok(())
                            })??;
                            println!("connection layout: {theme}, {width}x700, auto={automatic}, diagnostics={diagnostics}");
                            if width==900. && !automatic && diagnostics && hold>0 { Timer::after(Duration::from_millis(hold)).await; }
                        }
                    }
                }
            }
            println!("GPUI connection checks passed: 16 layouts, long addresses/config paths, unknown modes, errors, waiting exit, backups, offline intent persistence, endpoint reset and immediate automatic connection selection");
            Ok(())
        }.await;
        *OUTCOME.lock().unwrap()=Some(result.map_err(|e|e.to_string()));
        let _=cx.update(|_,cx|cx.quit());
    }).detach();
}

#[::core::prelude::v1::test]
#[ignore = "opens a temporary GPUI window for connection form and layout checks"]
fn gpui_connection_layout_and_offline_intent() {
    let dir = tempfile::tempdir().unwrap();
    Config {
        endpoint: "http://127.0.0.1:9".into(),
        theme: "light".into(),
        floating: false,
        taskbar: false,
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
    {
        let mut shared = backend.shared.lock().unwrap();
        shared.endpoint = format!(
            r"\\.\pipe\verge-mihomo-production-{}",
            "abcde0123456789".repeat(8)
        );
        shared.status = "连接中断：无法连接控制接口，请检查代理客户端是否运行。".into();
        shared.process_status = ProcessControlStatus::Waiting;
    }
    RUN.store(true, Ordering::Release);
    let result = run(backend.clone());
    RUN.store(false, Ordering::Release);
    backend.writer.close(Duration::from_secs(5)).unwrap();
    result.unwrap();
    OUTCOME
        .lock()
        .unwrap()
        .take()
        .expect("connection regression did not finish")
        .unwrap();
}
