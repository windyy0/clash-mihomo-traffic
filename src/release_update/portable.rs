use super::{Release, UpdateProgress, installer};
use crate::native;
use anyhow::{Context, Result, ensure};
use std::{
    ffi::OsStr,
    fs::{self, File, Metadata},
    io::{self, Read},
    os::windows::{fs::MetadataExt, process::CommandExt},
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, WAIT_OBJECT_0},
    Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT,
    System::Threading::{CREATE_NO_WINDOW, OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
};

const STAGE_PREFIX: &str = ".mihomo-update-";
const HELPER: &str = "apply.exe";
const MARKER: &str = "portable.flag";
const PROGRAM: &str = "mihomo-traffic.exe";
// Replace the executable last, after the accompanying documents are in place.
const FILES: [&str; 3] = ["LICENSE", "THIRD-PARTY-NOTICES.txt", PROGRAM];

pub(crate) fn download_and_update(
    release: &Release,
    data_dir: &Path,
    mut progress: impl FnMut(UpdateProgress),
) -> Result<()> {
    let executable = std::env::current_exe()?;
    let directory = executable.parent().context("无法找到便携版目录。")?;
    ensure!(directory.join(MARKER).is_file(), "找不到便携版标记。");
    let stage = tempfile::Builder::new()
        .prefix(STAGE_PREFIX)
        .tempdir_in(directory)
        .context("便携版目录不可写，无法暂存更新。")?;
    let archive = installer::download_package(release, true, stage.path(), &mut progress)?;
    unpack(&archive, stage.path()).context("无法解压便携版更新包。")?;
    fs::remove_file(archive)?;
    let helper = stage.path().join(HELPER);
    fs::copy(&executable, &helper).context("无法准备便携版更新程序。")?;
    progress(UpdateProgress::Installing);
    let mut child = command(&helper)
        .arg("--apply-portable-update")
        .arg(
            executable
                .file_name()
                .context("无法找到便携版程序文件名。")?,
        )
        .arg(data_dir)
        .spawn()
        .context("无法启动便携版更新程序。")?;
    let _ = stage.keep();
    ensure!(child.wait()?.success(), "便携版更新未完成，可以重试。");
    Ok(())
}

fn unpack(archive: &Path, stage: &Path) -> Result<()> {
    let mut zip = zip::ZipArchive::new(File::open(archive)?)?;
    ensure!(zip.len() == FILES.len() + 1, "便携包包含意外的文件。");
    {
        let marker = zip.by_name(MARKER)?;
        ensure!(
            marker.is_file() && !marker.is_symlink() && marker.size() == 0,
            "便携包标记无效。"
        );
    }
    let mut remaining = installer::MAX_PACKAGE as u64;
    for name in FILES {
        let entry = zip.by_name(name)?;
        let size = entry.size();
        ensure!(
            entry.is_file() && !entry.is_symlink() && size > 0 && size <= remaining,
            "便携包文件无效：{name}"
        );
        let mut output = File::create_new(stage.join(name))?;
        let copied = io::copy(&mut entry.take(size + 1), &mut output)?;
        ensure!(copied == size, "便携包文件大小不符：{name}");
        remaining -= copied;
    }
    Ok(())
}

fn command(executable: &Path) -> Command {
    let mut command = Command::new(executable);
    command
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

fn is_stage_name(name: &OsStr) -> bool {
    name.to_str()
        .and_then(|name| name.strip_prefix(STAGE_PREFIX))
        .is_some_and(|suffix| {
            !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_alphanumeric())
        })
}

fn is_regular(metadata: &Metadata) -> bool {
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
}

pub(crate) fn apply(executable_name: &OsStr, data_dir: &Path) -> Result<()> {
    let helper = std::env::current_exe()?;
    let stage = helper.parent().context("找不到更新暂存目录。")?;
    let name = stage
        .file_name()
        .filter(|name| is_stage_name(name))
        .context("更新暂存目录无效。")?;
    ensure!(
        helper.file_name() == Some(OsStr::new(HELPER)) && is_regular(&fs::symlink_metadata(stage)?),
        "更新暂存目录无效。"
    );
    let directory = stage.parent().context("找不到便携版目录。")?;
    let path = Path::new(executable_name);
    ensure!(
        path.file_name() == Some(executable_name)
            && !executable_name.to_string_lossy().contains(':')
            && path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("exe")),
        "便携版程序文件名无效。"
    );
    ensure!(data_dir.is_absolute(), "便携版数据目录无效。");
    let executable = directory.join(path);
    ensure!(
        directory.join(MARKER).is_file() && executable.is_file(),
        "找不到要更新的便携版。"
    );
    let _updating = native::try_instance("Local\\MihomoTrafficPortableUpdate")?
        .context("便携版更新正在进行。")?;
    native::request_matching_app_exit(Some(&executable.canonicalize()?))?;
    let deadline = Instant::now() + Duration::from_secs(40);
    let instance = loop {
        if let Some(instance) = native::try_instance(native::DESKTOP_MUTEX_NAME)? {
            break instance;
        }
        ensure!(
            Instant::now() < deadline,
            "程序未能及时退出，便携版更新已取消。"
        );
        thread::sleep(Duration::from_millis(100));
    };

    let restart = || {
        let mut command = command(&executable);
        command
            .current_dir(directory)
            .arg("--data-dir")
            .arg(data_dir);
        command
    };
    let result = replace_payload(stage, &executable, || {
        restart()
            .arg("--portable-update-cleanup")
            .arg(name)
            .arg(std::process::id().to_string())
            .spawn()
            .context("无法启动更新后的程序。")?;
        Ok(())
    });
    drop(instance);
    if let Err(error) = result {
        let restart: io::Result<Child> = restart().spawn();
        let detail = match restart {
            Ok(_) => "便携版更新未完成。".to_string(),
            Err(error) => format!("便携版更新未完成，且无法重新打开程序：{error}"),
        };
        return Err(error.context(format!("{detail}\n暂存目录：{}", stage.display())));
    }
    Ok(())
}

fn replace_payload(
    stage: &Path,
    executable: &Path,
    activate: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let directory = executable.parent().context("找不到便携版目录。")?;
    let backup = stage.join("backup");
    fs::create_dir(&backup)?;
    let targets = [
        directory.join(FILES[0]),
        directory.join(FILES[1]),
        executable.to_path_buf(),
    ];
    let mut saved = [false; FILES.len()];
    for (index, (name, target)) in FILES.iter().zip(&targets).enumerate() {
        match fs::symlink_metadata(target) {
            Ok(metadata) => {
                ensure!(
                    metadata.is_file() && is_regular(&metadata),
                    "程序文件路径无效：{}",
                    target.display()
                );
                fs::copy(target, backup.join(name)).with_context(|| format!("无法备份 {name}"))?;
                saved[index] = true;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    let mut replaced = 0;
    let result = (|| {
        for (name, target) in FILES.iter().zip(&targets) {
            native::replace_file(&stage.join(name), target)
                .with_context(|| format!("无法替换 {name}"))?;
            replaced += 1;
        }
        activate()
    })();
    if let Err(error) = result {
        let mut failures = Vec::new();
        for index in (0..replaced).rev() {
            let target = &targets[index];
            let restored = if saved[index] {
                native::replace_file(&backup.join(FILES[index]), target)
            } else {
                fs::remove_file(target).map_err(Into::into)
            };
            if let Err(error) = restored {
                failures.push(format!("{}：{error}", FILES[index]));
            }
        }
        if failures.is_empty() {
            return Err(error);
        }
        return Err(error.context(format!("部分文件无法还原：{}", failures.join("、"))));
    }
    Ok(())
}

pub(crate) fn cleanup(name: &OsStr, helper_pid: u32) {
    if helper_pid == 0
        || helper_pid == std::process::id()
        || !is_stage_name(name)
        || crate::config::portable_data_dir().is_none()
    {
        return;
    }
    let Ok(executable) = std::env::current_exe() else {
        return;
    };
    let Some(directory) = executable.parent() else {
        return;
    };
    let stage = directory.join(name);
    if !fs::symlink_metadata(&stage).is_ok_and(|meta| meta.is_dir() && is_regular(&meta))
        || !stage.join(HELPER).is_file()
    {
        return;
    }
    // The new process must wait for the helper to release the desktop mutex
    // and its staged executable before starting up and removing the stage.
    let helper = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, helper_pid) };
    if !helper.is_null() {
        let exited = unsafe { WaitForSingleObject(helper, 10_000) };
        unsafe { CloseHandle(helper) };
        if exited != WAIT_OBJECT_0 {
            return;
        }
    }
    let _ = fs::remove_dir_all(stage);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn extracts_only_the_expected_portable_files() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("package.zip");
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for unexpected in [false, true] {
            let mut zip = zip::ZipWriter::new(File::create(&archive).unwrap());
            zip.start_file(MARKER, options).unwrap();
            for name in FILES {
                let name = if unexpected && name == PROGRAM {
                    "data/config.toml"
                } else {
                    name
                };
                zip.start_file(name, options).unwrap();
                zip.write_all(name.as_bytes()).unwrap();
            }
            zip.finish().unwrap();
            let stage = tempfile::tempdir_in(dir.path()).unwrap();
            let result = unpack(&archive, stage.path());
            assert!(!stage.path().join("data").exists());
            if unexpected {
                assert!(result.is_err());
            } else {
                result.unwrap();
                assert_eq!(
                    fs::read(stage.path().join(PROGRAM)).unwrap(),
                    PROGRAM.as_bytes()
                );
            }
        }
    }

    #[test]
    fn portable_replacement_preserves_data_and_rolls_back_a_failed_launch() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let renamed = root.join("Portable Traffic.exe");
        fs::write(&renamed, b"old exe").unwrap();
        fs::write(root.join("LICENSE"), b"old license").unwrap();
        fs::write(root.join(MARKER), b"marker").unwrap();
        fs::create_dir(root.join("data")).unwrap();
        fs::write(root.join("data/config.toml"), b"settings").unwrap();

        for failed in [true, false] {
            let stage = tempfile::tempdir_in(root).unwrap();
            for name in FILES {
                fs::write(stage.path().join(name), b"new").unwrap();
            }
            let result = replace_payload(stage.path(), &renamed, || {
                assert_eq!(fs::read(&renamed).unwrap(), b"new");
                assert_eq!(
                    fs::read(root.join("data/config.toml")).unwrap(),
                    b"settings"
                );
                ensure!(!failed, "could not launch");
                Ok(())
            });
            if failed {
                assert!(result.is_err());
                assert_eq!(fs::read(&renamed).unwrap(), b"old exe");
                assert_eq!(fs::read(root.join("LICENSE")).unwrap(), b"old license");
                assert!(!root.join("THIRD-PARTY-NOTICES.txt").exists());
            } else {
                result.unwrap();
                assert_eq!(fs::read(&renamed).unwrap(), b"new");
                assert_eq!(
                    fs::read(root.join("THIRD-PARTY-NOTICES.txt")).unwrap(),
                    b"new"
                );
            }
            assert_eq!(fs::read(root.join(MARKER)).unwrap(), b"marker");
            assert_eq!(
                fs::read(root.join("data/config.toml")).unwrap(),
                b"settings"
            );
        }
    }
}
