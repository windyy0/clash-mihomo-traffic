//! Read-only subscription summaries from the verified client's local cache.
//! Subscription URLs, credentials and downloaded profile contents never enter the view.
use crate::{
    backend::Backend,
    client_config::{Kind, Target},
    model::Bytes,
};
use anyhow::{Result, ensure};
use serde_yaml::Value;
use std::{
    io::Read,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Subscription {
    pub id: String,
    pub name: String,
    pub active: bool,
    pub used: Option<Bytes>,
    pub total: Option<u64>,
    pub expires: Option<i64>,
    pub updated: Option<i64>,
}
impl Subscription {
    pub fn remaining(&self) -> Option<u64> {
        Some(self.total?.saturating_sub(self.used?.total()))
    }
    pub fn fraction(&self) -> Option<f32> {
        Some((self.used?.total() as f64 / self.total? as f64).clamp(0., 1.) as f32)
    }
}

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct View {
    pub scope: String,
    pub items: Vec<Subscription>,
    pub saved: Vec<Subscription>,
    pub local_profile: bool,
    pub loading: bool,
    pub stale: bool,
    pub read_at: Option<i64>,
    pub message: String,
    pub saved_stale: bool,
    pub saved_message: String,
    pub saved_read_at: Option<i64>,
}

#[derive(Default)]
pub struct Control {
    refresh: AtomicBool,
}
impl Control {
    pub fn refresh(&self) {
        self.refresh.store(true, Ordering::Release);
    }
}

fn count(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}
fn timestamp(value: &Value) -> Option<i64> {
    let value = i64::try_from(count(value)?).ok()?.checked_mul(1000)?;
    (value > 0 && chrono::DateTime::from_timestamp_millis(value).is_some()).then_some(value)
}

pub(crate) fn parse_verge(source: &str) -> Result<View> {
    // Keep parser errors generic: serde's diagnostic can quote private source values.
    let document: Value = serde_yaml::from_str(source.trim_start_matches('\u{feff}'))
        .map_err(|_| anyhow::anyhow!("订阅缓存格式无效"))?;
    let items = document["items"]
        .as_sequence()
        .ok_or_else(|| anyhow::anyhow!("订阅缓存结构暂不支持"))?;
    ensure!(items.len() <= 4096, "订阅条目过多，未读取");
    let current = document["current"].as_str();
    let local_profile = items.iter().any(|item| {
        item["uid"].as_str() == current
            && current.is_some()
            && item["type"].as_str() == Some("local")
    });
    let mut subscriptions = Vec::new();
    let mut ids = std::collections::HashSet::new();
    for item in items
        .iter()
        .filter(|item| item["type"].as_str() == Some("remote"))
    {
        let Some(id) = item["uid"].as_str().filter(|id| !id.is_empty()) else {
            continue;
        };
        ensure!(ids.insert(id), "订阅标识重复，请在客户端检查配置");
        let name = item["name"].as_str().unwrap_or_default().trim();
        let name = if name.is_empty() || name.contains("://") {
            "未命名订阅".into()
        } else {
            name.chars()
                .filter(|ch| !ch.is_control())
                .take(128)
                .collect()
        };
        let info = &item["extra"];
        let used = count(&info["download"])
            .zip(count(&info["upload"]))
            .map(|(down, up)| Bytes { down, up });
        subscriptions.push(Subscription {
            id: id.into(),
            name,
            active: current == Some(id),
            used,
            total: count(&info["total"]).filter(|value| *value > 0),
            expires: timestamp(&info["expire"]),
            updated: timestamp(&item["updated"]),
        });
    }
    Ok(View {
        items: subscriptions,
        local_profile,
        ..Default::default()
    })
}

fn read(target: &Target) -> Result<View> {
    ensure!(target.kind == Kind::Verge, "此客户端暂不支持读取订阅信息");
    let root = target
        .root
        .canonicalize()
        .map_err(|_| anyhow::anyhow!("客户端目录不可用"))?;
    let file = root
        .join("profiles.yaml")
        .canonicalize()
        .map_err(|_| anyhow::anyhow!("尚未找到客户端订阅缓存"))?;
    ensure!(file.starts_with(&root), "订阅缓存位于客户端目录以外");
    let mut bytes = Vec::new();
    std::fs::File::open(file)
        .and_then(|file| file.take(16 * 1024 * 1024 + 1).read_to_end(&mut bytes))
        .map_err(|_| anyhow::anyhow!("无法读取订阅缓存"))?;
    ensure!(bytes.len() <= 16 * 1024 * 1024, "订阅缓存过大，未读取");
    let source =
        std::str::from_utf8(&bytes).map_err(|_| anyhow::anyhow!("订阅缓存编码暂不支持"))?;
    parse_verge(source)
}

#[derive(Clone, PartialEq, Eq)]
struct Scope {
    endpoint: String,
    configured: String,
    generation: u64,
    identity: String,
    verified: bool,
    secret: String,
    target: Option<Target>,
}
impl Scope {
    fn capture(handle: &Backend) -> Self {
        let target = handle.permanent_control.subscription_target();
        let config = handle.config.lock().unwrap().clone();
        let generation = handle.process_generation.load(Ordering::Acquire);
        let state = handle.shared.lock().unwrap();
        let target =
            target.filter(|target| Kind::from_name(&state.client_name) == Some(target.kind));
        Self {
            endpoint: state.endpoint.clone(),
            configured: config.endpoint,
            generation,
            identity: state.core_identity.clone(),
            verified: state.core_verified,
            secret: config.secret,
            target,
        }
    }
    fn key(&self) -> String {
        format!(
            "{}|{}|{}|{}|{:?}",
            self.configured, self.generation, self.endpoint, self.identity, self.target
        )
    }
}

pub fn start(handle: Backend) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    std::thread::Builder::new()
        .name("subscription-cache".into())
        .spawn(move || {
            let mut scope: Option<Scope> = None;
            let mut due = Instant::now();
            while !handle.stop.load(Ordering::Relaxed) {
                let next = Scope::capture(&handle);
                if scope.as_ref() != Some(&next) {
                    let mut state = handle.shared.lock().unwrap();
                    state.subscriptions = View {
                        scope: next.key(),
                        ..Default::default()
                    };
                    scope = Some(next.clone());
                    due = Instant::now();
                }
                if handle.subscriptions.refresh.swap(false, Ordering::AcqRel) {
                    due = Instant::now();
                }
                if Instant::now() >= due {
                    handle.shared.lock().unwrap().subscriptions.loading = true;
                    let saved = if let Some(target) = &next.target {
                        read(target)
                    } else {
                        Err(anyhow::anyhow!("连接 Clash Verge Rev 后可查看已保存订阅"))
                    };
                    let current = if handle.shared.lock().unwrap().connected {
                        let mut config = handle.config.lock().unwrap().clone();
                        config.endpoint = next.endpoint.clone();
                        runtime.block_on(read_current(
                            &config,
                            next.verified.then_some(next.identity.clone()),
                        ))
                    } else {
                        Err(anyhow::anyhow!("连接内核后读取当前配置的订阅"))
                    };
                    if !handle.stop.load(Ordering::Relaxed) && Scope::capture(&handle) == next {
                        let mut state = handle.shared.lock().unwrap();
                        let view = &mut state.subscriptions;
                        view.loading = false;
                        let now = chrono::Utc::now().timestamp_millis();
                        match current {
                            Ok(items) => {
                                view.items = items;
                                view.stale = false;
                                view.message.clear();
                                view.read_at = Some(now);
                            }
                            Err(error) => {
                                view.stale = !view.items.is_empty();
                                view.message = error.to_string();
                            }
                        }
                        match saved {
                            Ok(saved) => {
                                view.saved = saved.items;
                                view.local_profile = saved.local_profile;
                                view.saved_stale = false;
                                view.saved_message.clear();
                                view.saved_read_at = Some(now);
                            }
                            Err(error) => {
                                view.saved_stale = !view.saved.is_empty();
                                view.saved_message = error.to_string();
                            }
                        }
                        drop(state);
                        handle.query_ready.notify_one();
                    }
                    due = Instant::now() + Duration::from_secs(60);
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        })?;
    Ok(())
}

fn name(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() || value.contains("://") {
        return "未命名订阅".into();
    }
    value
        .chars()
        .filter(|ch| !ch.is_control())
        .take(128)
        .collect()
}

#[derive(serde::Deserialize)]
struct Providers {
    providers: std::collections::BTreeMap<String, Provider>,
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Provider {
    #[serde(default)]
    vehicle_type: String,
    subscription_info: Option<serde_json::Value>,
    updated_at: Option<String>,
}
pub(crate) fn parse_current(bytes: &[u8]) -> Result<Vec<Subscription>> {
    let response: Providers =
        serde_json::from_slice(bytes).map_err(|_| anyhow::anyhow!("订阅接口格式暂不支持"))?;
    ensure!(response.providers.len() <= 4096, "订阅条目过多，未读取");
    let number = |info: &serde_json::Value, upper: &str, lower: &str| {
        let value = info.get(upper).or_else(|| info.get(lower))?;
        value
            .as_u64()
            .or_else(|| value.as_str()?.parse::<u64>().ok())
    };
    Ok(response
        .providers
        .into_iter()
        .filter(|(_, p)| {
            p.vehicle_type.eq_ignore_ascii_case("http")
                || p.subscription_info.as_ref().is_some_and(|v| v.is_object())
        })
        .enumerate()
        .map(|(index, (key, provider))| {
            let info = provider.subscription_info.unwrap_or_default();
            let expires = number(&info, "Expire", "expire")
                .and_then(|v| i64::try_from(v).ok())
                .and_then(|v| v.checked_mul(1000))
                .filter(|v| *v > 0 && chrono::DateTime::from_timestamp_millis(*v).is_some());
            Subscription {
                id: format!("provider-{index}"),
                name: name(&key),
                active: false,
                used: number(&info, "Download", "download")
                    .zip(number(&info, "Upload", "upload"))
                    .map(|(down, up)| Bytes { down, up }),
                total: number(&info, "Total", "total").filter(|v| *v > 0),
                expires,
                updated: provider
                    .updated_at
                    .and_then(|v| chrono::DateTime::parse_from_rfc3339(&v).ok())
                    .map(|v| v.timestamp_millis()),
            }
        })
        .collect())
}

async fn read_current(
    config: &crate::config::Config,
    identity: Option<String>,
) -> Result<Vec<Subscription>> {
    let mut client =
        crate::mihomo::Client::new(config).map_err(|_| anyhow::anyhow!("订阅接口暂不可用"))?;
    if let Some(identity) = identity {
        client.pin_identity(identity);
    }
    let bytes = client
        .request("/providers/proxies")
        .await
        .map_err(|_| anyhow::anyhow!("无法读取当前配置的订阅，请检查内核连接"))?;
    parse_current(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "read-only smoke check against the explicitly supplied local client configuration"]
    fn current_live_provider_metadata_smoke() {
        let path = std::env::var_os("MIHOMO_SUBSCRIPTION_QA_CONFIG_DIR")
            .expect("an explicit config directory is required");
        let config = crate::config::Config::load(std::path::Path::new(&path)).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let items = runtime.block_on(read_current(&config, None)).unwrap();
        println!(
            "Current configuration: {} remote providers; {} supply quota metadata",
            items.len(),
            items.iter().filter(|i| i.total.is_some()).count()
        );
        for item in &items {
            println!(
                "{} · remaining {:?} · expiry supplied {}",
                item.name,
                item.remaining(),
                item.expires.is_some()
            );
        }
        assert!(!items.is_empty());
    }
    #[test]
    fn current_providers_keep_source_quotas_missing_fields_and_credentials_separate() {
        let bytes=br#"{"providers":{"current-only":{"vehicleType":"HTTP","subscriptionInfo":{"Upload":20,"Download":80,"Total":1000,"Expire":1800000000},"updatedAt":"2026-11-03T06:00:00+08:00","proxies":[{"password":"PRIVATE"}]},"no-quota":{"vehicleType":"http"},"default":{"vehicleType":"Compatible"},"local-file":{"vehicleType":"File"},"over-quota":{"vehicleType":"HTTP","subscriptionInfo":{"Upload":50,"Download":100,"Total":100,"Expire":0}}}}"#;
        let items = parse_current(bytes).unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].name, "current-only");
        assert_eq!(items[0].remaining(), Some(900));
        assert!(items[0].updated.is_some());
        assert!(items[0].expires.is_some());
        assert!(items[1].used.is_none());
        assert!(items[1].remaining().is_none());
        assert_eq!(items[2].remaining(), Some(0));
        assert_eq!(items[2].expires, None);
        assert!(!format!("{items:?}").contains("PRIVATE"));
        assert!(parse_current(b"{\"providers\":{\"secret\":PRIVATE}").is_err());
    }

    #[test]
    fn current_api_uses_only_a_read_request_and_preserves_authentication() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut chunk = [0_u8; 2048];
            while !bytes.windows(4).any(|w| w == b"\r\n\r\n") {
                let len = socket.read(&mut chunk).unwrap();
                assert!(len > 0);
                bytes.extend_from_slice(&chunk[..len]);
            }
            let request = String::from_utf8(bytes).unwrap();
            assert!(request.starts_with("GET /providers/proxies HTTP/1.1\r\n"));
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("authorization: bearer fixture-secret")
            );
            let body = r#"{"providers":{"Fixture":{"vehicleType":"HTTP"}}}"#;
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        });
        let config = crate::config::Config {
            endpoint: format!("http://127.0.0.1:{port}"),
            secret: "fixture-secret".into(),
            ..Default::default()
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let items = runtime.block_on(read_current(&config, None)).unwrap();
        assert_eq!(items[0].name, "Fixture");
        server.join().unwrap();
    }
    #[test]
    fn remote_summaries_keep_missing_fields_unknown_and_never_include_urls() {
        let source = "current: local\nitems:\n- {uid: merge, type: merge}\n- {uid: local, type: local, name: Local}\n- uid: primary\n  type: remote\n  name: My subscription\n  url: https://secret.example/token?key=PRIVATE\n  updated: 1700000000\n  extra: {upload: 10, download: 80, total: 100, expire: 1800000000}\n- {uid: missing, type: remote, name: Missing}\n- {uid: zero, type: remote, extra: {upload: 0, download: 0, total: 0, expire: 0}}\n";
        let view = parse_verge(source).unwrap();
        assert!(view.local_profile);
        assert_eq!(view.items.len(), 3);
        assert!(!view.items[0].active);
        assert_eq!(view.items[0].remaining(), Some(10));
        assert_eq!(view.items[0].fraction(), Some(0.9));
        assert_eq!(view.items[0].updated, Some(1_700_000_000_000));
        assert_eq!(view.items[1].remaining(), None);
        assert_eq!(view.items[2].fraction(), None);
        assert_eq!(view.items[2].expires, None);
        assert!(!format!("{:?}", view.items).contains("PRIVATE"));
    }
    #[test]
    fn over_quota_and_invalid_values_are_not_reported_as_free_capacity() {
        let view = parse_verge("current: a\nitems:\n- {uid: a, type: remote, extra: {upload: 30, download: 80, total: 100, expire: -1}}\n- {uid: b, type: remote, extra: {upload: 0, download: invalid, total: 100}}\n").unwrap();
        assert!(view.items[0].active);
        assert_eq!(view.items[0].remaining(), Some(0));
        assert_eq!(view.items[0].fraction(), Some(1.));
        assert_eq!(view.items[0].expires, None);
        assert_eq!(view.items[1].remaining(), None);
        assert!(parse_verge("items: [{uid: a, type: remote}, {uid: a, type: remote}]").is_err());
    }
    #[test]
    fn reading_metadata_is_read_only() {
        let dir = tempfile::tempdir().unwrap();
        let source = b"current: a\nitems:\n- {uid: a, type: remote, name: Example}\n";
        std::fs::write(dir.path().join("profiles.yaml"), source).unwrap();
        let target = Target {
            kind: Kind::Verge,
            root: dir.path().into(),
            executable: None,
        };
        assert_eq!(read(&target).unwrap().items.len(), 1);
        assert_eq!(
            std::fs::read(dir.path().join("profiles.yaml")).unwrap(),
            source
        );
        std::fs::write(
            dir.path().join("profiles.yaml"),
            "items: [\nurl: https://secret.example/PRIVATE",
        )
        .unwrap();
        assert!(!read(&target).err().unwrap().to_string().contains("PRIVATE"));
    }
}
