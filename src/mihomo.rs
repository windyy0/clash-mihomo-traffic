use crate::{config::Config, model::Snapshot};
use anyhow::{Context, Result};
use http_body_util::{BodyExt, Full};
use std::{
    net::{Ipv4Addr, Ipv6Addr, SocketAddr},
    time::Duration,
};
use tokio::net::windows::named_pipe::ClientOptions;

pub(crate) struct Client {
    pub(crate) endpoint: String,
    authorization: Option<reqwest::header::HeaderValue>,
    http: reqwest::Client,
    pub(crate) identity: String,
    pub(crate) verified: bool,
    pub(crate) process_id: Option<u32>,
    pub(crate) client_name: String,
    expected_identity: Option<String>,
}
impl Client {
    pub(crate) fn new(config: &Config) -> Result<Self> {
        let endpoint = if config.endpoint == "auto" {
            discover()?
        } else {
            config.endpoint.trim_end_matches('/').into()
        };
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .resolve_to_addrs(
                "localhost",
                &[
                    SocketAddr::from((Ipv6Addr::LOCALHOST, 0)),
                    SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
                ],
            )
            .build()?;
        let authorization = if config.secret.is_empty() {
            None
        } else {
            let mut value =
                reqwest::header::HeaderValue::from_str(&format!("Bearer {}", config.secret))
                    .context("Mihomo Secret 含有无效字符")?;
            value.set_sensitive(true);
            Some(value)
        };
        Ok(Self {
            endpoint,
            authorization,
            http,
            identity: uuid::Uuid::new_v4().to_string(),
            verified: false,
            process_id: None,
            client_name: String::new(),
            expected_identity: None,
        })
    }
    fn identify_client(&mut self, pid: u32) {
        if self.process_id != Some(pid) {
            self.process_id = Some(pid);
            self.client_name = crate::native::proxy_client_name(pid).unwrap_or_default();
        }
        match crate::native::process_identity(pid) {
            Ok(identity) => {
                self.identity = identity;
                self.verified = true;
            }
            Err(_) => self.clear_identity(),
        }
    }

    fn clear_identity(&mut self) {
        if self.verified {
            self.identity = uuid::Uuid::new_v4().to_string();
            self.verified = false;
        }
        self.process_id = None;
        self.client_name.clear();
    }

    pub(crate) fn pin_identity(&mut self, identity: String) {
        self.expected_identity = (!identity.is_empty()).then_some(identity);
    }
    fn check_identity(&self) -> Result<()> {
        anyhow::ensure!(
            self.expected_identity
                .as_ref()
                .is_none_or(|expected| self.verified && expected == &self.identity),
            "内核进程已经变化，已取消旧操作"
        );
        Ok(())
    }
    pub(crate) async fn request(&mut self, path: &str) -> Result<Vec<u8>> {
        self.request_with_body(reqwest::Method::GET, path, None)
            .await
    }
    pub(crate) async fn request_with_body(
        &mut self,
        method: reqwest::Method,
        path: &str,
        json: Option<&serde_json::Value>,
    ) -> Result<Vec<u8>> {
        tokio::time::timeout(Duration::from_secs(3), self.exchange(method, path, json))
            .await
            .context("mihomo 接口请求超时")?
    }
    async fn exchange(
        &mut self,
        method: reqwest::Method,
        path: &str,
        json: Option<&serde_json::Value>,
    ) -> Result<Vec<u8>> {
        const MAX: usize = 16 * 1024 * 1024;
        if self.endpoint.starts_with(r"\\.\pipe\") {
            let pipe = ClientOptions::new()
                .open(&self.endpoint)
                .context("Mihomo 命名管道不可用")?;
            use std::os::windows::io::AsRawHandle;
            match crate::native::pipe_server_process(pipe.as_raw_handle()) {
                Ok(pid) => self.identify_client(pid),
                Err(_) => self.clear_identity(),
            }
            self.check_identity()?;
            let (mut sender, connection) =
                hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(pipe)).await?;
            let _task = AbortOnDrop(tokio::spawn(async move {
                let _ = connection.await;
            }));
            async {
                let mut request = hyper::Request::builder()
                    .method(method)
                    .uri(path)
                    .header("Host", "localhost");
                let payload = if let Some(json) = json {
                    request = request.header("Content-Type", "application/json");
                    serde_json::to_vec(json)?
                } else {
                    Vec::new()
                };
                if let Some(value) = &self.authorization {
                    request = request.header(reqwest::header::AUTHORIZATION, value.clone());
                }
                let response = sender
                    .send_request(request.body(Full::new(hyper::body::Bytes::from(payload)))?)
                    .await?;
                ensure_success(response.status())?;
                let mut body = response.into_body();
                let mut bytes = vec![];
                while let Some(frame) = body.frame().await {
                    if let Ok(data) = frame?.into_data() {
                        anyhow::ensure!(bytes.len() + data.len() <= MAX, "连接列表超过 16 MB 限制");
                        bytes.extend_from_slice(&data);
                    }
                }
                Ok(bytes)
            }
            .await
        } else {
            let url = reqwest::Url::parse(&self.endpoint)?;
            if let Some(pid) = url
                .host_str()
                .zip(url.port_or_known_default())
                .and_then(|(host, port)| crate::native::local_api_process(host, port))
            {
                self.identify_client(pid);
            } else {
                self.clear_identity();
            }
            self.check_identity()?;
            let mut request = self
                .http
                .request(method, format!("{}{}", self.endpoint, path));
            if let Some(json) = json {
                request = request.json(json);
            }
            if let Some(value) = &self.authorization {
                request = request.header(reqwest::header::AUTHORIZATION, value.clone());
            }
            let mut response = request.send().await.map_err(reqwest::Error::without_url)?;
            ensure_success(response.status())?;
            let mut bytes = vec![];
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(reqwest::Error::without_url)?
            {
                anyhow::ensure!(bytes.len() + chunk.len() <= MAX, "连接列表超过 16 MB 限制");
                bytes.extend_from_slice(&chunk);
            }
            Ok(bytes)
        }
    }
    pub(crate) async fn snapshot(&mut self) -> Result<Snapshot> {
        let data = self.request("/connections").await?;
        let snap: Snapshot = serde_json::from_slice(&data)
            .map_err(|_| anyhow::anyhow!("Mihomo 连接接口格式不兼容"))?;
        anyhow::ensure!(
            snap.connections.as_ref().map_or(0, Vec::len) <= 20_000,
            "活跃连接超过 20,000，已暂停采样以限制资源占用"
        );
        Ok(snap)
    }
}
// Cancelling a timed-out or obsolete pipe request also closes its HTTP driver.
struct AbortOnDrop(tokio::task::JoinHandle<()>);
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}
fn ensure_success(status: reqwest::StatusCode) -> Result<()> {
    anyhow::ensure!(status.is_success(), "Mihomo 返回 HTTP {}", status);
    Ok(())
}

fn discover() -> Result<String> {
    std::fs::read_dir(r"\\.\pipe\")
        .context("无法枚举 Windows 命名管道")?
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|s| s.starts_with("verge-mihomo-") || s == "mihomo" || s == "clash")
        .min_by(|a, b| {
            let priority = |name: &str| !name.starts_with("verge-mihomo-production");
            (priority(a), a).cmp(&(priority(b), b))
        })
        .map(|s| format!(r"\\.\pipe\{s}"))
        .context("没有发现 Mihomo。请先启动代理客户端，或在设置中填写本机控制地址。")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    #[test]
    fn snapshot_requires_counters_but_accepts_missing_connections() {
        let snapshot: Snapshot =
            serde_json::from_str(r#"{"uploadTotal":1,"downloadTotal":2}"#).unwrap();
        assert_eq!((snapshot.upload_total, snapshot.download_total), (1, 2));
        assert!(snapshot.connections.is_none());
        assert!(serde_json::from_str::<Snapshot>(r#"{"downloadTotal":2}"#).is_err());
        assert!(
            serde_json::from_str::<Snapshot>(
                r#"{"uploadTotal":1,"downloadTotal":2,"connections":{}}"#
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn controller_refuses_redirects() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = Config {
            endpoint: format!("http://{}", listener.local_addr().unwrap()),
            secret: "fixture-secret".into(),
            ..Config::default()
        };
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(socket.read_u8().await.unwrap());
            }
            assert!(
                String::from_utf8(request)
                    .unwrap()
                    .to_ascii_lowercase()
                    .contains("authorization: bearer fixture-secret")
            );
            socket
                .write_all(b"HTTP/1.1 307 Temporary Redirect\r\nLocation: /other\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
            drop(socket);
            assert!(
                tokio::time::timeout(Duration::from_millis(150), listener.accept())
                    .await
                    .is_err()
            );
        });
        let error = Client::new(&config)
            .unwrap()
            .request("/version")
            .await
            .unwrap_err();
        assert!(error.to_string().contains("307"));
        server.await.unwrap();
    }
}
