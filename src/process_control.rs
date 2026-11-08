//! Runtime process identification is controlled independently of traffic sampling.
use crate::{
    backend::Backend,
    config::{Config, ProcessControl},
    mihomo::Client,
    model::ProcessControlStatus,
};
use anyhow::{Context, Result, bail};
use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

const MAINTAIN_INTERVAL: Duration = Duration::from_secs(60);
const REFRESH_INTERVAL: Duration = Duration::from_secs(10);

#[derive(Clone, PartialEq, Eq)]
struct Key {
    endpoint: String,
    secret: String,
    policy: ProcessControl,
    generation: u64,
    resolved_endpoint: String,
    core_identity: String,
}
impl Key {
    fn read(handle: &Backend) -> (Self, Config) {
        let config = handle.config.lock().unwrap().clone();
        let shared = handle.shared.lock().unwrap();
        (
            Self {
                endpoint: config.endpoint.clone(),
                secret: config.secret.clone(),
                policy: config.process_control,
                generation: handle.process_generation.load(Ordering::Relaxed),
                resolved_endpoint: shared.endpoint.clone(),
                core_identity: shared.core_identity.clone(),
            },
            config,
        )
    }
    fn current(&self, handle: &Backend) -> bool {
        !handle.stop.load(Ordering::Relaxed)
            && !handle.permanent_control.paused.load(Ordering::Acquire)
            && *self == Self::read(handle).0
            && handle.shared.lock().unwrap().connected
    }
}

struct Outcome {
    mode: String,
    result: Result<()>,
}

async fn read_mode(client: &mut Client) -> Result<String> {
    let bytes = client.request("/configs").await?;
    let config: serde_json::Value =
        serde_json::from_slice(&bytes).context("mihomo 运行配置格式无效")?;
    Ok(config["find-process-mode"]
        .as_str()
        .unwrap_or_default()
        .into())
}

async fn check_and_apply(
    client: &mut Client,
    policy: ProcessControl,
    mut before_patch: impl FnMut() -> bool,
) -> Outcome {
    let mut mode = String::new();
    let result = async {
        mode = read_mode(client).await?;
        if !matches!(mode.as_str(), "always" | "strict" | "off") {
            bail!("内核未返回受支持的进程识别模式");
        }
        if let Some(target) = policy.target()
            && mode != target
        {
            if !before_patch() {
                bail!("连接或识别设置已变化，已取消旧操作");
            }
            client
                .request_with_body(
                    reqwest::Method::PATCH,
                    "/configs",
                    Some(&serde_json::json!({"find-process-mode": target})),
                )
                .await?;
            // A 204 alone does not prove that an older core accepted this field.
            mode = read_mode(client).await?;
            if mode != target {
                bail!("内核未应用请求的进程识别模式，请重试或检查客户端配置");
            }
        }
        Ok(())
    }
    .await;
    Outcome { mode, result }
}

pub async fn run(handle: Backend) {
    let mut previous: Option<Key> = None;
    let mut due = Instant::now();
    while !handle.stop.load(Ordering::Relaxed) {
        let (key, config) = Key::read(&handle);
        let connected = handle.shared.lock().unwrap().connected;
        let target_changed = {
            let shared = handle.shared.lock().unwrap();
            connected
                && (config.process_endpoint.as_ref().is_some_and(|endpoint| {
                    !crate::client_config::same_control_target(endpoint, &shared.endpoint)
                }) || config
                    .process_client
                    .as_ref()
                    .is_some_and(|client| client != &shared.client_name))
        };
        if target_changed && config.process_control != ProcessControl::Unmanaged {
            let _ = handle.writer.update(|latest| {
                if latest.endpoint != key.endpoint
                    || latest.secret != key.secret
                    || latest.process_control != key.policy
                {
                    return false;
                }
                latest.process_control = ProcessControl::Unmanaged;
                latest.process_endpoint = None;
                latest.process_client = None;
                true
            });
            tokio::time::sleep(Duration::from_millis(100)).await;
            continue;
        }
        if previous.as_ref() != Some(&key) {
            previous = Some(key.clone());
            due = Instant::now();
            let mut shared = handle.shared.lock().unwrap();
            shared.process_mode.clear();
            shared.process_error.clear();
            shared.process_status = ProcessControlStatus::Checking;
        }
        if !connected {
            let mut shared = handle.shared.lock().unwrap();
            shared.process_mode.clear();
            shared.process_status = ProcessControlStatus::Waiting;
            due = Instant::now();
        } else if Instant::now() >= due && !handle.permanent_control.paused.load(Ordering::Acquire)
        {
            let mut config = config;
            if !key.resolved_endpoint.is_empty() {
                config.endpoint = key.resolved_endpoint.clone();
            }
            let operation = async {
                match Client::new(&config) {
                    Ok(mut client) => {
                        client.pin_identity(key.core_identity.clone());
                        check_and_apply(&mut client, key.policy, || {
                            if !key.current(&handle) {
                                return false;
                            }
                            handle.shared.lock().unwrap().process_status =
                                ProcessControlStatus::Applying;
                            true
                        })
                        .await
                    }
                    Err(error) => Outcome {
                        mode: String::new(),
                        result: Err(error),
                    },
                }
            };
            tokio::pin!(operation);
            let outcome = loop {
                tokio::select! {
                    result = &mut operation => break Some(result),
                    _ = handle.process_wake.notified() => {},
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {},
                }
                if !key.current(&handle) {
                    break None;
                }
            };
            if let Some(outcome) = outcome
                && key.current(&handle)
            {
                let mut result = outcome.result;
                if result.is_ok() && key.policy.once() {
                    // Conditional edit of the latest snapshot: never resurrect old
                    // intent or replace a simultaneous theme/layout/endpoint edit.
                    let acknowledged = handle.writer.update(|latest| {
                        if latest.endpoint != key.endpoint
                            || latest.secret != key.secret
                            || latest.process_control != key.policy
                            || handle.process_generation.load(Ordering::Relaxed) != key.generation
                        {
                            return false;
                        }
                        latest.process_control = ProcessControl::Unmanaged;
                        latest.process_endpoint = None;
                        latest.process_client = None;
                        true
                    });
                    match acknowledged {
                        Ok(Some(_)) => {
                            let expected = Key {
                                policy: ProcessControl::Unmanaged,
                                ..key.clone()
                            };
                            if !expected.current(&handle) {
                                continue;
                            }
                            previous = Some(expected);
                        }
                        Ok(None) => continue,
                        Err(error) => result = Err(error),
                    }
                }
                let mut shared = handle.shared.lock().unwrap();
                shared.process_mode = outcome.mode;
                match result {
                    Ok(()) => {
                        shared.process_error.clear();
                        shared.process_status = if key.policy == ProcessControl::Unmanaged {
                            ProcessControlStatus::Idle
                        } else {
                            ProcessControlStatus::Applied
                        };
                        due = Instant::now()
                            + if matches!(key.policy, ProcessControl::Maintain(_)) {
                                MAINTAIN_INTERVAL
                            } else {
                                REFRESH_INTERVAL
                            };
                    }
                    Err(error) => {
                        shared.process_status = ProcessControlStatus::Failed;
                        shared.process_error = format!("{error:#}");
                        due = Instant::now() + Duration::from_secs(30);
                    }
                }
            }
        }
        tokio::select! {
            _ = handle.process_wake.notified() => {},
            _ = tokio::time::sleep(Duration::from_millis(100)) => {},
        }
    }
}

#[cfg(test)]
#[path = "process_control_tests.rs"]
mod tests;
