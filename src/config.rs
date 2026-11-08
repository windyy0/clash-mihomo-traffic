mod credential;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum TaskbarPositionMode {
    #[default]
    AutoAvoid,
    Manual,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProcessMode {
    #[default]
    Strict,
    Always,
    Off,
}
impl ProcessMode {
    pub const ALL: [Self; 3] = [Self::Strict, Self::Always, Self::Off];
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.as_str() == value)
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Strict => "strict",
            Self::Always => "always",
            Self::Off => "off",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Strict => "按需",
            Self::Always => "始终",
            Self::Off => "关闭",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ProcessControl {
    #[default]
    Unmanaged,
    ApplyOnce(ProcessMode),
    Maintain(ProcessMode),
}

impl<'de> Deserialize<'de> for ProcessControl {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "kebab-case")]
        enum Wire {
            Unmanaged,
            KeepAlways,
            RestoreStrict,
            ApplyOnce(ProcessMode),
            Maintain(ProcessMode),
        }
        Ok(match Wire::deserialize(deserializer)? {
            Wire::Unmanaged => Self::Unmanaged,
            Wire::KeepAlways => Self::Maintain(ProcessMode::Always),
            Wire::RestoreStrict => Self::ApplyOnce(ProcessMode::Strict),
            Wire::ApplyOnce(mode) => Self::ApplyOnce(mode),
            Wire::Maintain(mode) => Self::Maintain(mode),
        })
    }
}

impl ProcessControl {
    // Source compatibility for the pre-migration regression fixtures.
    #[cfg(test)]
    #[allow(non_upper_case_globals)]
    pub const KeepAlways: Self = Self::Maintain(ProcessMode::Always);
    #[cfg(test)]
    #[allow(non_upper_case_globals)]
    pub const RestoreStrict: Self = Self::ApplyOnce(ProcessMode::Strict);
    pub fn target(self) -> Option<&'static str> {
        match self {
            Self::Unmanaged => None,
            Self::ApplyOnce(mode) | Self::Maintain(mode) => Some(mode.as_str()),
        }
    }
    pub fn maintaining(self) -> bool {
        matches!(self, Self::Maintain(_))
    }
    pub fn once(self) -> bool {
        matches!(self, Self::ApplyOnce(_))
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum TrayIconStyle {
    Monochrome,
    #[default]
    Color,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RecentUsageRefreshMode {
    Always,
    Manual,
    #[default]
    AfterFiveMinutes,
}
impl RecentUsageRefreshMode {
    pub const ALL: [Self; 3] = [Self::Always, Self::Manual, Self::AfterFiveMinutes];

    pub fn label(self) -> &'static str {
        match self {
            Self::Always => "每次点击刷新",
            Self::Manual => "仅手动刷新",
            Self::AfterFiveMinutes => "超过5分钟刷新",
        }
    }

    pub fn should_refresh(self, last_success: Option<i64>, now: i64) -> bool {
        let Some(last_success) = last_success else {
            return true;
        };
        match self {
            Self::Always => true,
            Self::Manual => false,
            Self::AfterFiveMinutes => {
                now < last_success || now.saturating_sub(last_success) > 5 * 60 * 1000
            }
        }
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetentionPolicy {
    pub days: Option<u32>,
    pub max_bytes: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RetentionTimeUnit {
    #[default]
    Days,
    Months,
}
impl RetentionTimeUnit {
    pub const ALL: [Self; 2] = [Self::Days, Self::Months];
    pub fn label(self) -> &'static str {
        match self {
            Self::Days => "天",
            Self::Months => "月",
        }
    }
    pub fn multiplier(self) -> u64 {
        match self {
            Self::Days => 1,
            Self::Months => 30,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RetentionSizeUnit {
    Mb,
    #[default]
    Gb,
}
impl RetentionSizeUnit {
    pub const ALL: [Self; 2] = [Self::Mb, Self::Gb];
    pub fn label(self) -> &'static str {
        match self {
            Self::Mb => "MB",
            Self::Gb => "GB",
        }
    }
    pub fn multiplier(self) -> u64 {
        match self {
            Self::Mb => 1,
            Self::Gb => 1024,
        }
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub endpoint: String,
    #[serde(skip_serializing)]
    pub secret: String,
    #[serde(skip)]
    pub(crate) locked_secret: Option<toml::Value>,
    pub process_control: ProcessControl,
    pub process_endpoint: Option<String>,
    pub process_client: Option<String>,
    pub client_config_dir: Option<String>,
    #[serde(default = "Config::legacy_theme")]
    pub theme: String,
    #[serde(default)]
    pub color_theme: crate::theme::ColorTheme,
    pub collapsed: bool,
    pub close_to_tray: bool,
    pub tray_visible: bool,
    pub tray_icon_style: TrayIconStyle,
    pub open_hotkey: String,
    pub recent_usage_refresh_mode: RecentUsageRefreshMode,
    pub autorun: bool,
    pub update_check_frequency: crate::release_update::CheckFrequency,
    pub floating: bool,
    pub floating_locked: bool,
    pub floating_today: bool,
    pub floating_content: crate::taskbar_content::TaskbarContent,
    pub floating_row_gap: u8,
    pub floating_background_color: Option<u32>,
    pub floating_background_transparency: u8,
    pub floating_x: Option<i32>,
    pub floating_y: Option<i32>,
    /// Applies only to the floating window; retain this key for existing configs.
    pub fullscreen_hide: bool,
    pub taskbar: bool,
    pub taskbar_horizontal: bool,
    pub taskbar_content: crate::taskbar_content::TaskbarContent,
    pub taskbar_left: bool,
    pub taskbar_offset: i32,
    pub taskbar_text_layout_version: u8,
    pub taskbar_position_mode: TaskbarPositionMode,
    pub taskbar_auto_prefer_left: bool,
    pub retention_days: u32,
    pub retention_days_enabled: bool,
    pub retention_size_enabled: bool,
    pub retention_size_mb: u64,
    pub retention_time_unit: RetentionTimeUnit,
    pub retention_size_unit: RetentionSizeUnit,
    pub backup_dir: Option<String>,
    pub seconds_enabled: bool,
    pub seconds_days: u32,
    pub seconds_days_enabled: bool,
    pub seconds_time_unit: RetentionTimeUnit,
    pub seconds_size_enabled: bool,
    pub seconds_size_mb: u64,
    pub seconds_size_unit: RetentionSizeUnit,
    pub storage_limit_enabled: bool,
    pub storage_limit_mb: u64,
    pub storage_limit_unit: RetentionSizeUnit,
    pub storage_limit_migration_notice: bool,
}
impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match toml::Value::try_from(self) {
            Ok(mut fields) => {
                if let Some(table) = fields.as_table_mut() {
                    table.remove("endpoint");
                    table.remove("process_endpoint");
                }
                fmt::Debug::fmt(&fields, f)
            }
            Err(_) => f.write_str("Config { .. }"),
        }
    }
}
impl Default for Config {
    fn default() -> Self {
        Self {
            endpoint: "auto".into(),
            secret: String::new(),
            locked_secret: None,
            process_control: ProcessControl::Unmanaged,
            process_endpoint: None,
            process_client: None,
            client_config_dir: None,
            theme: "system".into(),
            color_theme: crate::theme::ColorTheme::Mica,
            collapsed: false,
            close_to_tray: true,
            tray_visible: true,
            tray_icon_style: TrayIconStyle::default(),
            open_hotkey: crate::hotkey::DEFAULT.into(),
            recent_usage_refresh_mode: RecentUsageRefreshMode::default(),
            autorun: false,
            update_check_frequency: Default::default(),
            floating: false,
            floating_locked: false,
            floating_today: false,
            floating_content: crate::taskbar_content::TaskbarContent::floating(),
            floating_row_gap: 8,
            floating_background_color: None,
            floating_background_transparency: 0,
            floating_x: None,
            floating_y: None,
            fullscreen_hide: true,
            taskbar: false,
            taskbar_horizontal: false,
            taskbar_content: Default::default(),
            taskbar_left: false,
            taskbar_offset: 0,
            taskbar_text_layout_version: crate::taskbar_text::LAYOUT_VERSION,
            taskbar_position_mode: TaskbarPositionMode::AutoAvoid,
            taskbar_auto_prefer_left: false,
            retention_days: 90,
            retention_days_enabled: true,
            retention_size_enabled: false,
            retention_size_mb: 1024,
            retention_time_unit: RetentionTimeUnit::Months,
            retention_size_unit: RetentionSizeUnit::Gb,
            backup_dir: None,
            seconds_enabled: true,
            seconds_days: 3,
            seconds_days_enabled: true,
            seconds_time_unit: RetentionTimeUnit::Days,
            seconds_size_enabled: false,
            seconds_size_mb: 2048,
            seconds_size_unit: RetentionSizeUnit::Gb,
            storage_limit_enabled: false,
            storage_limit_mb: 2048,
            storage_limit_unit: RetentionSizeUnit::Gb,
            storage_limit_migration_notice: false,
        }
    }
}
impl Config {
    pub fn backup_directory(&self) -> Result<PathBuf> {
        if let Some(path) = self.backup_dir.as_deref().filter(|p| !p.trim().is_empty()) {
            anyhow::ensure!(Path::new(path).is_absolute(), "备份目录须为绝对路径");
            return Ok(PathBuf::from(path));
        }
        default_backup_directory()
    }
    #[cfg(test)]
    pub fn retention_policy(&self) -> RetentionPolicy {
        RetentionPolicy {
            days: self.retention_days_enabled.then_some(self.retention_days),
            max_bytes: self
                .retention_size_enabled
                .then_some(self.retention_size_mb * 1024 * 1024),
        }
    }
    fn legacy_theme() -> String {
        "light".into()
    }

    pub fn load(dir: &Path) -> Result<Self> {
        Self::read(dir).map(|(config, _)| config)
    }

    pub fn load_for_start(dir: &Path) -> Result<(Self, String)> {
        let (config, migrate_secret) = Self::read(dir)?;
        let notice = if migrate_secret && config.save(dir).is_err() {
            "旧连接密码暂未能加密保存，请检查数据目录权限后重启。".into()
        } else {
            String::new()
        };
        Ok((config, notice))
    }

    fn read(dir: &Path) -> Result<(Self, bool)> {
        let p = dir.join("config.toml");
        let source = match std::fs::read_to_string(p) {
            Ok(source) => source,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok((Self::default(), false));
            }
            Err(error) => return Err(error.into()),
        };
        // Parser diagnostics can include the original line, including the Secret.
        let raw: toml::Value =
            toml::from_str(&source).map_err(|_| anyhow::anyhow!("配置文件格式错误"))?;
        let mut v: Self =
            toml::from_str(&source).map_err(|_| anyhow::anyhow!("配置文件格式错误"))?;
        let migrate_secret = !v.secret.is_empty();
        if let Some(ciphertext) = raw.get("secret_dpapi") {
            v.secret.clear();
            match ciphertext
                .as_str()
                .and_then(|value| credential::unprotect(value).ok())
            {
                Some(secret) => v.secret = secret,
                None => v.locked_secret = Some(ciphertext.clone()),
            }
        }
        // Existing configurations retain the old opt-in and unit interpretation.
        if raw.get("update_check_frequency").is_none()
            && let Some(enabled) = raw.get("auto_check_updates").and_then(toml::Value::as_bool)
        {
            use crate::release_update::CheckFrequency;
            v.update_check_frequency = if enabled {
                CheckFrequency::Daily
            } else {
                CheckFrequency::Never
            };
        }
        if raw.get("seconds_enabled").is_none() {
            v.seconds_enabled = false;
        }
        if raw.get("retention_time_unit").is_none() {
            v.retention_time_unit = RetentionTimeUnit::Days;
        }
        if raw.get("storage_limit_enabled").is_none() {
            // Never inherit an enabled deletion policy when combining quotas.
            v.storage_limit_enabled = false;
            v.storage_limit_migration_notice = v.retention_size_enabled || v.seconds_size_enabled;
        }
        if v.backup_dir.as_ref().is_some_and(|p| p.trim().is_empty()) {
            v.backup_dir = None;
        }
        if raw.get("floating_content").is_none() {
            v.floating_content = crate::taskbar_content::TaskbarContent::floating_legacy(
                raw.get("floating_today")
                    .and_then(toml::Value::as_bool)
                    .unwrap_or(true),
            );
        }
        v.floating_today = v
            .floating_content
            .item(crate::taskbar_content::Metric::Today)
            .is_some();
        if raw.get("taskbar_content").is_none() {
            v.taskbar_content =
                crate::taskbar_content::TaskbarContent::legacy(v.taskbar_horizontal);
        }
        v.taskbar_horizontal = v.taskbar_content.rows.len() == 1;
        if raw.get("taskbar_position_mode").is_none() {
            v.taskbar_position_mode = TaskbarPositionMode::Manual;
            v.taskbar_auto_prefer_left = v.taskbar_left;
        }
        if raw.get("taskbar_text_layout_version").is_none()
            && !v.taskbar_left
            && v.taskbar_offset != 0
            && let Ok(width) = crate::taskbar_text::reserved_width(v.taskbar_horizontal)
        {
            let scale = crate::native::taskbar_dpi() as f32 / 96.;
            v.upgrade_taskbar_width(width, scale);
        }
        if raw.get("taskbar_content").is_none()
            && raw
                .get("taskbar_text_layout_version")
                .and_then(toml::Value::as_integer)
                .unwrap_or(0)
                < 2
            && !v.taskbar_left
            && v.taskbar_offset != 0
            && let (Ok(previous), Ok(width)) = (
                crate::taskbar_text::reserved_width(v.taskbar_horizontal),
                crate::taskbar_text::reserved_content_width(&v.taskbar_content),
            )
        {
            let scale = crate::native::taskbar_dpi() as f32 / 96.;
            v.taskbar_offset = v
                .taskbar_offset
                .saturating_add((width * scale).ceil() as i32 - (previous * scale).ceil() as i32);
        }
        if raw.get("taskbar_content").is_some()
            && raw
                .get("taskbar_text_layout_version")
                .and_then(toml::Value::as_integer)
                .unwrap_or(2)
                < 3
            && !v.taskbar_left
            && v.taskbar_offset != 0
            && let (Ok(previous), Ok(width)) = (
                crate::taskbar_text::previous_content_width(&v.taskbar_content),
                crate::taskbar_text::reserved_content_width(&v.taskbar_content),
            )
        {
            let scale = crate::native::taskbar_dpi() as f32 / 96.;
            v.taskbar_offset = v
                .taskbar_offset
                .saturating_add((width * scale).ceil() as i32 - (previous * scale).ceil() as i32);
        }
        v.taskbar_text_layout_version = crate::taskbar_text::LAYOUT_VERSION;
        v.validate()?;
        Ok((v, migrate_secret))
    }
    pub fn upgrade_floating_default(&mut self) -> bool {
        if ![false, true].into_iter().any(|today| {
            self.floating_content == crate::taskbar_content::TaskbarContent::floating_legacy(today)
        }) {
            return false;
        }
        self.floating_content = crate::taskbar_content::TaskbarContent::floating();
        self.floating_today = false;
        true
    }

    fn upgrade_taskbar_width(&mut self, width: f32, scale: f32) {
        // A custom manual coordinate should survive the narrower right anchor.
        // Zero offset follows the improved default; left anchors are unchanged.
        if !self.taskbar_left && self.taskbar_offset != 0 {
            let previous = if self.taskbar_horizontal { 224. } else { 124. };
            self.taskbar_offset = self
                .taskbar_offset
                .saturating_add((width * scale).ceil() as i32 - (previous * scale).round() as i32);
        }
    }
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.storage_limit_mb > 0 && self.storage_limit_mb.checked_mul(1024 * 1024).is_some(),
            "存储容量须为有效的正整数MB"
        );
        anyhow::ensure!(
            (1..=3650).contains(&self.seconds_days),
            "秒级明细保留时间须为1–3650天"
        );
        anyhow::ensure!(
            self.seconds_size_mb > 0 && self.seconds_size_mb.checked_mul(1024 * 1024).is_some(),
            "秒级容量须为有效的正整数MB"
        );
        if let Some(path) = self.backup_dir.as_deref().filter(|p| !p.trim().is_empty()) {
            anyhow::ensure!(Path::new(path).is_absolute(), "备份目录须为绝对路径");
        }
        if let Some(path) = &self.client_config_dir {
            anyhow::ensure!(Path::new(path).is_absolute(), "客户端配置目录须为绝对路径");
        }
        self.taskbar_content.validate()?;
        self.floating_content
            .validate_for(crate::taskbar_content::DisplayTarget::Floating)?;
        anyhow::ensure!(self.floating_row_gap <= 32, "悬浮窗行间距须为 0–32");
        anyhow::ensure!(
            self.floating_background_transparency <= 100,
            "背景透明度须为 0–100%"
        );
        anyhow::ensure!(
            self.floating_background_color
                .is_none_or(|color| color <= 0xffffff),
            "悬浮窗背景须为有效 RGB 颜色"
        );
        if self.endpoint != "auto" && !self.endpoint.starts_with(r"\\.\pipe\") {
            let u = reqwest::Url::parse(&self.endpoint)
                .context("请输入 auto、命名管道或本机 HTTP 地址")?;
            anyhow::ensure!(
                matches!(u.scheme(), "http" | "https"),
                "仅支持 HTTP / HTTPS"
            );
            anyhow::ensure!(
                matches!(
                    u.host_str(),
                    Some("127.0.0.1" | "localhost" | "[::1]" | "::1")
                ),
                "此版本仅连接本机 Mihomo"
            );
            anyhow::ensure!(
                u.username().is_empty()
                    && u.password().is_none()
                    && u.query().is_none()
                    && u.fragment().is_none(),
                "地址不能包含凭据或查询参数"
            );
        }
        anyhow::ensure!(
            (1..=3650).contains(&self.retention_days),
            "明细保留天数须为 1–3650"
        );
        anyhow::ensure!(
            self.retention_size_mb > 0 && self.retention_size_mb.checked_mul(1024 * 1024).is_some(),
            "容量须为有效的正整数 MB"
        );
        Ok(())
    }
    pub fn save(&self, dir: &Path) -> Result<()> {
        self.validate()?;
        let mut settings = toml::Table::try_from(self)?;
        if !self.secret.is_empty() {
            settings.insert(
                "secret_dpapi".into(),
                credential::protect(&self.secret)?.into(),
            );
        } else if let Some(ciphertext) = &self.locked_secret {
            settings.insert("secret_dpapi".into(), ciphertext.clone());
        }
        let path = dir.join("config.toml");
        let temp = dir.join("config.toml.tmp");
        let serialized = toml::to_string_pretty(&settings)?;
        let result = (|| {
            let mut file = std::fs::File::create(&temp)?;
            file.write_all(serialized.as_bytes())?;
            file.sync_all()?;
            drop(file);
            crate::native::replace_file(&temp, &path)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        result
    }
}
pub fn default_backup_directory() -> Result<PathBuf> {
    if let Some(dir) = portable_data_dir() {
        return Ok(dir.join("backups"));
    }
    default_backup_directory_in(dirs::document_dir().as_deref(), cfg!(debug_assertions))
}

fn default_backup_directory_in(documents: Option<&Path>, debug: bool) -> Result<PathBuf> {
    let documents = documents.context("无法获取系统文档目录，请选择自定义备份目录")?;
    anyhow::ensure!(
        documents.is_absolute(),
        "系统文档目录无效，请选择自定义备份目录"
    );
    Ok(documents
        .join(if debug {
            "MihomoTraffic-Debug"
        } else {
            "MihomoTraffic"
        })
        .join("backups"))
}

pub(crate) fn portable_data_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let directory = exe.parent()?;
    directory
        .join("portable.flag")
        .is_file()
        .then(|| directory.join("data"))
}

pub fn default_data_dir() -> PathBuf {
    if let Some(dir) = portable_data_dir() {
        return dir;
    }
    if cfg!(debug_assertions) {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("dev-data")
    } else {
        dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("MihomoTraffic")
    }
}
pub fn data_dir() -> Result<PathBuf> {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|s| s == "--data-dir")
        && let Some(path) = args.get(i + 1)
    {
        return std::path::absolute(path).context("数据目录无效");
    }
    crate::data_location::resolve(&default_data_dir())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recent_usage_refresh_modes_obey_success_age_and_exact_boundary() {
        let last = 1_000_000;
        for mode in RecentUsageRefreshMode::ALL {
            assert!(mode.should_refresh(None, last));
            for age in [0, 299_999, 300_000, 300_001, 900_000] {
                assert_eq!(
                    mode.should_refresh(Some(last), last + age),
                    match mode {
                        RecentUsageRefreshMode::Always => true,
                        RecentUsageRefreshMode::Manual => false,
                        RecentUsageRefreshMode::AfterFiveMinutes => age > 300_000,
                    },
                    "mode={mode:?}, age={age}"
                );
            }
        }
        assert!(RecentUsageRefreshMode::AfterFiveMinutes.should_refresh(Some(last), last - 1));
    }

    #[test]
    fn recent_usage_refresh_defaults_for_old_configs_and_survives_restart() {
        let dir = tempfile::tempdir().unwrap();
        let fresh = Config::load(dir.path()).unwrap();
        assert_eq!(
            fresh.recent_usage_refresh_mode,
            RecentUsageRefreshMode::AfterFiveMinutes
        );
        std::fs::write(dir.path().join("config.toml"), "retention_days = 123\n").unwrap();
        let legacy = Config::load(dir.path()).unwrap();
        assert_eq!(
            legacy.recent_usage_refresh_mode,
            fresh.recent_usage_refresh_mode
        );
        for mode in RecentUsageRefreshMode::ALL {
            let config = Config {
                recent_usage_refresh_mode: mode,
                ..legacy.clone()
            };
            config.save(dir.path()).unwrap();
            assert_eq!(Config::load(dir.path()).unwrap(), config);
        }
    }

    #[test]
    fn backup_default_uses_redirected_documents_and_separates_debug() {
        let root = tempfile::tempdir().unwrap();
        let documents = root.path().join("中文文档 redirected data");
        assert_eq!(
            default_backup_directory_in(Some(&documents), false).unwrap(),
            documents.join("MihomoTraffic").join("backups")
        );
        assert_eq!(
            default_backup_directory_in(Some(&documents), true).unwrap(),
            documents.join("MihomoTraffic-Debug").join("backups")
        );
        assert!(
            !documents.exists(),
            "path resolution must not create directories"
        );
        assert!(
            default_backup_directory_in(None, false)
                .unwrap_err()
                .to_string()
                .contains("自定义备份目录")
        );
        assert!(default_backup_directory_in(Some(Path::new("relative/documents")), false).is_err());
    }

    #[test]
    fn empty_backup_preferences_use_document_default_without_creating_it() {
        let root = tempfile::tempdir().unwrap();
        for value in ["", "   "] {
            let config = Config {
                backup_dir: Some(value.into()),
                ..Default::default()
            };
            config.validate().unwrap();
            assert_eq!(
                config.backup_directory().unwrap(),
                default_backup_directory().unwrap()
            );
            config.save(root.path()).unwrap();
            let loaded = Config::load(root.path()).unwrap();
            assert_eq!(loaded.backup_dir, None);
            assert_eq!(
                loaded.backup_directory().unwrap(),
                default_backup_directory().unwrap()
            );
        }
    }

    #[test]
    fn retention_defaults_legacy_values_and_all_combinations_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let fresh = Config::load(dir.path()).unwrap();
        assert_eq!(fresh.retention_days, 90);
        assert_eq!(fresh.retention_time_unit, RetentionTimeUnit::Months);
        assert!(fresh.seconds_enabled);
        assert!(!fresh.storage_limit_enabled);
        assert_eq!(fresh.seconds_days, 3);
        assert_eq!(fresh.seconds_time_unit, RetentionTimeUnit::Days);
        assert_eq!(fresh.storage_limit_mb, 2048);
        assert!(fresh.retention_days_enabled);
        assert!(!fresh.retention_size_enabled);
        assert_eq!(fresh.retention_size_mb, 1024);
        for days in [30, 123] {
            fs_write_legacy(dir.path(), days);
            let loaded = Config::load(dir.path()).unwrap();
            assert_eq!(loaded.retention_days, days);
            assert_eq!(loaded.retention_policy().days, Some(days));
            assert_eq!(loaded.retention_policy().max_bytes, None);
        }
        for days_enabled in [true, false] {
            for size_enabled in [true, false] {
                let config = Config {
                    retention_days_enabled: days_enabled,
                    retention_size_enabled: size_enabled,
                    retention_days: 123,
                    retention_size_mb: 2048,
                    ..fresh.clone()
                };
                config.save(dir.path()).unwrap();
                let saved = Config::load(dir.path()).unwrap();
                assert_eq!(saved, config);
                assert_eq!(saved.retention_policy().days, days_enabled.then_some(123));
                assert_eq!(
                    saved.retention_policy().max_bytes,
                    size_enabled.then_some(2048 * 1024 * 1024)
                );
            }
        }
        for mb in [0, u64::MAX] {
            assert!(
                Config {
                    retention_size_mb: mb,
                    ..fresh.clone()
                }
                .validate()
                .is_err()
            );
        }
    }

    fn fs_write_legacy(dir: &Path, days: u32) {
        std::fs::write(
            dir.join("config.toml"),
            format!("retention_days = {days}\n"),
        )
        .unwrap();
    }
    #[test]
    fn storage_display_units_roundtrip_without_changing_retention_policy() {
        let dir = tempfile::tempdir().unwrap();
        let base = Config {
            retention_days: 123,
            retention_size_mb: 500,
            retention_size_enabled: true,
            ..Default::default()
        };
        for retention_time_unit in RetentionTimeUnit::ALL {
            for retention_size_unit in RetentionSizeUnit::ALL {
                let config = Config {
                    retention_time_unit,
                    retention_size_unit,
                    ..base.clone()
                };
                config.save(dir.path()).unwrap();
                let loaded = Config::load(dir.path()).unwrap();
                assert_eq!(loaded, config);
                assert_eq!(loaded.retention_policy(), base.retention_policy());
            }
        }
        fs_write_legacy(dir.path(), 31);
        let loaded = Config::load(dir.path()).unwrap();
        assert_eq!(loaded.retention_time_unit, RetentionTimeUnit::Days);
        assert_eq!(loaded.retention_size_unit, RetentionSizeUnit::Gb);
        assert_eq!(loaded.retention_days, 31);
    }
    #[test]
    fn tray_preferences_default_for_existing_configs_and_survive_restart() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            Config::load(dir.path()).unwrap().tray_icon_style,
            TrayIconStyle::Color
        );
        let path = dir.path().join("config.toml");
        let source = "endpoint = 'auto'\nclose_to_tray = false\n";
        std::fs::write(&path, source).unwrap();
        let legacy = Config::load(dir.path()).unwrap();
        assert!(legacy.tray_visible);
        assert_eq!(legacy.tray_icon_style, TrayIconStyle::Color);
        assert!(!legacy.close_to_tray);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
        for tray_visible in [false, true] {
            for tray_icon_style in [TrayIconStyle::Monochrome, TrayIconStyle::Color] {
                let saved = Config {
                    tray_visible,
                    tray_icon_style,
                    ..legacy.clone()
                };
                saved.save(dir.path()).unwrap();
                assert_eq!(Config::load(dir.path()).unwrap(), saved);
            }
        }
    }
    #[test]
    fn process_control_preferences_and_pending_disable_survive_restart() {
        let dir = tempfile::tempdir().unwrap();
        for process_control in [
            ProcessControl::Unmanaged,
            ProcessControl::KeepAlways,
            ProcessControl::RestoreStrict,
        ] {
            let config = Config {
                process_control,
                ..Default::default()
            };
            config.save(dir.path()).unwrap();
            assert_eq!(
                Config::load(dir.path()).unwrap().process_control,
                process_control
            );
        }
    }
    #[test]
    fn process_modes_migrate_legacy_and_roundtrip_all_new_policies() {
        for (legacy, expected) in [
            ("unmanaged", ProcessControl::Unmanaged),
            ("keep-always", ProcessControl::Maintain(ProcessMode::Always)),
            (
                "restore-strict",
                ProcessControl::ApplyOnce(ProcessMode::Strict),
            ),
        ] {
            let loaded: Config =
                toml::from_str(&format!("process_control = '{legacy}'\n")).unwrap();
            assert_eq!(loaded.process_control, expected);
        }
        for mode in ProcessMode::ALL {
            for policy in [
                ProcessControl::ApplyOnce(mode),
                ProcessControl::Maintain(mode),
            ] {
                let config = Config {
                    process_control: policy,
                    client_config_dir: Some(r"C:\client\config".into()),
                    ..Default::default()
                };
                let loaded: Config = toml::from_str(&toml::to_string(&config).unwrap()).unwrap();
                assert_eq!(loaded.process_control, policy);
                assert_eq!(loaded.client_config_dir, config.client_config_dir);
            }
        }
    }
    #[test]
    fn fresh_defaults_match_current_preferences_and_preserve_legacy_styles() {
        use crate::{taskbar_content::TaskbarContent, theme::ColorTheme};
        let dir = tempfile::tempdir().unwrap();
        let fresh = Config::load(dir.path()).unwrap();
        assert_eq!(fresh.theme, "system");
        assert_eq!(fresh.color_theme, ColorTheme::Mica);
        assert_eq!(fresh.taskbar_content.font_family, "微软雅黑");
        assert_eq!(fresh.taskbar_content.font_size, 12.);
        assert_eq!(fresh.taskbar_content.font_weight.value(), 400);
        assert_eq!(fresh.floating_content.font_family, "Segoe UI");
        assert_eq!(fresh.floating_content.font_size, 13.);
        assert_eq!(fresh.floating_content.font_weight.value(), 400);
        assert_eq!(fresh.endpoint, "auto");
        assert!(fresh.secret.is_empty());
        assert_eq!(fresh.process_control, ProcessControl::Unmanaged);
        assert!(!fresh.floating && !fresh.taskbar);
        assert_eq!((fresh.floating_x, fresh.floating_y), (None, None));
        assert!(!dir.path().join("config.toml").exists());
        let source = "[taskbar_content]\nrows = [[{ metric = 'cpu' }]]\n";
        let path = dir.path().join("config.toml");
        std::fs::write(&path, source).unwrap();
        let legacy = Config::load(dir.path()).unwrap();
        assert_eq!(legacy.theme, "light");
        assert_eq!(legacy.process_control, ProcessControl::Unmanaged);
        assert_eq!(legacy.color_theme, ColorTheme::CloudBlue);
        assert_eq!(legacy.taskbar_content.font_family, "Segoe UI");
        assert_eq!(legacy.taskbar_content.font_size, 12.);
        assert!(!legacy.floating && !legacy.taskbar);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
        let enabled = Config {
            floating: true,
            taskbar: true,
            ..legacy
        };
        enabled.save(dir.path()).unwrap();
        let loaded = Config::load(dir.path()).unwrap();
        assert!(loaded.floating && loaded.taskbar);
        let custom = Config {
            theme: "dark".into(),
            color_theme: ColorTheme::PorcelainLilac,
            taskbar_content: TaskbarContent {
                font_family: "Consolas".into(),
                font_size: 12.5,
                ..Default::default()
            },
            ..fresh
        };
        custom.save(dir.path()).unwrap();
        assert_eq!(Config::load(dir.path()).unwrap(), custom);
    }

    #[test]
    fn floating_default_upgrades_only_the_untouched_editor_preset() {
        use crate::taskbar_content::{Metric, TaskbarContent};
        let dir = tempfile::tempdir().unwrap();
        let fresh = Config::load(dir.path()).unwrap();
        assert_eq!(fresh.floating_content, TaskbarContent::floating());
        assert_eq!(fresh.floating_content.position(Metric::Cpu), Some((0, 0)));
        assert_eq!(
            fresh.floating_content.position(Metric::Memory),
            Some((1, 0))
        );
        assert_eq!(
            fresh.floating_content.position(Metric::Upload),
            Some((0, 1))
        );
        assert_eq!(
            fresh.floating_content.position(Metric::Download),
            Some((1, 1))
        );
        assert!(!fresh.floating_today);
        assert_eq!(fresh.floating_content.font_size, 13.);
        for today in [false, true] {
            let mut saved = Config {
                floating_today: today,
                floating_content: TaskbarContent::floating_legacy(today),
                ..fresh.clone()
            };
            saved.save(dir.path()).unwrap();
            let path = dir.path().join("config.toml");
            let source = std::fs::read_to_string(&path).unwrap();
            let mut loaded = Config::load(dir.path()).unwrap();
            assert_eq!(loaded, saved);
            assert!(loaded.upgrade_floating_default());
            assert_eq!(loaded.floating_content, fresh.floating_content);
            assert_eq!(loaded.taskbar_content, saved.taskbar_content);
            assert!(!loaded.floating_today);
            assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
            saved
                .floating_content
                .item_mut(Metric::Download)
                .unwrap()
                .label = "下载速度".into();
            saved.save(dir.path()).unwrap();
            let mut loaded = Config::load(dir.path()).unwrap();
            assert_eq!(loaded, saved);
            assert!(!loaded.upgrade_floating_default());
        }
    }
    #[test]
    fn floating_legacy_migration_is_independent_and_does_not_write() {
        for today in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let config = Config {
                floating_today: today,
                floating_x: Some(-300),
                floating_y: Some(120),
                ..Config::default()
            };
            let mut raw = toml::Value::try_from(&config).unwrap();
            for key in [
                "floating_content",
                "floating_row_gap",
                "floating_background_color",
                "floating_background_transparency",
            ] {
                raw.as_table_mut().unwrap().remove(key);
            }
            let source = toml::to_string_pretty(&raw).unwrap();
            let path = dir.path().join("config.toml");
            std::fs::write(&path, &source).unwrap();
            let loaded = Config::load(dir.path()).unwrap();
            assert_eq!(
                loaded.floating_content,
                crate::taskbar_content::TaskbarContent::floating_legacy(today)
            );
            assert_eq!(loaded.taskbar_content, config.taskbar_content);
            assert_eq!(
                (loaded.floating_x, loaded.floating_y),
                (Some(-300), Some(120))
            );
            assert_eq!(std::fs::read_to_string(path).unwrap(), source);
            assert_eq!(
                (
                    loaded.floating_row_gap,
                    loaded.floating_background_transparency
                ),
                (8, 0)
            );
        }
    }

    #[test]
    fn malformed_configuration_diagnostics_do_not_echo_secrets() {
        let dir = tempfile::tempdir().unwrap();
        for source in ["secret = 'fixture-secret", "secret = ['fixture-secret']"] {
            std::fs::write(dir.path().join("config.toml"), source).unwrap();
            let error = Config::load(dir.path()).unwrap_err();
            assert_eq!(format!("{error:#}"), "配置文件格式错误");
        }
    }

    #[test]
    fn startup_migrates_legacy_secret_and_roundtrips_encrypted_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let secret = "fixture-secret-dpapi";
        let source = format!("secret = '{secret}'\ntheme = 'dark'\n");
        std::fs::write(&path, &source).unwrap();
        std::fs::create_dir(dir.path().join("config.toml.tmp")).unwrap();
        let (config, notice) = Config::load_for_start(dir.path()).unwrap();
        assert!(!notice.is_empty());
        assert!(!notice.contains(secret));
        assert_eq!(config.secret, secret);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), source);

        std::fs::remove_dir(dir.path().join("config.toml.tmp")).unwrap();
        let (config, notice) = Config::load_for_start(dir.path()).unwrap();
        assert!(notice.is_empty());
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(!saved.contains(secret));
        let fields: toml::Table = toml::from_str(&saved).unwrap();
        assert!(!fields.contains_key("secret"));
        assert!(fields["secret_dpapi"].as_str().is_some());
        assert_eq!(config.theme, "dark");
        assert_eq!(Config::load(dir.path()).unwrap(), config);
        assert!(!format!("{config:?}").contains(secret));
        assert!(!toml::to_string(&config).unwrap().contains(secret));
        Config::load_for_start(dir.path()).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), saved);
    }

    #[test]
    fn unavailable_credential_survives_unrelated_saves_and_can_be_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "secret_dpapi = 'invalid-ciphertext'\nsecret = 'fixture-legacy-secret'\n",
        )
        .unwrap();
        let (mut config, notice) = Config::load_for_start(dir.path()).unwrap();
        assert!(notice.is_empty());
        assert!(config.secret.is_empty());
        assert!(config.locked_secret.is_some());
        config.theme = "dark".into();
        config.save(dir.path()).unwrap();
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(saved.contains("invalid-ciphertext"));
        assert!(!saved.contains("fixture-legacy-secret"));
        assert_eq!(Config::load(dir.path()).unwrap(), config);

        config.secret = "fixture-new-secret".into();
        config.locked_secret = None;
        config.save(dir.path()).unwrap();
        assert!(
            !std::fs::read_to_string(path)
                .unwrap()
                .contains("invalid-ciphertext")
        );
        assert_eq!(Config::load(dir.path()).unwrap(), config);
    }

    #[test]
    fn floating_settings_roundtrip_and_validate_each_surface() {
        use crate::taskbar_content::{Metric, TaskbarContent};
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        assert_eq!((config.floating_x, config.floating_y), (None, None));
        for metric in Metric::ALL {
            config.floating_content.add(metric);
        }
        config.floating_today = true;
        config.floating_content.set_rows(13);
        config.taskbar_content.font_size = 12.5;
        config.floating_content.font_size = 13.25;
        config.floating_content.font_weight =
            crate::taskbar_content::TaskbarFontWeight::new(450).unwrap();
        config.floating_background_color = Some(0x3B82F6);
        config.floating_background_transparency = 100;
        config.floating_row_gap = 32;
        config.save(dir.path()).unwrap();
        assert_eq!(Config::load(dir.path()).unwrap(), config);
        config.taskbar_content = config.floating_content.clone();
        assert!(config.validate().is_err());
        config.taskbar_content = TaskbarContent::default();
        config.floating_background_transparency = 101;
        assert!(config.validate().is_err());
        config.floating_background_transparency = 50;
        config.floating_background_color = Some(0x1000000);
        assert!(config.validate().is_err());
        config.floating_background_color = None;
        config.floating_row_gap = 33;
        assert!(config.validate().is_err());
    }
    #[test]
    fn taskbar_prefix_spacing_keeps_old_layout_and_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.taskbar_content.prefix_gap = 4;
        config.save(dir.path()).unwrap();
        let path = dir.path().join("config.toml");
        let source = std::fs::read_to_string(&path)
            .unwrap()
            .replace("prefix_gap = 4\n", "");
        std::fs::write(&path, &source).unwrap();
        assert_eq!(Config::load(dir.path()).unwrap(), config);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
        config.taskbar_content.gap = 0;
        config.taskbar_content.prefix_gap = 32;
        config.save(dir.path()).unwrap();
        assert_eq!(Config::load(dir.path()).unwrap(), config);
        config.taskbar_content.prefix_gap = 33;
        assert!(config.save(dir.path()).is_err());
    }
    #[test]
    fn aligned_parts_upgrade_preserves_version_two_manual_left_edge() {
        let dir = tempfile::tempdir().unwrap();
        let content = crate::taskbar_content::TaskbarContent::default();
        let old = Config {
            taskbar_content: content.clone(),
            taskbar_offset: 123,
            taskbar_text_layout_version: 2,
            taskbar_position_mode: TaskbarPositionMode::Manual,
            ..Config::default()
        };
        old.save(dir.path()).unwrap();
        let source = std::fs::read(dir.path().join("config.toml")).unwrap();
        let loaded = Config::load(dir.path()).unwrap();
        let scale = crate::native::taskbar_dpi() as f32 / 96.;
        let previous = crate::taskbar_text::previous_content_width(&content).unwrap();
        let next = crate::taskbar_text::reserved_content_width(&content).unwrap();
        assert_eq!(
            loaded.taskbar_offset - (next * scale).ceil() as i32,
            old.taskbar_offset - (previous * scale).ceil() as i32
        );
        assert_eq!(loaded.taskbar_text_layout_version, 3);
        assert_eq!(
            std::fs::read(dir.path().join("config.toml")).unwrap(),
            source
        );
        loaded.save(dir.path()).unwrap();
        assert_eq!(Config::load(dir.path()).unwrap(), loaded);
    }
    #[test]
    fn new_default_does_not_replace_saved_or_legacy_taskbar_layouts() {
        use crate::taskbar_content::{Metric, TaskbarContent};
        let dir = tempfile::tempdir().unwrap();
        let new = Config::load(dir.path()).unwrap();
        assert_eq!(new.taskbar_content.rows[0][0].metric, Metric::Cpu);
        assert_eq!(new.taskbar_content.rows[1][0].metric, Metric::Memory);
        assert_eq!(new.taskbar_content.rows[0][1].metric, Metric::Upload);
        assert_eq!(new.taskbar_content.rows[1][1].metric, Metric::Download);
        let saved = Config {
            taskbar_content: TaskbarContent::legacy(false),
            ..new
        };
        saved.save(dir.path()).unwrap();
        assert_eq!(Config::load(dir.path()).unwrap(), saved);
        std::fs::write(
            dir.path().join("config.toml"),
            "taskbar_horizontal = false\n",
        )
        .unwrap();
        assert_eq!(
            Config::load(dir.path()).unwrap().taskbar_content,
            TaskbarContent::legacy(false)
        );
    }
    #[test]
    fn cell_layout_upgrade_preserves_version_one_manual_coordinates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        for horizontal in [false, true] {
            let source = format!(
                "taskbar_horizontal = {horizontal}\ntaskbar_text_layout_version = 1\ntaskbar_offset = 84\ntaskbar_position_mode = \"manual\"\n"
            );
            std::fs::write(&path, &source).unwrap();
            let loaded = Config::load(dir.path()).unwrap();
            let scale = crate::native::taskbar_dpi() as f32 / 96.;
            let old = crate::taskbar_text::reserved_width(horizontal).unwrap();
            let next =
                crate::taskbar_text::reserved_content_width(&loaded.taskbar_content).unwrap();
            assert_eq!(
                loaded.taskbar_offset - (next * scale).ceil() as i32,
                84 - (old * scale).ceil() as i32
            );
            assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
            loaded.save(dir.path()).unwrap();
            assert_eq!(Config::load(dir.path()).unwrap(), loaded);
        }
    }

    #[test]
    fn legacy_rows_and_custom_content_roundtrip_without_rewriting_on_load() {
        use crate::taskbar_content::{Metric, TaskbarContent};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        for horizontal in [false, true] {
            let source = format!("taskbar_horizontal = {horizontal}\n");
            std::fs::write(&path, &source).unwrap();
            let mut config = Config::load(dir.path()).unwrap();
            assert_eq!(config.taskbar_content, TaskbarContent::legacy(horizontal));
            assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
            config.taskbar_content.add(Metric::Cpu);
            config.taskbar_content.add(Metric::Memory);
            config.taskbar_content.move_item(Metric::Memory, 0, 0);
            config
                .taskbar_content
                .item_mut(Metric::Download)
                .unwrap()
                .label = "下行".into();
            config
                .taskbar_content
                .item_mut(Metric::Memory)
                .unwrap()
                .detailed = true;
            config.taskbar_content.gap = 18;
            config.save(dir.path()).unwrap();
            assert_eq!(Config::load(dir.path()).unwrap(), config);
        }
    }

    #[test]
    fn compact_layout_preserves_custom_manual_coordinates() {
        for (horizontal, old_width, new_width) in [(false, 124., 82.), (true, 224., 154.)] {
            for scale in [1f32, 1.25, 1.5, 2.] {
                for offset in [-32, 84] {
                    let mut config = Config {
                        taskbar_horizontal: horizontal,
                        taskbar_offset: offset,
                        ..Config::default()
                    };
                    let old_x = 2032 - (old_width * scale).round() as i32 - 12 + offset;
                    config.upgrade_taskbar_width(new_width, scale);
                    let new_x =
                        2032 - (new_width * scale).ceil() as i32 - 12 + config.taskbar_offset;
                    assert_eq!(new_x, old_x);
                }
            }
        }
        let mut default = Config::default();
        default.upgrade_taskbar_width(82., 1.5);
        assert_eq!(default.taskbar_offset, 0);
        let mut left = Config {
            taskbar_left: true,
            taskbar_offset: 84,
            ..Config::default()
        };
        left.upgrade_taskbar_width(82., 1.5);
        assert_eq!(left.taskbar_offset, 84);
    }
    #[test]
    fn compact_layout_upgrade_is_not_applied_twice() {
        let dir = tempfile::tempdir().unwrap();
        let source =
            "taskbar_left = false\ntaskbar_offset = 84\ntaskbar_position_mode = \"manual\"\n";
        let path = dir.path().join("config.toml");
        std::fs::write(&path, source).unwrap();
        let loaded = Config::load(dir.path()).unwrap();
        let scale = crate::native::taskbar_dpi() as f32 / 96.;
        let width = crate::taskbar_text::reserved_content_width(&loaded.taskbar_content).unwrap();
        assert_eq!(
            loaded.taskbar_offset,
            84 + (width * scale).ceil() as i32 - (124. * scale).round() as i32
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
        loaded.save(dir.path()).unwrap();
        assert_eq!(Config::load(dir.path()).unwrap(), loaded);
    }
    #[test]
    fn upgrades_preserve_manual_position_but_new_installs_use_auto() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            Config::load(dir.path()).unwrap().taskbar_position_mode,
            TaskbarPositionMode::AutoAvoid
        );
        std::fs::write(
            dir.path().join("config.toml"),
            "taskbar_left = true\ntaskbar_offset = -32\n",
        )
        .unwrap();
        let loaded = Config::load(dir.path()).unwrap();
        assert_eq!(loaded.taskbar_position_mode, TaskbarPositionMode::Manual);
        assert!(loaded.taskbar_left && loaded.taskbar_auto_prefer_left);
        assert_eq!(loaded.taskbar_offset, -32);
        loaded.save(dir.path()).unwrap();
        assert_eq!(Config::load(dir.path()).unwrap(), loaded);
    }
    #[test]
    fn atomic_preferences_roundtrip() {
        let d = tempfile::tempdir().unwrap();
        let mut config = Config {
            taskbar_horizontal: true,
            taskbar_content: crate::taskbar_content::TaskbarContent::legacy(true),
            theme: "dark".into(),
            ..Default::default()
        };
        config.save(d.path()).unwrap();
        assert_eq!(Config::load(d.path()).unwrap(), config);
        config.theme = "light".into();
        config.save(d.path()).unwrap();
        assert_eq!(Config::load(d.path()).unwrap().theme, "light");
    }
    #[test]
    fn position_modes_keep_independent_user_preferences() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config {
            taskbar_position_mode: TaskbarPositionMode::Manual,
            taskbar_left: true,
            taskbar_offset: 123,
            taskbar_auto_prefer_left: false,
            ..Config::default()
        };
        config.save(dir.path()).unwrap();
        config.taskbar_position_mode = TaskbarPositionMode::AutoAvoid;
        config.taskbar_auto_prefer_left = true;
        config.save(dir.path()).unwrap();
        let mut loaded = Config::load(dir.path()).unwrap();
        assert_eq!((loaded.taskbar_left, loaded.taskbar_offset), (true, 123));
        loaded.taskbar_position_mode = TaskbarPositionMode::Manual;
        loaded.save(dir.path()).unwrap();
        assert!(Config::load(dir.path()).unwrap().taskbar_auto_prefer_left);
    }
}
