use super::{API, CheckError, Release, fetch_with, github_client, request_tag};
use anyhow::{Context, Result, anyhow, ensure};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

pub(super) const MAX_PACKAGE: usize = 256 * 1024 * 1024;

pub fn download_and_install(release: &Release, install_dir: &Path) -> Result<()> {
    let downloads = dirs::download_dir().context("无法找到系统下载目录。")?;
    let dir = tempfile::Builder::new()
        .prefix("MihomoTraffic-")
        .tempdir_in(downloads)
        .context("无法在系统下载目录中保存安装包。")?;
    let setup = download_package(release, false, dir.path())?;
    // Inno Setup stops the running app after the user confirms the installation.
    let mut target = std::ffi::OsString::from("/DIR=");
    target.push(install_dir);
    let mut installer = Command::new(setup)
        .current_dir(dir.path())
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("无法打开安装程序。")?;
    let _ = dir.keep();
    let status = installer.wait().context("无法读取安装结果。")?;
    ensure!(status.success(), "安装未完成，可以重试。");
    Ok(())
}

pub(super) fn download_package(release: &Release, portable: bool, dir: &Path) -> Result<PathBuf> {
    ensure!(release.is_newer(), "只能安装较新的版本。");
    let tag = release.tag();
    let version = Release::parse_tag(tag).context("无法识别更新版本。")?;
    // Read the digest from GitHub for the exact version the user selected.
    let asset = fetch_with(async |client| {
        request_tag(client, API, tag)
            .await?
            .matching_asset(&version, portable)
            .cloned()
            .ok_or(CheckError::Unavailable)
    })
    .map_err(|error| anyhow!(error.message()))?;
    let expected = asset
        .digest
        .as_deref()
        .and_then(|digest| digest.strip_prefix("sha256:"))
        .filter(|hash| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
        .context("无法校验此更新包，请在 GitHub 查看发行版。")?;

    let partial = dir.join("package.download");
    let mut file = File::create(&partial).context("无法创建更新包文件。")?;
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("无法启动更新包下载。")?
        .block_on(async {
            let client = github_client()
                .timeout(Duration::from_secs(300))
                .read_timeout(Duration::from_secs(20))
                .redirect(reqwest::redirect::Policy::custom(|attempt| {
                    if attempt.previous().len() <= 5
                        && matches!(
                            attempt.url().host_str(),
                            Some(
                                "github.com"
                                    | "release-assets.githubusercontent.com"
                                    | "objects.githubusercontent.com"
                            )
                        )
                    {
                        attempt.follow()
                    } else {
                        attempt.error("unexpected installer download redirect")
                    }
                }))
                .build()
                .context("无法启动更新包下载。")?;
            download_verified(&client, &asset.browser_download_url, expected, &mut file).await
        })?;
    drop(file);

    let package = dir.join(asset.name);
    std::fs::rename(partial, &package).context("无法保存校验后的更新包。")?;
    Ok(package)
}

pub(super) async fn download_verified(
    client: &reqwest::Client,
    url: &str,
    expected: &str,
    file: &mut File,
) -> Result<()> {
    let mut response = client
        .get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(reqwest::Error::without_url)
        .context("下载更新包失败，请检查网络后重试。")?;
    let mut hash = Sha256::new();
    let mut length = 0;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(reqwest::Error::without_url)
        .context("更新包下载中断，请重试。")?
    {
        length += chunk.len();
        ensure!(length <= MAX_PACKAGE, "更新包超出大小限制。");
        file.write_all(&chunk).context("写入更新包失败。")?;
        hash.update(&chunk);
    }
    ensure!(
        length != 0 && format!("{:x}", hash.finalize()).eq_ignore_ascii_case(expected),
        "更新包校验失败，请重新下载。"
    );
    Ok(())
}
