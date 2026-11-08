use super::Backend;
use crate::{
    config::{Config, ProcessControl},
    config_writer::ConfigWriter,
    model::Shared,
    speed_stats::SecondsPolicy,
    storage_management::Policy,
};
use anyhow::Result;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

pub(super) const SAVE_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) fn start_writer(
    save_dir: PathBuf,
    shared: Arc<Mutex<Shared>>,
    config: Arc<Mutex<Config>>,
    saved_seconds: Arc<Mutex<SecondsPolicy>>,
    saved_storage: Arc<Mutex<Policy>>,
) -> Result<ConfigWriter> {
    ConfigWriter::start(
        config,
        move |old, next| {
            if old.autorun != next.autorun {
                crate::native::autorun(next.autorun, &save_dir)?;
            }
            if let Err(error) = next.save(&save_dir) {
                if old.autorun != next.autorun {
                    let _ = crate::native::autorun(old.autorun, &save_dir);
                }
                return Err(error);
            }
            *saved_seconds.lock().unwrap() = crate::speed_stats::SecondsPolicy::from(next);
            *saved_storage.lock().unwrap() = crate::storage_management::Policy::from(next);
            Ok(())
        },
        move |error| shared.lock().unwrap().notice = error,
    )
}

impl Backend {
    pub fn config_snapshot(&self) -> Config {
        self.config.lock().unwrap().clone()
    }

    pub fn save_ui_config(&self, mut next: Config) -> Result<()> {
        self.writer.update(|latest| {
            // Backup folder validation and process control can update settings
            // while the UI still holds an older snapshot.
            next.backup_dir = latest.backup_dir.clone();
            if latest.endpoint == next.endpoint {
                next.process_control = latest.process_control;
                next.client_config_dir = latest.client_config_dir.clone();
                next.process_endpoint = latest.process_endpoint.clone();
                next.process_client = latest.process_client.clone();
            } else {
                next.process_control = ProcessControl::Unmanaged;
                next.client_config_dir = None;
                next.process_endpoint = None;
                next.process_client = None;
            }
            *latest = next;
            true
        })?;
        Ok(())
    }

    pub fn set_backup_directory(&self, selected: Option<PathBuf>) -> Result<()> {
        let default = crate::config::default_backup_directory();
        let choice = match selected {
            Some(path) => crate::backup::directory_choice(&path, default.as_deref().ok())?,
            None => {
                default?;
                None
            }
        };
        self.writer.update(|config| {
            if config.backup_dir == choice {
                return false;
            }
            config.backup_dir = choice;
            true
        })?;
        self.writer.flush(SAVE_TIMEOUT)
    }

    pub fn prepare_migration(&self, target: &Path) -> Result<()> {
        self.writer.flush(SAVE_TIMEOUT)?;
        std::fs::create_dir_all(target)?;
        crate::data_location::prepare(&self.dir, target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn shutdown_stops_collector_even_when_settings_cannot_be_saved() {
        let dir = tempfile::tempdir().unwrap();
        let original = Config {
            endpoint: "http://127.0.0.1:9".into(),
            ..Config::default()
        };
        original.save(dir.path()).unwrap();
        let backend = Backend::start(dir.path().into()).unwrap();
        std::fs::create_dir(dir.path().join("config.toml.tmp")).unwrap();
        backend
            .save_ui_config(Config {
                theme: "dark".into(),
                ..original.clone()
            })
            .unwrap();

        assert!(backend.shutdown().is_err());
        assert!(backend.done.load(Ordering::Acquire));
        assert_eq!(backend.config_snapshot().theme, original.theme);
        assert_eq!(Config::load(dir.path()).unwrap().theme, original.theme);
    }
}
