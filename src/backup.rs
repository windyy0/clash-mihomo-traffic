use crate::config::Config;
use anyhow::{Context, Result, ensure};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub fn validate_directory(path: &Path) -> Result<()> {
    ensure!(
        path.is_absolute() && path.is_dir(),
        "备份目录不存在或无法访问"
    );
    let probe = path.join(format!(".mihomo-backup-test-{}.tmp", uuid::Uuid::new_v4()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .context("备份目录不可写")?;
    let written = file
        .write_all(b"Mihomo Traffic backup directory test")
        .and_then(|_| file.sync_all());
    drop(file);
    let cleanup = fs::remove_file(&probe);
    written.context("备份目录写入失败")?;
    cleanup.context("无法清理备份目录测试文件")?;
    Ok(())
}

pub fn directory_choice(path: &Path, default: Option<&Path>) -> Result<Option<String>> {
    validate_directory(path)?;
    Ok(
        (!default.is_some_and(|default| crate::data_location::same_directory(path, default)))
            .then(|| path.display().to_string()),
    )
}

pub fn ensure_directory(config: &Config) -> Result<PathBuf> {
    let directory = config.backup_directory()?;
    fs::create_dir_all(&directory)
        .with_context(|| format!("无法使用备份目录：{}", directory.display()))?;
    Ok(directory)
}

pub fn destination(config: &Config, prefix: &str) -> Result<PathBuf> {
    let directory = ensure_directory(config)?;
    // Include a random suffix so repeated clicks never replace another backup.
    Ok(directory.join(format!(
        "{prefix}-{}-{}.sqlite3",
        chrono::Local::now().format("%Y%m%d-%H%M%S"),
        uuid::Uuid::new_v4().simple()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        backend::{Backend, Command},
        storage::Storage,
    };
    use std::{
        sync::atomic::Ordering,
        time::{Duration, Instant},
    };

    #[test]
    fn legacy_uses_document_default_custom_roundtrips_and_old_backups_stay() {
        let root = tempfile::tempdir().unwrap();
        let old = root.path().join("backups");
        fs::create_dir(&old).unwrap();
        fs::write(old.join("original.sqlite3"), b"keep original").unwrap();
        fs::write(root.path().join("config.toml"), "retention_days = 30\n").unwrap();
        let mut config = Config::load(root.path()).unwrap();
        assert_eq!(config.backup_dir, None);
        assert_eq!(
            config.backup_directory().unwrap(),
            crate::config::default_backup_directory().unwrap()
        );
        assert_eq!(config.retention_days, 30);
        let custom = root.path().join("独立备份 data");
        config.backup_dir = Some(custom.display().to_string());
        config.save(root.path()).unwrap();
        assert_eq!(
            Config::load(root.path())
                .unwrap()
                .backup_directory()
                .unwrap(),
            custom
        );
        config.backup_dir = Some("relative/backups".into());
        assert!(config.validate().is_err());
        assert!(config.backup_directory().is_err());
        config.backup_dir = None;
        config.save(root.path()).unwrap();
        let reset = Config::load(root.path()).unwrap();
        assert!(reset.backup_dir.is_none());
        assert_eq!(
            reset.backup_directory().unwrap(),
            crate::config::default_backup_directory().unwrap()
        );
        assert_eq!(
            fs::read(old.join("original.sqlite3")).unwrap(),
            b"keep original"
        );
    }

    #[test]
    fn selection_normalizes_default_and_custom_works_without_documents() {
        let root = tempfile::tempdir().unwrap();
        let default = root
            .path()
            .join("文档重定向 data")
            .join("MihomoTraffic")
            .join("backups");
        fs::create_dir_all(&default).unwrap();
        assert_eq!(directory_choice(&default, Some(&default)).unwrap(), None);
        assert_eq!(
            directory_choice(&default.join("."), Some(&default)).unwrap(),
            None
        );
        let custom = root.path().join("独立备份 backups");
        fs::create_dir(&custom).unwrap();
        let expected = Some(custom.display().to_string());
        assert_eq!(directory_choice(&custom, Some(&default)).unwrap(), expected);
        assert_eq!(directory_choice(&custom, None).unwrap(), expected);
        assert_eq!(fs::read_dir(&default).unwrap().count(), 0);
        assert_eq!(fs::read_dir(&custom).unwrap().count(), 0);
    }

    #[test]
    fn directory_validation_and_unique_names_do_not_move_existing_files() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let old = data.join("backups");
        let custom = root.path().join("新目录 backups");
        fs::create_dir_all(&old).unwrap();
        fs::create_dir(&custom).unwrap();
        fs::write(old.join("original.sqlite3"), b"keep").unwrap();
        validate_directory(&custom).unwrap();
        assert_eq!(fs::read_dir(&custom).unwrap().count(), 0);
        let config = Config {
            backup_dir: Some(custom.display().to_string()),
            ..Default::default()
        };
        let first = destination(&config, "traffic").unwrap();
        let second = destination(&config, "traffic").unwrap();
        assert_eq!(first.parent(), Some(custom.as_path()));
        assert_ne!(first, second);
        assert_eq!(fs::read(old.join("original.sqlite3")).unwrap(), b"keep");
        let blocked = root.path().join("not-a-folder");
        fs::write(&blocked, b"blocked").unwrap();
        assert!(validate_directory(&blocked).is_err());
        let unavailable = Config {
            backup_dir: Some(blocked.display().to_string()),
            ..Default::default()
        };
        assert!(destination(&unavailable, "traffic").is_err());
        assert_eq!(fs::read_dir(old).unwrap().count(), 1);
        let not_yet_created = root
            .path()
            .join("文档 redirected")
            .join("MihomoTraffic")
            .join("backups");
        let unopened = Config {
            backup_dir: Some(not_yet_created.display().to_string()),
            ..Default::default()
        };
        assert!(!not_yet_created.exists());
        assert_eq!(ensure_directory(&unopened).unwrap(), not_yet_created);
        assert!(not_yet_created.is_dir());
        assert_eq!(fs::read_dir(&not_yet_created).unwrap().count(), 0);
    }

    fn wait_for(mut ready: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(6);
        while !ready() {
            assert!(Instant::now() < deadline, "backup operation timed out");
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn total(path: &Path) -> u64 {
        Storage::open(path)
            .unwrap()
            .total(i64::MIN, i64::MAX)
            .unwrap()
            .total()
    }

    struct Running(Backend);
    impl Drop for Running {
        fn drop(&mut self) {
            let _ = self.0.request_shutdown();
            let deadline = Instant::now() + Duration::from_secs(6);
            while !self.0.done.load(Ordering::Acquire) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(25));
            }
        }
    }

    #[test]
    fn collector_routes_both_backups_and_aborts_restore_when_directory_unavailable() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let custom = root.path().join("外部备份 data");
        fs::create_dir(&data).unwrap();
        Config {
            endpoint: "http://127.0.0.1:9".into(),
            retention_days_enabled: false,
            backup_dir: Some(custom.display().to_string()),
            ..Default::default()
        }
        .save(&data)
        .unwrap();
        let database = data.join("traffic.sqlite3");
        Storage::open(&database)
            .unwrap()
            .db
            .execute("INSERT INTO totals VALUES(60000,100,0)", [])
            .unwrap();
        let source = root.path().join("restore.sqlite3");
        Storage::open(&source)
            .unwrap()
            .db
            .execute("INSERT INTO totals VALUES(60000,500,0)", [])
            .unwrap();
        let running = Running(Backend::start(data).unwrap());
        let handle = &running.0;
        handle.send(Command::Backup);
        wait_for(|| {
            handle
                .shared
                .lock()
                .unwrap()
                .notice
                .starts_with("备份已保存")
        });
        let saved = fs::read_dir(&custom)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("traffic-")
                    && p.extension().is_some_and(|s| s == "sqlite3")
            })
            .unwrap();
        assert_eq!(total(&saved), 100);
        handle.shared.lock().unwrap().notice.clear();
        handle.send(Command::Restore(source));
        wait_for(|| {
            handle
                .shared
                .lock()
                .unwrap()
                .notice
                .starts_with("备份已恢复")
        });
        assert_eq!(total(&database), 500);
        let rollback = fs::read_dir(&custom)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("before-restore-")
                    && p.extension().is_some_and(|s| s == "sqlite3")
            })
            .unwrap();
        assert_eq!(total(&rollback), 100);
        let blocked = root.path().join("blocked");
        fs::write(&blocked, b"not a directory").unwrap();
        handle
            .writer
            .update(|c| {
                c.backup_dir = Some(blocked.display().to_string());
                true
            })
            .unwrap();
        handle.writer.flush(Duration::from_secs(5)).unwrap();
        handle.shared.lock().unwrap().notice.clear();
        handle.send(Command::Restore(saved));
        wait_for(|| handle.shared.lock().unwrap().notice.starts_with("操作失败"));
        assert!(handle.shared.lock().unwrap().notice.contains("备份目录"));
        assert_eq!(total(&database), 500);
    }
}
