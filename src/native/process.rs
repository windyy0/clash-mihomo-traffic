//! Local process, listener, and client discovery and Windows launch settings.
use super::wide;
use anyhow::{Context, Result, ensure};
use std::{
    path::Path,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::*,
    System::{Pipes::GetNamedPipeServerProcessId, Registry::*, Threading::*},
};

pub fn trim_working_set() {
    unsafe { SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX) };
}

/// A directly opened Inno uninstaller holds its uninstall log until it exits.
/// Continue with the confirmed choice after that process has released the log.
pub fn continue_uninstall(parent: u32, filename: &str, remove_data: bool) -> Result<()> {
    ensure!(
        parent != 0 && parent != std::process::id(),
        "卸载进程编号无效"
    );
    ensure!(
        filename
            .strip_prefix("unins")
            .and_then(|name| name.strip_suffix(".exe"))
            .is_some_and(|digits| digits.len() == 3 && digits.bytes().all(|b| b.is_ascii_digit())),
        "卸载程序文件名无效"
    );
    let uninstaller = std::env::current_exe()?.with_file_name(filename);
    ensure!(
        uninstaller.is_file() && uninstaller.with_extension("dat").is_file(),
        "找不到卸载程序或卸载记录"
    );

    let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, parent) };
    if process.is_null() {
        let error = std::io::Error::last_os_error();
        // The first uninstaller may have already exited before this helper starts.
        if error.raw_os_error() != Some(ERROR_INVALID_PARAMETER as i32) {
            return Err(error).context("无法等待卸载确认窗口退出");
        }
    } else {
        let result = unsafe {
            let result = WaitForSingleObject(process, 30_000);
            CloseHandle(process);
            result
        };
        ensure!(result == WAIT_OBJECT_0, "等待卸载确认窗口退出超时");
    }

    let mut command = std::process::Command::new(uninstaller);
    command.args(["/SILENT", "/NORESTART"]);
    if remove_data {
        command.arg("/MIHOMODELETEDATA=1");
    }
    command.spawn().context("无法继续卸载")?;
    Ok(())
}

pub fn pipe_server_process(handle: HANDLE) -> Result<u32> {
    unsafe {
        let mut pid = 0;
        anyhow::ensure!(
            GetNamedPipeServerProcessId(handle, &mut pid) != 0,
            "无法识别内核进程"
        );
        Ok(pid)
    }
}
pub fn pipe_owner(path: &str) -> Result<u32> {
    use std::os::windows::io::AsRawHandle;
    let pipe = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)?;
    pipe_server_process(pipe.as_raw_handle())
}

// Identify the owner of this connection, rather than another proxy client that
// happens to be running. Services can be ancestors when the core is elevated.
pub fn proxy_client_name(pid: u32) -> Option<String> {
    use windows_sys::Win32::System::Diagnostics::ToolHelp::*;

    let mut processes = std::collections::HashMap::new();
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut available = Process32FirstW(snapshot, &mut entry);
        while available != 0 {
            let len = entry
                .szExeFile
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(entry.szExeFile.len());
            processes.insert(
                entry.th32ProcessID,
                (
                    entry.th32ParentProcessID,
                    String::from_utf16_lossy(&entry.szExeFile[..len]),
                ),
            );
            available = Process32NextW(snapshot, &mut entry);
        }
        CloseHandle(snapshot);
    }
    let created = |id| {
        process_identity(id)
            .ok()
            .and_then(|identity| identity.split_once(':')?.1.parse::<u64>().ok())
    };
    let mut current = pid;
    let mut current_created = created(current);
    let mut visited = std::collections::HashSet::new();
    for _ in 0..16 {
        if !visited.insert(current) {
            break;
        }
        let (parent, executable) = processes.get(&current)?;
        let name = match executable.to_ascii_lowercase().as_str() {
            "clash-verge.exe" | "clash-verge-service.exe" => Some("Clash Verge Rev"),
            "clash-nyanpasu.exe" | "clash-nyanpasu-service.exe" => Some("Clash Nyanpasu"),
            "flclash.exe" | "flclashhelperservice.exe" => Some("FlClash"),
            "clashn.exe" => Some("ClashN"),
            "sparkle.exe" | "sparkle-service.exe" => Some("Sparkle"),
            "clash for windows.exe" => Some("Clash for Windows"),
            _ => None,
        };
        if let Some(name) = name {
            return Some(name.into());
        }
        if *parent == 0 {
            break;
        }
        let parent_created = created(*parent);
        // A recycled parent PID no longer identifies the process that launched
        // this child. Stop instead of attributing it to an unrelated client.
        if !matches!((parent_created, current_created), (Some(p), Some(c)) if p <= c) {
            break;
        }
        current = *parent;
        current_created = parent_created;
    }
    None
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProxyApplication {
    pub executable: std::path::PathBuf,
    pub identity: String,
}

pub(super) fn process_executable(pid: u32) -> Option<std::path::PathBuf> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return None;
        }
        let mut buffer = vec![0u16; 32768];
        let mut len = buffer.len() as u32;
        let ok = QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut len);
        CloseHandle(process);
        (ok != 0)
            .then(|| std::path::PathBuf::from(String::from_utf16_lossy(&buffer[..len as usize])))
    }
}

pub fn proxy_applications(executable_name: &str) -> Result<Vec<ProxyApplication>> {
    use windows_sys::Win32::System::Diagnostics::ToolHelp::*;
    let mut applications = Vec::new();
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        anyhow::ensure!(snapshot != INVALID_HANDLE_VALUE, "无法检查客户端是否已退出");
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut available = Process32FirstW(snapshot, &mut entry);
        let mut unreadable = false;
        while available != 0 {
            let len = entry
                .szExeFile
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(entry.szExeFile.len());
            if String::from_utf16_lossy(&entry.szExeFile[..len])
                .eq_ignore_ascii_case(executable_name)
            {
                if let (Some(executable), Ok(identity)) = (
                    process_executable(entry.th32ProcessID),
                    process_identity(entry.th32ProcessID),
                ) {
                    applications.push(ProxyApplication {
                        executable,
                        identity,
                    });
                } else {
                    unreadable = true;
                }
            }
            available = Process32NextW(snapshot, &mut entry);
        }
        CloseHandle(snapshot);
        anyhow::ensure!(
            !unreadable,
            "有客户端进程无法核验，请正常退出该客户端后重试"
        );
    }
    Ok(applications)
}

pub fn nyanpasu_custom_config_dir() -> Option<std::path::PathBuf> {
    unsafe {
        let mut key = null_mut();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            wide("Software\\Clash Nyanpasu").as_ptr(),
            0,
            KEY_READ,
            &mut key,
        ) != 0
        {
            return None;
        }
        let mut buffer = vec![0u16; 32768];
        let mut len = (buffer.len() * 2) as u32;
        let mut kind = 0;
        let result = RegQueryValueExW(
            key,
            wide("AppDir").as_ptr(),
            null_mut(),
            &mut kind,
            buffer.as_mut_ptr() as _,
            &mut len,
        );
        RegCloseKey(key);
        if result != 0 || kind != REG_SZ {
            return None;
        }
        let len = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
        let path = std::path::PathBuf::from(String::from_utf16_lossy(&buffer[..len]));
        path.is_absolute().then_some(path)
    }
}

pub fn local_api_process(host: &str, port: u16) -> Option<u32> {
    use std::net::{Ipv4Addr, Ipv6Addr};
    use windows_sys::Win32::{
        NetworkManagement::IpHelper::*,
        Networking::WinSock::{AF_INET, AF_INET6},
    };

    let mut owners = std::collections::HashSet::new();
    if host == "localhost" || host.parse::<Ipv4Addr>().is_ok() {
        let address = if host == "localhost" {
            Ipv4Addr::LOCALHOST
        } else {
            host.parse().ok()?
        };
        let rows = tcp_listener_rows::<MIB_TCPROW_OWNER_PID>(
            AF_INET as u32,
            std::mem::offset_of!(MIB_TCPTABLE_OWNER_PID, table),
        )?;
        for row in rows {
            if u16::from_be(row.dwLocalPort as u16) == port
                && (row.dwLocalAddr == 0 || row.dwLocalAddr.to_ne_bytes() == address.octets())
            {
                owners.insert(row.dwOwningPid);
            }
        }
    }
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if host == "localhost" || host.parse::<Ipv6Addr>().is_ok() {
        let address = if host == "localhost" {
            Ipv6Addr::LOCALHOST
        } else {
            host.parse().ok()?
        };
        let rows = tcp_listener_rows::<MIB_TCP6ROW_OWNER_PID>(
            AF_INET6 as u32,
            std::mem::offset_of!(MIB_TCP6TABLE_OWNER_PID, table),
        )?;
        for row in rows {
            if u16::from_be(row.dwLocalPort as u16) == port
                && (row.ucLocalAddr == [0; 16] || row.ucLocalAddr == address.octets())
            {
                owners.insert(row.dwOwningPid);
            }
        }
    }
    // localhost can resolve to either IP family. Ambiguous ownership must not
    // turn into a guessed client name.
    if owners.len() == 1 {
        owners.into_iter().next().filter(|pid| *pid != 0)
    } else {
        None
    }
}

fn tcp_listener_rows<T: Copy>(family: u32, offset: usize) -> Option<Vec<T>> {
    use windows_sys::Win32::NetworkManagement::IpHelper::*;

    let mut buffer = vec![0u64; 1024];
    for _ in 0..3 {
        let mut bytes = (buffer.len() * 8) as u32;
        let status = unsafe {
            GetExtendedTcpTable(
                buffer.as_mut_ptr() as _,
                &mut bytes,
                0,
                family,
                TCP_TABLE_OWNER_PID_LISTENER,
                0,
            )
        };
        if status == ERROR_INSUFFICIENT_BUFFER {
            if bytes == 0 || bytes > 16 * 1024 * 1024 {
                return None;
            }
            buffer.resize((bytes as usize).div_ceil(8), 0);
            continue;
        }
        if status != ERROR_SUCCESS || bytes as usize > buffer.len() * 8 || (bytes as usize) < offset
        {
            return None;
        }
        let count = unsafe { *(buffer.as_ptr() as *const u32) } as usize;
        if count > (bytes as usize - offset) / std::mem::size_of::<T>() {
            return None;
        }
        let rows = unsafe { (buffer.as_ptr() as *const u8).add(offset) as *const T };
        return Some(
            (0..count)
                .map(|i| unsafe { rows.add(i).read_unaligned() })
                .collect(),
        );
    }
    None
}

pub fn process_identity(pid: u32) -> Result<String> {
    unsafe {
        let p = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if p.is_null() {
            return process_identity_snapshot(pid);
        }
        let mut c = FILETIME::default();
        let mut e = c;
        let mut k = c;
        let mut u = c;
        let ok = GetProcessTimes(p, &mut c, &mut e, &mut k, &mut u);
        CloseHandle(p);
        anyhow::ensure!(ok != 0, "无法读取内核启动时间");
        let created = ((c.dwHighDateTime as u64) << 32) | c.dwLowDateTime as u64;
        Ok(format!("{pid}:{created}"))
    }
}
fn process_identity_snapshot(pid: u32) -> Result<String> {
    use ntapi::ntexapi::{
        NtQuerySystemInformation, SYSTEM_PROCESS_INFORMATION, SystemProcessInformation,
    };
    // Read the OS process snapshot when the service DACL denies OpenProcess.
    // Vec<u64> guarantees alignment for the native records. Never dereference
    // ImageName or the variable trailing thread array.
    unsafe {
        let mut needed = 0u32;
        let mut buffer = vec![0u64; 65536];
        for _ in 0..3 {
            let status = NtQuerySystemInformation(
                SystemProcessInformation,
                buffer.as_mut_ptr() as _,
                (buffer.len() * 8) as u32,
                &mut needed,
            );
            if status >= 0 {
                let mut offset = 0usize;
                while offset + std::mem::size_of::<SYSTEM_PROCESS_INFORMATION>() <= buffer.len() * 8
                {
                    let info = &*((buffer.as_ptr() as *const u8).add(offset)
                        as *const SYSTEM_PROCESS_INFORMATION);
                    if info.UniqueProcessId as usize == pid as usize && pid != 0 {
                        let created = *info.CreateTime.QuadPart();
                        anyhow::ensure!(created > 0, "内核进程启动时间不可用");
                        return Ok(format!("{pid}:{created}"));
                    }
                    if info.NextEntryOffset == 0 {
                        break;
                    }
                    offset += info.NextEntryOffset as usize;
                }
                anyhow::bail!("未找到管道服务进程 {pid}");
            }
            anyhow::ensure!(
                needed > 0 && needed <= 16 * 1024 * 1024,
                "无法读取系统进程快照"
            );
            buffer.resize((needed as usize + 65536).div_ceil(8), 0);
        }
        anyhow::bail!("系统进程快照持续变化")
    }
}

pub fn autorun(enabled: bool, dir: &Path) -> Result<()> {
    unsafe {
        let mut key = null_mut();
        let code = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            wide("Software\\Microsoft\\Windows\\CurrentVersion\\Run").as_ptr(),
            0,
            null(),
            0,
            KEY_SET_VALUE,
            null(),
            &mut key,
            null_mut(),
        );
        anyhow::ensure!(code == 0, "无法打开开机启动设置：{code}");
        let name = wide("MihomoTraffic");
        let code = if enabled {
            let exe = std::env::current_exe()?;
            let value = wide(format!(
                "\"{}\" --background --data-dir \"{}\"",
                exe.display(),
                dir.display()
            ));
            RegSetValueExW(
                key,
                name.as_ptr(),
                0,
                REG_SZ,
                value.as_ptr() as _,
                (value.len() * 2) as u32,
            )
        } else {
            RegDeleteValueW(key, name.as_ptr())
        };
        RegCloseKey(key);
        anyhow::ensure!(
            code == 0 || code == ERROR_FILE_NOT_FOUND,
            "修改开机启动失败：{code}"
        );
        Ok(())
    }
}
pub fn open_folder(path: &Path) -> Result<()> {
    use std::os::windows::{ffi::OsStringExt, process::CommandExt};
    anyhow::ensure!(path.is_absolute() && path.is_dir(), "目录不存在或无法访问");
    let mut root = [0u16; 32768];
    let length = unsafe {
        windows_sys::Win32::System::SystemInformation::GetWindowsDirectoryW(
            root.as_mut_ptr(),
            root.len() as u32,
        )
    } as usize;
    anyhow::ensure!(length > 0 && length < root.len(), "无法定位资源管理器");
    let explorer = std::path::PathBuf::from(std::ffi::OsString::from_wide(&root[..length]))
        .join("explorer.exe");
    // ShellExecute may pump messages and re-enter GPUI's borrowed window state.
    // The caller keeps directory I/O and process creation off the UI thread.
    std::process::Command::new(explorer)
        .arg(path)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()?;
    Ok(())
}
