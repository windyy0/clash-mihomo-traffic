use super::*;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

pub static RUN: AtomicBool = AtomicBool::new(false);
static OUTCOME: Mutex<Option<Result<(), String>>> = Mutex::new(None);

fn seed(app: &mut TrafficApp, mode: usize) {
    let now = chrono::Utc::now().timestamp_millis();
    let mut data = Shared {
        connected: mode != 3,
        client_name: "Clash Verge Rev".into(),
        status: "等待连接".into(),
        down: 8.42 * 1024. * 1024.,
        up: 512. * 1024.,
        ..Default::default()
    };
    for (index, name) in [
        "浏览器.exe",
        "Codex.exe",
        "音乐.exe",
        "同步服务.exe",
        "idle.exe",
    ]
    .into_iter()
    .enumerate()
    {
        let key = format!(r"C:\应用程序\{}\{name}", "long path ".repeat(10));
        std::sync::Arc::make_mut(&mut data.connections).push(Connection {
            id: format!("overview-{index}"),
            metadata: Metadata {
                network: if index == 0 { "udp" } else { "tcp" }.into(),
                process_path: key.clone(),
                ..Default::default()
            },
            ..Default::default()
        });
        if index < 4 {
            data.application_rates.insert(
                key.clone(),
                Bytes {
                    down: (5 - index) as u64 * 100_000,
                    up: 20_000,
                },
            );
        }
    }
    for index in 0..300 {
        data.live.push_back(LiveSample {
            from: now - (300 - index) * 1000,
            to: now - (299 - index) * 1000,
            down: (index % 23 + 1) as f64 * 100_000.,
            up: (index % 7) as f64 * 10_000.,
            valid: true,
        });
    }
    data.subscriptions = crate::subscriptions::View {
        scope: "scope-a".into(),
        local_profile: true,
        read_at: Some(now),
        items: vec![
            crate::subscriptions::Subscription {
                id: "primary".into(),
                name: if mode == 4 {
                    "非常长的订阅名称 · ".repeat(16)
                } else {
                    "常用订阅".into()
                },
                active: false,
                used: (!matches!(mode, 1 | 7)).then_some(Bytes {
                    down: 150 * 1024_u64.pow(3),
                    up: 35 * 1024_u64.pow(3),
                }),
                total: (!matches!(mode, 1 | 7)).then_some(if mode == 4 {
                    u64::MAX
                } else {
                    500 * 1024_u64.pow(3)
                }),
                expires: (!matches!(mode, 1 | 7)).then_some(now + 27 * 86_400_000),
                updated: Some(now - 3_600_000),
            },
            crate::subscriptions::Subscription {
                id: "secondary".into(),
                name: "备用订阅".into(),
                active: false,
                used: Some(Bytes { down: 100, up: 200 }),
                total: Some(1000),
                expires: None,
                updated: None,
            },
        ],
        ..Default::default()
    };
    let mut third = data.subscriptions.items[1].clone();
    third.id = "third".into();
    third.name = "第三个子订阅".into();
    data.subscriptions.items.push(third);
    data.subscriptions.saved = data.subscriptions.items[..1].to_vec();
    data.subscriptions.saved[0].name = "已保存的独立订阅".into();
    data.subscriptions.saved_read_at = Some(now);
    if mode == 2 {
        data.subscriptions.items.clear();
    }
    if mode == 5 {
        data.subscriptions
            .items
            .extend(data.subscriptions.items.clone());
    }

    if matches!(mode, 6 | 7) {
        data.subscriptions.items.truncate(1);
    }

    let mut report = Report {
        query: Some(app.query.clone()),
        total: Bytes {
            down: 15 * 1024_u64.pow(3),
            up: 3 * 1024_u64.pow(3),
        },
        observed: Bytes {
            down: 14 * 1024_u64.pow(3),
            up: 3 * 1024_u64.pow(3),
        },
        app_count: 5,
        ..Default::default()
    };
    for index in 0..120 {
        report.series.push(SeriesPoint {
            time: app.query.from + index * 60_000,
            down: (index % 11 + 1) as f64 * 1024. * 1024.,
            up: index as f64 * 10000.,
        });
    }
    report.apps = data
        .connections
        .iter()
        .enumerate()
        .map(|(index, c)| Row {
            key: c.app(),
            bytes: Bytes {
                down: (5 - index) as u64 * 1_000_000_000,
                up: 200_000_000,
            },
        })
        .collect();
    report.domains = (0..5)
        .map(|index| Row {
            key: format!("{}.example.com", "very-long-domain-".repeat(6)),
            bytes: Bytes {
                down: (5 - index) * 900_000_000,
                up: 200_000_000,
            },
        })
        .collect();
    report.routes = vec![
        Row {
            key: "香港 01 · 非常长的出站名称".repeat(4),
            bytes: Bytes {
                down: 10 * 1024_u64.pow(3),
                up: 0,
            },
        },
        Row {
            key: "DIRECT".into(),
            bytes: Bytes {
                down: 4 * 1024_u64.pow(3),
                up: 0,
            },
        },
        Row {
            key: "新加坡 02".into(),
            bytes: Bytes {
                down: 3 * 1024_u64.pow(3),
                up: 0,
            },
        },
    ];
    data.report = report.clone();
    data.report_id = app.query_id;
    app.report = report;
    app.data = data.clone();
    app.overview_state.usage_source = Some((data.storage_revision, data.storage_epoch));
    app.overview_state.subscription_scope = data.subscriptions.scope.clone();
    *app.backend.shared.lock().unwrap() = data;
}

pub fn schedule(window: &mut Window, cx: &mut Context<TrafficApp>) {
    cx.spawn_in(window, async move |this,cx| {
        let result: anyhow::Result<()> = async {
            this.update_in(cx,|app,window,cx| {
                app.act(Action::Nav(Page::Overview),window,cx);
                seed(app,0);
                let query = app.query_id;
                let config = toml::to_string(&app.config)?;
                app.act(Action::Overview(OverviewAction::SubscriptionSource(true)),window,cx);
                anyhow::ensure!(app.overview_state.show_saved,"saved subscriptions did not open");
                anyhow::ensure!(app.data.subscriptions.items.len()==3 && app.data.subscriptions.saved.len()==1,"current and saved sources were mixed");
                anyhow::ensure!(app.query_id==query && toml::to_string(&app.config)?==config,"subscription view changed settings or statistics");
                app.backend.shared.lock().unwrap().subscriptions.scope="scope-b".into();
                app.poll_overview(cx);
                anyhow::ensure!(!app.overview_state.show_saved,"new source did not default to current configuration");
                Ok::<_,anyhow::Error>(())
            })??;
            for theme in ["light","dark"] {
                for width in [900.,1240.,1600.] {
                    for mode in [0,4] {
                        this.update_in(cx,|app,window,cx| {
                            window.resize(size(px(width),px(if width==900. {700.} else {870.})));
                            app.act(Action::Theme(theme),window,cx);
                            app.act(Action::ColorTheme(ColorTheme::Mica),window,cx);
                            app.page_scroll.set_offset(point(px(0.),px(0.)));
                            seed(app,mode);
                            app.show_overview_exit(mode == 4);
                            app.overview_state.exit.results[0] = crate::exit_ip::Probe::Found(Box::new(crate::exit_ip::Detection {
                                record: serde_json::from_str(r#"{"ip":"192.0.2.42","location":{"country":"United States","country_code":"US"}}"#)?,
                                kind: Some(("家宽", "Proxycheck")),
                                score: Some((0, "IPQuery")),
                                risk_source: None,
                                mode: "rule".into(),
                                time: chrono::Local::now(),
                            }));
                            app.overview_state.exit.results[1] = crate::exit_ip::Probe::Found(Box::new(crate::exit_ip::Detection {
                                record: serde_json::from_str(r#"{
                                    "ip":"2001:db8:1234:5678:9abc:def0:1234:5678",
                                    "isp":{"org":"Long example network organization name","asn":"AS64496"},
                                    "location":{"country":"United States","country_code":"US","city":"Los Angeles"},
                                    "risk":{"is_datacenter":true,"risk_score":20}
                                }"#)?,
                                kind: Some(("机房", "IPQuery")),
                                score: Some((20, "IPQuery")),
                                risk_source: None,
                                mode: "rule".into(),
                                time: chrono::Local::now(),
                            }));
                            cx.notify();
                            Ok::<_, anyhow::Error>(())
                        })??;
                        Timer::after(Duration::from_millis(500)).await;
                        this.update_in(cx,|app,window,_| {
                            let bounds=app.overview_state.bounds.borrow();
                            for name in ["overview-usage","overview-download","overview-upload","overview-subscription","overview-chart","overview-status","overview-current-connections","overview-current-applications","overview-apps","overview-domains","overview-routes"] {
                                let rect=bounds.get(name).ok_or_else(||anyhow::anyhow!("missing panel {name}"))?;
                                anyhow::ensure!(rect.size.width>px(80.) && rect.right()<=window.viewport_size().width+px(1.), "panel {name} overflows at {theme} {width}: {rect:?}");
                            }
                            let usage=bounds["overview-usage"];
                            let subscription=bounds["overview-subscription"];
                            anyhow::ensure!(bounds["overview-routes"].size.height==bounds["overview-apps"].size.height, "outbound card did not align");
                            if width>=1240. {
                                anyhow::ensure!((usage.top()-subscription.top()).abs()<px(2.), "top cards not aligned");
                                anyhow::ensure!(bounds["overview-routes"].bottom()<=window.viewport_size().height, "dashboard does not fit the default first screen");
                            } else { anyhow::ensure!(subscription.top()>=usage.bottom(),"narrow subscription did not reflow"); }
                            anyhow::ensure!(app.page_scroll.max_offset().width<=px(1.),"dashboard has horizontal overflow");
                            Ok::<_,anyhow::Error>(())
                        })??;
                    }
                }
            }
            for mode in [1,2,3,5,6,7] {
                this.update_in(cx,|app,window,cx| {
                    window.resize(size(px(900.),px(700.)));
                    seed(app,mode);
                    cx.notify();
                })?;
                Timer::after(Duration::from_millis(450)).await;
            }
            this.update_in(cx,|app,_window,cx| {
                seed(app,0);
                app.show_overview_exit(false);
                app.hover_overview_usage(0,true,cx);
                let old=app.overview_state.pending.clone().unwrap();
                app.hover_overview_usage(1,true,cx);
                let current=app.overview_state.pending.clone().unwrap();
                *app.backend.overview_usage.reply.lock().unwrap()=Some(crate::overview_usage::Reply{request:old,result:Ok(Default::default())});
                app.poll_overview(cx);
                anyhow::ensure!(app.overview_state.hover_data.is_none() && app.overview_state.pending.as_ref()==Some(&current),"stale bucket data was accepted");
                let detail=crate::overview_usage::Detail{observed:true,total:app.overview_state.hover.unwrap().bin.bytes,applications:app.report.apps.iter().take(3).cloned().collect(),count:3,unassigned:Bytes{down:1024,up:512}};
                *app.backend.overview_usage.reply.lock().unwrap()=Some(crate::overview_usage::Reply{request:current,result:Ok(detail)});
                app.poll_overview(cx);
                anyhow::ensure!(app.overview_state.hover_data.is_some(),"bucket application details were not received");
                app.pin_overview_usage(1,cx);
                anyhow::ensure!(app.overview_state.hover.is_some_and(|h|h.pinned),"bucket did not pin");
                cx.notify();
                Ok::<_,anyhow::Error>(())
            })??;
            Timer::after(Duration::from_millis(300)).await;
            this.update_in(cx,|app,window,_| {
                let rect=app.overview_state.hover_bounds.get();
                anyhow::ensure!(rect.size.width>=px(254.) && rect.size.width<=px(336.) && rect.left()>=px(0.) && rect.right()<=window.viewport_size().width && rect.bottom()<=window.viewport_size().height,"bucket detail popup overflowed: {rect:?}");
                Ok::<_,anyhow::Error>(())
            })??;
            cx.update(|window,cx| {window.dispatch_keystroke(Keystroke::parse("escape").unwrap(),cx);})?;
            this.update_in(cx,|app,window,cx| {
                anyhow::ensure!(app.overview_state.hover.is_none(),"Escape did not close the bucket popup");
                app.pin_overview_usage(1,cx);
                let position=app.overview_state.bar_bounds.borrow()[&1].center();
                app.dismiss_overview_hover(position,cx);
                app.hover_overview_usage(1,true,cx);
                anyhow::ensure!(app.overview_state.hover.is_none(),"dismissed bucket immediately reopened under the pointer");
                app.hover_overview_usage(1,false,cx);app.hover_overview_usage(1,true,cx);
                anyhow::ensure!(app.overview_state.hover.is_some(),"leaving and reentering the bucket did not restore hover");
                app.close_overview_hover(cx);

                let bin=app.overview_bins()[0];let key=app.report.apps[0].key.clone();
                app.open_overview_range(bin,Some(key.clone()),window,cx);
                anyhow::ensure!(app.page==Page::History && (app.query.from,app.query.to)==(bin.from,bin.to),"bucket history lost its exact range");
                anyhow::ensure!(app.active_detail_context().is_some_and(|c|c.key==key && !c.group_apps),"application history changed raw process accounting");
                app.act(Action::Nav(Page::Overview),window,cx);seed(app,0);
                let bin=app.overview_bins()[1];app.open_overview_range(bin,None,window,cx);
                anyhow::ensure!(app.page==Page::History && app.query.from==bin.from && !app.usage_detail_open(),"period history opened the wrong view");
                app.act(Action::Nav(Page::Overview),window,cx);seed(app,0);
                app.act(Action::Period(TimePeriod::LastSevenDays),window,cx);
                seed(app,0);
                let down=app.data.down;
                app.act(Action::Overview(OverviewAction::Records("route")),window,cx);
                anyhow::ensure!(app.page==Page::History && app.query.dimension=="route", "outbound link lost its dimension");
                app.act(Action::Nav(Page::Overview),window,cx);
                app.act(Action::Overview(OverviewAction::Activity),window,cx);
                anyhow::ensure!(app.page==Page::Apps && app.activity.details, "activity link did not open current activity");
                app.act(Action::Nav(Page::Overview),window,cx);
                seed(app,0);
                anyhow::ensure!(app.data.down==down,"period controls changed live speeds");
                Ok::<_,anyhow::Error>(())
            })??;
            let hold=std::env::var("MIHOMO_OVERVIEW_QA_HOLD_MS").ok().and_then(|v|v.parse::<u64>().ok()).unwrap_or(0);
            if hold>0 {
                this.update_in(cx,|app,window,cx| {
                    let width=std::env::var("MIHOMO_OVERVIEW_QA_WIDTH").ok().and_then(|v|v.parse::<f32>().ok()).unwrap_or(1240.);
                    window.resize(size(px(width),px(870.)));
                    let theme=if std::env::var("MIHOMO_OVERVIEW_QA_THEME").is_ok_and(|v|v=="dark") { "dark" } else { "light" };
                    app.act(Action::Theme(theme),window,cx);
                    app.page_scroll.set_offset(point(px(0.),px(0.)));
                    let single=std::env::var("MIHOMO_OVERVIEW_QA_SINGLE").is_ok_and(|v|v=="1");
                    seed(app,if single {6}else{0});
                    if std::env::var("MIHOMO_OVERVIEW_QA_POPUP").is_ok_and(|v|v=="1") {
                        app.hover_overview_usage(1,true,cx);
                        let request=app.overview_state.pending.clone().unwrap();
                        let total=app.overview_state.hover.unwrap().bin.bytes;
                        let applications=app.report.apps.iter().take(3).enumerate().map(|(index,item)|Row{key:item.key.clone(),bytes:Bytes{down:total.down/(2_u64<<index),up:total.up/(2_u64<<index)}}).collect();
                        let detail=crate::overview_usage::Detail{observed:true,total,applications,count:3,unassigned:Bytes{down:total.down/8,up:total.up/8}};
                        *app.backend.overview_usage.reply.lock().unwrap()=Some(crate::overview_usage::Reply{request,result:Ok(detail)});
                        app.poll_overview(cx);app.pin_overview_usage(1,cx);
                    }
                    cx.notify();
                })?;
                println!("overview visual review ready");
                Timer::after(Duration::from_millis(hold)).await;
            }
            println!("GPUI overview passed: 12 layouts, first-screen fit, narrow reflow, large/missing/local/offline data, source-separated subscription lists, bucket application details, stale reply rejection, pin/Escape and exact history navigation");
            Ok(())
        }.await;
        *OUTCOME.lock().unwrap()=Some(result.map_err(|e|e.to_string()));
        let _=cx.update(|_,cx|cx.quit());
    }).detach();
}

#[::core::prelude::v1::test]
#[ignore = "opens an isolated overview dashboard for layout and navigation checks"]
fn gpui_overview_bento_layout_and_subscription_selection() {
    let dir = tempfile::tempdir().unwrap();
    Config {
        endpoint: "http://127.0.0.1:9".into(),
        floating: false,
        taskbar: false,
        ..Default::default()
    }
    .save(dir.path())
    .unwrap();
    let backend = Backend::start(dir.path().into()).unwrap();
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
        .expect("overview checks did not finish")
        .unwrap();
}
