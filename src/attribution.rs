//! Exact process observations enrich application presentation without replacing raw accounting.
use crate::model::{Connection, Snapshot, app_name};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    net::IpAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Owner {
    pub key: String,
    pub label: String,
    pub icon: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Attribution {
    pub owner: Owner,
    pub pid: u32,
}
fn normalized(path: &str) -> String {
    path.strip_prefix(r"\\?\")
        .unwrap_or(path)
        .replace('/', "\\")
        .to_lowercase()
}
pub(crate) const PACKAGE_KEY_PREFIX: &str = "package:";

pub(crate) fn package_owner(path: &str) -> Option<Owner> {
    use windows_sys::Win32::Storage::Packaging::Appx::PackageFamilyNameFromFullName;

    let mut components = path
        .strip_prefix(r"\\?\")
        .unwrap_or(path)
        .split(['\\', '/']);
    let drive = components.next()?.as_bytes();
    if !matches!(drive, [letter, b':'] if letter.is_ascii_alphabetic()) {
        return None;
    }
    match components.next()?.to_ascii_lowercase().as_str() {
        "windowsapps" => {}
        "program files" if components.next()?.eq_ignore_ascii_case("WindowsApps") => {}
        _ => return None,
    }
    let full_name = components.next()?;
    if !full_name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return None;
    }
    let mut file = None;
    for component in components {
        if component.is_empty() || component == "." || component == ".." || component.contains(':')
        {
            return None;
        }
        file = Some(component);
    }
    if !file?
        .rsplit_once('.')
        .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("exe"))
    {
        return None;
    }
    // A historical install can be gone from disk. Use the OS parser to
    // validate its full package name and obtain the version-independent ID.
    let full = crate::native::wide(full_name);
    let mut family = [0u16; 128];
    let mut size = family.len() as u32;
    if unsafe { PackageFamilyNameFromFullName(full.as_ptr(), &mut size, family.as_mut_ptr()) } != 0
    {
        return None;
    }
    let family = String::from_utf16(family.get(..size.checked_sub(1)? as usize)?).ok()?;
    let name = full_name.split('_').next()?;
    Some(Owner {
        key: format!("{PACKAGE_KEY_PREFIX}{}", family.to_ascii_lowercase()),
        label: name.rsplit('.').next()?.into(),
        icon: path.into(),
    })
}

fn is_codex_desktop(path: &str) -> bool {
    let path = normalized(path);
    path.split('\\')
        .any(|part| part.starts_with("openai.codex_"))
        && path.ends_with(r"\app\chatgpt.exe")
}
fn application(path: &str) -> Owner {
    package_owner(path).unwrap_or_else(|| Owner {
        key: path.into(),
        label: app_name(path).into(),
        icon: path.into(),
    })
}
#[derive(Clone, Debug)]
pub(super) struct Process {
    pub pid: u32,
    pub parent: u32,
    pub path: String,
    pub created: i64,
}
#[derive(Clone, Debug)]
pub(super) struct Endpoint {
    pub network: &'static str,
    pub local: IpAddr,
    pub port: u16,
    pub remote: Option<(IpAddr, u16)>,
    pub pid: u32,
    pub created: i64,
    pub service: Option<Owner>,
}
#[derive(Clone, Default)]
pub(super) struct Observation {
    pub captured: i64,
    pub processes: HashMap<u32, Process>,
    pub endpoints: Vec<Endpoint>,
    pub listeners: Vec<(u32, IpAddr, u16)>,
}
impl Observation {
    fn owner(&self, pid: u32) -> Option<Owner> {
        let process = self.processes.get(&pid)?;
        let name = app_name(&process.path).to_ascii_lowercase();
        if let Some(owner) = package_owner(&process.path) {
            return Some(owner);
        }
        if !matches!(name.as_str(), "codex.exe" | "msedgewebview2.exe") {
            return None;
        }
        let mut current = process;
        let mut seen = HashSet::new();
        for _ in 0..24 {
            if !seen.insert(current.pid) {
                return None;
            }
            let parent = self.processes.get(&current.parent)?;
            if parent.created > current.created {
                return None;
            }
            if name == "codex.exe" {
                if is_codex_desktop(&parent.path) {
                    return package_owner(&parent.path);
                }
                // Only recognized Codex launch ancestry may absorb a standalone CLI.
                if !matches!(
                    app_name(&parent.path).to_ascii_lowercase().as_str(),
                    "codex.exe" | "node_repl.exe" | "node.exe" | "codex-command-runner.exe"
                ) {
                    return None;
                }
            } else if !app_name(&parent.path).eq_ignore_ascii_case("msedgewebview2.exe") {
                if parent.path.is_empty()
                    || matches!(
                        app_name(&parent.path).to_ascii_lowercase().as_str(),
                        "services.exe" | "explorer.exe" | "svchost.exe"
                    )
                {
                    return None;
                }
                return Some(application(&parent.path));
            }
            current = parent;
        }
        None
    }
    fn resolve(
        &self,
        connection: &Connection,
        now: i64,
        core_pid: Option<u32>,
    ) -> Option<Attribution> {
        if now < self.captured || now - self.captured > 2500 {
            return None;
        }
        let path = normalized(&connection.app());
        let start = connection.started();
        if start <= 0 {
            return None;
        }
        let port: u16 = connection.metadata.source_port.parse().ok()?;
        let remote_port: u16 = connection.metadata.destination_port.parse().ok()?;
        let source: Option<IpAddr> = connection.metadata.source_ip.parse().ok();
        let destination: Option<IpAddr> = connection.metadata.destination_ip.parse().ok();
        let candidates: Vec<_> = self
            .endpoints
            .iter()
            .filter(|endpoint| {
                let Some(process) = self.processes.get(&endpoint.pid) else {
                    return false;
                };
                endpoint
                    .network
                    .eq_ignore_ascii_case(&connection.metadata.network)
                    && endpoint.port == port
                    && normalized(&process.path) == path
                    && process.created <= start + 100
                    && endpoint.created <= start + 100
                    && connection
                        .metadata
                        .process_id
                        .is_none_or(|pid| pid == endpoint.pid)
                    && endpoint.remote.is_none_or(|(ip, port)| {
                        port == remote_port
                            || (ip.is_loopback()
                                && self.listeners.iter().any(|(pid, address, listener)| {
                                    Some(*pid) == core_pid
                                        && *listener == port
                                        && (address.is_unspecified() || *address == ip)
                                }))
                    })
            })
            .collect();
        let exact: Vec<_> = candidates
            .iter()
            .copied()
            .filter(|endpoint| {
                source.is_none_or(|ip| endpoint.local.is_unspecified() || endpoint.local == ip)
                    && endpoint
                        .remote
                        .is_none_or(|(ip, _)| destination == Some(ip))
            })
            .collect();
        // TUN can rewrite destination addresses. A local port/path with one
        // consistent owner is still usable; conflicting candidates remain raw.
        let candidates = if exact.is_empty() { candidates } else { exact };
        let mut resolved: Option<Attribution> = None;
        for endpoint in candidates {
            let owner = if app_name(&path).eq_ignore_ascii_case("svchost.exe") {
                endpoint.service.clone()?
            } else {
                self.owner(endpoint.pid)?
            };
            if resolved
                .as_ref()
                .is_some_and(|old| old.owner.key != owner.key)
            {
                return None;
            }
            resolved = Some(Attribution {
                owner,
                pid: endpoint.pid,
            });
        }
        resolved
    }
}
pub struct Resolver {
    observation: Arc<Mutex<Arc<Observation>>>,
    enabled: Arc<AtomicBool>,
    core: String,
    known: HashMap<(String, String, String), Attribution>,
    observed_at: i64,
}
impl Resolver {
    pub fn new(stop: Arc<AtomicBool>) -> Self {
        let observation = Arc::new(Mutex::new(Arc::new(Observation::default())));
        let worker = observation.clone();
        let enabled = Arc::new(AtomicBool::new(false));
        let needed = enabled.clone();
        let _ = std::thread::Builder::new()
            .name("application-attribution".into())
            .spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    if needed.load(Ordering::Relaxed) {
                        let next = platform::observe();
                        if let Ok(mut current) = worker.lock() {
                            *current = Arc::new(next);
                        }
                    }
                    for _ in 0..10 {
                        if stop.load(Ordering::Relaxed) {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }
            });
        Self {
            observation,
            enabled,
            core: String::new(),
            known: HashMap::new(),
            observed_at: 0,
        }
    }
    pub fn annotate(&mut self, snapshot: &mut Snapshot, core: &str, local: bool, now: i64) {
        self.enabled.store(
            local
                && snapshot
                    .connections
                    .as_deref()
                    .unwrap_or_default()
                    .iter()
                    .any(|connection| {
                        !connection.metadata.source_port.is_empty()
                            && matches!(
                                app_name(&connection.app()).to_ascii_lowercase().as_str(),
                                "codex.exe" | "msedgewebview2.exe" | "svchost.exe"
                            )
                    }),
            Ordering::Relaxed,
        );
        if self.core != core || !local || now < self.observed_at {
            self.known.clear();
            self.core = core.into();
        }
        self.observed_at = now;
        let observation = self.observation.lock().unwrap().clone();
        let aliases: HashMap<_, _> = snapshot
            .connections
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|c| (normalized(&c.app()), c.app()))
            .collect();
        let mut live = HashSet::new();
        for connection in snapshot.connections.as_mut().into_iter().flatten() {
            connection.attribution = None;
            let key = (
                connection.id.clone(),
                connection.start.clone(),
                connection.app(),
            );
            live.insert(key.clone());
            let attribution = self
                .known
                .get(&key)
                .cloned()
                .or_else(|| {
                    if !local {
                        return None;
                    }
                    observation.resolve(
                        connection,
                        now,
                        core.split(':').next().and_then(|value| value.parse().ok()),
                    )
                })
                .or_else(|| package_owner(&key.2).map(|owner| Attribution { owner, pid: 0 }));
            if let Some(mut attribution) = attribution {
                if let Some(raw) = aliases.get(&normalized(&attribution.owner.key)) {
                    attribution.owner.key = raw.clone();
                }
                self.known.insert(key, attribution.clone());
                connection.attribution = Some(attribution);
            }
        }
        self.known.retain(|key, _| live.contains(key));
    }
}
#[path = "attribution_windows.rs"]
mod platform;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Metadata;
    fn fixture() -> (Observation, Connection) {
        let host = Process {
            pid: 10,
            parent: 0,
            path: r"C:\Apps\host.exe".into(),
            created: 100,
        };
        let child = Process {
            pid: 11,
            parent: 10,
            path: r"C:\Runtime\msedgewebview2.exe".into(),
            created: 200,
        };
        let connection = Connection {
            id: "connection".into(),
            start: chrono::DateTime::from_timestamp_millis(10_000)
                .unwrap()
                .to_rfc3339(),
            metadata: Metadata {
                network: "tcp".into(),
                source_ip: "127.0.0.1".into(),
                source_port: "50000".into(),
                destination_ip: "1.2.3.4".into(),
                destination_port: "443".into(),
                process_path: child.path.clone(),
                ..Default::default()
            },
            ..Default::default()
        };
        let endpoint = Endpoint {
            network: "tcp",
            local: "127.0.0.1".parse().unwrap(),
            port: 50000,
            remote: Some(("1.2.3.4".parse().unwrap(), 443)),
            pid: 11,
            created: 9900,
            service: None,
        };
        (
            Observation {
                captured: 10_000,
                processes: [(10, host), (11, child)].into(),
                endpoints: vec![endpoint],
                ..Default::default()
            },
            connection,
        )
    }
    #[test]
    fn webview_owner_requires_live_matching_endpoint_and_valid_ancestry() {
        let (mut snapshot, mut connection) = fixture();
        assert_eq!(
            snapshot
                .resolve(&connection, 10_100, None)
                .unwrap()
                .owner
                .key,
            r"C:\Apps\host.exe"
        );
        assert!(snapshot.resolve(&connection, 13_000, None).is_none());
        connection.metadata.source_port.clear();
        assert!(snapshot.resolve(&connection, 10_100, None).is_none());
        connection.metadata.source_port = "50000".into();
        snapshot.processes.get_mut(&10).unwrap().created = 300;
        assert!(snapshot.resolve(&connection, 10_100, None).is_none());
        snapshot.processes.get_mut(&10).unwrap().created = 100;
        snapshot.endpoints[0].created = 13_000;
        assert!(snapshot.resolve(&connection, 10_100, None).is_none());
    }
    #[test]
    fn conflicting_hosts_stay_raw_and_same_host_connections_can_share_a_port() {
        let (mut snapshot, connection) = fixture();
        snapshot.processes.insert(
            20,
            Process {
                pid: 20,
                parent: 0,
                path: r"C:\Other\other.exe".into(),
                created: 100,
            },
        );
        snapshot.processes.insert(
            21,
            Process {
                pid: 21,
                parent: 20,
                path: connection.app(),
                created: 200,
            },
        );
        let mut endpoint = snapshot.endpoints[0].clone();
        endpoint.pid = 21;
        snapshot.endpoints.push(endpoint);
        assert!(snapshot.resolve(&connection, 10_100, None).is_none());
        snapshot.processes.get_mut(&21).unwrap().parent = 10;
        assert!(snapshot.resolve(&connection, 10_100, None).is_some());
    }
    #[test]
    fn proxy_and_tun_matching_require_a_unique_owner_and_verified_proxy_listener() {
        let (mut snapshot, mut connection) = fixture();
        snapshot.endpoints[0].remote = Some(("127.0.0.1".parse().unwrap(), 7890));
        snapshot
            .listeners
            .push((123, "0.0.0.0".parse().unwrap(), 7890));
        assert!(snapshot.resolve(&connection, 10_100, None).is_none());
        assert!(snapshot.resolve(&connection, 10_100, Some(124)).is_none());
        assert!(snapshot.resolve(&connection, 10_100, Some(123)).is_some());
        snapshot.endpoints[0].remote = Some(("1.2.3.4".parse().unwrap(), 443));
        connection.metadata.destination_ip = "198.18.0.1".into();
        assert!(snapshot.resolve(&connection, 10_100, None).is_some());
        connection.metadata.process_id = Some(21);
        assert!(snapshot.resolve(&connection, 10_100, None).is_none());
    }
    #[test]
    fn codex_desktop_ancestry_does_not_merge_chatgpt_or_standalone_cli() {
        let (mut snapshot, mut connection) = fixture();
        let codex = r"C:\Program Files\WindowsApps\OpenAI.Codex_26.1027.6604.0_x64__2p2nqsd0c76g0\app\ChatGPT.exe";
        snapshot.processes.get_mut(&10).unwrap().path = codex.into();
        let cli = r"C:\Codex\codex.exe";
        snapshot.processes.get_mut(&11).unwrap().path = cli.into();
        connection.metadata.process_path = cli.into();
        assert_eq!(
            snapshot
                .resolve(&connection, 10_100, None)
                .unwrap()
                .owner
                .label,
            "Codex"
        );
        snapshot.processes.get_mut(&10).unwrap().path =
            codex.replace("OpenAI.Codex_", "OpenAI.ChatGPT-Desktop_");
        assert!(snapshot.resolve(&connection, 10_100, None).is_none());
        snapshot.processes.get_mut(&10).unwrap().path = r"C:\Windows\powershell.exe".into();
        assert!(snapshot.resolve(&connection, 10_100, None).is_none());
    }
    #[test]
    fn services_require_an_observed_service_and_do_not_guess_shared_hosts() {
        let (mut snapshot, mut connection) = fixture();
        connection.metadata.process_path = r"C:\Windows\System32\svchost.exe".into();
        snapshot.processes.get_mut(&11).unwrap().path = connection.app();
        assert!(snapshot.resolve(&connection, 10_100, None).is_none());
        snapshot.endpoints[0].service = Some(Owner {
            key: "service:wuauserv".into(),
            label: "Windows Update".into(),
            icon: connection.app(),
        });
        assert_eq!(
            snapshot
                .resolve(&connection, 10_100, None)
                .unwrap()
                .owner
                .key,
            "service:wuauserv"
        );
    }
    #[test]
    fn frozen_ownership_survives_parent_exit_but_not_core_change_or_clock_rollback() {
        let (observation, connection) = fixture();
        let mut resolver = Resolver {
            observation: Arc::new(Mutex::new(Arc::new(observation))),
            enabled: Arc::new(AtomicBool::new(false)),
            core: String::new(),
            known: HashMap::new(),
            observed_at: 0,
        };
        let mut snapshot = Snapshot {
            connections: Some(vec![connection]),
            ..Default::default()
        };
        resolver.annotate(&mut snapshot, "123:1", true, 10_100);
        assert!(
            snapshot.connections.as_ref().unwrap()[0]
                .attribution
                .is_some()
        );
        *resolver.observation.lock().unwrap() = Arc::new(Observation::default());
        resolver.annotate(&mut snapshot, "123:1", true, 20_000);
        assert!(
            snapshot.connections.as_ref().unwrap()[0]
                .attribution
                .is_some()
        );
        resolver.annotate(&mut snapshot, "123:1", true, 19_000);
        assert!(
            snapshot.connections.as_ref().unwrap()[0]
                .attribution
                .is_none()
        );
        resolver.known.insert(
            (
                snapshot.connections.as_ref().unwrap()[0].id.clone(),
                snapshot.connections.as_ref().unwrap()[0].start.clone(),
                snapshot.connections.as_ref().unwrap()[0].app(),
            ),
            Attribution {
                owner: application("host"),
                pid: 11,
            },
        );
        resolver.annotate(&mut snapshot, "123:2", true, 20_000);
        assert!(
            snapshot.connections.as_ref().unwrap()[0]
                .attribution
                .is_none()
        );
        resolver.annotate(&mut snapshot, "123:2", false, 20_100);
        assert!(resolver.known.is_empty());
    }
}
