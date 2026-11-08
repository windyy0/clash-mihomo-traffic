use crate::{
    model::{DiskMetrics, GpuMetrics, Shared},
    traffic_icon::DirectionIcon,
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum Metric {
    Download,
    Upload,
    Cpu,
    Memory,
    Today,
    TotalSpeed,
    Month,
    Cumulative,
    Gpu,
    Vram,
    Disk,
    DiskRead,
    DiskWrite,
}

impl Metric {
    pub const ALL: [Self; 13] = [
        Self::Download,
        Self::Upload,
        Self::TotalSpeed,
        Self::Today,
        Self::Month,
        Self::Cumulative,
        Self::Cpu,
        Self::Memory,
        Self::Gpu,
        Self::Vram,
        Self::Disk,
        Self::DiskRead,
        Self::DiskWrite,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Download => "下载",
            Self::Upload => "上传",
            Self::Cpu => "CPU",
            Self::Memory => "内存",
            Self::Today => "今日流量",
            Self::TotalSpeed => "合计网速",
            Self::Month => "本月流量",
            Self::Cumulative => "累计流量",
            Self::Gpu => "GPU 使用率",
            Self::Vram => "显存占用",
            Self::Disk => "磁盘活动率",
            Self::DiskRead => "读取速度",
            Self::DiskWrite => "写入速度",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Download => "↓",
            Self::Upload => "↑",
            Self::Cpu => "CPU:",
            Self::Memory => "内存:",
            Self::Today => "今日:",
            Self::TotalSpeed => "↕",
            Self::Month => "本月:",
            Self::Cumulative => "累计:",
            Self::Gpu => "GPU:",
            Self::Vram => "显存:",
            Self::Disk => "磁盘:",
            Self::DiskRead => "读:",
            Self::DiskWrite => "写:",
        }
    }

    pub fn group(self) -> &'static str {
        match self {
            Self::Download | Self::Upload | Self::TotalSpeed => "网络",
            Self::Today | Self::Month | Self::Cumulative => "流量",
            Self::Cpu | Self::Memory => "系统",
            Self::Gpu | Self::Vram => "显卡",
            Self::Disk | Self::DiskRead | Self::DiskWrite => "磁盘",
        }
    }

    pub fn is_rate(self) -> bool {
        matches!(
            self,
            Self::Download | Self::Upload | Self::TotalSpeed | Self::DiskRead | Self::DiskWrite
        )
    }

    pub fn is_memory(self) -> bool {
        matches!(self, Self::Memory | Self::Vram)
    }

    pub fn unit_choices(self) -> &'static [(ByteUnit, &'static str)] {
        use ByteUnit::*;
        match self {
            Self::Download | Self::Upload | Self::TotalSpeed | Self::Memory => {
                &[(Auto, "自动"), (K, "K"), (M, "M")]
            }
            Self::Today | Self::Month | Self::Cumulative => {
                &[(Auto, "自动"), (M, "M"), (G, "G"), (T, "T")]
            }
            Self::Vram | Self::DiskRead | Self::DiskWrite => &[(Auto, "自动"), (M, "M"), (G, "G")],
            Self::Cpu | Self::Gpu | Self::Disk => &[],
        }
    }

    pub fn formats(self) -> Vec<(ValueFormat, &'static str)> {
        use ValueFormat::*;
        if self.is_memory() {
            vec![
                (Percent, "百分比"),
                (Used, "已用容量"),
                (UsedTotal, "已用 / 总量"),
            ]
        } else if self.is_rate() {
            vec![
                (Auto, "自动单位"),
                (Kb, "KB/s"),
                (Mb, "MB/s"),
                (Compact, "紧凑单位"),
            ]
        } else if matches!(self, Self::Today | Self::Month | Self::Cumulative) {
            vec![
                (Auto, "自动单位"),
                (Mb, "MB"),
                (Gb, "GB"),
                (Compact, "紧凑单位"),
            ]
        } else {
            vec![(Auto, "百分比")]
        }
    }

    pub fn scope(self) -> &'static str {
        match self.group() {
            "网络" | "流量" if self == Self::Cumulative => {
                "仅统计 mihomo 接管的流量 · 自启用记录起累计"
            }
            "网络" | "流量" => "仅统计 mihomo 接管的流量",
            "显卡" => "所选显卡 · 最繁忙引擎使用率与显存",
            "磁盘" if self == Self::Disk => "磁盘忙碌程度，与容量占用无关；所有磁盘取最繁忙值",
            "磁盘" => "所选物理磁盘；所有磁盘合计读写速度",
            _ => "整台计算机",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ValueFormat {
    #[default]
    Auto,
    Compact,
    Kb,
    Mb,
    Gb,
    Percent,
    Used,
    UsedTotal,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ByteUnit {
    #[default]
    Auto,
    B,
    K,
    M,
    G,
    T,
}

impl ByteUnit {
    // Preserve all saved units; each metric exposes its own smaller menu.
    pub const ALL: [(Self, &'static str); 6] = [
        (Self::Auto, "自动"),
        (Self::B, "B"),
        (Self::K, "K"),
        (Self::M, "M"),
        (Self::G, "G"),
        (Self::T, "T"),
    ];

    fn exponent(self) -> Option<usize> {
        match self {
            Self::Auto => None,
            Self::B => Some(0),
            Self::K => Some(1),
            Self::M => Some(2),
            Self::G => Some(3),
            Self::T => Some(4),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum UnitStyle {
    Full,
    #[default]
    Short,
}

impl UnitStyle {
    pub fn label(self) -> &'static str {
        match self {
            Self::Full => "完整",
            Self::Short => "简写",
        }
    }

    pub fn toggled(self) -> Self {
        match self {
            Self::Full => Self::Short,
            Self::Short => Self::Full,
        }
    }

    fn suffix(self, exponent: usize) -> &'static str {
        match self {
            Self::Full => ["B", "KB", "MB", "GB", "TB"][exponent],
            Self::Short => ["B", "K", "M", "G", "T"][exponent],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct TaskbarItem {
    pub metric: Metric,
    pub label: String,
    pub detailed: bool,
    pub format: ValueFormat,
    /// None preserves the unit behavior of older format/detailed settings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<ByteUnit>,
    pub unit_style: UnitStyle,
    pub hide_percent: bool,
    /// RGB; None follows the theme or keeps the background transparent.
    pub text_color: Option<u32>,
    pub background_color: Option<u32>,
    /// None chooses the metric's automatic precision; Some accepts 0..=2.
    pub precision: Option<u8>,
    /// Empty means the automatic GPU or all physical disks.
    pub device: String,
}

impl Default for TaskbarItem {
    fn default() -> Self {
        Self::new(Metric::Download)
    }
}

impl TaskbarItem {
    /// Preserve saved direction markers, with either punctuation style.
    pub fn direction_icon(&self) -> Option<DirectionIcon> {
        let marker = self
            .label
            .strip_suffix(':')
            .or_else(|| self.label.strip_suffix('：'))
            .unwrap_or(&self.label);
        match (self.metric, marker) {
            (Metric::Upload, "↑") => Some(DirectionIcon::Upload),
            (Metric::Download, "↓") => Some(DirectionIcon::Download),
            (Metric::TotalSpeed, "↕") => Some(DirectionIcon::TotalSpeed),
            _ => None,
        }
    }

    /// Apply punctuation for display without rewriting saved custom prefixes.
    pub fn prefix_label(&self) -> Cow<'_, str> {
        if matches!(
            self.metric,
            Metric::Upload | Metric::Download | Metric::TotalSpeed
        ) && !self.label.is_empty()
            && !self.label.ends_with(':')
            && !self.label.ends_with('：')
        {
            Cow::Owned(format!("{}:", self.label))
        } else {
            Cow::Borrowed(&self.label)
        }
    }

    /// An icon occupies the marker's position; return its editable punctuation.
    pub fn prefix_text(&self) -> Cow<'_, str> {
        if self.direction_icon().is_some() {
            Cow::Borrowed(if self.label.ends_with('：') {
                "："
            } else {
                ":"
            })
        } else {
            self.prefix_label()
        }
    }

    pub fn new(metric: Metric) -> Self {
        Self {
            metric,
            label: metric.label().into(),
            detailed: false,
            format: if metric.is_memory() {
                if metric == Metric::Vram {
                    ValueFormat::Used
                } else {
                    ValueFormat::Percent
                }
            } else {
                ValueFormat::Auto
            },
            precision: None,
            unit: None,
            unit_style: UnitStyle::Short,
            hide_percent: false,
            text_color: None,
            background_color: None,
            device: String::new(),
        }
    }

    pub fn text(&self, value: &str, horizontal: bool) -> String {
        if self.label.is_empty() {
            value.into()
        } else {
            format!(
                "{}{space}{value}",
                self.prefix_label(),
                space = if horizontal { " " } else { "  " }
            )
        }
    }

    pub fn effective_format(&self) -> ValueFormat {
        if self.detailed {
            if self.metric.is_memory() {
                ValueFormat::UsedTotal
            } else if self.metric.is_rate() || self.metric.group() == "流量" {
                ValueFormat::Compact
            } else {
                self.format
            }
        } else if self.format == ValueFormat::Auto && self.metric.is_memory() {
            ValueFormat::Percent
        } else {
            self.format
        }
    }

    pub fn normalize_legacy(&mut self) {
        self.format = self.effective_format();
        if self.detailed
            && !self.metric.is_rate()
            && !self.metric.is_memory()
            && self.metric.group() != "流量"
        {
            self.precision = self.precision.or(Some(1));
        }
        self.detailed = false;
    }

    fn decimals(&self, automatic: usize) -> usize {
        self.precision.map(usize::from).unwrap_or(automatic).min(2)
    }

    fn percent(&self, value: Option<f64>) -> String {
        value
            .filter(|v| v.is_finite())
            .map(|v| {
                format!(
                    "{:.*}{}",
                    self.decimals(usize::from(self.detailed)),
                    v.clamp(0., 100.),
                    if self.hide_percent { "" } else { "%" }
                )
            })
            .unwrap_or_else(|| "—".into())
    }

    pub fn effective_unit(&self) -> ByteUnit {
        self.unit.unwrap_or_else(|| match self.effective_format() {
            ValueFormat::Kb => ByteUnit::K,
            ValueFormat::Mb => ByteUnit::M,
            ValueFormat::Gb | ValueFormat::UsedTotal => ByteUnit::G,
            _ => ByteUnit::Auto,
        })
    }

    fn byte_exponent(&self, value: f64) -> usize {
        self.effective_unit().exponent().unwrap_or_else(|| {
            let mut value = value.max(0.);
            let mut exponent = 0;
            while value >= 1024. && exponent < 4 {
                value /= 1024.;
                exponent += 1;
            }
            exponent
        })
    }

    fn bytes(&self, value: f64, rate: bool) -> String {
        if !value.is_finite() {
            return "—".into();
        }
        let value = value.max(0.);
        let unit = self.byte_exponent(value);
        let value = value / 1024f64.powi(unit as i32);
        format!(
            "{:.*}{}{}{}",
            self.decimals(usize::from(unit > 0)),
            value,
            if self.effective_format() == ValueFormat::Compact {
                ""
            } else {
                " "
            },
            self.unit_style.suffix(unit),
            if rate { "/s" } else { "" }
        )
    }

    fn memory_value(&self, memory: Option<(u64, u64)>) -> String {
        let Some((used, total)) = memory.filter(|(_, total)| *total > 0) else {
            return "—".into();
        };
        match self.effective_format() {
            ValueFormat::Used => self.bytes(used as f64, false),
            ValueFormat::UsedTotal => {
                let unit = self.byte_exponent(total as f64);
                let divisor = 1024f64.powi(unit as i32);
                format!(
                    "{:.*} / {:.*} {}",
                    self.decimals(usize::from(unit > 0)),
                    used as f64 / divisor,
                    self.decimals(usize::from(unit > 0)),
                    total as f64 / divisor,
                    self.unit_style.suffix(unit),
                )
            }
            _ => self.percent(Some(used as f64 / total as f64 * 100.)),
        }
    }

    pub fn gpu<'a>(&self, data: &'a Shared) -> Option<&'a GpuMetrics> {
        if self.device.is_empty() {
            data.system.gpus.first()
        } else {
            data.system.gpus.iter().find(|gpu| gpu.id == self.device)
        }
    }

    pub fn disk<'a>(&self, data: &'a Shared) -> Option<&'a DiskMetrics> {
        data.system.disks.iter().find(|disk| disk.id == self.device)
    }

    pub fn value(&self, data: &Shared) -> String {
        match self.metric {
            Metric::Download if !data.connected => "离线".into(),
            Metric::Upload | Metric::TotalSpeed if !data.connected => "—".into(),
            Metric::Download => self.bytes(data.down, true),
            Metric::Upload => self.bytes(data.up, true),
            Metric::TotalSpeed => self.bytes(data.up + data.down, true),
            Metric::Today => self.bytes(data.today.total() as f64, false),
            Metric::Month => self.bytes(data.month.total() as f64, false),
            Metric::Cumulative => self.bytes(data.cumulative.total() as f64, false),
            Metric::Cpu => self.percent(data.system.cpu_percent),
            Metric::Memory => self.memory_value(data.system.memory),
            Metric::Gpu => self.percent(self.gpu(data).and_then(|gpu| gpu.percent)),
            Metric::Vram => self.memory_value(self.gpu(data).and_then(|gpu| gpu.memory)),
            Metric::Disk => self.percent(self.disk(data).and_then(|disk| disk.percent)),
            Metric::DiskRead | Metric::DiskWrite => self
                .disk(data)
                .and_then(|disk| {
                    if self.metric == Metric::DiskRead {
                        disk.read
                    } else {
                        disk.write
                    }
                })
                .map(|v| self.bytes(v, true))
                .unwrap_or_else(|| "—".into()),
        }
    }

    /// Three integer digits at the highest precision this rate can display.
    pub fn network_number_sample(&self) -> Option<String> {
        matches!(
            self.metric,
            Metric::Upload | Metric::Download | Metric::TotalSpeed
        )
        .then(|| {
            let precision = self.decimals(usize::from(self.effective_unit() != ByteUnit::B));
            format!("{:.*}", precision, 888.)
        })
    }

    pub fn samples(&self) -> Vec<String> {
        let mut values = vec!["—".into(), "离线".into()];
        if self.metric.is_rate() || self.metric.group() == "流量" {
            let exponents: Vec<i32> = match self.effective_unit().exponent() {
                Some(unit) => vec![unit as i32],
                None => (0..=4).collect(),
            };
            for exponent in exponents {
                values.push(self.bytes(1023.99 * 1024f64.powi(exponent), self.metric.is_rate()));
            }
        } else if self.metric.is_memory() {
            values.push(self.memory_value(Some((1024u64.pow(4), 1024u64.pow(4)))));
        } else {
            values.push(self.percent(Some(100.)));
        }
        values
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskbarFontWeight(u16);

impl Default for TaskbarFontWeight {
    fn default() -> Self {
        Self::NORMAL
    }
}

impl Serialize for TaskbarFontWeight {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_u16(self.0)
    }
}

impl<'de> Deserialize<'de> for TaskbarFontWeight {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum SavedWeight {
            Number(u16),
            Name(String),
        }
        let value = match SavedWeight::deserialize(deserializer)? {
            SavedWeight::Number(value) => value,
            SavedWeight::Name(name) => match name.as_str() {
                "light" => 300,
                "normal" => 400,
                "bold" => 700,
                _ => return Err(serde::de::Error::custom("未知的字重名称")),
            },
        };
        Self::new(value).ok_or_else(|| serde::de::Error::custom("字重须为 100–900 的整数"))
    }
}

impl TaskbarFontWeight {
    pub const LIGHT: Self = Self(300);
    pub const NORMAL: Self = Self(400);
    pub const BOLD: Self = Self(700);
    pub const ALL: [Self; 3] = [Self::LIGHT, Self::NORMAL, Self::BOLD];

    pub fn new(value: u16) -> Option<Self> {
        (100..=900).contains(&value).then_some(Self(value))
    }

    pub fn label(self) -> &'static str {
        match self.0 {
            300 => "细",
            400 => "正常",
            700 => "粗",
            _ => "自定义",
        }
    }

    pub fn value(self) -> u16 {
        self.0
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TaskbarContent {
    pub rows: Vec<Vec<TaskbarItem>>,
    pub gap: u8,
    #[serde(default = "TaskbarContent::legacy_prefix_gap")]
    pub prefix_gap: u8,
    #[serde(default = "TaskbarContent::legacy_font_family")]
    pub font_family: String,
    pub font_size: f32,
    pub font_weight: TaskbarFontWeight,
}

/// The serialized content shape is shared; each display keeps its own limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayTarget {
    Taskbar,
    Floating,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveDirection {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropIntent {
    Swap(Metric),
    /// An insertion boundary in the original row, before removing the source.
    Insert {
        row: usize,
        index: usize,
    },
}

impl DropIntent {
    pub fn over_cell(
        metric: Metric,
        row: usize,
        column: usize,
        fraction: f32,
        added: bool,
    ) -> Self {
        if added && (0.2..=0.8).contains(&fraction) {
            Self::Swap(metric)
        } else {
            Self::Insert {
                row,
                index: column + usize::from(fraction > 0.5),
            }
        }
    }
}

impl Default for TaskbarContent {
    fn default() -> Self {
        Self {
            rows: vec![
                vec![
                    TaskbarItem::new(Metric::Cpu),
                    TaskbarItem::new(Metric::Upload),
                ],
                vec![
                    TaskbarItem::new(Metric::Memory),
                    TaskbarItem::new(Metric::Download),
                ],
            ],
            gap: 10,
            prefix_gap: 2,
            font_family: "微软雅黑".into(),
            font_size: 12.,
            font_weight: TaskbarFontWeight::NORMAL,
        }
    }
}

impl TaskbarContent {
    pub fn floating() -> Self {
        Self {
            font_family: Self::legacy_font_family(),
            font_size: 13.,
            ..Self::default()
        }
    }

    pub fn floating_legacy(today: bool) -> Self {
        let mut rows = vec![vec![
            TaskbarItem::new(Metric::Download),
            TaskbarItem::new(Metric::Upload),
        ]];
        if today {
            rows.push(vec![TaskbarItem::new(Metric::Today)]);
        }
        Self {
            rows,
            font_family: Self::legacy_font_family(),
            font_size: 13.,
            ..Self::default()
        }
    }

    // Configurations from before the prefix control used a fixed 4 DIP gap.
    fn legacy_prefix_gap() -> u8 {
        4
    }

    fn legacy_font_family() -> String {
        "Segoe UI".into()
    }

    pub fn position(&self, metric: Metric) -> Option<(usize, usize)> {
        self.rows.iter().enumerate().find_map(|(row, items)| {
            items
                .iter()
                .position(|item| item.metric == metric)
                .map(|column| (row, column))
        })
    }

    pub fn directional_target(
        &self,
        metric: Metric,
        direction: MoveDirection,
    ) -> Option<DropIntent> {
        let (row, column) = self.position(metric)?;
        let (next_row, next_column) = match direction {
            MoveDirection::Left => (row, column.checked_sub(1)?),
            MoveDirection::Right if column + 1 < self.rows[row].len() => (row, column + 1),
            MoveDirection::Right => return None,
            MoveDirection::Up => (row.checked_sub(1)?, column),
            MoveDirection::Down if row + 1 < self.rows.len() => (row + 1, column),
            MoveDirection::Down => return None,
        };
        Some(self.rows[next_row].get(next_column).map_or(
            DropIntent::Insert {
                row: next_row,
                index: self.rows[next_row].len(),
            },
            |item| DropIntent::Swap(item.metric),
        ))
    }

    /// One calculation shared by the provisional preview and the committed move.
    /// Unadded items can only be inserted; an occupied slot is never overwritten.
    pub fn placed(&self, item: &TaskbarItem, intent: DropIntent) -> Option<Self> {
        let mut next = self.clone();
        match intent {
            DropIntent::Swap(target) => {
                let (source_row, source_column) = self.position(item.metric)?;
                let (target_row, target_column) = self.position(target)?;
                next.rows[source_row][source_column] = self.rows[target_row][target_column].clone();
                next.rows[target_row][target_column] = self.rows[source_row][source_column].clone();
            }
            DropIntent::Insert { row, index } => {
                if row >= self.rows.len() || index > self.rows[row].len() {
                    return None;
                }
                if self.item(item.metric).is_some() {
                    next.move_item(item.metric, row, index);
                } else {
                    next.rows[row].insert(index, item.clone());
                }
            }
        }
        Some(next)
    }

    pub fn legacy(horizontal: bool) -> Self {
        let down = TaskbarItem::new(Metric::Download);
        let up = TaskbarItem::new(Metric::Upload);
        Self {
            rows: if horizontal {
                vec![vec![down, up]]
            } else {
                vec![vec![down], vec![up]]
            },
            gap: 10,
            prefix_gap: 4,
            font_family: Self::legacy_font_family(),
            ..Self::default()
        }
    }

    pub fn validate(&self) -> Result<()> {
        self.validate_for(DisplayTarget::Taskbar)
    }

    pub fn validate_for(&self, target: DisplayTarget) -> Result<()> {
        ensure!(
            !self.font_family.trim().is_empty()
                && self.font_family.chars().count() <= 128
                && !self.font_family.chars().any(char::is_control),
            "字体名称无效"
        );
        ensure!(
            (8. ..=20.).contains(&self.font_size),
            "字号须为 8–20 px，可输入小数"
        );
        ensure!(
            (1..=if target == DisplayTarget::Taskbar {
                2
            } else {
                Metric::ALL.len()
            })
                .contains(&self.rows.len()),
            "显示布局行数超出允许范围"
        );
        ensure!(self.gap <= 32, "指标间距须为 0–32");
        ensure!(self.prefix_gap <= 32, "前缀间距须为 0–32");
        let mut seen = std::collections::HashSet::new();
        for item in self.rows.iter().flatten() {
            ensure!(
                [item.text_color, item.background_color]
                    .into_iter()
                    .flatten()
                    .all(|color| color <= 0xffffff),
                "颜色须为有效的 RGB 颜色"
            );
            ensure!(seen.insert(item.metric), "不能重复添加同一指标");
            ensure!(
                Self::valid_label(&item.label),
                "指标文案最多 16 个字符，不能包含换行或控制字符"
            );
            ensure!(
                item.precision.is_none_or(|precision| precision <= 2),
                "小数位须为自动、0、1 或 2"
            );
            ensure!(
                item.device.len() <= 256 && !item.device.chars().any(char::is_control),
                "监测设备标识无效"
            );
            ensure!(
                item.format == ValueFormat::Auto
                    || item
                        .metric
                        .formats()
                        .iter()
                        .any(|(format, _)| *format == item.format),
                "该指标不支持此显示格式"
            );
        }
        ensure!(!seen.is_empty(), "至少保留一个指标");
        Ok(())
    }

    pub fn valid_label(label: &str) -> bool {
        label.chars().count() <= 16 && !label.chars().any(char::is_control)
    }

    pub fn item(&self, metric: Metric) -> Option<&TaskbarItem> {
        self.rows
            .iter()
            .flatten()
            .find(|item| item.metric == metric)
    }

    pub fn item_mut(&mut self, metric: Metric) -> Option<&mut TaskbarItem> {
        self.rows
            .iter_mut()
            .flatten()
            .find(|item| item.metric == metric)
    }

    pub fn add(&mut self, metric: Metric) {
        if self.item(metric).is_none() {
            let row = self.rows.iter_mut().min_by_key(|row| row.len()).unwrap();
            row.push(TaskbarItem::new(metric));
        }
    }

    pub fn remove(&mut self, metric: Metric) {
        if self.rows.iter().map(Vec::len).sum::<usize>() > 1 {
            for row in &mut self.rows {
                row.retain(|item| item.metric != metric);
            }
        }
    }

    /// `index` is an insertion boundary in the target row before removal.
    pub fn move_item(&mut self, metric: Metric, target_row: usize, index: usize) {
        if target_row >= self.rows.len() {
            return;
        }
        let source = self.rows.iter().enumerate().find_map(|(r, row)| {
            row.iter()
                .position(|item| item.metric == metric)
                .map(|i| (r, i))
        });
        let Some((source_row, source_index)) = source else {
            return;
        };
        let item = self.rows[source_row].remove(source_index);
        let index = index.saturating_sub(usize::from(
            source_row == target_row && source_index < index,
        ));
        let index = index.min(self.rows[target_row].len());
        self.rows[target_row].insert(index, item);
    }

    pub fn set_rows(&mut self, count: usize) {
        if count == self.rows.len() || !(1..=Metric::ALL.len()).contains(&count) {
            return;
        }
        let columns = self.rows.iter().map(Vec::len).max().unwrap_or(0);
        let items: Vec<_> = (0..columns)
            .flat_map(|column| {
                self.rows
                    .iter()
                    .filter_map(move |row| row.get(column).cloned())
            })
            .collect();
        self.rows = vec![Vec::new(); count];
        for (index, item) in items.into_iter().enumerate() {
            self.rows[index % count].push(item);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn network_prefixes_add_one_colon_and_keep_empty_labels_hidden() {
        for metric in [Metric::Upload, Metric::Download, Metric::TotalSpeed] {
            let mut item = TaskbarItem::new(metric);
            let marker = item.label.clone();
            let saved = toml::to_string(&item).unwrap();
            assert_eq!(item.prefix_label(), format!("{marker}:"));
            assert_eq!(item.text("128 K/s", true), format!("{marker}: 128 K/s"));
            assert_eq!(toml::to_string(&item).unwrap(), saved);
            assert!(item.direction_icon().is_some());
            assert_eq!(item.prefix_text(), ":");
            for suffix in [":", "："] {
                item.label = format!("{marker}{suffix}");
                assert_eq!(item.prefix_label(), item.label);
                assert!(item.direction_icon().is_some());
                assert_eq!(item.prefix_text(), suffix);
            }
            item.label = "网速".into();
            assert_eq!(item.prefix_text(), "网速:");
            assert!(item.direction_icon().is_none());
            item.label.clear();
            assert!(item.prefix_text().is_empty());
            assert!(item.direction_icon().is_none());
            assert_eq!(item.text("128 K/s", false), "128 K/s");
        }
        let mut cpu = TaskbarItem::new(Metric::Cpu);
        cpu.label = "CPU".into();
        assert_eq!(cpu.prefix_text(), "CPU");
    }

    #[test]
    fn floating_multiline_reflow_preserves_column_order_and_item_properties() {
        let mut content = TaskbarContent::floating();
        for metric in Metric::ALL {
            content.add(metric);
        }
        let cpu = content.item_mut(Metric::Cpu).unwrap();
        cpu.label = "处理器".into();
        cpu.text_color = Some(0x123456);
        cpu.hide_percent = true;
        let flatten = |content: &TaskbarContent| {
            let columns = content.rows.iter().map(Vec::len).max().unwrap();
            (0..columns)
                .flat_map(|col| {
                    content
                        .rows
                        .iter()
                        .filter_map(move |row| row.get(col).cloned())
                })
                .collect::<Vec<_>>()
        };
        let original = flatten(&content);
        for rows in [1, 2, 3, 13, 3, 1] {
            content.set_rows(rows);
            assert_eq!(flatten(&content), original);
            content.validate_for(DisplayTarget::Floating).unwrap();
        }
        content.set_rows(13);
        content.remove(Metric::Cpu);
        assert!(content.rows.iter().any(Vec::is_empty));
        let moved = content
            .placed(
                &TaskbarItem::new(Metric::Cpu),
                DropIntent::Insert {
                    row: 12,
                    index: content.rows[12].len(),
                },
            )
            .unwrap();
        assert_eq!(moved.position(Metric::Cpu), Some((12, 1)));
        let target = moved
            .directional_target(Metric::Cpu, MoveDirection::Up)
            .unwrap();
        let moved = moved
            .placed(moved.item(Metric::Cpu).unwrap(), target)
            .unwrap();
        assert_eq!(moved.position(Metric::Cpu).unwrap().0, 11);
        moved.validate_for(DisplayTarget::Floating).unwrap();
        assert!(moved.validate().is_err());
    }

    #[test]
    fn hidden_percent_applies_to_all_usage_metrics_and_preserves_capacity_units() {
        let data = Shared {
            system: crate::model::SystemMetrics {
                cpu_percent: Some(12.34),
                memory: Some((1, 2)),
                gpus: vec![GpuMetrics {
                    percent: Some(12.34),
                    memory: Some((1, 2)),
                    ..Default::default()
                }],
                disks: vec![DiskMetrics {
                    percent: Some(12.34),
                    ..Default::default()
                }],
            },
            ..Default::default()
        };
        for metric in [
            Metric::Cpu,
            Metric::Memory,
            Metric::Gpu,
            Metric::Vram,
            Metric::Disk,
        ] {
            let mut item = TaskbarItem::new(metric);
            if metric.is_memory() {
                item.format = ValueFormat::Percent;
            }
            item.precision = Some(2);
            let shown = item.value(&data);
            assert!(shown.ends_with('%'));
            item.hide_percent = true;
            assert_eq!(item.value(&data), shown.trim_end_matches('%'));
            assert!(item.samples().iter().all(|value| !value.contains('%')));
            if metric.is_memory() {
                item.format = ValueFormat::Used;
                assert!(item.value(&data).ends_with('B'));
                item.format = ValueFormat::UsedTotal;
                assert!(item.value(&data).contains('/'));
            }
            assert_eq!(item.value(&Shared::default()), "—");
        }
    }

    #[test]
    fn appearance_roundtrips_and_old_configs_keep_the_original_defaults() {
        let legacy: TaskbarContent =
            toml::from_str("gap = 8\nrows = [[{ metric = 'cpu' }]]\n").unwrap();
        assert_eq!(legacy.font_family, "Segoe UI");
        assert_eq!(legacy.font_size, 12.);
        assert_eq!(legacy.font_weight, TaskbarFontWeight::NORMAL);
        let cpu = legacy.item(Metric::Cpu).unwrap();
        assert!(!cpu.hide_percent && cpu.text_color.is_none() && cpu.background_color.is_none());
        let mut content = legacy;
        content.font_family = "Microsoft YaHei UI".into();
        content.font_size = 20.;
        content.font_weight = TaskbarFontWeight::BOLD;
        let cpu = content.item_mut(Metric::Cpu).unwrap();
        cpu.text_color = Some(0);
        cpu.background_color = Some(0xffffff);
        cpu.hide_percent = true;
        content.validate().unwrap();
        for weight in TaskbarFontWeight::ALL {
            content.font_weight = weight;
            let saved = toml::to_string(&content).unwrap();
            assert_eq!(toml::from_str::<TaskbarContent>(&saved).unwrap(), content);
        }
        for (name, value) in [("light", 300), ("normal", 400), ("bold", 700)] {
            let loaded: TaskbarContent =
                toml::from_str(&format!("font_weight = '{name}'")).unwrap();
            assert_eq!(loaded.font_weight.value(), value);
        }
        for value in [100, 350, 450, 550, 900] {
            content.font_weight = TaskbarFontWeight::new(value).unwrap();
            let saved = toml::to_string(&content).unwrap();
            assert!(saved.contains(&format!("font_weight = {value}")));
            assert_eq!(toml::from_str::<TaskbarContent>(&saved).unwrap(), content);
        }
        for invalid in ["99", "901", "-1", "450.5", "'unknown'"] {
            assert!(toml::from_str::<TaskbarContent>(&format!("font_weight = {invalid}")).is_err());
        }
        for size in ["8", "12", "12.25", "12.5", "12.75", "13.125", "20"] {
            let loaded: TaskbarContent = toml::from_str(&format!("font_size = {size}")).unwrap();
            loaded.validate().unwrap();
            assert_eq!(loaded.font_size, size.parse::<f32>().unwrap());
            let saved = toml::to_string(&loaded).unwrap();
            assert_eq!(toml::from_str::<TaskbarContent>(&saved).unwrap(), loaded);
        }
        for size in ["7.99", "20.01", "nan", "inf", "-inf"] {
            let loaded: TaskbarContent = toml::from_str(&format!("font_size = {size}")).unwrap();
            assert!(loaded.validate().is_err());
        }
        content.font_size = 21.;
        assert!(content.validate().is_err());
        content.font_size = 12.;
        content.item_mut(Metric::Cpu).unwrap().text_color = Some(0x1000000);
        assert!(content.validate().is_err());
    }

    #[test]
    fn all_metrics_format_live_values_and_keep_unavailable_devices_unknown() {
        use crate::model::{Bytes, SystemMetrics};
        let data = Shared {
            connected: true,
            down: 2. * 1024f64.powi(2),
            up: 1024f64.powi(2),
            today: Bytes {
                up: 1024u64.pow(3),
                down: 0,
            },
            month: Bytes {
                up: 2 * 1024u64.pow(3),
                down: 0,
            },
            cumulative: Bytes {
                up: 3 * 1024u64.pow(3),
                down: 0,
            },
            system: SystemMetrics {
                cpu_percent: Some(12.34),
                memory: Some((4 * 1024u64.pow(3), 8 * 1024u64.pow(3))),
                gpus: vec![GpuMetrics {
                    id: "gpu-a".into(),
                    name: "GPU A".into(),
                    percent: Some(27.55),
                    memory: Some((2 * 1024u64.pow(3), 8 * 1024u64.pow(3))),
                    ..Default::default()
                }],
                disks: vec![DiskMetrics {
                    id: String::new(),
                    name: "所有磁盘".into(),
                    percent: Some(8.1),
                    read: Some(4. * 1024f64.powi(2)),
                    write: Some(1024.),
                }],
            },
            ..Default::default()
        };
        for (metric, expected) in [
            (Metric::Download, "2.0 M/s"),
            (Metric::Upload, "1.0 M/s"),
            (Metric::TotalSpeed, "3.0 M/s"),
            (Metric::Today, "1.0 G"),
            (Metric::Month, "2.0 G"),
            (Metric::Cumulative, "3.0 G"),
            (Metric::Cpu, "12%"),
            (Metric::Memory, "50%"),
            (Metric::Gpu, "28%"),
            (Metric::Vram, "2.0 G"),
            (Metric::Disk, "8%"),
            (Metric::DiskRead, "4.0 M/s"),
            (Metric::DiskWrite, "1.0 K/s"),
        ] {
            assert_eq!(
                TaskbarItem::new(metric).value(&data),
                expected,
                "{metric:?}"
            );
        }
        let mut gpu = TaskbarItem::new(Metric::Gpu);
        gpu.precision = Some(2);
        assert_eq!(gpu.value(&data), "27.55%");
        gpu.device = "removed-device".into();
        assert_eq!(gpu.value(&data), "—");
        let mut rate = TaskbarItem::new(Metric::TotalSpeed);
        rate.format = ValueFormat::Kb;
        rate.precision = Some(0);
        assert_eq!(rate.value(&data), "3072 K/s");
        let mut layout = TaskbarContent::default();
        for metric in Metric::ALL {
            layout.add(metric);
        }
        layout.validate().unwrap();
        assert_eq!(layout.rows.iter().flatten().count(), 13);
        let serialized = toml::to_string(&layout).unwrap();
        assert_eq!(
            toml::from_str::<TaskbarContent>(&serialized).unwrap(),
            layout
        );
    }

    #[test]
    fn explicit_units_cover_rates_totals_and_memory_without_changing_legacy_settings() {
        let mut item = TaskbarItem::new(Metric::Download);
        item.unit_style = UnitStyle::Full;
        for (unit, expected) in [
            (ByteUnit::Auto, "1.0 MB/s"),
            (ByteUnit::B, "1048576 B/s"),
            (ByteUnit::K, "1024.0 KB/s"),
            (ByteUnit::M, "1.0 MB/s"),
            (ByteUnit::G, "0.0 GB/s"),
            (ByteUnit::T, "0.0 TB/s"),
        ] {
            item.unit = Some(unit);
            assert_eq!(item.bytes(1024f64.powi(2), true), expected);
            assert_eq!(
                item.bytes(1024f64.powi(2), false),
                expected.trim_end_matches("/s")
            );
            item.precision = Some(2);
            assert!(item.bytes(0., true).starts_with("0.00 "));
            item.precision = None;
        }
        item = TaskbarItem::new(Metric::Memory);
        item.unit_style = UnitStyle::Full;
        item.format = ValueFormat::UsedTotal;
        let memory = Some((1024u64.pow(3), 2 * 1024u64.pow(3)));
        assert_eq!(item.memory_value(memory), "1.0 / 2.0 GB");
        item.unit = Some(ByteUnit::M);
        assert_eq!(item.memory_value(memory), "1024.0 / 2048.0 MB");
        item.format = ValueFormat::Percent;
        assert_eq!(item.memory_value(memory), "50%");
        let old: TaskbarItem = toml::from_str("metric = 'upload'\nformat = 'kb'\n").unwrap();
        assert_eq!(old.effective_unit(), ByteUnit::K);
        assert_eq!(old.bytes(1024., true), "1.0 K/s");
        assert!(!toml::to_string(&old).unwrap().contains("unit ="));
        for unit in ByteUnit::ALL.map(|(unit, _)| unit) {
            let mut layout = TaskbarContent::default();
            layout.item_mut(Metric::Upload).unwrap().unit = Some(unit);
            layout.validate().unwrap();
            let saved = toml::to_string(&layout).unwrap();
            assert_eq!(toml::from_str::<TaskbarContent>(&saved).unwrap(), layout);
        }
    }

    #[test]
    fn unit_choices_follow_metric_categories() {
        use Metric::*;
        for (metrics, expected) in [
            (&[Download, Upload, TotalSpeed][..], &["自动", "K", "M"][..]),
            (
                &[Today, Month, Cumulative][..],
                &["自动", "M", "G", "T"][..],
            ),
            (&[Vram, DiskRead, DiskWrite][..], &["自动", "M", "G"][..]),
            (&[Memory][..], &["自动", "K", "M"][..]),
            (&[Cpu, Gpu, Disk][..], &[][..]),
        ] {
            for metric in metrics {
                assert_eq!(
                    metric
                        .unit_choices()
                        .iter()
                        .map(|(_, name)| *name)
                        .collect::<Vec<_>>(),
                    expected,
                    "{metric:?}"
                );
            }
        }
    }

    #[test]
    fn unit_style_changes_only_the_suffix_and_roundtrips_independently() {
        let mut item = TaskbarItem::new(Metric::Upload);
        assert_eq!(
            Metric::Upload
                .unit_choices()
                .iter()
                .map(|(_, label)| *label)
                .collect::<Vec<_>>(),
            ["自动", "K", "M"]
        );
        assert_eq!(item.unit_style, UnitStyle::Short);
        assert!(
            toml::to_string(&item)
                .unwrap()
                .contains("unit_style = \"short\"")
        );
        item.unit_style = UnitStyle::Full;
        let saved_full = toml::to_string(&item).unwrap();
        assert!(saved_full.contains("unit_style = \"full\""));
        assert_eq!(
            toml::from_str::<TaskbarItem>(&saved_full)
                .unwrap()
                .unit_style,
            UnitStyle::Full
        );
        for (unit, value, full, short) in [
            (ByteUnit::Auto, 128. * 1024., "128.0 KB/s", "128.0 K/s"),
            (
                ByteUnit::K,
                2. * 1024f64.powi(2),
                "2048.0 KB/s",
                "2048.0 K/s",
            ),
            (ByteUnit::M, 2. * 1024f64.powi(2), "2.0 MB/s", "2.0 M/s"),
            (ByteUnit::Auto, 128., "128 B/s", "128 B/s"),
            (ByteUnit::Auto, 2. * 1024f64.powi(3), "2.0 GB/s", "2.0 G/s"),
            (ByteUnit::T, 2. * 1024f64.powi(4), "2.0 TB/s", "2.0 T/s"),
        ] {
            item.unit = Some(unit);
            item.unit_style = UnitStyle::Full;
            assert_eq!(item.bytes(value, true), full);
            item.unit_style = item.unit_style.toggled();
            assert_eq!(item.bytes(value, true), short);
            assert_eq!(item.bytes(value, false), short.trim_end_matches("/s"));
            assert_eq!(
                toml::from_str::<TaskbarItem>(&toml::to_string(&item).unwrap()).unwrap(),
                item
            );
        }
        item = TaskbarItem::new(Metric::Memory);
        item.format = ValueFormat::UsedTotal;
        item.unit = Some(ByteUnit::M);
        item.unit_style = UnitStyle::Short;
        assert_eq!(
            item.memory_value(Some((1024u64.pow(3), 2 * 1024u64.pow(3)))),
            "1024.0 / 2048.0 M"
        );
        item.format = ValueFormat::Percent;
        assert_eq!(item.memory_value(Some((1, 2))), "50%");
        let legacy: TaskbarItem =
            toml::from_str("metric = 'download'\nformat = 'compact'\n").unwrap();
        assert_eq!(legacy.unit_style, UnitStyle::Short);
        assert_eq!(legacy.bytes(1024., true), "1.0K/s");
        let explicit_full: TaskbarItem =
            toml::from_str("metric = 'download'\nunit_style = 'full'\n").unwrap();
        assert_eq!(explicit_full.bytes(1024., true), "1.0 KB/s");
    }

    #[test]
    fn drag_reorders_in_both_directions_and_moves_to_empty_rows() {
        let mut layout = TaskbarContent::legacy(true);
        layout.add(Metric::Cpu);
        layout.move_item(Metric::Download, 0, 3);
        assert_eq!(
            layout.rows[0]
                .iter()
                .map(|item| item.metric)
                .collect::<Vec<_>>(),
            [Metric::Upload, Metric::Cpu, Metric::Download]
        );
        layout.move_item(Metric::Download, 0, 0);
        layout.set_rows(2);
        layout.move_item(Metric::Upload, 0, 1);
        assert!(layout.rows[1].is_empty());
        layout.move_item(Metric::Cpu, 1, 0);
        assert_eq!(layout.rows[1][0].metric, Metric::Cpu);
        layout.validate().unwrap();
    }

    #[test]
    fn layout_validation_and_removal_keep_one_unique_metric() {
        let mut layout = TaskbarContent::legacy(false);
        layout.remove(Metric::Upload);
        layout.remove(Metric::Download);
        assert!(layout.item(Metric::Download).is_some());
        layout.add(Metric::Download);
        layout.validate().unwrap();
        layout.rows[0].push(TaskbarItem::new(Metric::Download));
        assert!(layout.validate().is_err());
        assert!(!TaskbarContent::valid_label("上行\n"));
        assert!(!TaskbarContent::valid_label(&"字".repeat(17)));
        assert!(TaskbarContent::valid_label(""));
    }

    fn metrics(layout: &TaskbarContent) -> Vec<Vec<Metric>> {
        layout
            .rows
            .iter()
            .map(|row| row.iter().map(|item| item.metric).collect())
            .collect()
    }

    #[test]
    fn default_grid_and_four_way_moves_match_visible_coordinates() {
        use Metric::*;
        use MoveDirection::*;
        let grid = TaskbarContent::default();
        assert_eq!(metrics(&grid), [vec![Cpu, Upload], vec![Memory, Download]]);
        assert_eq!(metrics(&TaskbarContent::floating()), metrics(&grid));
        grid.validate().unwrap();
        for (source, direction, target) in [
            (Cpu, Right, Upload),
            (Cpu, Down, Memory),
            (Download, Up, Upload),
            (Download, Left, Memory),
        ] {
            let intent = grid.directional_target(source, direction).unwrap();
            assert_eq!(intent, DropIntent::Swap(target));
            let moved = grid.placed(grid.item(source).unwrap(), intent).unwrap();
            assert_eq!(moved.position(source), grid.position(target));
            assert_eq!(moved.position(target), grid.position(source));
            moved.validate().unwrap();
        }
        for (source, direction) in [(Cpu, Left), (Cpu, Up), (Download, Right), (Download, Down)] {
            assert!(grid.directional_target(source, direction).is_none());
        }
        let mut short = grid.clone();
        short.remove(Download);
        let intent = short.directional_target(Upload, Down).unwrap();
        assert_eq!(intent, DropIntent::Insert { row: 1, index: 1 });
        assert_eq!(
            metrics(&short.placed(short.item(Upload).unwrap(), intent).unwrap()),
            [vec![Cpu], vec![Memory, Upload]]
        );
        short.set_rows(1);
        assert!(short.directional_target(Cpu, Up).is_none());
        assert!(short.directional_target(Cpu, Down).is_none());
    }

    #[test]
    fn center_swap_edges_insert_and_new_items_keep_all_existing_metrics() {
        use Metric::*;
        let grid = TaskbarContent::default();
        assert_eq!(
            DropIntent::over_cell(Upload, 0, 1, 0.5, true),
            DropIntent::Swap(Upload)
        );
        assert_eq!(
            DropIntent::over_cell(Upload, 0, 1, 0.1, true),
            DropIntent::Insert { row: 0, index: 1 }
        );
        assert_eq!(
            DropIntent::over_cell(Upload, 0, 1, 0.9, true),
            DropIntent::Insert { row: 0, index: 2 }
        );
        assert_eq!(
            DropIntent::over_cell(Upload, 0, 1, 0.5, false),
            DropIntent::Insert { row: 0, index: 1 }
        );
        let download = grid.item(Download).unwrap();
        let before = grid
            .placed(download, DropIntent::Insert { row: 0, index: 1 })
            .unwrap();
        assert_eq!(
            metrics(&before),
            [vec![Cpu, Download, Upload], vec![Memory]]
        );
        let after = before
            .placed(
                before.item(Cpu).unwrap(),
                DropIntent::Insert { row: 0, index: 3 },
            )
            .unwrap();
        assert_eq!(metrics(&after), [vec![Download, Upload, Cpu], vec![Memory]]);
        assert_eq!(
            after
                .placed(after.item(Cpu).unwrap(), DropIntent::Swap(Cpu))
                .unwrap(),
            after
        );
        let new_item = TaskbarItem {
            label: "图形".into(),
            precision: Some(2),
            device: "chosen-gpu".into(),
            ..TaskbarItem::new(Gpu)
        };
        assert!(grid.placed(&new_item, DropIntent::Swap(Cpu)).is_none());
        let added = grid
            .placed(&new_item, DropIntent::Insert { row: 1, index: 1 })
            .unwrap();
        assert_eq!(
            metrics(&added),
            [vec![Cpu, Upload], vec![Memory, Gpu, Download]]
        );
        assert_eq!(added.item(Gpu), Some(&new_item));
        added.validate().unwrap();
        assert!(
            grid.placed(download, DropIntent::Insert { row: 2, index: 0 })
                .is_none()
        );
        assert!(
            grid.placed(download, DropIntent::Insert { row: 0, index: 99 })
                .is_none()
        );
    }

    #[test]
    fn system_values_remain_available_offline_and_labels_can_be_empty() {
        let data = Shared {
            system: crate::model::SystemMetrics {
                cpu_percent: Some(18.4),
                memory: Some((8 * 1024u64.pow(3), 16 * 1024u64.pow(3))),
                ..Default::default()
            },
            ..Shared::default()
        };
        let cpu = TaskbarItem::new(Metric::Cpu);
        assert_eq!(cpu.value(&data), "18%");
        let mut memory = TaskbarItem::new(Metric::Memory);
        assert_eq!(memory.value(&data), "50%");
        memory.detailed = true;
        assert_eq!(memory.value(&data), "8.0 / 16.0 G");
        memory.label.clear();
        assert_eq!(memory.text(&memory.value(&data), false), "8.0 / 16.0 G");
        assert_eq!(TaskbarItem::new(Metric::Download).value(&data), "离线");
        assert_eq!(TaskbarItem::new(Metric::Cpu).value(&Shared::default()), "—");
    }
}
