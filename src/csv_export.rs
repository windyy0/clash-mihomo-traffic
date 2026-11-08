use anyhow::{Context, Result, ensure};
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Uses Windows' known Downloads folder, including redirected locations.
pub fn directory() -> Result<PathBuf> {
    directory_from(dirs::download_dir())
}
fn directory_from(downloads: Option<PathBuf>) -> Result<PathBuf> {
    let path = downloads.context("无法获取系统下载目录")?;
    ensure!(path.is_absolute(), "系统下载目录不是有效的绝对路径");
    Ok(path)
}
pub fn ensure_directory() -> Result<PathBuf> {
    let path = directory()?;
    fs::create_dir_all(&path)
        .with_context(|| format!("无法使用CSV导出目录：{}", path.display()))?;
    Ok(path)
}
pub fn destination(dimension: &str) -> Result<PathBuf> {
    destination_in(&directory()?, dimension)
}
fn destination_in(path: &Path, dimension: &str) -> Result<PathBuf> {
    ensure!(path.is_absolute(), "CSV导出目录须为绝对路径");
    ensure!(
        ["app", "domain", "route", "source"].contains(&dimension),
        "未知统计维度"
    );
    fs::create_dir_all(path).with_context(|| format!("无法使用CSV导出目录：{}", path.display()))?;
    // Independent of the database location; repeated exports get distinct files.
    Ok(path.join(format!(
        "traffic-{dimension}-{}-{}.csv",
        chrono::Local::now().format("%Y%m%d-%H%M%S"),
        uuid::Uuid::new_v4().simple()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        accounting::{Batch, Fact, Ledger},
        model::{Bytes, Query},
        storage::Storage,
    };
    #[test]
    fn downloads_resolution_preserves_redirected_unicode_path_without_creating_it() {
        let root = tempfile::tempdir().unwrap();
        let redirected = root.path().join("重定向下载");
        assert_eq!(
            directory_from(Some(redirected.clone())).unwrap(),
            redirected
        );
        assert!(!redirected.exists());
        assert!(
            directory_from(None)
                .unwrap_err()
                .to_string()
                .contains("系统下载目录")
        );
        assert!(directory_from(Some(PathBuf::from("relative/downloads"))).is_err());
    }
    #[test]
    fn csv_exports_go_to_downloads_independent_of_database_and_keep_previous_files() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("database");
        fs::create_dir(&data).unwrap();
        let mut storage = Storage::open(&data.join("traffic.sqlite3")).unwrap();
        let bytes = Bytes { up: 10, down: 20 };
        storage
            .commit(
                &Ledger {
                    sequence: 1,
                    time: 61_000,
                    ..Default::default()
                },
                &Batch {
                    totals: vec![(60_000, bytes)],
                    facts: vec![Fact {
                        time: 60_000,
                        app: "demo.exe".into(),
                        domain: "example.com".into(),
                        route: "DIRECT".into(),
                        source: "local".into(),
                        bytes,
                    }],
                    ..Default::default()
                },
            )
            .unwrap();
        let downloads = directory_from(Some(root.path().join("下载位置"))).unwrap();
        let first = destination_in(&downloads, "app").unwrap();
        let second = destination_in(&downloads, "app").unwrap();
        assert_ne!(first, second);
        assert_eq!(first.parent(), Some(downloads.as_path()));
        let query = Query {
            from: 0,
            to: 120_000,
            dimension: "app".into(),
            ..Default::default()
        };
        storage.export(&query, &first).unwrap();
        let previous = fs::read(&first).unwrap();
        storage.export(&query, &second).unwrap();
        assert_eq!(fs::read(&first).unwrap(), previous);
        let text = String::from_utf8(previous).unwrap();
        assert!(text.contains("demo.exe"));
        assert!(text.contains(",10,20,30,"));
        assert!(!data.join("exports").exists());
        let blocked = root.path().join("blocked");
        fs::write(&blocked, b"file").unwrap();
        assert!(destination_in(&blocked, "app").is_err());
        assert!(destination_in(&downloads, "../escape").is_err());
    }
}
