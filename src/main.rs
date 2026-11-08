#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod accounting;
mod analysis;
mod app_activity;
mod app_icons;
mod app_trend;
mod attribution;
mod backend;
mod backup;
mod client_config;
mod config;
mod config_writer;
mod csv_export;
mod data_location;
mod date_range;
mod detail_cache;
mod exit_ip;
mod floating_backdrop;
mod history;
mod hotkey;
mod live_chart;
mod mica_effect;
mod migration_progress;
mod mihomo;
mod model;
mod native;
mod overview_usage;
mod permanent_control;
mod process_control;
mod release_update;
mod speed_stats;
mod speed_worker;
mod storage;
mod storage_management;
mod subscriptions;
mod system_metrics;
mod taskbar_content;
mod taskbar_layout;
mod taskbar_probe;
mod taskbar_text;
mod theme;
mod traffic_drill;
mod traffic_icon;
mod tray;
mod ui;
mod wallpaper;

fn start() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).is_some_and(|arg| arg == "--desktop-ui") {
        return ui::run_interface_process(args.get(2).map(String::as_str));
    }
    if let Some(index) = args.iter().position(|a| a == "--relocate-data") {
        let path = args
            .get(index + 1)
            .ok_or_else(|| anyhow::anyhow!("缺少迁移请求文件"))?;
        return data_location::coordinate(std::path::Path::new(path));
    }
    if args.iter().any(|a| a == "--exit") {
        return native::request_app_exit();
    }
    if args.iter().any(|a| a == "--purge-personal-data") {
        return data_location::purge_personal_data(&config::default_data_dir());
    }
    if args.get(1).is_some_and(|arg| arg == "--continue-uninstall") {
        let [_, _, parent, filename, choice] = args.as_slice() else {
            anyhow::bail!("卸载参数不完整");
        };
        anyhow::ensure!(matches!(choice.as_str(), "keep" | "remove"), "卸载选项无效");
        return native::continue_uninstall(parent.parse()?, filename, choice == "remove");
    }
    if args.iter().any(|a| a == "--probe-taskbar") {
        return taskbar_probe::diagnostic();
    }
    if args.iter().any(|a| a == "--probe") {
        return backend::probe();
    }
    let headless = args.iter().any(|a| a == "--headless");
    let _instance = if headless {
        None
    } else {
        match native::single_instance()? {
            Some(instance) => Some(instance),
            None => return Ok(()),
        }
    };
    if args.iter().any(|a| a == "--upgrade-floating-default") {
        let dir = config::data_dir()?;
        let mut config = config::Config::load(&dir)?;
        if config.upgrade_floating_default() {
            config.save(&dir)?;
        }
        return Ok(());
    }
    let backend = backend::Backend::start(config::data_dir()?)?;
    if let Some(index) = args.iter().position(|a| a == "--migration-notice")
        && let Some(path) = args.get(index + 1)
        && let Ok(message) = std::fs::read_to_string(path)
    {
        backend.notice(message);
        let _ = std::fs::remove_file(path);
    }
    let run_result = if headless {
        let seconds = args
            .iter()
            .position(|s| s == "--seconds")
            .and_then(|i| args.get(i + 1))
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(10);
        std::thread::sleep(std::time::Duration::from_secs(seconds));
        let state = backend.snapshot();
        println!(
            "connected={} connections={} recorded_today={} status={}",
            state.connected,
            state.connections.len(),
            state.today.total(),
            state.status
        );
        if state.connected {
            Ok(())
        } else {
            Err(anyhow::anyhow!("未能连接 Mihomo"))
        }
    } else {
        ui::run_desktop(backend.clone())
    };
    let shutdown_result = backend.shutdown();
    run_result?;
    shutdown_result
}
fn main() {
    if let Err(e) = start() {
        eprintln!("{e:#}");
        if !std::env::args().any(|s| {
            matches!(
                s.as_str(),
                "--probe"
                    | "--headless"
                    | "--probe-taskbar"
                    | "--relocate-data"
                    | "--desktop-ui"
                    | "--migration-ready"
                    | "--purge-personal-data"
            )
        }) {
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
                    std::ptr::null_mut(),
                    native::wide(format!("{e:#}")).as_ptr(),
                    native::wide("Mihomo Traffic").as_ptr(),
                    0x10,
                );
            }
        }
        std::process::exit(1);
    }
}
