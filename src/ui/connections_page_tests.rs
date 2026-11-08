//! Exercises the live list with isolated data, without contacting a user's core.
use super::*;
use connections_page::{Protocol, Sort};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

pub static RUN: AtomicBool = AtomicBool::new(false);
static OUTCOME: Mutex<Option<Result<(), String>>> = Mutex::new(None);

fn seed(app: &mut TrafficApp) {
    let mut shared = app.backend.shared.lock().unwrap();
    shared.connected = true;
    shared.status = "已连接".into();
    let connections = (0..43)
        .map(|index| Connection {
            id: format!("connection-{index:02}"),
            metadata: Metadata {
                network: if index % 3 == 0 { "udp" } else { "TCP" }.into(),
                host: format!(
                    "{}-{}.example.com",
                    "long-destination-name".repeat(4),
                    index
                ),
                destination_ip: "2001:db8::1234".into(),
                destination_port: "443".into(),
                source_ip: "127.0.0.1".into(),
                source_port: (50000 + index).to_string(),
                process_path: format!(
                    r"C:\应用程序\{}\browser-{}.exe",
                    "long path segment ".repeat(10),
                    index % 3
                ),
                r#type: "HTTP(S)".into(),
                ..Default::default()
            },
            download: (index + 1) * 123_456_789,
            upload: (43 - index) * 10_000_000,
            start: chrono::DateTime::from_timestamp_millis(
                chrono::Utc::now().timestamp_millis() - (index as i64 + 1) * 60_000,
            )
            .unwrap()
            .to_rfc3339(),
            chains: vec![
                "香港 01 · 高速线路 long proxy node".into(),
                "自动选择".into(),
                "代理".into(),
            ],
            rule: "DomainSuffix".into(),
            attribution: Some(crate::attribution::Attribution {
                owner: crate::attribution::Owner {
                    key: "host:browser".into(),
                    label: "浏览器".into(),
                    icon: "browser.exe".into(),
                },
                pid: 12345,
            }),
        })
        .collect();
    shared.connections = std::sync::Arc::new(connections);
    let owner = shared.connections[0]
        .attribution
        .as_ref()
        .unwrap()
        .owner
        .clone();
    shared.identities.insert("host:browser".into(), owner);
    app.data = shared.clone();
}

pub fn schedule(window: &mut Window, cx: &mut Context<TrafficApp>) {
    cx.spawn_in(window, async move |this, cx| {
        let result: anyhow::Result<()> = async {
            this.update_in(cx, |app, window, cx| {
                app.act(Action::Nav(Page::Connections), window, cx);
                seed(app);
                anyhow::ensure!(app.filtered_connections().len() == 43, "initial list missing rows");
                app.act(Action::Next(1), window, cx);
                anyhow::ensure!(app.connections_view.page == 1, "connection pagination did not advance");
                app.page_number.update(cx, |input, cx| input.set_value("5", window, cx));
                app.act(Action::JumpToPage, window, cx);
                anyhow::ensure!(app.connections_view.page == 4, "connection page jump did not land on the last page");
                app.act(Action::Next(1), window, cx);
                anyhow::ensure!(app.connections_view.page == 4, "connection pagination exceeded the last page");
                app.act(Action::Connections(ConnectionsAction::Protocol(Protocol::Udp)), window, cx);
                anyhow::ensure!(app.connections_view.page == 0 && app.filtered_connections().len() == 15, "protocol filter failed to reset page or match mixed case");
                app.act(Action::Connections(ConnectionsAction::Protocol(Protocol::All)), window, cx);
                app.act(Action::Connections(ConnectionsAction::Sort(Sort::Upload)), window, cx);
                anyhow::ensure!(app.filtered_connections()[0].id == "connection-00", "upload sort wrong");
                app.act(Action::Connections(ConnectionsAction::Order), window, cx);
                anyhow::ensure!(app.filtered_connections()[0].id == "connection-42", "ascending upload sort wrong");
                app.act(Action::Connections(ConnectionsAction::Sort(Sort::Duration)), window, cx);
                app.act(Action::Connections(ConnectionsAction::Order), window, cx);
                anyhow::ensure!(app.filtered_connections()[0].id == "connection-42", "duration sort wrong");
                for search in ["浏览器", "2001:db8", "127.0.0.1", "DomainSuffix", "自动选择", "50000"] {
                    app.last_search = search.into();
                    anyhow::ensure!(!app.filtered_connections().is_empty(), "search missed {search}");
                }
                app.last_search.clear();
                app.activity.connection_group = Some("host:browser".into());
                anyhow::ensure!(app.filtered_connections().len() == 43, "owner scope lost helpers");
                app.activity.connection_group = Some(app.data.connections[0].app());
                app.connections_view.raw_process = true;
                anyhow::ensure!(app.filtered_connections().len() == 15, "raw process scope is not exact");
                app.page = Page::History;
                app.selected = app.data.connections[0].app();
                app.activity.selected_process = true;
                app.act(Action::AppConnections, window, cx);
                anyhow::ensure!(app.page == Page::Connections && app.connections_view.raw_process
                    && app.filtered_connections().len() == 15 && app.last_search.is_empty(),
                    "history detail did not enter an exact process scope");
                app.act(Action::Connections(ConnectionsAction::ClearGroup), window, cx);
                app.connections_view.page = 2;
                app.act(Action::Connections(ConnectionsAction::Expand("connection-42".into())), window, cx);
                std::sync::Arc::make_mut(&mut app.data.connections).truncate(1);
                app.sync_connections_view();
                anyhow::ensure!(app.connections_view.page == 0 && app.connections_view.expanded.is_none(), "closed connections left a blank page or stale detail");
                seed(app);
                app.connections_view = connections_page::State::default();
                Ok::<_, anyhow::Error>(())
            })??;
            for theme in ["light", "dark"] {
                for width in [900., 1240., 1600.] {
                    for expanded in [false, true] {
                        this.update_in(cx, |app, window, cx| {
                            window.resize(size(px(width), px(800.)));
                            app.act(Action::Theme(theme), window, cx);
                            app.connections_view.expanded = expanded.then(|| "connection-42".into());
                            app.page_scroll.set_offset(point(px(0.), px(0.)));
                            cx.notify();
                        })?;
                        Timer::after(Duration::from_millis(400)).await;
                        this.update_in(cx, |app, _, _| {
                            anyhow::ensure!(app.page_scroll.bounds().size.height > px(0.), "viewport collapsed");
                            anyhow::ensure!(app.page_scroll.max_offset().width <= px(1.), "connections overflow at {width} {theme}, expanded={expanded}: {:?}", app.page_scroll.max_offset());
                            Ok::<_, anyhow::Error>(())
                        })??;
                    }
                }
            }
            this.update_in(cx, |app, _, cx| {
                app.page_scroll.set_offset(point(px(0.), -app.page_scroll.max_offset().height));
                cx.notify();
            })?;
            Timer::after(Duration::from_millis(150)).await;
            this.update_in(cx, |app, window, cx| {
                use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE};

                let focus = app.page_number.read(cx).focus_handle(cx);
                window.focus(&focus);
                anyhow::ensure!(focus.is_focused(window), "page input did not take focus");
                let bounds = app.page_scroll.bounds();
                let x = (f32::from(bounds.left() + px(4.)) * window.scale_factor()) as u16;
                let y = (f32::from(bounds.top() + px(4.)) * window.scale_factor()) as u16;
                let position = (u32::from(x) | (u32::from(y) << 16)) as isize;
                for (event, buttons) in [(WM_MOUSEMOVE, 0), (WM_LBUTTONDOWN, 1), (WM_LBUTTONUP, 0)] {
                    anyhow::ensure!(
                        unsafe { PostMessageW(app.main_hwnd as _, event, buttons, position) } != 0,
                        "blank-area click failed"
                    );
                }
                Ok::<_, anyhow::Error>(())
            })??;
            Timer::after(Duration::from_millis(150)).await;
            this.update_in(cx, |app, window, cx| {
                anyhow::ensure!(
                    !app.page_number.read(cx).focus_handle(cx).is_focused(window),
                    "blank-area click left the page input focused"
                );
                app.page_scroll.set_offset(point(px(0.), px(0.)));
                app.toggle_picker("connections-sort", vec![("累计下载".into(), Action::Connections(ConnectionsAction::Sort(Sort::Download)), false)], window, cx);
                Ok::<_, anyhow::Error>(())
            })??;
            cx.update(|window, cx| {
                window.dispatch_keystroke(Keystroke::parse("enter").unwrap(), cx);
            })?;
            this.update_in(cx, |app, _, _| {
                app.connections_view.expanded = Some("connection-42".into());
            })?;
            cx.update(|window, cx| {
                window.dispatch_keystroke(Keystroke::parse("escape").unwrap(), cx);
            })?;
            this.update_in(cx, |app, window, cx| {
                anyhow::ensure!(app.connections_view.sort == Sort::Download && app.picker.is_none(), "sort menu did not select or close");
                anyhow::ensure!(app.connections_view.expanded.is_none(), "Escape did not close the connection detail");
                app.connections_view = connections_page::State::default();
                app.search.update(cx, |input, cx| input.set_value("no-such-connection", window, cx));
                app.last_search = "no-such-connection".into();
                anyhow::ensure!(app.filtered_connections().is_empty(), "empty search still has rows");
                cx.notify();
                Ok::<_, anyhow::Error>(())
            })??;
            Timer::after(Duration::from_millis(600)).await;
            let hold = std::env::var("MIHOMO_CONNECTIONS_QA_HOLD_MS").ok().and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
            if hold > 0 {
                this.update_in(cx, |app, window, cx| {
                    let width = std::env::var("MIHOMO_CONNECTIONS_QA_WIDTH").ok().and_then(|s| s.parse::<f32>().ok()).unwrap_or(1240.);
                    let theme = if std::env::var("MIHOMO_CONNECTIONS_QA_THEME").is_ok_and(|s| s == "dark") { "dark" } else { "light" };
                    window.resize(size(px(width), px(870.)));
                    app.act(Action::Theme(theme), window, cx);
                    app.search.update(cx, |input, cx| input.set_value("", window, cx));
                    app.last_search.clear();
                    app.connections_view.expanded = Some("connection-42".into());
                    cx.notify();
                })?;
                println!("connections visual preview ready");
                Timer::after(Duration::from_millis(hold)).await;
            }
            println!("GPUI connections passed: protocol/search/sort, exact owner and process scopes, shrinking pages, expanded details, menu and 12 wide/narrow light/dark layouts");
            Ok(())
        }.await;
        *OUTCOME.lock().unwrap() = Some(result.map_err(|error| error.to_string()));
        let _ = cx.update(|_, cx| cx.quit());
    }).detach();
}

#[::core::prelude::v1::test]
fn connection_endpoints_handle_ipv6_and_missing_values() {
    assert_eq!(
        connections_page::endpoint("2001:db8::1", "443"),
        "[2001:db8::1]:443"
    );
    assert_eq!(
        connections_page::endpoint("example.com", "443"),
        "example.com:443"
    );
    assert_eq!(connections_page::endpoint("", "443"), "—");
}

#[::core::prelude::v1::test]
#[ignore = "opens an isolated GPUI window to verify the current connections list"]
fn gpui_current_connections_layout_and_filters() {
    let dir = tempfile::tempdir().unwrap();
    Config {
        endpoint: "http://127.0.0.1:9".into(),
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
    RUN.store(true, Ordering::Release);
    let result = run(backend.clone());
    RUN.store(false, Ordering::Release);
    backend.writer.close(Duration::from_secs(5)).unwrap();
    result.unwrap();
    OUTCOME
        .lock()
        .unwrap()
        .take()
        .expect("connections checks did not finish")
        .unwrap();
}
