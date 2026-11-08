//! Client-specific durable settings. Generated core configs are never edited.
//! File changes retain the original text outside the fields we own.
use crate::{config::ProcessMode, native};
use anyhow::{Context, Result, bail, ensure};
use serde_yaml::Value as Yaml;
use std::{
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
};

const FIELD: &str = "find-process-mode";
const NYAN_UID: &str = "mihomo-traffic-process-mode";
const NYAN_FILE: &str = "mihomo-traffic-process-mode.yaml";
const LIMIT: u64 = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Verge,
    Nyanpasu,
    FlClash,
}
impl Kind {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "Clash Verge Rev" => Some(Self::Verge),
            "Clash Nyanpasu" => Some(Self::Nyanpasu),
            "FlClash" => Some(Self::FlClash),
            _ => None,
        }
    }
    pub fn executable(self) -> &'static str {
        match self {
            Self::Verge => "clash-verge.exe",
            Self::Nyanpasu => "clash-nyanpasu.exe",
            Self::FlClash => "FlClash.exe",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Verge => "Clash Verge Rev",
            Self::Nyanpasu => "Clash Nyanpasu",
            Self::FlClash => "FlClash",
        }
    }
    pub fn needs_exit(self) -> bool {
        self != Self::Verge
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub kind: Kind,
    pub root: PathBuf,
    pub executable: Option<PathBuf>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SavedMode {
    Explicit(ProcessMode),
    Inherited,
    Unknown(String),
    #[default]
    Unavailable,
}
impl SavedMode {
    pub fn label(&self) -> String {
        match self {
            Self::Explicit(mode) => format!("{}（{}）", mode.label(), mode.as_str()),
            Self::Inherited => "继承客户端配置（未设置全局覆写）".into(),
            Self::Unknown(value) => format!("无法确定：{value}"),
            Self::Unavailable => "未获取".into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub mode: SavedMode,
    pub source: PathBuf,
    pub note: String,
}

fn text(path: &Path) -> Result<String> {
    ensure!(
        fs::metadata(path)?.len() <= LIMIT,
        "配置文件超过 16 MB，未读取"
    );
    fs::read_to_string(path).with_context(|| format!("无法读取 {}", path.display()))
}
fn yaml(source: &str) -> Result<Yaml> {
    let value: Yaml = serde_yaml::from_str(source.trim_start_matches('\u{feff}'))
        .context("客户端 YAML 配置无效")?;
    ensure!(value.is_mapping() || value.is_null(), "配置顶层必须是映射");
    Ok(if value.is_null() {
        Yaml::Mapping(Default::default())
    } else {
        value
    })
}
fn mode(value: &Yaml) -> SavedMode {
    match value.get(FIELD) {
        None => SavedMode::Inherited,
        Some(Yaml::String(value)) => ProcessMode::parse(value)
            .map(SavedMode::Explicit)
            .unwrap_or_else(|| SavedMode::Unknown(format!("未知模式 {value}"))),
        Some(_) => SavedMode::Unknown("模式字段类型无效".into()),
    }
}
fn child(root: &Path, name: &str) -> Result<PathBuf> {
    ensure!(
        !name.is_empty()
            && Path::new(name)
                .components()
                .all(|c| matches!(c, Component::Normal(_))),
        "客户端配置引用了不安全的相对路径"
    );
    ensure!(
        !name.contains(':') && !name.contains('\\'),
        "不支持的配置文件路径"
    );
    let path = root.join(name);
    // Existing symlinks/junctions must never redirect a write outside the verified root.
    let parent = path
        .parent()
        .context("配置文件没有父目录")?
        .canonicalize()?;
    ensure!(
        parent.starts_with(root.canonicalize()?),
        "配置文件位于客户端目录以外"
    );
    if path.exists() {
        ensure!(
            path.canonicalize()?.starts_with(root.canonicalize()?),
            "配置引用指向目录以外"
        );
    }
    Ok(path)
}

impl Target {
    fn verge_merge(&self) -> Result<PathBuf> {
        let metadata = yaml(&text(&self.root.join("profiles.yaml"))?)?;
        let items = metadata["items"]
            .as_sequence()
            .context("不支持的 Verge 订阅元数据格式")?;
        let matches: Vec<_> = items
            .iter()
            .filter(|item| item["uid"].as_str() == Some("Merge"))
            .collect();
        ensure!(
            matches.len() == 1 && matches[0]["type"].as_str() == Some("merge"),
            "没有唯一的 Verge 全局扩展配置"
        );
        let file = matches[0]["file"].as_str().context("全局扩展未指定文件")?;
        child(&self.root.join("profiles"), file)
    }
    fn nyan_document(&self) -> Result<(String, Yaml)> {
        let source = text(&self.root.join("profiles.yaml"))?;
        let document = yaml(&source)?;
        ensure!(
            document["_nyanpasu"]["document"].as_str() == Some("profiles")
                && document["_nyanpasu"]["schema_revision"].as_u64() == Some(4),
            "暂不支持此 Nyanpasu 配置结构（支持 2.0 / schema 4）"
        );
        let items = document["items"]
            .as_sequence()
            .context("Nyanpasu items 不是列表")?;
        ensure!(
            items
                .iter()
                .all(|item| matches!(item["type"].as_str(), Some("config" | "transform"))),
            "不支持的 Nyanpasu 配置项"
        );
        for field in ["global_transforms", "valid"] {
            if let Some(value) = document.get(field) {
                ensure!(
                    value
                        .as_sequence()
                        .is_some_and(|list| list.iter().all(|v| v.as_str().is_some())),
                    "Nyanpasu {field} 不是字符串列表"
                );
            }
        }
        Ok((source, document))
    }
    fn nyan_owned(&self, document: &Yaml) -> Result<Option<PathBuf>> {
        let items = document["items"]
            .as_sequence()
            .context("无效的 Nyanpasu 配置项")?;
        let owned: Vec<_> = items
            .iter()
            .filter(|item| item["uid"].as_str() == Some(NYAN_UID))
            .collect();
        ensure!(owned.len() <= 1, "本程序的 Nyanpasu 覆写标识重复");
        let Some(item) = owned.first() else {
            return Ok(None);
        };
        ensure!(
            item["type"].as_str() == Some("transform")
                && item["transform"]["type"].as_str() == Some("overlay")
                && item["transform"]["source"]["type"].as_str() == Some("local")
                && item["transform"]["source"]["binding"]["type"].as_str() == Some("managed")
                && item["name"].as_str() == Some("Mihomo Traffic · 进程识别"),
            "同名 Nyanpasu 覆写不属于本程序，未修改"
        );
        let file = item["transform"]["source"]["binding"]["file"]
            .as_str()
            .context("覆写文件缺失")?;
        ensure!(file == NYAN_FILE, "本程序的覆写文件位置已变化，未修改");
        Ok(Some(child(&self.root.join("profiles"), file)?))
    }
    fn fl_document(&self) -> Result<(String, serde_json::Value, serde_json::Value)> {
        let source = text(&self.root.join("shared_preferences.json"))?;
        let preferences: serde_json::Value =
            serde_json::from_str(&source).context("FlClash 设置不是有效 JSON")?;
        let config: serde_json::Value = serde_json::from_str(
            preferences["flutter.config"]
                .as_str()
                .context("不支持此 FlClash 设置格式：缺少 flutter.config")?,
        )
        .context("FlClash 内层设置无效")?;
        ensure!(
            config["patchClashConfig"].is_object(),
            "不支持此 FlClash 设置结构：缺少 patchClashConfig"
        );
        Ok((source, preferences, config))
    }
    pub fn read(&self) -> Result<Snapshot> {
        match self.kind {
            Kind::Verge => {
                let core = yaml(&text(&self.root.join("config.yaml"))?)?;
                ensure!(
                    core.get("mixed-port").is_some()
                        && core.get("external-controller-pipe").is_some(),
                    "未识别为 Verge Rev Windows 配置目录"
                );
                let path = self.verge_merge()?;
                let value = if path.exists() {
                    yaml(&text(&path)?)?
                } else {
                    Yaml::Mapping(Default::default())
                };
                Ok(Snapshot {
                    mode: mode(&value),
                    source: path,
                    note: "全局扩展配置；全局脚本或订阅扩展仍可能覆盖保存值。".into(),
                })
            }
            Kind::Nyanpasu => {
                let (_, document) = self.nyan_document()?;
                let owned = self.nyan_owned(&document)?;
                let enabled = document["global_transforms"]
                    .as_sequence()
                    .is_some_and(|list| list.iter().any(|v| v.as_str() == Some(NYAN_UID)));
                let (saved, source) = if let Some(path) = owned {
                    let saved = if enabled {
                        mode(&yaml(&text(&path)?)?)
                    } else {
                        SavedMode::Inherited
                    };
                    (saved, path)
                } else {
                    let has_transforms = document["global_transforms"]
                        .as_sequence()
                        .is_some_and(|list| !list.is_empty());
                    (
                        if has_transforms {
                            SavedMode::Unknown("已有全局处理链，本程序尚未设置覆写".into())
                        } else {
                            SavedMode::Inherited
                        },
                        self.root.join("profiles").join(NYAN_FILE),
                    )
                };
                Ok(Snapshot {
                    mode: saved,
                    source,
                    note: "本程序专用全局覆写；保留其他处理链，后续添加的处理步骤仍可能覆盖它。"
                        .into(),
                })
            }
            Kind::FlClash => {
                let (_, _, config) = self.fl_document()?;
                let saved = match config["patchClashConfig"].get(FIELD) {
                    Some(value) => value
                        .as_str()
                        .and_then(ProcessMode::parse)
                        .map(SavedMode::Explicit)
                        .unwrap_or_else(|| SavedMode::Unknown("未知进程模式".into())),
                    None => SavedMode::Inherited,
                };
                Ok(Snapshot {
                    mode: saved,
                    source: self.root.join("shared_preferences.json"),
                    note: "客户端全局设置中的进程识别模式；正常退出客户端后保存，重新启动时读取。"
                        .into(),
                })
            }
        }
    }
    pub fn running(&self) -> Result<bool> {
        Ok(!native::proxy_applications(self.kind.executable())?.is_empty())
    }
    pub fn prepare(&self, desired: ProcessMode) -> Result<ChangeSet> {
        self.prepare_value(Some(desired))
    }
    pub fn prepare_restore(&self) -> Result<ChangeSet> {
        self.prepare_value(None)
    }
    fn prepare_value(&self, desired: Option<ProcessMode>) -> Result<ChangeSet> {
        self.read()?;
        let edit_field = |source: &str| match desired {
            Some(mode) => set_yaml_field(source, FIELD, &Yaml::String(mode.as_str().into())),
            None => remove_yaml_field(source, FIELD),
        };
        let mut edits = Vec::new();
        let mut guards = Vec::new();
        match self.kind {
            Kind::Verge => {
                let metadata = self.root.join("profiles.yaml");
                guards.push((metadata.clone(), fs::read(&metadata)?));
                let path = self.verge_merge()?;
                let before = read_optional(&path)?;
                let source = std::str::from_utf8(before.as_deref().unwrap_or_default())?;
                let after = edit_field(source)?;
                edits.push(Edit {
                    path,
                    before,
                    after: after.into_bytes(),
                });
            }
            Kind::Nyanpasu => {
                let (source, document) = self.nyan_document()?;
                let existing = self.nyan_owned(&document)?;
                if desired.is_none() && existing.is_none() {
                    ensure!(
                        self.read()?.mode == SavedMode::Inherited,
                        "现有全局处理链不属于本程序，无法自动恢复继承"
                    );
                    return Ok(ChangeSet {
                        root: self.root.clone(),
                        edits,
                        guards: vec![(self.root.join("profiles.yaml"), source.into_bytes())],
                    });
                }
                let path = existing
                    .clone()
                    .unwrap_or(self.root.join("profiles").join(NYAN_FILE));
                let before = read_optional(&path)?;
                ensure!(
                    existing.is_some() || before.is_none(),
                    "覆写文件已存在但不属于本程序，未覆盖"
                );
                let overlay = std::str::from_utf8(before.as_deref().unwrap_or_default())?;
                let after = edit_field(overlay)?;
                edits.push(Edit {
                    path,
                    before,
                    after: after.into_bytes(),
                });
                let mut updated = source.clone();
                if existing.is_none() {
                    let item = serde_yaml::to_value(
                        serde_json::json!({"uid":NYAN_UID,"name":"Mihomo Traffic · 进程识别","type":"transform","transform":{"type":"overlay","source":{"type":"local","binding":{"type":"managed","file":NYAN_FILE}}}}),
                    )?;
                    updated = append_yaml_sequence(&updated, "items", item)?;
                }
                if desired.is_some() {
                    updated = append_yaml_sequence(
                        &updated,
                        "global_transforms",
                        Yaml::String(NYAN_UID.into()),
                    )?;
                    updated = append_yaml_sequence(&updated, "valid", Yaml::String(FIELD.into()))?;
                }
                if updated != source
                    || edits
                        .iter()
                        .any(|edit| edit.before.as_deref() != Some(edit.after.as_slice()))
                {
                    let revision = document
                        .get("revision")
                        .map(|revision| revision.as_u64().context("无效的 Nyanpasu 状态修订号"))
                        .transpose()?
                        .unwrap_or(0)
                        .checked_add(1)
                        .context("状态修订号溢出")?;
                    updated = set_yaml_field(&updated, "revision", &Yaml::Number(revision.into()))?;
                }
                edits.push(Edit {
                    path: self.root.join("profiles.yaml"),
                    before: Some(source.into_bytes()),
                    after: updated.into_bytes(),
                });
            }
            Kind::FlClash => {
                let (source, mut preferences, mut config) = self.fl_document()?;
                if desired.is_none() && config["patchClashConfig"].get(FIELD).is_none() {
                    return Ok(ChangeSet {
                        root: self.root.clone(),
                        edits,
                        guards: vec![(
                            self.root.join("shared_preferences.json"),
                            source.into_bytes(),
                        )],
                    });
                }
                if let Some(desired) = desired {
                    config["patchClashConfig"][FIELD] = desired.as_str().into();
                } else {
                    config["patchClashConfig"]
                        .as_object_mut()
                        .unwrap()
                        .remove(FIELD);
                }
                let inner = serde_json::to_string(&config)?;
                let replacement = serde_json::to_string(&inner)?;
                let (start, end) = json_field_span(&source, "flutter.config")?;
                let mut after = source.clone();
                after.replace_range(start..end, &replacement);
                preferences["flutter.config"] = inner.into();
                ensure!(
                    serde_json::from_str::<serde_json::Value>(&after)? == preferences,
                    "FlClash 设置修改后校验不一致"
                );
                edits.push(Edit {
                    path: self.root.join("shared_preferences.json"),
                    before: Some(source.into_bytes()),
                    after: after.into_bytes(),
                });
            }
        }
        edits.retain(|edit| {
            edit.before.as_deref() != Some(edit.after.as_slice())
                && !(edit.before.is_none() && edit.after.is_empty())
        });
        Ok(ChangeSet {
            root: self.root.clone(),
            edits,
            guards,
        })
    }
}

/// Discovery is tied to the client owning the currently verified core, not any
/// installed client. Ambiguous installs require explicit directory selection.
pub fn discover(
    kind: Kind,
    selected: Option<&str>,
    endpoint: &str,
    core_pid: Option<u32>,
) -> Result<Target> {
    let applications = native::proxy_applications(kind.executable())?;
    ensure!(
        applications.len() <= 1,
        "发现多个同类客户端实例，无法唯一关联永久配置"
    );
    let executable = applications.first().map(|app| app.executable.clone());
    let mut roots = Vec::new();
    if let Some(selected) = selected {
        roots.push(PathBuf::from(selected));
    } else {
        let roaming = dirs::config_dir().context("无法读取当前用户配置目录")?;
        match kind {
            Kind::Verge => roots.push(roaming.join("io.github.clash-verge-rev.clash-verge-rev")),
            Kind::Nyanpasu => {
                if let Some(path) = native::nyanpasu_custom_config_dir() {
                    roots.push(path);
                }
                roots.push(roaming.join("Clash Nyanpasu").join("config"));
                if let Some(parent) = executable.as_deref().and_then(Path::parent) {
                    roots.push(parent.join(".config").join("clash-nyanpasu"));
                    roots.push(parent.join(".config").join("Clash Nyanpasu"));
                }
            }
            Kind::FlClash => {
                roots.push(roaming.join("com.follow").join("clash"));
                roots.push(roaming.join("FlClash"));
                if let Some(parent) = executable.as_deref().and_then(Path::parent) {
                    roots.push(parent.join("data"));
                }
            }
        }
    }
    let mut valid = Vec::new();
    let mut last_error = None;
    for root in roots {
        if !root.is_dir() {
            continue;
        }
        let root = root.canonicalize()?;
        if valid.iter().any(|t: &Target| t.root == root) {
            continue;
        }
        let target = Target {
            kind,
            root,
            executable: executable.clone(),
        };
        match target.read() {
            Ok(_) => {
                if kind == Kind::Verge {
                    let config = yaml(&text(&target.root.join("config.yaml"))?)?;
                    let configured = config["external-controller-pipe"]
                        .as_str()
                        .unwrap_or_default();
                    let same_pipe = same_control_target(configured, endpoint);
                    let valid = if endpoint.starts_with(r"\\.\pipe\") {
                        same_pipe
                    } else {
                        let production = verge_pipe_key(configured)
                            .map(|key| format!(r"\\.\pipe\verge-mihomo-production-{key}"));
                        [Some(configured), production.as_deref()]
                            .into_iter()
                            .flatten()
                            .any(|pipe| {
                                native::pipe_owner(pipe)
                                    .ok()
                                    .zip(core_pid)
                                    .is_some_and(|(owner, core)| owner == core)
                            })
                    };
                    if !valid {
                        last_error = Some(anyhow::anyhow!("配置目录与当前内核管道不匹配"));
                        continue;
                    }
                }
                valid.push(target);
            }
            Err(error) => last_error = Some(error),
        }
    }
    ensure!(
        valid.len() <= 1,
        "发现多个配置目录，请选择当前客户端使用的目录"
    );
    valid.pop().ok_or_else(|| {
        last_error
            .unwrap_or_else(|| anyhow::anyhow!("未找到受支持的客户端配置目录，请选择配置目录"))
    })
}

// Verge's service and sidecar use different names for the same owner. Config
// generation can retain the sidecar default even while the service is active.
fn verge_pipe_key(pipe: &str) -> Option<&str> {
    let name = pipe.strip_prefix(r"\\.\pipe\verge-mihomo-")?;
    let key = name
        .strip_prefix("sidecar-release-")
        .or_else(|| name.strip_prefix("production-"))?;
    (key.len() == 64 && key.bytes().all(|c| c.is_ascii_hexdigit())).then_some(key)
}
pub fn same_control_target(a: &str, b: &str) -> bool {
    a == b
        || verge_pipe_key(a)
            .zip(verge_pipe_key(b))
            .is_some_and(|(a, b)| a == b)
}

#[derive(Debug)]
struct Edit {
    path: PathBuf,
    before: Option<Vec<u8>>,
    after: Vec<u8>,
}
pub struct ChangeSet {
    root: PathBuf,
    edits: Vec<Edit>,
    guards: Vec<(PathBuf, Vec<u8>)>,
}
fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::metadata(path) {
        Ok(info) => {
            ensure!(
                info.is_file() && info.len() <= LIMIT,
                "配置文件类型或大小不受支持"
            );
            Ok(Some(fs::read(path)?))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
fn sharing_error(error: &std::io::Error) -> bool {
    use windows_sys::Win32::Foundation::{ERROR_LOCK_VIOLATION, ERROR_SHARING_VIOLATION};
    matches!(
        error.raw_os_error().map(|code| code as u32),
        Some(ERROR_LOCK_VIOLATION | ERROR_SHARING_VIOLATION)
    )
}

fn temporarily_occupied(error: &anyhow::Error, paths: [&Path; 2]) -> bool {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::{
        Foundation::ERROR_ACCESS_DENIED,
        Storage::FileSystem::{DELETE, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE},
    };
    let Some(error) = error.downcast_ref::<std::io::Error>() else {
        return false;
    };
    if sharing_error(error) {
        return true;
    }
    // A handle that blocked MoveFileExW can close before we probe it. Retry
    // ACCESS_DENIED only when a file is shared or we can get DELETE access now.
    error.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32)
        && paths.into_iter().all(|path| {
            if fs::metadata(path).is_ok_and(|info| info.permissions().readonly()) {
                return false;
            }
            match fs::OpenOptions::new()
                .access_mode(DELETE)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
                .open(path)
            {
                Ok(_) => true,
                Err(error) => sharing_error(&error) || error.kind() == std::io::ErrorKind::NotFound,
            }
        })
}

fn ensure_edit_path(path: &Path, root: &Path) -> Result<()> {
    ensure!(
        path.parent()
            .context("无效的配置路径")?
            .canonicalize()?
            .starts_with(root),
        "写入路径不属于当前客户端"
    );
    if path.exists() {
        ensure!(
            path.canonicalize()?.starts_with(root),
            "配置文件指向客户端目录以外"
        );
    }
    Ok(())
}

fn atomic_write(path: &Path, bytes: &[u8], before_retry: impl Fn() -> Result<()>) -> Result<()> {
    let temp = path.with_file_name(format!(".mihomo-traffic-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        for delay_ms in [25, 50, 100] {
            match native::replace_file(&temp, path) {
                Err(error) if temporarily_occupied(&error, [&temp, path]) => {
                    std::thread::sleep(std::time::Duration::from_millis(delay_ms));
                    before_retry()?;
                }
                result => return result,
            }
        }
        native::replace_file(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}
impl ChangeSet {
    pub fn commit(
        &self,
        backup_root: &Path,
        current: impl Fn() -> bool,
    ) -> Result<Option<PathBuf>> {
        ensure!(current(), "连接或操作已变化，已取消保存");
        let root = self.root.canonicalize()?;
        for (path, bytes) in &self.guards {
            ensure!(
                fs::read(path)? == *bytes,
                "客户端配置已被其他程序修改，请刷新后重试"
            );
        }
        for edit in &self.edits {
            ensure_edit_path(&edit.path, &root)?;
            ensure!(
                read_optional(&edit.path)? == edit.before,
                "客户端配置已被其他程序修改，请刷新后重试"
            );
        }
        if self.edits.is_empty() {
            return Ok(None);
        }
        let backup = backup_root.join(format!(
            "{}-{}",
            chrono::Local::now().format("%Y%m%d-%H%M%S"),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&backup)?;
        let mut manifest = Vec::new();
        for (index, edit) in self.edits.iter().enumerate() {
            if let Some(bytes) = &edit.before {
                let mut file = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(backup.join(format!("{index}.original")))?;
                file.write_all(bytes)?;
                file.sync_all()?;
            }
            manifest.push(serde_json::json!({"path":edit.path,"existed":edit.before.is_some(),"backup":format!("{index}.original")}));
        }
        atomic_write(
            &backup.join("manifest.json"),
            &serde_json::to_vec_pretty(&manifest)?,
            || Ok(()),
        )?;
        let mut written: Vec<&Edit> = Vec::new();
        let result = (|| -> Result<()> {
            for edit in &self.edits {
                let still_current = || {
                    ensure!(current(), "连接或操作已变化，已取消保存");
                    ensure_edit_path(&edit.path, &root)?;
                    for (path, bytes) in &self.guards {
                        ensure!(
                            fs::read(path)? == *bytes,
                            "客户端元数据已变化，请刷新后重试"
                        );
                    }
                    ensure!(
                        read_optional(&edit.path)? == edit.before,
                        "客户端配置发生并发修改，未覆盖"
                    );
                    Ok(())
                };
                still_current()?;
                atomic_write(&edit.path, &edit.after, still_current)?;
                written.push(edit);
                ensure!(fs::read(&edit.path)? == edit.after, "配置回读不一致");
            }
            Ok(())
        })();
        if let Err(error) = result {
            let mut rollback_errors = Vec::new();
            for edit in written.into_iter().rev() {
                let rollback = (|| -> Result<()> {
                    let still_written = || {
                        ensure_edit_path(&edit.path, &root)?;
                        ensure!(
                            read_optional(&edit.path)?.as_deref() == Some(edit.after.as_slice()),
                            "回滚前发现外部修改，保留外部内容"
                        );
                        Ok(())
                    };
                    still_written()?;
                    match &edit.before {
                        Some(bytes) => atomic_write(&edit.path, bytes, still_written),
                        None => fs::remove_file(&edit.path).map_err(Into::into),
                    }
                })();
                if let Err(e) = rollback {
                    rollback_errors.push(e.to_string());
                }
            }
            if rollback_errors.is_empty() {
                return Err(error.context("保存失败，本次写入已回滚"));
            }
            bail!(
                "{error:#}；部分回滚失败：{}；备份：{}",
                rollback_errors.join("；"),
                backup.display()
            );
        }
        Ok(Some(backup))
    }
}

// A conservative text editor for root YAML fields. It rejects representations
// it cannot locate instead of reserializing the user's complete document.
fn field_span(source: &str, key: &str) -> Result<Option<(usize, usize, usize)>> {
    let mut offset = 0;
    let mut found = None;
    let mut end = source.len();
    for line in source.split_inclusive('\n') {
        let plain = line.trim_start_matches('\u{feff}');
        if !plain.starts_with(char::is_whitespace)
            && !plain.starts_with(['#', '-', '.', '%'])
            && let Some(colon) = plain.find(':')
        {
            let candidate = plain[..colon].trim();
            let parsed = serde_yaml::from_str::<String>(candidate).ok();
            if parsed.as_deref() == Some(key) {
                ensure!(found.is_none(), "配置包含重复字段 {key}");
                let prefix = line.len() - plain.len();
                found = Some((offset, offset + prefix + colon + 1));
            } else if found.is_some() && end == source.len() {
                end = offset;
            }
        }
        offset += line.len();
    }
    Ok(found.map(|(start, value)| (start, value, end)))
}
fn newline(source: &str) -> &'static str {
    if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}
fn serialized(value: &Yaml, nl: &str) -> Result<String> {
    Ok(serde_yaml::to_string(value)?.trim_end().replace('\n', nl))
}
fn set_yaml_field(source: &str, key: &str, value: &Yaml) -> Result<String> {
    let before = yaml(source)?;
    let nl = newline(source);
    let value_text = serialized(value, nl)?;
    let replacement = if value.is_mapping() || value.is_sequence() {
        format!(
            "{nl}{}{nl}",
            value_text
                .lines()
                .map(|line| format!("  {line}"))
                .collect::<Vec<_>>()
                .join(nl)
        )
    } else {
        format!(" {value_text}{nl}")
    };
    let mut result = source.to_owned();
    if let Some((_, start, end)) = field_span(source, key)? {
        let old = &source[start..end];
        ensure!(
            !old.contains('&') && !old.contains('*'),
            "字段 {key} 使用了 YAML 锚点，请先在客户端中整理该字段"
        );
        let comments = old
            .lines()
            .filter_map(|line| line.find('#').map(|index| &line[index..]))
            .collect::<Vec<_>>()
            .join(nl);
        let replacement = if comments.is_empty() {
            replacement
        } else {
            format!("{replacement}{comments}{nl}")
        };
        result.replace_range(start..end, &replacement);
    } else {
        ensure!(before.get(key).is_none(), "无法安全定位字段 {key}，未修改");
        ensure!(
            !source
                .trim_start_matches('\u{feff}')
                .trim_start()
                .starts_with('{'),
            "暂不支持顶层行内 YAML 映射"
        );
        if !result.is_empty() && !result.ends_with('\n') {
            result.push_str(nl);
        }
        result.push_str(&format!("{key}:{replacement}"));
    }
    let mut expected = before;
    expected
        .as_mapping_mut()
        .unwrap()
        .insert(key.into(), value.clone());
    ensure!(
        yaml(&result)? == expected,
        "修改后的 YAML 与预期不一致，未保存"
    );
    Ok(result)
}
fn remove_yaml_field(source: &str, key: &str) -> Result<String> {
    let mut expected = yaml(source)?;
    if expected.get(key).is_none() {
        return Ok(source.into());
    }
    let (start, value, end) =
        field_span(source, key)?.context("无法安全定位要恢复的 YAML 字段，未修改")?;
    ensure!(
        !source[value..end].contains(['&', '*']),
        "字段使用了 YAML 锚点，请先在客户端整理后再恢复"
    );
    let mut retained = String::new();
    if start == 0 && source.starts_with('\u{feff}') {
        retained.push('\u{feff}');
    }
    for line in source[start..end].split_inclusive('\n') {
        let plain = line.trim_start_matches('\u{feff}');
        if plain.trim().is_empty() || plain.trim_start().starts_with('#') || plain.trim() == "..." {
            retained.push_str(plain);
        } else if let Some(comment) = plain.find('#') {
            retained.push_str(&plain[comment..]);
        }
    }
    let mut result = source.to_owned();
    result.replace_range(start..end, &retained);
    expected
        .as_mapping_mut()
        .unwrap()
        .remove(Yaml::String(key.into()));
    ensure!(
        yaml(&result)? == expected,
        "恢复后的 YAML 校验不一致，未保存"
    );
    Ok(result)
}
fn append_yaml_sequence(source: &str, key: &str, item: Yaml) -> Result<String> {
    let before = yaml(source)?;
    let mut sequence = match before.get(key) {
        None => Vec::new(),
        Some(v) => v.as_sequence().context("目标字段不是列表")?.clone(),
    };
    if sequence.contains(&item) {
        return Ok(source.into());
    }
    sequence.push(item.clone());
    let Some((_, start, end)) = field_span(source, key)? else {
        return set_yaml_field(source, key, &Yaml::Sequence(sequence));
    };
    let old = &source[start..end];
    let first = old.lines().next().unwrap_or_default().trim();
    if !first.is_empty() && !first.starts_with('#') {
        return set_yaml_field(source, key, &Yaml::Sequence(sequence));
    }
    let indent = old
        .lines()
        .find_map(|line| {
            line.trim_start()
                .starts_with("- ")
                .then_some(line.len() - line.trim_start().len())
        })
        .unwrap_or(2);
    let nl = newline(source);
    let block = serialized(&Yaml::Sequence(vec![item]), nl)?
        .lines()
        .map(|line| format!("{}{line}", " ".repeat(indent)))
        .collect::<Vec<_>>()
        .join(nl);
    let mut result = source.to_owned();
    // Keep root-level comments attached to the following field, rather than
    // moving them into the new sequence item.
    let mut insertion = end;
    for line in source[start..end].split_inclusive('\n').rev() {
        if line.trim().is_empty() || line.starts_with('#') {
            insertion -= line.len();
        } else {
            break;
        }
    }
    let first_newline = source[start..end].find('\n').map_or(end, |n| start + n + 1);
    insertion = insertion.max(first_newline);
    let separator = if source[..insertion].ends_with('\n') {
        ""
    } else {
        nl
    };
    result.insert_str(insertion, &format!("{separator}{block}{nl}"));
    let mut expected = before;
    expected
        .as_mapping_mut()
        .unwrap()
        .insert(key.into(), Yaml::Sequence(sequence));
    ensure!(yaml(&result)? == expected, "添加列表项后校验失败，未保存");
    Ok(result)
}
fn json_field_span(source: &str, wanted: &str) -> Result<(usize, usize)> {
    fn skip(source: &str, cursor: &mut usize) {
        while source
            .as_bytes()
            .get(*cursor)
            .is_some_and(u8::is_ascii_whitespace)
        {
            *cursor += 1;
        }
    }
    let mut cursor = 0;
    skip(source, &mut cursor);
    ensure!(
        source.as_bytes().get(cursor) == Some(&b'{'),
        "设置不是 JSON 对象"
    );
    cursor += 1;
    loop {
        skip(source, &mut cursor);
        ensure!(
            source.as_bytes().get(cursor) != Some(&b'}'),
            "没有找到设置字段 {wanted}"
        );
        let mut stream =
            serde_json::Deserializer::from_str(&source[cursor..]).into_iter::<String>();
        let key = stream.next().context("缺少 JSON 键")??;
        cursor += stream.byte_offset();
        skip(source, &mut cursor);
        ensure!(
            source.as_bytes().get(cursor) == Some(&b':'),
            "JSON 缺少冒号"
        );
        cursor += 1;
        skip(source, &mut cursor);
        let start = cursor;
        let mut stream =
            serde_json::Deserializer::from_str(&source[cursor..]).into_iter::<serde_json::Value>();
        stream.next().context("缺少 JSON 值")??;
        cursor += stream.byte_offset();
        if key == wanted {
            return Ok((start, cursor));
        }
        skip(source, &mut cursor);
        ensure!(
            source.as_bytes().get(cursor) == Some(&b','),
            "没有找到设置字段 {wanted}"
        );
        cursor += 1;
    }
}

#[cfg(test)]
#[path = "client_config_tests.rs"]
mod tests;
