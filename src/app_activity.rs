use crate::model::{Bytes, Shared};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

pub const ACTIVITY_WINDOW_MS: i64 = 5 * 60 * 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ActivityState {
    Transferring,
    Idle,
}
impl ActivityState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Transferring => "正在传输",
            Self::Idle => "暂无传输",
        }
    }
}
#[derive(Clone, Debug, Default)]
pub struct AppActivity {
    pub key: String,
    pub rate: Bytes,
    pub connections: usize,
}
impl AppActivity {
    pub fn state(&self) -> ActivityState {
        if self.rate.total() > 0 {
            ActivityState::Transferring
        } else {
            ActivityState::Idle
        }
    }
}
#[cfg(test)]
pub fn activity(state: &Shared) -> Vec<AppActivity> {
    collect_activity(state, false)
}
pub fn application_activity(state: &Shared) -> Vec<AppActivity> {
    collect_activity(state, true)
}
fn collect_activity(state: &Shared, grouped: bool) -> Vec<AppActivity> {
    if !state.connected {
        return vec![];
    }
    let mut apps = HashMap::<String, AppActivity>::new();
    for connection in state.connections.iter() {
        let key = if grouped {
            connection.application()
        } else {
            connection.app()
        };
        apps.entry(key.clone())
            .or_insert_with(|| AppActivity {
                key,
                ..Default::default()
            })
            .connections += 1;
    }
    let rates = if grouped && !state.application_rates.is_empty() {
        &state.application_rates
    } else {
        &state.app_rates
    };
    for (key, rate) in rates {
        apps.entry(key.clone())
            .or_insert_with(|| AppActivity {
                key: key.clone(),
                ..Default::default()
            })
            .rate = *rate;
    }
    let mut apps: Vec<_> = apps
        .into_values()
        .filter(|app| app.rate.total() > 0 || app.connections > 0)
        .collect();
    apps.sort_by(|a, b| {
        a.state()
            .cmp(&b.state())
            .then_with(|| b.rate.total().cmp(&a.rate.total()))
            .then_with(|| a.key.cmp(&b.key))
    });
    apps
}
#[derive(Default)]
pub struct ActivityOrder {
    ranks: HashMap<String, usize>,
    sorted_at: Option<Instant>,
}
impl ActivityOrder {
    pub fn arrange(&mut self, apps: &mut [AppActivity], now: Instant) {
        if apps.is_empty() {
            *self = Self::default();
            return;
        }
        if self
            .sorted_at
            .is_none_or(|last| now.saturating_duration_since(last) >= Duration::from_secs(3))
        {
            self.ranks = apps
                .iter()
                .enumerate()
                .map(|(index, app)| (app.key.clone(), index))
                .collect();
            self.sorted_at = Some(now);
        } else {
            apps.sort_by(|a, b| {
                a.state()
                    .cmp(&b.state())
                    .then_with(|| {
                        self.ranks
                            .get(&a.key)
                            .unwrap_or(&usize::MAX)
                            .cmp(self.ranks.get(&b.key).unwrap_or(&usize::MAX))
                    })
                    .then_with(|| a.key.cmp(&b.key))
            });
        }
    }
}
pub fn share_label(value: u64, total: u64) -> String {
    if total == 0 {
        return "—".into();
    }
    let share = (value as f64 / total as f64).min(1.) * 100.;
    if value > 0 && share < 0.1 {
        "<0.1%".into()
    } else {
        format!("{share:.1}%")
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Connection, Metadata};
    #[test]
    fn live_list_only_has_rates_and_current_connections_and_keeps_paths() {
        let mut state = Shared {
            connected: true,
            ..Default::default()
        };
        for key in [
            r"C:\One\same.exe",
            r"C:\One\same.exe",
            r"D:\Two\same.exe",
            "idle",
            "未知进程",
        ] {
            std::sync::Arc::make_mut(&mut state.connections).push(Connection {
                metadata: Metadata {
                    process: key.into(),
                    ..Default::default()
                },
                ..Default::default()
            });
        }
        state
            .app_rates
            .insert(r"D:\Two\same.exe".into(), Bytes { down: 1, up: 100 });
        state
            .app_rates
            .insert("finished-last-sample".into(), Bytes { down: 1, up: 0 });
        let apps = activity(&state);
        assert_eq!(apps.len(), 5);
        assert_eq!(apps[0].key, r"D:\Two\same.exe");
        assert_eq!(apps[1].key, "finished-last-sample");
        assert_eq!(
            apps.iter()
                .find(|app| app.key == r"C:\One\same.exe")
                .unwrap()
                .connections,
            2
        );
        assert_eq!(apps.last().unwrap().state(), ActivityState::Idle);
        state.app_rates.clear();
        assert_eq!(activity(&state).len(), 4);
        state.connected = false;
        assert!(activity(&state).is_empty());
    }
    #[test]
    fn stable_order_updates_values_and_pins_new_transfers_immediately() {
        let now = Instant::now();
        let mut state = Shared {
            connected: true,
            ..Default::default()
        };
        state
            .app_rates
            .insert("a".into(), Bytes { down: 100, up: 0 });
        state
            .app_rates
            .insert("b".into(), Bytes { down: 10, up: 0 });
        let mut order = ActivityOrder::default();
        let mut apps = activity(&state);
        order.arrange(&mut apps, now);
        state.app_rates.get_mut("b").unwrap().down = 1000;
        apps = activity(&state);
        order.arrange(&mut apps, now + Duration::from_secs(1));
        assert_eq!(apps[0].key, "a");
        assert_eq!(apps[1].rate.down, 1000);
        apps = activity(&state);
        order.arrange(&mut apps, now + Duration::from_secs(3));
        assert_eq!(apps[0].key, "b");
        assert_eq!(share_label(1, 10000), "<0.1%");
    }
}
