//! Directory relocation runs in a separate, hidden coordinator. It waits for
//! the old process to exit before opening either ledger, including on Windows.
use crate::{config::Config, storage::Storage};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command},
    time::{Duration, Instant},
};

const MANAGED: &[&str] = &[
    "config.toml",
    crate::release_update::CACHE_FILE,
    "traffic.sqlite3",
    "traffic.sqlite3-wal",
    "traffic.sqlite3-shm",
    "exports",
    "backups",
    "traffic.log",
    "traffic.previous.log",
    "client-config-backups",
];

#[derive(Serialize, Deserialize)]
struct Location {
    directory: PathBuf,
}

pub fn resolve(default: &Path) -> Result<PathBuf> {
    let path = default.join("data-location.toml");
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(default.to_path_buf());
        }
        Err(error) => return Err(error).context("无法读取数据目录定位文件"),
    };
    let location: Location = toml::from_str(&contents).context("数据目录定位文件格式错误")?;
    ensure!(
        location.directory.is_absolute() && location.directory.is_dir(),
        "已保存的数据目录不存在：{}",
        location.directory.display()
    );
    ensure!(
        location.directory.join("traffic.sqlite3").is_file(),
        "已保存的数据目录缺少数据库：{}",
        location.directory.display()
    );
    Ok(location.directory)
}

fn set_location(default: &Path, target: &Path) -> Result<()> {
    fs::create_dir_all(default)?;
    let path = default.join("data-location.toml");
    if same_directory(default, target) {
        if path.exists() {
            fs::remove_file(path)?;
        }
    } else {
        let temporary = default.join("data-location.toml.tmp");
        fs::write(
            &temporary,
            toml::to_string(&Location {
                directory: target.to_path_buf(),
            })?,
        )?;
        fs::OpenOptions::new()
            .write(true)
            .open(&temporary)?
            .sync_all()?;
        crate::native::replace_file(&temporary, &path)?;
    }
    Ok(())
}

pub fn same_directory(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => path_key(&a) == path_key(&b),
        _ => a == b,
    }
}

fn path_key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

fn user_path(path: PathBuf) -> PathBuf {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::path::{Component, Prefix};
    let wide: Vec<_> = path.as_os_str().encode_wide().collect();
    match path.components().next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::VerbatimDisk(_) => PathBuf::from(std::ffi::OsString::from_wide(&wide[4..])),
            Prefix::VerbatimUNC(_, _) => {
                let mut unc = vec![92, 92];
                unc.extend_from_slice(&wide[8..]);
                PathBuf::from(std::ffi::OsString::from_wide(&unc))
            }
            _ => path,
        },
        _ => path,
    }
}

pub fn validate(source: &Path, target: &Path) -> Result<(PathBuf, PathBuf)> {
    ensure!(
        source.is_absolute() && target.is_absolute(),
        "数据目录须为绝对路径"
    );
    let source = fs::canonicalize(source).context("原数据目录不可访问")?;
    let target = fs::canonicalize(target).context("目标数据目录不可访问")?;
    let source_key = PathBuf::from(path_key(&source));
    let target_key = PathBuf::from(path_key(&target));
    ensure!(
        !source_key.starts_with(&target_key) && !target_key.starts_with(&source_key),
        "请选择不同的目录，原目录与目标目录不能互为父子目录"
    );
    ensure!(target.is_dir(), "目标不是文件夹");
    Ok((user_path(source), user_path(target)))
}

fn lock_directory(dir: &Path) -> Result<fs::File> {
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join("collector.lock"))?;
    file.try_lock().context("数据目录正被另一实例使用")?;
    Ok(file)
}

fn check_tree(path: &Path) -> Result<()> {
    use std::os::windows::fs::MetadataExt;
    let meta = fs::symlink_metadata(path)?;
    ensure!(
        meta.file_attributes() & 0x400 == 0,
        "数据文件不能包含链接或联接：{}",
        path.display()
    );
    if meta.is_dir() {
        for entry in fs::read_dir(path)? {
            check_tree(&entry?.path())?;
        }
    } else {
        ensure!(meta.is_file(), "不支持的数据文件类型");
    }
    Ok(())
}

fn copy_tree(source: &Path, target: &Path) -> Result<()> {
    #[cfg(test)]
    if COPY_FAILURE.with(|fail| fail.replace(false)) {
        anyhow::bail!("simulated copy failure");
    }
    check_tree(source)?;
    if source.is_dir() {
        fs::create_dir_all(target)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_tree(&entry.path(), &target.join(entry.file_name()))?;
        }
    } else {
        fs::copy(source, target).with_context(|| format!("无法复制 {}", source.display()))?;
        fs::OpenOptions::new()
            .write(true)
            .open(target)?
            .sync_all()?;
    }
    Ok(())
}

fn remove_owned(root: &Path, path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let resolved = user_path(fs::canonicalize(path)?);
    ensure!(
        resolved.starts_with(root) && resolved != root,
        "拒绝删除数据目录之外的文件"
    );
    check_tree(path)?;
    if path.is_dir() {
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(())
}

/// `activate` must either start the target successfully, or stop its child and
/// return an error. Only this coordinator owns the destination during install.
fn migrate(
    source: &Path,
    target: &Path,
    default: &Path,
    activate: impl FnOnce(&Path, &Config) -> Result<()>,
) -> Result<()> {
    let (source, target) = validate(source, target)?;
    let _source_lock = lock_directory(&source)?;
    let target_lock = lock_directory(&target)?;
    let id = uuid::Uuid::new_v4().to_string();
    let stage = target.join(format!(".migration-{id}"));
    let rollback = target.join(format!("before-migration-{id}"));
    let archived_backup = target
        .join("backups")
        .join(format!("before-migration-{id}"));
    let locator_path = default.join("data-location.toml");
    let old_locator = if locator_path.exists() {
        Some(fs::read(&locator_path)?)
    } else {
        None
    };
    let config = Config::load(&source)?;
    for name in MANAGED {
        for root in [&source, &target] {
            let path = root.join(name);
            if path.exists() {
                check_tree(&path)?;
            }
        }
    }
    fs::create_dir(&stage)?;
    let staging = (|| -> Result<()> {
        ensure!(
            source.join("traffic.sqlite3").is_file(),
            "原数据目录缺少流量数据库"
        );
        Storage::open(&source.join("traffic.sqlite3"))?.backup(&stage.join("traffic.sqlite3"))?;
        let db = rusqlite::Connection::open(stage.join("traffic.sqlite3"))?;
        let check: String = db.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        ensure!(check == "ok", "迁移数据库完整性校验失败");
        drop(db);
        for name in MANAGED
            .iter()
            .filter(|name| !name.starts_with("traffic.sqlite3"))
        {
            let path = source.join(name);
            if path.exists() {
                copy_tree(&path, &stage.join(name))?;
            }
        }
        config.save(&stage)?;
        Ok(())
    })();
    if let Err(error) = staging {
        let _ = remove_owned(&target, &stage);
        return Err(error);
    }
    fs::create_dir(&rollback)?;
    let mut saved = Vec::new();
    let mut installed = Vec::new();
    let mut archived = false;
    let result = (|| -> Result<()> {
        for name in MANAGED {
            if target.join(name).exists() {
                fs::rename(target.join(name), rollback.join(name))
                    .with_context(|| format!("无法备份目标中的 {name}"))?;
                saved.push(*name);
            }
        }
        for name in MANAGED {
            if stage.join(name).exists() {
                fs::rename(stage.join(name), target.join(name))
                    .with_context(|| format!("无法迁入 {name}"))?;
                installed.push(*name);
            }
        }
        // Keep destination snapshots with ordinary backups so subsequent
        // directory moves carry the entire backup history along as well.
        if !saved.is_empty() {
            if !target.join("backups").exists() {
                fs::create_dir(target.join("backups"))?;
                installed.push("backups");
            }
            fs::rename(&rollback, &archived_backup)?;
            archived = true;
        }
        set_location(default, &target)?;
        if config.autorun {
            crate::native::autorun(true, &target)?;
        }
        drop(target_lock);
        activate(&target, &config)?;
        Ok(())
    })();
    if let Err(error) = result {
        // The failed child has exited; reacquire before rolling anything back.
        let _rollback_lock =
            lock_directory(&target).context("迁移失败，目标仍被占用；原目录和目标备份已保留")?;
        if archived {
            fs::rename(&archived_backup, &rollback)?;
        }
        for name in installed.iter().rev() {
            remove_owned(&target, &target.join(name))?;
        }
        // A child may have created fresh WAL sidecars while verifying startup.
        if installed.contains(&"traffic.sqlite3") {
            for name in ["traffic.sqlite3-wal", "traffic.sqlite3-shm"] {
                remove_owned(&target, &target.join(name))?;
            }
        }
        for name in saved {
            fs::rename(rollback.join(name), target.join(name))?;
        }
        match old_locator {
            Some(bytes) => {
                let temporary = default.join("data-location.toml.tmp");
                fs::write(&temporary, bytes)?;
                crate::native::replace_file(&temporary, &locator_path)?;
            }
            None if locator_path.exists() => fs::remove_file(&locator_path)?,
            None => {}
        }
        if config.autorun {
            crate::native::autorun(true, &source)?;
        }
        let _ = remove_owned(&target, &stage);
        let _ = remove_owned(&target, &rollback);
        return Err(error);
    }
    let _ = remove_owned(&target, &stage);
    if saved.is_empty() {
        let _ = remove_owned(&target, &rollback);
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct Request {
    source: PathBuf,
    target: PathBuf,
    default: PathBuf,
    parent: u32,
}

fn spawn_hidden(args: &[&std::ffi::OsStr]) -> Result<Child> {
    use std::os::windows::process::CommandExt;
    Command::new(std::env::current_exe()?)
        .args(args)
        .creation_flags(0x08000000)
        .spawn()
        .context("无法启动迁移协调进程")
}

struct StartupChild {
    child: Child,
    ready: bool,
}
impl Drop for StartupChild {
    fn drop(&mut self) {
        if !self.ready {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

pub fn prepare(source: &Path, target: &Path) -> Result<()> {
    let (source, target) = validate(source, target)?;
    let _lock = lock_directory(&target)?;
    let probe = target.join(format!(".write-test-{}", uuid::Uuid::new_v4()));
    fs::write(&probe, b"").context("目标目录不可写")?;
    fs::remove_file(&probe)?;
    drop(_lock);
    let request = Request {
        source: source.clone(),
        target,
        default: crate::config::default_data_dir(),
        parent: std::process::id(),
    };
    let path = source.join(format!(".migration-request-{}.json", uuid::Uuid::new_v4()));
    fs::write(&path, serde_json::to_vec(&request)?)?;
    let child = spawn_hidden(&[std::ffi::OsStr::new("--relocate-data"), path.as_os_str()])?;
    let mut child = StartupChild {
        child,
        ready: false,
    };
    let ready = path.with_extension("ready");
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if ready.exists() {
            let _ = fs::remove_file(ready);
            child.ready = true;
            return Ok(());
        }
        ensure!(child.child.try_wait()?.is_none(), "迁移协调进程启动失败");
        std::thread::sleep(Duration::from_millis(30));
    }
    let _ = fs::remove_file(path);
    anyhow::bail!("等待迁移协调进程超时")
}

fn launch(target: &Path, notice: Option<&Path>, ready: Option<&Path>) -> Result<Child> {
    let mut args = vec![std::ffi::OsStr::new("--data-dir"), target.as_os_str()];
    if let Some(path) = notice {
        args.extend([std::ffi::OsStr::new("--migration-notice"), path.as_os_str()]);
    }
    if let Some(path) = ready {
        args.extend([std::ffi::OsStr::new("--migration-ready"), path.as_os_str()]);
    }
    spawn_hidden(&args)
}

pub fn coordinate(path: &Path) -> Result<()> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, WAIT_OBJECT_0},
        System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
    };
    let request: Request = serde_json::from_slice(&fs::read(path)?)?;
    // Open the handle before signalling readiness so PID reuse cannot make us
    // wait on a different process after the UI exits.
    let parent = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, request.parent) };
    ensure!(!parent.is_null(), "无法等待旧进程退出");
    fs::write(path.with_extension("ready"), b"ready")?;
    let exited = unsafe { WaitForSingleObject(parent, 60_000) };
    unsafe {
        CloseHandle(parent);
    }
    ensure!(exited == WAIT_OBJECT_0, "旧进程未退出，已取消迁移");
    let progress = crate::migration_progress::Progress::show();
    let ready = request
        .target
        .join(format!(".startup-{}.ready", uuid::Uuid::new_v4()));
    let result = migrate(
        &request.source,
        &request.target,
        &request.default,
        |target, _| {
            let mut child = StartupChild {
                child: launch(target, None, Some(&ready))?,
                ready: false,
            };
            let deadline = Instant::now() + Duration::from_secs(30);
            while Instant::now() < deadline {
                if ready.exists() {
                    let _ = fs::remove_file(&ready);
                    child.ready = true;
                    return Ok(());
                }
                if child.child.try_wait()?.is_some() {
                    anyhow::bail!("新目录启动失败");
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            anyhow::bail!("新目录启动超时，已回退")
        },
    );
    drop(progress);
    if let Err(error) = result {
        use std::io::Write;
        if let Ok(mut log) = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(request.source.join("traffic.log"))
        {
            let _ = writeln!(
                log,
                "{} 目录迁移失败：{error:#}",
                chrono::Local::now().to_rfc3339()
            );
        }
        let notice = request
            .source
            .join(format!(".migration-error-{}.txt", uuid::Uuid::new_v4()));
        fs::write(&notice, format!("目录迁移失败，继续使用原目录：{error:#}"))?;
        launch(&request.source, Some(&notice), None)?;
    }
    let _ = fs::remove_file(path);
    Ok(())
}

#[cfg(test)]
thread_local! { static COPY_FAILURE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(root: &Path, name: &str, total: i64) -> PathBuf {
        let path = root.join(name);
        fs::create_dir_all(path.join("exports")).unwrap();
        fs::create_dir_all(path.join("backups")).unwrap();
        fs::create_dir_all(path.join("client-config-backups")).unwrap();
        Config {
            retention_days: 123,
            ..Default::default()
        }
        .save(&path)
        .unwrap();
        let storage = Storage::open(&path.join("traffic.sqlite3")).unwrap();
        storage
            .db
            .execute(
                "INSERT INTO seconds_samples VALUES(1,60000,61000,1000,?1,0)",
                [total],
            )
            .unwrap();
        storage
            .db
            .execute("INSERT INTO totals VALUES(60000,?1,0)", [total])
            .unwrap();
        storage
            .db
            .execute(
                "INSERT INTO meta VALUES('checkpoint',?1)",
                [serde_json::to_string(&crate::accounting::Ledger::default()).unwrap()],
            )
            .unwrap();
        drop(storage);
        fs::write(path.join("exports/test.csv"), total.to_string()).unwrap();
        fs::write(path.join("backups/test.sqlite3"), b"backup artifact").unwrap();
        fs::write(
            path.join("client-config-backups/manifest.json"),
            b"client backup",
        )
        .unwrap();
        fs::write(path.join("traffic.log"), b"log").unwrap();
        path
    }

    fn total(path: &Path) -> u64 {
        Storage::open(&path.join("traffic.sqlite3"))
            .unwrap()
            .total(0, i64::MAX)
            .unwrap()
            .total()
    }

    #[test]
    fn migrate_backs_up_destination_preserves_source_and_unrelated_files() {
        let root = tempfile::tempdir().unwrap();
        let source = fixture(root.path(), "source 中文", 123);
        let target = fixture(root.path(), "目标 data", 999);
        let default = root.path().join("default");
        let external_backups = root.path().join("external backups");
        fs::create_dir(&external_backups).unwrap();
        fs::write(external_backups.join("keep.sqlite3"), b"independent backup").unwrap();
        let mut source_config = Config::load(&source).unwrap();
        source_config.backup_dir = Some(external_backups.display().to_string());
        source_config.save(&source).unwrap();
        fs::write(target.join("unrelated.txt"), b"keep").unwrap();
        migrate(&source, &target, &default, |dir, config| {
            assert_eq!(config.retention_days, 123);
            assert_eq!(total(dir), 123);
            Config::load(dir)?;
            Storage::open(&dir.join("traffic.sqlite3"))?.checkpoint()?;
            Ok(())
        })
        .unwrap();
        assert_eq!(total(&source), 123);
        assert_eq!(total(&target), 123);
        let migrated = Storage::open(&target.join("traffic.sqlite3")).unwrap();
        assert_eq!(
            migrated
                .db
                .query_row("SELECT down FROM seconds_samples WHERE seq=1", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            123
        );
        drop(migrated);
        assert_eq!(
            Config::load(&target).unwrap().backup_directory().unwrap(),
            external_backups
        );
        assert_eq!(
            fs::read(external_backups.join("keep.sqlite3")).unwrap(),
            b"independent backup"
        );
        assert!(same_directory(&resolve(&default).unwrap(), &target));
        assert_eq!(fs::read(target.join("unrelated.txt")).unwrap(), b"keep");
        assert_eq!(fs::read(target.join("exports/test.csv")).unwrap(), b"123");
        let backup = fs::read_dir(target.join("backups"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("before-migration-")
            })
            .unwrap();
        assert_eq!(total(&backup), 999);
        assert_eq!(fs::read(backup.join("exports/test.csv")).unwrap(), b"999");
        assert_eq!(
            fs::read(target.join("client-config-backups/manifest.json")).unwrap(),
            b"client backup"
        );
    }

    #[test]
    fn migration_keeps_document_default_independent_and_retains_old_backups() {
        let root = tempfile::tempdir().unwrap();
        let source = fixture(root.path(), "source 中文", 123);
        let target = root.path().join("target data");
        fs::create_dir(&target).unwrap();
        let default = root.path().join("default");
        let before = Config::load(&source).unwrap();
        assert!(before.backup_dir.is_none());
        let documents_backup = before.backup_directory().unwrap();
        migrate(&source, &target, &default, |_, config| {
            assert!(config.backup_dir.is_none());
            assert_eq!(config.backup_directory()?, documents_backup);
            Ok(())
        })
        .unwrap();
        let after = Config::load(&target).unwrap();
        assert!(after.backup_dir.is_none());
        assert_eq!(after.backup_directory().unwrap(), documents_backup);
        assert_eq!(
            fs::read(source.join("backups/test.sqlite3")).unwrap(),
            b"backup artifact"
        );
        assert_eq!(
            fs::read(target.join("backups/test.sqlite3")).unwrap(),
            b"backup artifact"
        );
    }

    #[test]
    fn startup_failure_restores_target_and_locator() {
        let root = tempfile::tempdir().unwrap();
        let source = fixture(root.path(), "source", 123);
        let target = fixture(root.path(), "target", 999);
        let default = root.path().join("default");
        fs::create_dir(&default).unwrap();
        set_location(&default, &source).unwrap();
        let before = fs::read(default.join("data-location.toml")).unwrap();
        let result = migrate(&source, &target, &default, |dir, _| {
            assert_eq!(total(dir), 123);
            // Simulate the failed child creating WAL sidecars.
            fs::write(dir.join("traffic.sqlite3-wal"), b"failed startup")?;
            anyhow::bail!("simulated startup failure")
        });
        let error = result.unwrap_err();
        assert!(error.to_string().contains("startup failure"), "{error:#}");
        assert_eq!(total(&source), 123);
        assert_eq!(total(&target), 999);
        assert_eq!(fs::read(target.join("exports/test.csv")).unwrap(), b"999");
        assert_eq!(
            fs::read(default.join("data-location.toml")).unwrap(),
            before
        );
    }

    #[test]
    fn restore_default_removes_locator_and_keeps_original_default_as_backup() {
        let root = tempfile::tempdir().unwrap();
        let source = fixture(root.path(), "custom", 123);
        let default = fixture(root.path(), "default", 999);
        set_location(&default, &source).unwrap();
        migrate(&source, &default, &default, |_, _| Ok(())).unwrap();
        assert!(!default.join("data-location.toml").exists());
        assert!(same_directory(&resolve(&default).unwrap(), &default));
        assert_eq!(total(&default), 123);
    }

    #[test]
    fn copy_failure_and_directory_lock_leave_existing_data_untouched() {
        let root = tempfile::tempdir().unwrap();
        let source = fixture(root.path(), "source", 123);
        let target = fixture(root.path(), "target", 999);
        let default = root.path().join("default");
        COPY_FAILURE.with(|fail| fail.set(true));
        assert!(
            migrate(&source, &target, &default, |_, _| panic!(
                "must not activate"
            ))
            .is_err()
        );
        assert_eq!(total(&target), 999);
        assert!(!default.exists());
        let _lock = lock_directory(&target).unwrap();
        assert!(
            migrate(&source, &target, &default, |_, _| panic!(
                "must not activate"
            ))
            .is_err()
        );
        assert_eq!(total(&target), 999);
        assert_eq!(total(&source), 123);
    }

    #[test]
    fn rejects_nested_same_and_invalid_directories_and_invalid_locator() {
        let root = tempfile::tempdir().unwrap();
        let source = fixture(root.path(), "source", 123);
        assert!(validate(&source, &source).is_err());
        assert!(validate(&source, &source.join("exports")).is_err());
        assert!(validate(&source, root.path()).is_err());
        assert!(validate(&source, &root.path().join("missing")).is_err());
        fs::write(
            root.path().join("data-location.toml"),
            "directory = 'Z:\\missing-directory-123456'",
        )
        .unwrap();
        assert!(resolve(root.path()).is_err());
    }

    #[test]
    fn migration_supports_cross_volume_copy() {
        let source_root =
            tempfile::tempdir_in(Path::new(env!("CARGO_MANIFEST_DIR")).join("target")).unwrap();
        let destination_root = tempfile::tempdir().unwrap();
        let source = fixture(source_root.path(), "跨磁盘 source", 456);
        let target = destination_root.path().join("data target");
        fs::create_dir(&target).unwrap();
        migrate(
            &source,
            &target,
            &destination_root.path().join("locator"),
            |_, _| Ok(()),
        )
        .unwrap();
        assert_eq!(total(&target), 456);
        assert_eq!(total(&source), 456);
    }
}
