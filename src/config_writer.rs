use crate::config::Config;
use anyhow::{Result, anyhow};
use std::{
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};

type Persist = dyn Fn(&Config, &Config) -> Result<()> + Send + Sync;
struct State {
    revision: u64,
    completed: u64,
    committed: Config,
    pending: Option<(u64, Config, Instant)>,
    error: Option<(u64, String)>,
    closing: bool,
    finished: bool,
}
struct Inner {
    state: Mutex<State>,
    changed: Condvar,
    desired: Arc<Mutex<Config>>,
}

/// A single writer coalesces pending snapshots; a stale completion never
/// replaces the latest desired configuration used by the UI and shell.
#[derive(Clone)]
pub struct ConfigWriter(Arc<Inner>);
impl ConfigWriter {
    pub fn start(
        desired: Arc<Mutex<Config>>,
        persist: impl Fn(&Config, &Config) -> Result<()> + Send + Sync + 'static,
        report_error: impl Fn(String) + Send + 'static,
    ) -> Result<Self> {
        let committed = desired.lock().unwrap().clone();
        let inner = Arc::new(Inner {
            state: Mutex::new(State {
                revision: 0,
                completed: 0,
                committed,
                pending: None,
                error: None,
                closing: false,
                finished: false,
            }),
            changed: Condvar::new(),
            desired,
        });
        let worker = inner.clone();
        let persist: Box<Persist> = Box::new(persist);
        std::thread::Builder::new()
            .name("config-writer".into())
            .spawn(move || {
                loop {
                    let mut state = worker.state.lock().unwrap();
                    let (revision, config) = loop {
                        if let Some((revision, config, deadline)) = &state.pending {
                            if state.closing || Instant::now() >= *deadline {
                                let job = (*revision, config.clone());
                                state.pending = None;
                                break job;
                            }
                            let wait = deadline.saturating_duration_since(Instant::now());
                            state = worker.changed.wait_timeout(state, wait).unwrap().0;
                        } else if state.closing {
                            state.finished = true;
                            worker.changed.notify_all();
                            return;
                        } else {
                            state = worker.changed.wait(state).unwrap();
                        }
                    };
                    let previous = state.committed.clone();
                    drop(state);
                    let result = persist(&previous, &config);
                    let mut state = worker.state.lock().unwrap();
                    state.completed = revision;
                    match result {
                        Ok(()) => {
                            state.committed = config;
                            state.error = None;
                        }
                        Err(error) => {
                            let error = error.to_string();
                            state.error = Some((revision, error.clone()));
                            if state.revision == revision {
                                *worker.desired.lock().unwrap() = state.committed.clone();
                                drop(state);
                                crate::native::request_taskbar_update(false);
                                report_error(format!(
                                    "设置保存失败，已恢复上次保存的设置：{error}"
                                ));
                                state = worker.state.lock().unwrap();
                            }
                        }
                    }
                    worker.changed.notify_all();
                }
            })?;
        Ok(Self(inner))
    }
    #[cfg(test)]
    pub fn submit(&self, config: Config) -> Result<u64> {
        Ok(self
            .update(|next| {
                *next = config;
                true
            })?
            .unwrap())
    }
    /// Edit the latest desired snapshot while holding the writer lock. Background
    /// acknowledgements must not overwrite unrelated settings or newer intent.
    pub fn update(&self, edit: impl FnOnce(&mut Config) -> bool) -> Result<Option<u64>> {
        let mut state = self.0.state.lock().unwrap();
        anyhow::ensure!(!state.closing, "配置保存线程正在退出");
        let mut config = self.0.desired.lock().unwrap().clone();
        if !edit(&mut config) {
            return Ok(None);
        }
        config.validate()?;
        state.revision += 1;
        let revision = state.revision;
        *self.0.desired.lock().unwrap() = config.clone();
        state.pending = Some((
            revision,
            config,
            Instant::now() + Duration::from_millis(300),
        ));
        self.0.changed.notify_all();
        drop(state);
        crate::native::request_taskbar_update(false);
        Ok(Some(revision))
    }
    pub fn flush(&self, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        let mut state = self.0.state.lock().unwrap();
        let revision = state.revision;
        while state.completed < revision {
            let remaining = deadline.saturating_duration_since(Instant::now());
            anyhow::ensure!(!remaining.is_zero(), "等待配置保存超时");
            state = self.0.changed.wait_timeout(state, remaining).unwrap().0;
        }
        if let Some((failed, error)) = &state.error
            && *failed >= revision
        {
            return Err(anyhow!("{error}"));
        }
        Ok(())
    }
    pub fn close(&self, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        let mut state = self.0.state.lock().unwrap();
        state.closing = true;
        self.0.changed.notify_all();
        while !state.finished {
            let remaining = deadline.saturating_duration_since(Instant::now());
            anyhow::ensure!(!remaining.is_zero(), "等待配置保存超时");
            state = self.0.changed.wait_timeout(state, remaining).unwrap().0;
        }
        if let Some((revision, error)) = &state.error
            && *revision == state.revision
        {
            return Err(anyhow!("{error}"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn conditional_background_ack_preserves_newer_settings_and_intent() {
        use crate::config::ProcessControl;
        let desired = Arc::new(Mutex::new(Config {
            process_control: ProcessControl::RestoreStrict,
            ..Default::default()
        }));
        let writer = ConfigWriter::start(desired.clone(), |_, _| Ok(()), |_| {}).unwrap();
        writer
            .update(|next| {
                next.theme = "dark".into();
                true
            })
            .unwrap();
        writer
            .update(|next| {
                if next.process_control != ProcessControl::RestoreStrict {
                    return false;
                }
                next.process_control = ProcessControl::Unmanaged;
                true
            })
            .unwrap();
        assert_eq!(desired.lock().unwrap().theme, "dark");
        writer
            .update(|next| {
                next.process_control = ProcessControl::KeepAlways;
                true
            })
            .unwrap();
        assert!(
            writer
                .update(|next| {
                    if next.process_control != ProcessControl::RestoreStrict {
                        return false;
                    }
                    next.process_control = ProcessControl::Unmanaged;
                    true
                })
                .unwrap()
                .is_none()
        );
        assert_eq!(
            desired.lock().unwrap().process_control,
            ProcessControl::KeepAlways
        );
        writer.close(Duration::from_secs(2)).unwrap();
    }
    #[test]
    fn coalesces_and_flushes_on_exit() {
        let desired = Arc::new(Mutex::new(Config::default()));
        let saved = Arc::new(Mutex::new(Vec::new()));
        let output = saved.clone();
        let writer = ConfigWriter::start(
            desired.clone(),
            move |_, next| {
                output.lock().unwrap().push(next.taskbar_offset);
                Ok(())
            },
            |_| {},
        )
        .unwrap();
        for offset in 1..=50 {
            let config = Config {
                taskbar_offset: offset,
                ..Config::default()
            };
            writer.submit(config).unwrap();
        }
        writer.close(Duration::from_secs(2)).unwrap();
        assert_eq!(*saved.lock().unwrap(), vec![50]);
        assert_eq!(desired.lock().unwrap().taskbar_offset, 50);
    }
    #[test]
    fn failed_save_restores_last_successful_snapshot() {
        let original = Config::default();
        let desired = Arc::new(Mutex::new(original.clone()));
        let writer =
            ConfigWriter::start(desired.clone(), |_, _| Err(anyhow!("disk failure")), |_| {})
                .unwrap();
        writer
            .submit(Config {
                taskbar_offset: 42,
                ..original.clone()
            })
            .unwrap();
        assert!(writer.close(Duration::from_secs(2)).is_err());
        assert_eq!(*desired.lock().unwrap(), original);
    }
    #[test]
    fn failure_after_success_restores_committed_revision() {
        let desired = Arc::new(Mutex::new(Config::default()));
        let (saved_tx, saved_rx) = std::sync::mpsc::channel();
        let writer = ConfigWriter::start(
            desired.clone(),
            move |_, next| {
                if next.taskbar_offset == 12 {
                    anyhow::bail!("disk failure");
                }
                saved_tx.send(()).unwrap();
                Ok(())
            },
            |_| {},
        )
        .unwrap();
        writer
            .submit(Config {
                taskbar_offset: 11,
                ..Config::default()
            })
            .unwrap();
        saved_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        writer
            .submit(Config {
                taskbar_offset: 12,
                ..Config::default()
            })
            .unwrap();
        assert!(writer.close(Duration::from_secs(2)).is_err());
        assert_eq!(desired.lock().unwrap().taskbar_offset, 11);
    }
    #[test]
    fn stale_completion_cannot_replace_newer_request() {
        let desired = Arc::new(Mutex::new(Config::default()));
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let worker_gate = gate.clone();
        let writer = ConfigWriter::start(
            desired.clone(),
            move |_, config| {
                if config.taskbar_offset == 1 {
                    started_tx.send(()).unwrap();
                    let (lock, cv) = &*worker_gate;
                    let mut ready = lock.lock().unwrap();
                    while !*ready {
                        ready = cv.wait(ready).unwrap();
                    }
                    anyhow::bail!("older write failed");
                }
                Ok(())
            },
            |_| {},
        )
        .unwrap();
        writer
            .submit(Config {
                taskbar_offset: 1,
                ..Config::default()
            })
            .unwrap();
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        writer
            .submit(Config {
                taskbar_offset: 2,
                ..Config::default()
            })
            .unwrap();
        *gate.0.lock().unwrap() = true;
        gate.1.notify_all();
        writer.close(Duration::from_secs(2)).unwrap();
        assert_eq!(desired.lock().unwrap().taskbar_offset, 2);
    }
}
