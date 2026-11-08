use super::{Endpoint, Observation, Owner, Process};
use std::{
    collections::HashMap,
    ffi::c_void,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    ptr::null,
};
use windows_sys::Win32::{
    Foundation::*,
    NetworkManagement::IpHelper::*,
    Networking::WinSock::{AF_INET, AF_INET6},
    System::{Diagnostics::ToolHelp::*, Services::*, Threading::*},
};

fn timestamp(value: i64) -> i64 {
    value.saturating_sub(116_444_736_000_000_000) / 10_000
}
fn process(pid: u32, parent: u32) -> Option<Process> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut path = vec![0u16; 32768];
        let mut length = path.len() as u32;
        let mut created = FILETIME::default();
        let mut exited = created;
        let mut kernel = created;
        let mut user = created;
        let ok = QueryFullProcessImageNameW(handle, 0, path.as_mut_ptr(), &mut length) != 0
            && GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) != 0;
        CloseHandle(handle);
        ok.then(|| Process {
            pid,
            parent,
            path: String::from_utf16_lossy(&path[..length as usize]),
            created: timestamp(
                ((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
                    as i64,
            ),
        })
    }
}
fn process_tree() -> HashMap<u32, (u32, String)> {
    let mut result = HashMap::new();
    unsafe {
        let handle = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if handle == INVALID_HANDLE_VALUE {
            return result;
        }
        let mut entry = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut available = Process32FirstW(handle, &mut entry);
        while available != 0 {
            let length = entry
                .szExeFile
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(entry.szExeFile.len());
            result.insert(
                entry.th32ProcessID,
                (
                    entry.th32ParentProcessID,
                    String::from_utf16_lossy(&entry.szExeFile[..length]).to_ascii_lowercase(),
                ),
            );
            available = Process32NextW(handle, &mut entry);
        }
        CloseHandle(handle);
    }
    result
}
fn hydrate(
    pid: u32,
    tree: &HashMap<u32, (u32, String)>,
    processes: &mut HashMap<u32, Process>,
    depth: usize,
) {
    if depth == 24 || processes.contains_key(&pid) {
        return;
    }
    let Some((parent, _)) = tree.get(&pid) else {
        return;
    };
    if let Some(process) = process(pid, *parent) {
        processes.insert(pid, process);
        hydrate(*parent, tree, processes, depth + 1);
    }
}
fn rows<T: Copy>(udp: bool, family: u32, offset: usize) -> Vec<T> {
    let mut buffer = vec![0u64; 2048];
    for _ in 0..3 {
        let mut bytes = (buffer.len() * 8) as u32;
        let status = unsafe {
            if udp {
                GetExtendedUdpTable(
                    buffer.as_mut_ptr().cast(),
                    &mut bytes,
                    0,
                    family,
                    UDP_TABLE_OWNER_MODULE,
                    0,
                )
            } else {
                GetExtendedTcpTable(
                    buffer.as_mut_ptr().cast(),
                    &mut bytes,
                    0,
                    family,
                    TCP_TABLE_OWNER_MODULE_ALL,
                    0,
                )
            }
        };
        if status == ERROR_INSUFFICIENT_BUFFER && bytes > 0 && bytes <= 16 * 1024 * 1024 {
            buffer.resize((bytes as usize).div_ceil(8), 0);
            continue;
        }
        if status != ERROR_SUCCESS || bytes as usize > buffer.len() * 8 || (bytes as usize) < offset
        {
            break;
        }
        let count = unsafe { *(buffer.as_ptr().cast::<u32>()) } as usize;
        if count > (bytes as usize - offset) / size_of::<T>() {
            break;
        }
        let start = unsafe { buffer.as_ptr().cast::<u8>().add(offset).cast::<T>() };
        return (0..count)
            .map(|i| unsafe { start.add(i).read_unaligned() })
            .collect();
    }
    vec![]
}
fn text(pointer: *const u16, buffer: &[u64]) -> Option<String> {
    let start = buffer.as_ptr() as usize;
    let end = start + size_of_val(buffer);
    let address = pointer as usize;
    if address < start || address >= end || !address.is_multiple_of(2) {
        return None;
    }
    let available = ((end - address) / 2).min(32768);
    let text = unsafe { std::slice::from_raw_parts(pointer, available) };
    let length = text.iter().position(|c| *c == 0)?;
    Some(String::from_utf16_lossy(&text[..length]))
}
fn services() -> HashMap<u32, Vec<(String, String)>> {
    let mut result = HashMap::<u32, Vec<(String, String)>>::new();
    unsafe {
        let manager = OpenSCManagerW(null(), null(), SC_MANAGER_ENUMERATE_SERVICE);
        if manager.is_null() {
            return result;
        }
        let mut buffer = vec![0u64; 32768];
        let mut needed = 0;
        let mut count = 0;
        let mut resume = 0;
        let ok = EnumServicesStatusExW(
            manager,
            SC_ENUM_PROCESS_INFO,
            SERVICE_WIN32,
            SERVICE_ACTIVE,
            buffer.as_mut_ptr().cast(),
            size_of_val(buffer.as_slice()) as u32,
            &mut needed,
            &mut count,
            &mut resume,
            null(),
        );
        CloseServiceHandle(manager);
        if ok == 0
            || count as usize
                > size_of_val(buffer.as_slice()) / size_of::<ENUM_SERVICE_STATUS_PROCESSW>()
        {
            return result;
        }
        for index in 0..count as usize {
            let entry = &*buffer
                .as_ptr()
                .cast::<ENUM_SERVICE_STATUS_PROCESSW>()
                .add(index);
            let pid = entry.ServiceStatusProcess.dwProcessId;
            if pid == 0 {
                continue;
            }
            if let (Some(name), Some(label)) = (
                text(entry.lpServiceName, &buffer),
                text(entry.lpDisplayName, &buffer),
            ) {
                result.entry(pid).or_default().push((name, label));
            }
        }
    }
    result
}
fn creation_times() -> HashMap<u32, (i64, u32)> {
    use ntapi::ntexapi::{
        NtQuerySystemInformation, SYSTEM_PROCESS_INFORMATION, SystemProcessInformation,
    };
    let mut result = HashMap::new();
    let mut buffer = vec![0u64; 65536];
    for _ in 0..3 {
        let mut needed = 0;
        let status = unsafe {
            NtQuerySystemInformation(
                SystemProcessInformation,
                buffer.as_mut_ptr().cast(),
                size_of_val(buffer.as_slice()) as u32,
                &mut needed,
            )
        };
        if status < 0 {
            if needed == 0 || needed > 16 * 1024 * 1024 {
                break;
            }
            buffer.resize((needed as usize + 65536).div_ceil(8), 0);
            continue;
        }
        let mut offset = 0usize;
        while offset + size_of::<SYSTEM_PROCESS_INFORMATION>() <= size_of_val(buffer.as_slice()) {
            let info = unsafe {
                &*buffer
                    .as_ptr()
                    .cast::<u8>()
                    .add(offset)
                    .cast::<SYSTEM_PROCESS_INFORMATION>()
            };
            let created = unsafe { *info.CreateTime.QuadPart() };
            if created > 0 {
                result.insert(
                    info.UniqueProcessId as usize as u32,
                    (
                        timestamp(created),
                        info.InheritedFromUniqueProcessId as usize as u32,
                    ),
                );
            }
            let next = info.NextEntryOffset as usize;
            if next < size_of::<SYSTEM_PROCESS_INFORMATION>()
                || !next.is_multiple_of(align_of::<SYSTEM_PROCESS_INFORMATION>())
            {
                break;
            }
            let Some(next) = offset.checked_add(next) else {
                break;
            };
            offset = next;
        }
        break;
    }
    result
}
fn service_binary(name: &str) -> Option<String> {
    unsafe {
        let manager = OpenSCManagerW(null(), null(), SC_MANAGER_CONNECT);
        if manager.is_null() {
            return None;
        }
        let name: Vec<u16> = name.encode_utf16().chain([0]).collect();
        let handle = OpenServiceW(manager, name.as_ptr(), SERVICE_QUERY_CONFIG);
        CloseServiceHandle(manager);
        if handle.is_null() {
            return None;
        }
        let mut buffer = vec![0u64; 1024];
        let mut needed = 0;
        let ok = QueryServiceConfigW(
            handle,
            buffer.as_mut_ptr().cast(),
            size_of_val(buffer.as_slice()) as u32,
            &mut needed,
        );
        CloseServiceHandle(handle);
        if ok == 0 {
            return None;
        }
        let config = &*buffer.as_ptr().cast::<QUERY_SERVICE_CONFIGW>();
        let command = text(config.lpBinaryPathName, &buffer)?;
        let command = command.trim();
        let path = if let Some(quoted) = command.strip_prefix('"') {
            quoted.split_once('"')?.0
        } else {
            command.split_whitespace().next()?
        };
        let path = if path.to_ascii_lowercase().starts_with("%systemroot%\\") {
            format!("{}{}", std::env::var("SystemRoot").ok()?, &path[12..])
        } else {
            path.to_string()
        };
        (std::path::Path::new(&path).is_absolute()
            && crate::model::app_name(&path).eq_ignore_ascii_case("svchost.exe"))
        .then_some(path)
    }
}
fn module_name(mut query: impl FnMut(*mut c_void, &mut u32) -> u32) -> Option<String> {
    let mut buffer = vec![0u64; 1024];
    for _ in 0..2 {
        let mut bytes = size_of_val(buffer.as_slice()) as u32;
        let status = query(buffer.as_mut_ptr().cast(), &mut bytes);
        if status == ERROR_INSUFFICIENT_BUFFER && bytes > 0 && bytes < 64 * 1024 {
            buffer.resize((bytes as usize).div_ceil(8), 0);
            continue;
        }
        if status != ERROR_SUCCESS
            || bytes as usize > size_of_val(buffer.as_slice())
            || (bytes as usize) < size_of::<TCPIP_OWNER_MODULE_BASIC_INFO>()
        {
            return None;
        }
        let info = unsafe { &*buffer.as_ptr().cast::<TCPIP_OWNER_MODULE_BASIC_INFO>() };
        return text(info.pModuleName, &buffer);
    }
    None
}
fn service(
    pid: u32,
    processes: &HashMap<u32, Process>,
    services: &HashMap<u32, Vec<(String, String)>>,
    query: impl FnMut(*mut c_void, &mut u32) -> u32,
) -> Option<Owner> {
    let process = processes.get(&pid)?;
    if !crate::model::app_name(&process.path).eq_ignore_ascii_case("svchost.exe") {
        return None;
    }
    let choices = services.get(&pid)?;
    let chosen = if choices.len() == 1 {
        choices.first()?
    } else {
        let name = module_name(query)?;
        choices
            .iter()
            .find(|(candidate, _)| candidate.eq_ignore_ascii_case(&name))?
    };
    Some(Owner {
        key: format!("service:{}", chosen.0.to_ascii_lowercase()),
        label: format!("{}（{}）", chosen.1, chosen.0),
        icon: process.path.clone(),
    })
}
pub(super) fn observe() -> Observation {
    let mut observation = Observation {
        captured: chrono::Utc::now().timestamp_millis(),
        ..Default::default()
    };
    let tree = process_tree();
    let services = services();
    let tcp4 = rows::<MIB_TCPROW_OWNER_MODULE>(
        false,
        AF_INET as u32,
        std::mem::offset_of!(MIB_TCPTABLE_OWNER_MODULE, table),
    );
    let tcp6 = rows::<MIB_TCP6ROW_OWNER_MODULE>(
        false,
        AF_INET6 as u32,
        std::mem::offset_of!(MIB_TCP6TABLE_OWNER_MODULE, table),
    );
    let udp4 = rows::<MIB_UDPROW_OWNER_MODULE>(
        true,
        AF_INET as u32,
        std::mem::offset_of!(MIB_UDPTABLE_OWNER_MODULE, table),
    );
    let udp6 = rows::<MIB_UDP6ROW_OWNER_MODULE>(
        true,
        AF_INET6 as u32,
        std::mem::offset_of!(MIB_UDP6TABLE_OWNER_MODULE, table),
    );
    for pid in tcp4
        .iter()
        .map(|r| r.dwOwningPid)
        .chain(tcp6.iter().map(|r| r.dwOwningPid))
        .chain(udp4.iter().map(|r| r.dwOwningPid))
        .chain(udp6.iter().map(|r| r.dwOwningPid))
    {
        if tree.get(&pid).is_some_and(|(_, name)| {
            matches!(
                name.as_str(),
                "svchost.exe" | "codex.exe" | "msedgewebview2.exe" | "chatgpt.exe"
            )
        }) {
            hydrate(pid, &tree, &mut observation.processes, 0);
        }
    }
    // System service DACLs often deny OpenProcess to an ordinary desktop app.
    // SCM's active PID and configured host binary plus the OS creation snapshot
    // supply the same checks without requesting elevation or assuming a path.
    let mut starts = None;
    for pid in tcp4
        .iter()
        .map(|r| r.dwOwningPid)
        .chain(tcp6.iter().map(|r| r.dwOwningPid))
        .chain(udp4.iter().map(|r| r.dwOwningPid))
        .chain(udp6.iter().map(|r| r.dwOwningPid))
    {
        if observation.processes.contains_key(&pid)
            || tree.get(&pid).is_none_or(|(_, name)| name != "svchost.exe")
        {
            continue;
        }
        let Some(choices) = services.get(&pid) else {
            continue;
        };
        let Some((created, parent)) = starts.get_or_insert_with(creation_times).get(&pid) else {
            continue;
        };
        if tree
            .get(&pid)
            .is_none_or(|(expected, _)| expected != parent)
        {
            continue;
        }
        if let Some(path) = choices.iter().find_map(|(name, _)| service_binary(name)) {
            observation.processes.insert(
                pid,
                Process {
                    pid,
                    parent: *parent,
                    path,
                    created: *created,
                },
            );
        }
    }
    for row in tcp4 {
        if row.dwState == MIB_TCP_STATE_LISTEN as u32 {
            observation.listeners.push((
                row.dwOwningPid,
                IpAddr::V4(Ipv4Addr::from(row.dwLocalAddr.to_ne_bytes())),
                u16::from_be(row.dwLocalPort as u16),
            ));
        }
        if !observation.processes.contains_key(&row.dwOwningPid) {
            continue;
        }
        observation.endpoints.push(Endpoint {
            network: "tcp",
            local: IpAddr::V4(Ipv4Addr::from(row.dwLocalAddr.to_ne_bytes())),
            port: u16::from_be(row.dwLocalPort as u16),
            remote: Some((
                IpAddr::V4(Ipv4Addr::from(row.dwRemoteAddr.to_ne_bytes())),
                u16::from_be(row.dwRemotePort as u16),
            )),
            pid: row.dwOwningPid,
            created: timestamp(row.liCreateTimestamp),
            service: service(
                row.dwOwningPid,
                &observation.processes,
                &services,
                |buffer, bytes| unsafe {
                    GetOwnerModuleFromTcpEntry(&row, TCPIP_OWNER_MODULE_INFO_BASIC, buffer, bytes)
                },
            ),
        });
    }
    for row in tcp6 {
        if row.dwState == MIB_TCP_STATE_LISTEN as u32 {
            observation.listeners.push((
                row.dwOwningPid,
                IpAddr::V6(Ipv6Addr::from(row.ucLocalAddr)),
                u16::from_be(row.dwLocalPort as u16),
            ));
        }
        if !observation.processes.contains_key(&row.dwOwningPid) {
            continue;
        }
        observation.endpoints.push(Endpoint {
            network: "tcp",
            local: IpAddr::V6(Ipv6Addr::from(row.ucLocalAddr)),
            port: u16::from_be(row.dwLocalPort as u16),
            remote: Some((
                IpAddr::V6(Ipv6Addr::from(row.ucRemoteAddr)),
                u16::from_be(row.dwRemotePort as u16),
            )),
            pid: row.dwOwningPid,
            created: timestamp(row.liCreateTimestamp),
            service: service(
                row.dwOwningPid,
                &observation.processes,
                &services,
                |buffer, bytes| unsafe {
                    GetOwnerModuleFromTcp6Entry(&row, TCPIP_OWNER_MODULE_INFO_BASIC, buffer, bytes)
                },
            ),
        });
    }
    for row in udp4 {
        if !observation.processes.contains_key(&row.dwOwningPid) {
            continue;
        }
        observation.endpoints.push(Endpoint {
            network: "udp",
            local: IpAddr::V4(Ipv4Addr::from(row.dwLocalAddr.to_ne_bytes())),
            port: u16::from_be(row.dwLocalPort as u16),
            remote: None,
            pid: row.dwOwningPid,
            created: timestamp(row.liCreateTimestamp),
            service: service(
                row.dwOwningPid,
                &observation.processes,
                &services,
                |buffer, bytes| unsafe {
                    GetOwnerModuleFromUdpEntry(&row, TCPIP_OWNER_MODULE_INFO_BASIC, buffer, bytes)
                },
            ),
        });
    }
    for row in udp6 {
        if !observation.processes.contains_key(&row.dwOwningPid) {
            continue;
        }
        observation.endpoints.push(Endpoint {
            network: "udp",
            local: IpAddr::V6(Ipv6Addr::from(row.ucLocalAddr)),
            port: u16::from_be(row.dwLocalPort as u16),
            remote: None,
            pid: row.dwOwningPid,
            created: timestamp(row.liCreateTimestamp),
            service: service(
                row.dwOwningPid,
                &observation.processes,
                &services,
                |buffer, bytes| unsafe {
                    GetOwnerModuleFromUdp6Entry(&row, TCPIP_OWNER_MODULE_INFO_BASIC, buffer, bytes)
                },
            ),
        });
    }
    observation
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_service_pid_requires_a_matching_native_module_name() {
        let processes = [(
            42,
            Process {
                pid: 42,
                parent: 0,
                path: r"C:\Windows\System32\svchost.exe".into(),
                created: 100,
            },
        )]
        .into();
        let mut services: HashMap<_, _> =
            [(42, vec![("Dnscache".into(), "DNS Client".into())])].into();
        assert_eq!(
            service(42, &processes, &services, |_, _| panic!(
                "single service should not need a module lookup"
            ))
            .unwrap()
            .key,
            "service:dnscache"
        );
        services
            .get_mut(&42)
            .unwrap()
            .push(("wuauserv".into(), "Windows Update".into()));
        assert!(service(42, &processes, &services, |_, _| ERROR_ACCESS_DENIED).is_none());
        for (name, expected) in [("Dnscache", Some("service:dnscache")), ("unrelated", None)] {
            let name: Vec<u16> = name.encode_utf16().chain([0]).collect();
            let result = service(42, &processes, &services, |buffer, bytes| {
                unsafe {
                    let string = buffer.cast::<u16>().add(32);
                    std::ptr::copy_nonoverlapping(name.as_ptr(), string, name.len());
                    buffer.cast::<TCPIP_OWNER_MODULE_BASIC_INFO>().write(
                        TCPIP_OWNER_MODULE_BASIC_INFO {
                            pModuleName: string,
                            pModulePath: std::ptr::null_mut(),
                        },
                    );
                }
                *bytes = 128;
                ERROR_SUCCESS
            });
            assert_eq!(result.as_ref().map(|owner| owner.key.as_str()), expected);
        }
    }
    #[test]
    #[ignore = "reads this computer's live process, socket and Windows service metadata"]
    fn inspect_native_service_observation() {
        let start = std::time::Instant::now();
        let snapshot = observe();
        println!(
            "native: processes={} endpoints={} listeners={} services={} elapsed_ms={}",
            snapshot.processes.len(),
            snapshot.endpoints.len(),
            snapshot.listeners.len(),
            snapshot
                .endpoints
                .iter()
                .filter(|endpoint| endpoint.service.is_some())
                .count(),
            start.elapsed().as_millis()
        );
        let services: std::collections::BTreeSet<_> = snapshot
            .endpoints
            .iter()
            .filter_map(|endpoint| endpoint.service.as_ref().map(|owner| owner.label.as_str()))
            .collect();
        for label in services {
            println!("service: {label}");
        }
    }
}
