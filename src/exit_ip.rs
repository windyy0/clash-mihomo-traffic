use crate::{mihomo::Client, ui::UiBackend};
use serde::Deserialize;
use std::{net::IpAddr, sync::atomic::Ordering, time::Duration};

#[derive(Clone, PartialEq, Eq)]
pub struct Source {
    endpoint: String,
    identity: String,
    pid: u32,
    generation: u64,
}

impl Source {
    pub fn current(backend: &UiBackend) -> Option<Self> {
        let shared = backend.shared.lock().unwrap();
        if !shared.connected || !shared.core_verified || shared.core_identity.is_empty() {
            return None;
        }
        Some(Self {
            endpoint: shared.endpoint.clone(),
            identity: shared.core_identity.clone(),
            pid: shared.core_process_id?,
            generation: backend.process_generation.load(Ordering::Relaxed),
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct Ports {
    #[serde(default)]
    mixed_port: u16,
    #[serde(default)]
    port: u16,
    mode: String,
}

#[derive(Default, Deserialize)]
#[serde(default)]
pub struct Network {
    pub org: String,
    pub isp: String,
    pub asn: String,
}

#[derive(Default, Deserialize)]
#[serde(default)]
pub struct Location {
    pub country: String,
    pub country_code: String,
    pub city: String,
}

#[derive(Default, Deserialize)]
#[serde(default)]
pub struct Risk {
    pub is_mobile: Option<bool>,
    pub is_datacenter: Option<bool>,
    pub is_tor: Option<bool>,
    pub risk_score: Option<u8>,
}

#[derive(Deserialize)]
pub struct Record {
    pub ip: IpAddr,
    #[serde(default)]
    pub isp: Network,
    #[serde(default)]
    pub location: Location,
    #[serde(default)]
    pub risk: Risk,
}

pub struct Detection {
    pub record: Record,
    pub kind: Option<(&'static str, &'static str)>,
    pub score: Option<(u8, &'static str)>,
    pub risk_source: Option<&'static str>,
    pub mode: String,
    pub time: chrono::DateTime<chrono::Local>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    Good,
    Normal,
    Risky,
    Unknown,
}

impl Detection {
    pub fn quality(&self) -> (Quality, Option<&'static str>) {
        if self.record.risk.is_tor == Some(true) {
            return (Quality::Risky, Some("IPQuery"));
        }
        if let Some(source) = self.risk_source {
            return (Quality::Risky, Some(source));
        }
        match self.kind {
            Some(("家宽" | "移动网络" | "商用", source)) => (Quality::Good, Some(source)),
            Some(("机房", source)) => (Quality::Normal, Some(source)),
            _ => (Quality::Unknown, None),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    V4,
    V6,
}

impl Family {
    pub const ALL: [Self; 2] = [Self::V4, Self::V6];

    pub fn index(self) -> usize {
        match self {
            Self::V4 => 0,
            Self::V6 => 1,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::V4 => "IPv4",
            Self::V6 => "IPv6",
        }
    }
}

#[derive(Default)]
pub enum Probe {
    #[default]
    Idle,
    Loading,
    Unavailable,
    Found(Box<Detection>),
}

impl Probe {
    pub fn detection(&self) -> Option<&Detection> {
        match self {
            Self::Found(data) => Some(data),
            _ => None,
        }
    }
}

struct Services<'a> {
    discovery: [&'a str; 2],
    ipquery: &'a str,
    proxycheck: &'a str,
}

const SERVICES: Services<'static> = Services {
    discovery: ["https://api-ipv4.ip.sb/ip", "https://api-ipv6.ip.sb/ip"],
    ipquery: "https://api.ipquery.io",
    proxycheck: "https://proxycheck.io/v3",
};

async fn response(client: &reqwest::Client, url: &str) -> Option<Vec<u8>> {
    let mut response = client.get(url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if body.len() + chunk.len() > 32 * 1024 {
            return None;
        }
        body.extend_from_slice(&chunk);
    }
    Some(body)
}

#[derive(Deserialize)]
struct ProxyCheck {
    status: String,
    #[serde(flatten)]
    results: std::collections::HashMap<String, serde_json::Value>,
}

fn merge_proxycheck(result: &mut Detection, body: &[u8]) -> Option<()> {
    let response: ProxyCheck = serde_json::from_slice(body).ok()?;
    if !matches!(response.status.as_str(), "ok" | "warning") {
        return None;
    }
    let record = response.results.iter().find_map(|(key, value)| {
        (key.parse::<IpAddr>().ok()? == result.record.ip).then_some(value)
    })?;
    let network = &record["network"];
    let detections = &record["detections"];
    if detections["tor"].as_bool() == Some(true)
        || detections["compromised"].as_bool() == Some(true)
    {
        result.risk_source = Some("Proxycheck");
    }
    if result.kind.is_none() {
        let kind = if detections["hosting"].as_bool() == Some(true) {
            Some("机房")
        } else {
            match network["type"].as_str() {
                Some("Hosting") => Some("机房"),
                Some("Wireless") => Some("移动网络"),
                Some("Residential") => Some("家宽"),
                Some("Business") => Some("商用"),
                _ => None,
            }
        };
        result.kind = kind.map(|kind| (kind, "Proxycheck"));
    }
    if result.score.is_none() {
        result.score = detections["risk"]
            .as_u64()
            .filter(|score| *score <= 100)
            .map(|score| (score as u8, "Proxycheck"));
    }
    Some(())
}

async fn probe(
    client: &reqwest::Client,
    services: &Services<'_>,
    family: Family,
    mode: &str,
    current: impl Fn() -> bool,
) -> Option<Detection> {
    let address = response(client, services.discovery[family.index()]).await?;
    let ip: IpAddr = std::str::from_utf8(&address).ok()?.trim().parse().ok()?;
    let ip = ip.to_canonical();
    if ip.is_ipv4() != (family == Family::V4) || !current() {
        return None;
    }
    let url = format!("{}/{ip}?format=json", services.ipquery);
    let record = response(client, &url)
        .await
        .and_then(|body| serde_json::from_slice::<Record>(&body).ok())
        .filter(|record| record.ip == ip)
        .unwrap_or(Record {
            ip,
            isp: Network::default(),
            location: Location::default(),
            risk: Risk::default(),
        });
    let kind = match (record.risk.is_datacenter, record.risk.is_mobile) {
        (Some(true), _) => Some(("机房", "IPQuery")),
        (_, Some(true)) => Some(("移动网络", "IPQuery")),
        _ => None,
    };
    let score = record
        .risk
        .risk_score
        .filter(|score| *score <= 100)
        .map(|score| (score, "IPQuery"));
    let mut result = Detection {
        record,
        kind,
        score,
        risk_source: None,
        mode: mode.into(),
        time: chrono::Local::now(),
    };
    if (result.kind.is_none() || result.score.is_none()) && current() {
        let url = format!("{}/{ip}", services.proxycheck);
        if let Some(body) = response(client, &url).await {
            merge_proxycheck(&mut result, &body);
        }
    }
    current().then_some(result)
}

pub fn detect(
    backend: &UiBackend,
    source: &Source,
    mut publish: impl FnMut(Family, Option<Detection>),
) -> Result<(), &'static str> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "无法启动检测")?
        .block_on(async {
            let mut config = backend.config.lock().unwrap().clone();
            config.endpoint.clone_from(&source.endpoint);
            let mut core = Client::new(&config).map_err(|_| "无法连接 Mihomo")?;
            core.pin_identity(source.identity.clone());
            let body = core
                .request("/configs")
                .await
                .map_err(|_| "无法读取 Mihomo 代理配置")?;
            let ports: Ports =
                serde_json::from_slice(&body).map_err(|_| "无法识别 Mihomo 代理配置")?;
            let port = [ports.mixed_port, ports.port]
                .into_iter()
                .filter(|port| *port != 0)
                .find(|port| {
                    crate::native::local_api_process("127.0.0.1", *port) == Some(source.pid)
                })
                .ok_or("未找到本机 Mihomo 的 HTTP 或混合代理端口")?;
            if Source::current(backend).as_ref() != Some(source)
                || crate::native::process_identity(source.pid).ok().as_ref()
                    != Some(&source.identity)
            {
                return Err("Mihomo 已切换，请重新检测");
            }
            let proxy = reqwest::Proxy::all(format!("http://127.0.0.1:{port}"))
                .map_err(|_| "代理端口无效")?;
            let client = reqwest::Client::builder()
                .no_proxy()
                .proxy(proxy)
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(4))
                .build()
                .map_err(|_| "无法启动检测")?;
            let current = || Source::current(backend).as_ref() == Some(source);
            let v4 = probe(&client, &SERVICES, Family::V4, &ports.mode, current);
            let v6 = probe(&client, &SERVICES, Family::V6, &ports.mode, current);
            tokio::pin!(v4, v6);
            tokio::select! {
                result = &mut v4 => {
                    publish(Family::V4, result);
                    publish(Family::V6, v6.await);
                }
                result = &mut v6 => {
                    publish(Family::V6, result);
                    publish(Family::V4, v4.await);
                }
            }
            Ok(())
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn fixture_server() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    let mut chunk = [0u8; 1024];
                    while !request.ends_with(b"\r\n\r\n") {
                        let count = stream.read(&mut chunk).await.unwrap();
                        if count == 0 || request.len() > 4096 {
                            return;
                        }
                        request.extend_from_slice(&chunk[..count]);
                    }
                    let request = std::str::from_utf8(&request).unwrap();
                    let path = request.split_ascii_whitespace().nth(1).unwrap();
                    let (status, body) = match path {
                        "/v4" => ("200 OK", "192.0.2.8\n"),
                        "/v6" => ("200 OK", "2001:db8::8"),
                        "/primary/192.0.2.8?format=json" => {
                            ("200 OK", r#"{"ip":"192.0.2.8","risk":{"risk_score":12}}"#)
                        }
                        "/primary/2001:db8::8?format=json" => ("429 Too Many Requests", ""),
                        "/backup/192.0.2.8" => (
                            "200 OK",
                            r#"{"status":"ok","192.0.2.8":{"network":{"type":"Residential"},"detections":{"risk":75}}}"#,
                        ),
                        _ => ("503 Service Unavailable", ""),
                    };
                    let response = format!(
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    stream.write_all(response.as_bytes()).await.unwrap();
                });
            }
        });
        origin
    }

    #[tokio::test]
    async fn dual_stack_keeps_partial_results_and_primary_field_precedence() {
        let origin = fixture_server().await;
        let services = Services {
            discovery: [&format!("{origin}/v4"), &format!("{origin}/v6")],
            ipquery: &format!("{origin}/primary"),
            proxycheck: &format!("{origin}/backup"),
        };
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let (v4, v6) = tokio::join!(
            probe(&client, &services, Family::V4, "rule", || true),
            probe(&client, &services, Family::V6, "rule", || true),
        );
        let v4 = v4.unwrap();
        assert_eq!(v4.kind, Some(("家宽", "Proxycheck")));
        assert_eq!(v4.score, Some((12, "IPQuery")));
        assert_eq!(v4.quality(), (Quality::Good, Some("Proxycheck")));
        let v6 = v6.unwrap();
        assert_eq!(v6.record.ip.to_string(), "2001:db8::8");
        assert!(v6.kind.is_none() && v6.score.is_none());

        for (discovery, working) in [
            ([services.discovery[0], services.discovery[0]], Family::V4),
            ([services.discovery[1], services.discovery[1]], Family::V6),
        ] {
            let services = Services {
                discovery,
                ..services
            };
            for family in Family::ALL {
                assert_eq!(
                    probe(&client, &services, family, "rule", || true)
                        .await
                        .is_some(),
                    family == working
                );
            }
        }
        let checks = AtomicUsize::new(0);
        let stale = probe(&client, &services, Family::V4, "rule", || {
            checks.fetch_add(1, Ordering::Relaxed) == 0
        })
        .await;
        assert!(stale.is_none());
    }
}
