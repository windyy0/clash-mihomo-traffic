use crate::model::Bytes;
#[cfg(test)]
use crate::model::SeriesPoint;
use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    path::Path,
    time::Duration,
};

pub const MINUTE: i64 = 60_000;
const HISTOGRAM_STEP: i64 = 10_000;
const CACHE_LENGTH: i64 = 15 * MINUTE;
const BURST_INCREASE: u128 = 5 * 1024 * 1024;
#[path = "analysis_history.rs"]
mod history;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AnalysisRange {
    #[default]
    FiveMinutes,
    Hour,
    Day,
}
impl AnalysisRange {
    pub const ALL: [Self; 3] = [Self::FiveMinutes, Self::Hour, Self::Day];
    pub fn duration(self) -> i64 {
        match self {
            Self::FiveMinutes => 5 * MINUTE,
            Self::Hour => 60 * MINUTE,
            Self::Day => 1440 * MINUTE,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::FiveMinutes => "近5分钟",
            Self::Hour => "近1小时",
            Self::Day => "近24小时",
        }
    }
    pub fn precision(self) -> &'static str {
        if self == Self::FiveMinutes {
            "秒级采样"
        } else {
            "分钟历史"
        }
    }
    pub fn index(self) -> usize {
        match self {
            Self::FiveMinutes => 0,
            Self::Hour => 1,
            Self::Day => 2,
        }
    }
    pub fn bucket_sizes(self) -> &'static [BucketSize] {
        use BucketSize::*;
        match self {
            Self::FiveMinutes => &[Seconds10, Seconds30, Minute],
            Self::Hour => &[Minute, Minutes5, Minutes15],
            Self::Day => &[Minutes5, Minutes15, Minutes30, Hour],
        }
    }
    pub fn default_bucket(self) -> BucketSize {
        match self {
            Self::FiveMinutes => BucketSize::Seconds10,
            Self::Hour => BucketSize::Minutes5,
            Self::Day => BucketSize::Hour,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BucketSize {
    Seconds10,
    Seconds30,
    Minute,
    Minutes5,
    Minutes15,
    Minutes30,
    Hour,
    Hours3,
    Hours6,
    Hours12,
    Day,
    Days3,
    Week,
    Days30,
    Days90,
}
impl BucketSize {
    pub fn millis(self) -> i64 {
        match self {
            Self::Seconds10 => 10_000,
            Self::Seconds30 => 30_000,
            Self::Minute => MINUTE,
            Self::Minutes5 => 5 * MINUTE,
            Self::Minutes15 => 15 * MINUTE,
            Self::Minutes30 => 30 * MINUTE,
            Self::Hour => 60 * MINUTE,
            Self::Hours3 => 180 * MINUTE,
            Self::Hours6 => 360 * MINUTE,
            Self::Hours12 => 720 * MINUTE,
            Self::Day => 1440 * MINUTE,
            Self::Days3 => 4320 * MINUTE,
            Self::Week => 10080 * MINUTE,
            Self::Days30 => 43200 * MINUTE,
            Self::Days90 => 129600 * MINUTE,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Seconds10 => "10秒",
            Self::Seconds30 => "30秒",
            Self::Minute => "1分钟",
            Self::Minutes5 => "5分钟",
            Self::Minutes15 => "15分钟",
            Self::Minutes30 => "30分钟",
            Self::Hour => "1小时",
            Self::Hours3 => "3小时",
            Self::Hours6 => "6小时",
            Self::Hours12 => "12小时",
            Self::Day => "1天",
            Self::Days3 => "3天",
            Self::Week => "7天",
            Self::Days30 => "30天",
            Self::Days90 => "90天",
        }
    }
}

#[derive(Clone, Copy, Default, Debug)]
pub struct BucketValue {
    pub bytes: Bytes,
    pub covered_ms: u64,
}
impl BucketValue {
    pub fn complete(self, duration: i64) -> bool {
        self.covered_ms >= duration as u64
    }
    pub fn observed(self) -> bool {
        self.covered_ms > 0 || self.bytes.total() > 0
    }
}
#[derive(Clone, Copy, Debug)]
pub struct ComparisonBucket {
    pub from: i64,
    pub previous_from: i64,
    pub duration: i64,
    pub current: BucketValue,
    pub previous: BucketValue,
}
impl ComparisonBucket {
    pub fn contains(self, time: i64) -> bool {
        time >= self.from && time < self.from + self.duration
    }
    pub fn comparable(self) -> bool {
        self.current.complete(self.duration) && self.previous.complete(self.duration)
    }
}

#[derive(Clone, Default, Debug, serde::Serialize, serde::Deserialize)]
struct FineTimeline {
    covered: Vec<u32>,
    total: Trace,
    apps: BTreeMap<String, Trace>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AnalysisRequest {
    pub id: u64,
    pub generation: u64,
    pub range: AnalysisRange,
    pub anchor: i64,
    pub windows: Option<ComparisonWindows>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ComparisonWindows {
    pub from: i64,
    pub to: i64,
    pub previous_from: i64,
    pub previous_to: i64,
}
impl AnalysisRequest {
    pub fn new(id: u64, generation: u64, range: AnalysisRange, click: i64) -> Self {
        let step = if range == AnalysisRange::FiveMinutes {
            1000
        } else {
            MINUTE
        };
        Self {
            id,
            generation,
            range,
            anchor: click.div_euclid(step) * step,
            windows: None,
        }
    }
    pub fn historical(id: u64, generation: u64, from: i64, to: i64, now: i64) -> Result<Self> {
        anyhow::ensure!(
            from.rem_euclid(MINUTE) == 0 && to.rem_euclid(MINUTE) == 0,
            "历史分析时间须对齐到分钟"
        );
        anyhow::ensure!(
            to > from && to - from <= 3660 * 86_400_000_i64,
            "时间范围无效或超过十年"
        );
        let end = to.min(now.div_euclid(MINUTE) * MINUTE);
        anyhow::ensure!(end > from, "所选范围尚无已结束的分钟");
        let previous_from = from - (to - from);
        Ok(Self {
            id,
            generation,
            range: AnalysisRange::Day,
            anchor: end,
            windows: Some(ComparisonWindows {
                from,
                to: end,
                previous_from,
                previous_to: previous_from + end - from,
            }),
        })
    }
    pub fn duration(&self) -> i64 {
        self.windows
            .map_or_else(|| self.range.duration(), |w| w.to - w.from)
    }
    pub fn bucket_sizes(&self) -> Vec<BucketSize> {
        if self.windows.is_none() {
            return self.range.bucket_sizes().to_vec();
        }
        use BucketSize::*;
        [
            Minute, Minutes5, Minutes15, Minutes30, Hour, Hours3, Hours6, Hours12, Day, Days3,
            Week, Days30, Days90,
        ]
        .into_iter()
        .filter(|s| {
            s.millis() <= self.duration() && (self.duration() + s.millis() - 1) / s.millis() <= 180
        })
        .collect()
    }
    pub fn default_bucket(&self) -> BucketSize {
        if self.windows.is_none() {
            return self.range.default_bucket();
        }
        self.bucket_sizes()
            .into_iter()
            .find(|s| (self.duration() + s.millis() - 1) / s.millis() <= 60)
            .unwrap_or(BucketSize::Days90)
    }
}

#[derive(Clone, Debug)]
struct Sample {
    from: i64,
    to: i64,
    total: Bytes,
    apps: HashMap<String, Bytes>,
}

#[derive(Clone, Default)]
pub struct SampleWindow {
    samples: VecDeque<Sample>,
    last_time: Option<i64>,
}
impl SampleWindow {
    pub fn latest_sample(&self) -> Option<i64> {
        self.samples.back().map(|sample| sample.to)
    }
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    /// Returns true when the wall clock moved backwards and the cache was reset.
    pub fn advance(&mut self, now: i64) -> bool {
        let reset = self.last_time.is_some_and(|last| now < last);
        if reset {
            self.clear();
        }
        self.last_time = Some(now);
        while self
            .samples
            .front()
            .is_some_and(|sample| sample.to <= now - CACHE_LENGTH)
        {
            self.samples.pop_front();
        }
        reset
    }
    pub fn record(&mut self, from: i64, to: i64, total: Bytes, apps: &HashMap<String, Bytes>) {
        self.advance(to);
        if to <= from || to - from > 3000 || self.samples.back().is_some_and(|last| from < last.to)
        {
            return;
        }
        self.samples.push_back(Sample {
            from,
            to,
            total,
            apps: apps
                .iter()
                .filter(|(_, value)| value.total() > 0)
                .map(|(key, value)| (key.clone(), *value))
                .collect(),
        });
    }
    pub fn snapshot(&self, request: AnalysisRequest) -> Snapshot {
        let mut snapshot = Snapshot::new(request);
        for sample in &self.samples {
            let from = sample.from.max(snapshot.start);
            let to = sample.to.min(snapshot.request.anchor);
            if from >= to {
                continue;
            }
            let mut cursor = from;
            while cursor < to {
                let index = ((cursor - snapshot.start) / MINUTE) as usize;
                let end = to.min(snapshot.start + (index as i64 + 1) * MINUTE);
                snapshot.covered[index] += (end - cursor) as u32;
                let cut = |bytes| portion(bytes, sample.from, sample.to, cursor, end);
                snapshot.total.add(index, cut(sample.total));
                for (key, bytes) in &sample.apps {
                    snapshot
                        .apps
                        .entry(key.clone())
                        .or_default()
                        .add(index, cut(*bytes));
                }
                cursor = end;
            }
            let fine_start = snapshot.previous_from();
            let mut cursor = sample.from.max(fine_start);
            if let Some(fine) = &mut snapshot.fine {
                while cursor < to {
                    let index = ((cursor - fine_start) / HISTOGRAM_STEP) as usize;
                    let end = to.min(fine_start + (index as i64 + 1) * HISTOGRAM_STEP);
                    fine.covered[index] += (end - cursor) as u32;
                    let cut = |bytes| portion(bytes, sample.from, sample.to, cursor, end);
                    fine.total.add(index, cut(sample.total));
                    for (key, bytes) in &sample.apps {
                        fine.apps
                            .entry(key.clone())
                            .or_default()
                            .add(index, cut(*bytes));
                    }
                    cursor = end;
                }
            }
        }
        snapshot.finish();
        snapshot
    }
}

/// Cumulative integer apportionment conserves every byte across adjacent cuts.
fn portion(bytes: Bytes, from: i64, to: i64, start: i64, end: i64) -> Bytes {
    let part = |value: u64| {
        let cumulative = |time: i64| value as u128 * (time - from) as u128 / (to - from) as u128;
        (cumulative(end) - cumulative(start)) as u64
    };
    Bytes {
        down: part(bytes.down),
        up: part(bytes.up),
    }
}

#[derive(Clone, Default, Debug, serde::Serialize, serde::Deserialize)]
pub struct Trace {
    bins: BTreeMap<usize, Bytes>,
}
impl Trace {
    fn add(&mut self, index: usize, bytes: Bytes) {
        if bytes.total() > 0 {
            self.bins.entry(index).or_default().add(bytes);
        }
    }
    fn at(&self, index: usize) -> Bytes {
        self.bins.get(&index).copied().unwrap_or_default()
    }
}

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct Peak {
    pub time: i64,
    pub bytes: Bytes,
}
impl Peak {
    pub fn rate(self) -> f64 {
        self.bytes.total() as f64 / 60.
    }
}
#[derive(Clone, Default, Debug, serde::Serialize, serde::Deserialize)]
pub struct Metrics {
    pub current: Bytes,
    pub previous: Bytes,
    pub peak: Option<Peak>,
    pub previous_peak: Option<Peak>,
}
impl Metrics {
    pub fn delta(&self) -> i128 {
        self.current.total() as i128 - self.previous.total() as i128
    }
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct AppComparison {
    pub key: String,
    pub metrics: Metrics,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Burst {
    pub app: String,
    pub time: i64,
    pub bytes: Bytes,
    pub baseline: u64,
}
impl Burst {
    pub fn increase(&self) -> u64 {
        self.bytes.total().saturating_sub(self.baseline)
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Snapshot {
    pub request: AnalysisRequest,
    pub generated_at: i64,
    start: i64,
    covered: Vec<u32>,
    fine: Option<FineTimeline>,
    pub total: Trace,
    pub apps: BTreeMap<String, Trace>,
    pub rows: Vec<AppComparison>,
    pub bursts: Vec<Burst>,
    historical: Option<history::HistoricalSnapshot>,
}
impl Snapshot {
    fn new(request: AnalysisRequest) -> Self {
        let historical = request.windows.is_some();
        let duration = if historical {
            0
        } else {
            request.range.duration()
        };
        let start = request.anchor - 2 * duration - 5 * MINUTE;
        let fine = (request.range == AnalysisRange::FiveMinutes).then(|| FineTimeline {
            covered: vec![0; (2 * duration / HISTOGRAM_STEP) as usize],
            ..Default::default()
        });
        Self {
            request,
            generated_at: chrono::Utc::now().timestamp_millis(),
            start,
            covered: if historical {
                vec![]
            } else {
                vec![0; (2 * duration / MINUTE + 5) as usize]
            },
            fine,
            total: Trace::default(),
            apps: BTreeMap::new(),
            rows: vec![],
            bursts: vec![],
            historical: None,
        }
    }
    pub fn current_from(&self) -> i64 {
        self.request
            .windows
            .map_or(self.request.anchor - self.request.range.duration(), |w| {
                w.from
            })
    }
    pub fn previous_from(&self) -> i64 {
        self.request
            .windows
            .map_or(self.current_from() - self.request.range.duration(), |w| {
                w.previous_from
            })
    }
    pub fn previous_to(&self) -> i64 {
        self.request
            .windows
            .map_or(self.current_from(), |w| w.previous_to)
    }
    fn indices(&self) -> (usize, usize) {
        let n = (self.request.range.duration() / MINUTE) as usize;
        (5, 5 + n)
    }
    pub fn valid(&self, index: usize) -> bool {
        self.covered[index] >= 60_000
    }
    pub fn coverage(&self, previous: bool) -> i64 {
        if let Some(history) = &self.historical {
            return history.coverage[usize::from(previous)];
        }
        let (a, b) = self.indices();
        let slice = if previous {
            &self.covered[a..b]
        } else {
            &self.covered[b..]
        };
        slice.iter().map(|value| (*value).min(60_000) as i64).sum()
    }
    pub fn comparable(&self) -> bool {
        [false, true]
            .into_iter()
            .all(|previous| self.coverage(previous) >= self.request.duration())
    }
    pub fn can_compare_observed(&self) -> bool {
        self.coverage(false) > 0 && self.coverage(true) > 0
    }
    pub fn trace(&self, app: Option<&str>) -> Option<&Trace> {
        match app {
            Some(key) => self.apps.get(key),
            None => Some(&self.total),
        }
    }
    pub fn metrics(&self, app: Option<&str>) -> Metrics {
        if let Some(history) = &self.historical {
            return history.metrics(app);
        }
        let mut result = Metrics::default();
        let (previous, current) = self.indices();
        let zero_peak = |range: std::ops::Range<usize>| {
            range
                .into_iter()
                .find(|&index| self.valid(index))
                .map(|index| Peak {
                    time: self.start + index as i64 * MINUTE,
                    bytes: Bytes::default(),
                })
        };
        result.peak = zero_peak(current..self.covered.len());
        result.previous_peak = zero_peak(previous..current);
        let Some(trace) = self.trace(app) else {
            return result;
        };
        for (&index, &bytes) in trace.bins.range(previous..) {
            let (total, peak) = if index >= current {
                (&mut result.current, &mut result.peak)
            } else {
                (&mut result.previous, &mut result.previous_peak)
            };
            total.add(bytes);
            if self.valid(index)
                && peak.is_none_or(|value: Peak| bytes.total() > value.bytes.total())
            {
                *peak = Some(Peak {
                    time: self.start + index as i64 * MINUTE,
                    bytes,
                });
            }
        }
        result
    }
    #[cfg(test)]
    pub fn series(&self, app: Option<&str>) -> Vec<Option<SeriesPoint>> {
        let (_, start) = self.indices();
        (start..self.covered.len())
            .map(|index| {
                self.valid(index).then(|| {
                    let bytes = self
                        .trace(app)
                        .map(|trace| trace.at(index))
                        .unwrap_or_default();
                    SeriesPoint {
                        time: self.start + index as i64 * MINUTE,
                        down: bytes.down as f64 / 60.,
                        up: bytes.up as f64 / 60.,
                    }
                })
            })
            .collect()
    }
    pub fn histogram(&self, size: BucketSize, app: Option<&str>) -> Vec<ComparisonBucket> {
        if let Some(history) = &self.historical {
            return history.histogram(&self.request, size, app);
        }
        let size = if self.request.range.bucket_sizes().contains(&size) {
            size
        } else {
            self.request.range.default_bucket()
        };
        let duration = size.millis();
        let (covered, trace, base, previous, current) = if let Some(fine) = &self.fine {
            (
                &fine.covered,
                match app {
                    Some(key) => fine.apps.get(key),
                    None => Some(&fine.total),
                },
                HISTOGRAM_STEP,
                0,
                (self.request.range.duration() / HISTOGRAM_STEP) as usize,
            )
        } else {
            let (previous, current) = self.indices();
            (&self.covered, self.trace(app), MINUTE, previous, current)
        };
        let width = (duration / base) as usize;
        let aggregate = |start: usize| {
            let mut value = BucketValue {
                covered_ms: covered[start..start + width]
                    .iter()
                    .map(|ms| u64::from((*ms).min(base as u32)))
                    .sum(),
                ..Default::default()
            };
            if let Some(trace) = trace {
                for (_, bytes) in trace.bins.range(start..start + width) {
                    value.bytes.add(*bytes);
                }
            }
            value
        };
        (0..self.request.range.duration() / duration)
            .map(|index| ComparisonBucket {
                from: self.current_from() + index * duration,
                previous_from: self.previous_from() + index * duration,
                duration,
                current: aggregate(current + index as usize * width),
                previous: aggregate(previous + index as usize * width),
            })
            .collect()
    }
    fn finish(&mut self) {
        self.rows = self
            .apps
            .keys()
            .filter_map(|key| {
                let metrics = self.metrics(Some(key));
                (metrics.current.total() > 0 || metrics.previous.total() > 0).then(|| {
                    AppComparison {
                        key: key.clone(),
                        metrics,
                    }
                })
            })
            .collect();
        self.rows.sort_by(|a, b| {
            b.metrics
                .delta()
                .unsigned_abs()
                .cmp(&a.metrics.delta().unsigned_abs())
                .then_with(|| a.key.cmp(&b.key))
        });
        let (_, current) = self.indices();
        self.bursts.clear();
        for (key, trace) in &self.apps {
            for (&index, &bytes) in trace.bins.range(current..) {
                if !(index - 5..=index).all(|i| self.valid(i)) {
                    continue;
                }
                let baseline: u128 = (index - 5..index)
                    .map(|i| trace.at(i).total() as u128)
                    .sum();
                let value = bytes.total() as u128 * 5;
                if value >= baseline * 3 && value.saturating_sub(baseline) >= BURST_INCREASE * 5 {
                    self.bursts.push(Burst {
                        app: key.clone(),
                        time: self.start + index as i64 * MINUTE,
                        bytes,
                        baseline: (baseline / 5).min(u64::MAX as u128) as u64,
                    });
                }
            }
        }
        self.bursts.sort_by(|a, b| {
            b.time
                .cmp(&a.time)
                .then_with(|| b.increase().cmp(&a.increase()))
                .then_with(|| a.app.cmp(&b.app))
        });
    }

    #[cfg(test)]
    pub(crate) fn fixture(request: AnalysisRequest) -> Self {
        if request.windows.is_some() {
            return history::fixture(request);
        }
        let mut snapshot = Self::new(request);
        snapshot.covered.fill(60_000);
        let current = snapshot.indices().1;
        for app in 0..22 {
            let key = format!(
                r"C:\测试应用\{}\应用-{app:02}.exe",
                "long folder with spaces-".repeat(4)
            );
            let mut trace = Trace::default();
            for index in 0..snapshot.covered.len() {
                let bytes = if index < current {
                    Bytes {
                        down: 1024 * 1024,
                        up: 256 * 1024,
                    }
                } else if app == 21 {
                    Bytes::default()
                } else {
                    Bytes {
                        down: (22 - app as u64)
                            * 1024
                            * 1024
                            * if index == current + 1 { 2 } else { 1 },
                        up: 1024 * 1024,
                    }
                };
                trace.add(index, bytes);
                snapshot.total.add(index, bytes);
            }
            snapshot.apps.insert(key, trace);
        }
        snapshot.finish();
        // Synthetic test fixtures spread their generated minute values evenly.
        // Production seconds are populated exclusively from raw committed samples.
        if let Some(fine) = &mut snapshot.fine {
            fine.covered.fill(HISTOGRAM_STEP as u32);
            for index in 0..fine.covered.len() {
                let minute = 5 + index / 6;
                let from = (index % 6) as i64 * HISTOGRAM_STEP;
                let cut = |bytes| portion(bytes, 0, MINUTE, from, from + HISTOGRAM_STEP);
                fine.total.add(index, cut(snapshot.total.at(minute)));
                for (key, trace) in &snapshot.apps {
                    fine.apps
                        .entry(key.clone())
                        .or_default()
                        .add(index, cut(trace.at(minute)));
                }
            }
        }
        snapshot
    }
}

/// A short, read-only WAL transaction materializes the entire frozen result.
/// No collector mutex or read transaction survives this call.
pub fn historical(path: &Path, request: AnalysisRequest) -> Result<Snapshot> {
    if request.windows.is_some() {
        return history::read(path, request);
    }
    anyhow::ensure!(
        request.range != AnalysisRange::FiveMinutes,
        "秒级快照必须从采样缓存读取"
    );
    let mut db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    db.busy_timeout(Duration::from_secs(5))?;
    let tx = db.transaction()?;
    let mut snapshot = Snapshot::new(request);
    let earliest: Option<i64> =
        tx.query_row("SELECT MIN(time) FROM totals", [], |row| row.get(0))?;
    let checkpoint: Option<String> = tx
        .query_row("SELECT value FROM meta WHERE key='checkpoint'", [], |row| {
            row.get(0)
        })
        .optional()?;
    let latest = checkpoint
        .map(|value| serde_json::from_str::<crate::accounting::Ledger>(&value))
        .transpose()?
        .map(|ledger| ledger.time);
    let gaps: Vec<(i64, i64)> = tx
        .prepare("SELECT started,ended FROM gaps WHERE started<?2 AND ended>?1")?
        .query_map(params![snapshot.start, snapshot.request.anchor], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    {
        let mut statement =
            tx.prepare("SELECT time,up,down FROM totals WHERE time>=?1 AND time<?2 ORDER BY time")?;
        let rows = statement
            .query_map(params![snapshot.start, snapshot.request.anchor], |row| {
                Ok((row.get::<_, i64>(0)?, sql_bytes(row, 1)?))
            })?;
        for row in rows {
            let (time, bytes) = row?;
            let index = ((time - snapshot.start) / MINUTE) as usize;
            snapshot.total.add(index, bytes);
            if earliest.is_some_and(|first| time > first)
                && latest.is_some_and(|last| last >= time + MINUTE)
                && !gaps
                    .iter()
                    .any(|&(from, to)| from < time + MINUTE && to > time)
            {
                snapshot.covered[index] = MINUTE as u32;
            }
        }
    }
    {
        let mut statement = tx.prepare("SELECT d.name,f.time,f.up,f.down FROM application_minutes f JOIN dimensions d ON d.id=f.app WHERE f.time>=?1 AND f.time<?2 ORDER BY f.time")?;
        let rows =
            statement.query_map(params![snapshot.start, snapshot.request.anchor], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    sql_bytes(row, 2)?,
                ))
            })?;
        for row in rows {
            let (key, time, bytes) = row?;
            snapshot
                .apps
                .entry(key)
                .or_default()
                .add(((time - snapshot.start) / MINUTE) as usize, bytes);
        }
    }
    tx.commit().context("读取增量分析快照失败")?;
    snapshot.finish();
    Ok(snapshot)
}
fn sql_bytes(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<Bytes> {
    Ok(Bytes {
        up: row.get::<_, i64>(index)?.max(0) as u64,
        down: row.get::<_, i64>(index + 1)?.max(0) as u64,
    })
}

pub fn change_text(current: u64, previous: u64, comparable: bool) -> String {
    if !comparable {
        "数据不足".into()
    } else if previous == 0 {
        if current > 0 {
            "新增".into()
        } else {
            "—".into()
        }
    } else {
        format!(
            "{:+.1}%",
            (current as f64 - previous as f64) / previous as f64 * 100.
        )
    }
}
pub fn signed_bytes(value: i128) -> String {
    format!(
        "{}{}",
        if value > 0 {
            "+"
        } else if value < 0 {
            "−"
        } else {
            ""
        },
        crate::model::human_bytes(value.unsigned_abs().min(u64::MAX as u128) as u64)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn histogram_preserves_exact_boundaries_bytes_and_frozen_samples() {
        let anchor = 1_234_567_000;
        let mut samples = complete_window(anchor);
        let snapshot = samples.snapshot(request(AnalysisRange::FiveMinutes, anchor + 999));
        for size in AnalysisRange::FiveMinutes.bucket_sizes() {
            let bins = snapshot.histogram(*size, Some("app"));
            assert_eq!(bins.first().unwrap().from, anchor - 5 * MINUTE);
            assert_eq!(bins.last().unwrap().from + size.millis(), anchor);
            assert!(
                bins.windows(2)
                    .all(|pair| pair[0].from + size.millis() == pair[1].from)
            );
            assert!(
                bins.iter()
                    .all(|bin| bin.previous_from + 5 * MINUTE == bin.from && bin.comparable())
            );
            assert_eq!(
                bins.iter().map(|bin| bin.current.bytes.down).sum::<u64>(),
                snapshot.metrics(Some("app")).current.down
            );
            assert_eq!(
                bins.iter().map(|bin| bin.previous.bytes.down).sum::<u64>(),
                3000
            );
        }
        samples.record(anchor, anchor + 1000, bytes(1000), &HashMap::new());
        assert_eq!(
            snapshot
                .histogram(BucketSize::Seconds10, None)
                .iter()
                .map(|bin| bin.current.bytes.down)
                .sum::<u64>(),
            3000
        );
    }
    #[test]
    fn histogram_apportions_crossings_and_distinguishes_zero_partial_and_missing() {
        let anchor = 1000 * MINUTE + 20_000;
        let from = anchor - 5 * MINUTE;
        let mut samples = SampleWindow::default();
        samples.record(
            from + 9_500,
            from + 11_500,
            Bytes { down: 7, up: 3 },
            &HashMap::new(),
        );
        samples.record(from + 20_000, from + 21_000, bytes(0), &HashMap::new());
        let snapshot = samples.snapshot(request(AnalysisRange::FiveMinutes, anchor));
        let bins = snapshot.histogram(BucketSize::Seconds10, None);
        assert_eq!(bins[0].current.covered_ms, 500);
        assert_eq!(bins[1].current.covered_ms, 1500);
        assert_eq!(bins[0].current.bytes.down + bins[1].current.bytes.down, 7);
        assert_eq!(bins[0].current.bytes.up + bins[1].current.bytes.up, 3);
        assert!(bins[2].current.observed());
        assert_eq!(bins[2].current.bytes.total(), 0);
        assert!(!bins[3].current.observed());
        assert!(!bins[0].comparable());
        assert_eq!(snapshot.metrics(None).current, Bytes { down: 7, up: 3 });
    }
    #[test]
    fn all_histogram_intervals_keep_minute_statistics_and_application_identity() {
        for range in AnalysisRange::ALL {
            let snapshot = Snapshot::fixture(request(range, 10_000 * MINUTE));
            let first_key = snapshot.rows[0].key.clone();
            let metrics = snapshot.metrics(Some(&first_key));
            for size in range.bucket_sizes() {
                let bins = snapshot.histogram(*size, Some(&first_key));
                assert_eq!(bins.len(), (range.duration() / size.millis()) as usize);
                assert_eq!(
                    bins.iter()
                        .map(|bin| bin.current.bytes.total() as u128)
                        .sum::<u128>(),
                    metrics.current.total() as u128
                );
                assert_eq!(
                    bins.iter()
                        .map(|bin| bin.previous.bytes.total() as u128)
                        .sum::<u128>(),
                    metrics.previous.total() as u128
                );
                assert!(bins.iter().all(|bin| bin.comparable()));
            }
            assert_eq!(
                snapshot.metrics(Some(&first_key)).peak.unwrap().time,
                metrics.peak.unwrap().time
            );
        }
    }
    fn request(range: AnalysisRange, time: i64) -> AnalysisRequest {
        AnalysisRequest::new(1, 0, range, time)
    }
    fn bytes(down: u64) -> Bytes {
        Bytes { down, up: 0 }
    }
    fn complete_window(anchor: i64) -> SampleWindow {
        let mut window = SampleWindow::default();
        for second in 0..900 {
            let from = anchor - CACHE_LENGTH + second * 1000;
            window.record(
                from,
                from + 1000,
                bytes(10),
                &HashMap::from([("app".into(), bytes(10))]),
            );
        }
        window
    }
    #[test]
    fn exact_second_windows_are_adjacent_and_frozen() {
        let anchor = 9_999_020_000;
        let mut window = complete_window(anchor);
        let snapshot = window.snapshot(request(AnalysisRange::FiveMinutes, anchor + 900));
        assert_eq!(snapshot.request.anchor, anchor);
        assert_eq!(snapshot.current_from(), anchor - 300_000);
        assert_eq!(snapshot.previous_from(), anchor - 600_000);
        assert_eq!(snapshot.metrics(None).current, bytes(3000));
        assert_eq!(snapshot.metrics(None).previous, bytes(3000));
        assert!(snapshot.comparable());
        window.record(anchor, anchor + 1000, bytes(100_000), &HashMap::new());
        assert_eq!(snapshot.metrics(None).current, bytes(3000));
        assert!(
            window
                .snapshot(request(AnalysisRange::FiveMinutes, anchor + 1000))
                .metrics(None)
                .current
                .total()
                > snapshot.metrics(None).current.total()
        );
        assert_eq!(snapshot.series(None).len(), 5);
    }
    #[test]
    fn boundary_splits_conserve_bytes_and_do_not_use_future_traffic() {
        let mut window = SampleWindow::default();
        window.record(
            999_500,
            1_000_500,
            Bytes { down: 7, up: 3 },
            &HashMap::from([("a".into(), Bytes { down: 7, up: 3 })]),
        );
        let first = window.snapshot(request(AnalysisRange::FiveMinutes, 1_000_000));
        let second = window.snapshot(request(AnalysisRange::FiveMinutes, 1_300_000));
        assert_eq!(
            first.metrics(None).current.down + second.metrics(None).current.down,
            7
        );
        assert_eq!(
            first.metrics(None).current.up + second.metrics(None).current.up,
            3
        );
        assert_eq!(
            first.metrics(Some("a")).current,
            first.metrics(None).current
        );
        assert!(!first.comparable());
    }
    #[test]
    fn cache_is_bounded_zero_samples_cover_time_and_clock_reset_clears() {
        let mut window = complete_window(1_000_000);
        window.record(1_000_000, 1_001_000, Bytes::default(), &HashMap::new());
        assert_eq!(window.samples.len(), 900);
        assert!(
            window
                .snapshot(request(AnalysisRange::FiveMinutes, 1_001_000))
                .comparable()
        );
        assert!(window.advance(100));
        assert!(window.samples.is_empty());
        window.record(100, 10_000, bytes(10), &HashMap::new());
        assert!(window.samples.is_empty());
        window.clear();
        assert!(
            !window
                .snapshot(request(AnalysisRange::FiveMinutes, 10_000))
                .comparable()
        );
    }
    #[test]
    fn union_peaks_and_bursts_use_full_minutes_and_valid_baselines() {
        let mut snapshot = Snapshot::new(request(AnalysisRange::FiveMinutes, 1_000_000));
        snapshot.covered.fill(60_000);
        let current = snapshot.indices().1;
        let mut track = Trace::default();
        for index in 0..current {
            track.add(index, bytes(1024 * 1024));
        }
        track.add(current, bytes(6 * 1024 * 1024));
        track.add(current + 1, bytes(2 * 1024 * 1024));
        snapshot.apps.insert(r"C:\one\app.exe".into(), track);
        let mut stopped = Trace::default();
        stopped.add(current - 1, bytes(9));
        snapshot.apps.insert(r"D:\two\app.exe".into(), stopped);
        let mut fresh = Trace::default();
        fresh.add(current, bytes(5 * 1024 * 1024));
        snapshot.apps.insert("未知进程".into(), fresh);
        snapshot.finish();
        assert_eq!(snapshot.rows.len(), 3);
        assert_eq!(snapshot.bursts.len(), 2);
        assert_eq!(
            snapshot
                .bursts
                .iter()
                .find(|burst| burst.app == "未知进程")
                .unwrap()
                .baseline,
            0
        );
        let metrics = snapshot.metrics(Some(r"C:\one\app.exe"));
        assert_eq!(metrics.peak.unwrap().time, snapshot.current_from());
        assert_eq!(metrics.peak.unwrap().rate(), (6 * 1024 * 1024) as f64 / 60.);
        assert_eq!(snapshot.metrics(Some(r"D:\two\app.exe")).delta(), -9);
        snapshot.covered[current - 1] = 0;
        snapshot.finish();
        assert!(snapshot.bursts.is_empty());
        assert_eq!(change_text(1, 0, false), "数据不足");
        assert_eq!(change_text(1, 0, true), "新增");
        assert_eq!(change_text(0, 0, true), "—");
        assert_eq!(change_text(0, 10, true), "-100.0%");
    }
    #[test]
    fn historical_reads_minutes_in_one_snapshot_and_keeps_zero_coverage_and_gaps() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("traffic.sqlite3");
        let storage = crate::storage::Storage::open(&path).unwrap();
        let anchor = 200 * MINUTE;
        storage
            .db
            .execute(
                "INSERT INTO dimensions(id,kind,name) VALUES(1,'app','a')",
                [],
            )
            .unwrap();
        for minute in 0..200 {
            storage
                .db
                .execute(
                    "INSERT INTO totals VALUES(?1,0,?2)",
                    params![minute * MINUTE, if minute == 190 { 10_000_000 } else { 0 }],
                )
                .unwrap();
        }
        storage
            .db
            .execute(
                "INSERT INTO app_minutes VALUES(?1,1,2,10000000)",
                [190 * MINUTE],
            )
            .unwrap();
        let ledger = crate::accounting::Ledger {
            time: anchor,
            ..Default::default()
        };
        storage
            .db
            .execute(
                "INSERT INTO meta VALUES('checkpoint',?1)",
                [serde_json::to_string(&ledger).unwrap()],
            )
            .unwrap();
        let snapshot = historical(&path, request(AnalysisRange::Hour, anchor + 12345)).unwrap();
        assert_eq!(snapshot.request.anchor, anchor);
        assert!(snapshot.comparable());
        assert_eq!(
            snapshot.metrics(Some("a")).current,
            Bytes {
                down: 10_000_000,
                up: 2
            }
        );
        assert_eq!(snapshot.metrics(Some("a")).peak.unwrap().time, 190 * MINUTE);
        assert_eq!(snapshot.bursts.len(), 1);
        storage
            .db
            .execute(
                "INSERT INTO gaps(started,ended,up,down,reason) VALUES(?1,?2,0,0,'gap')",
                params![189 * MINUTE, 190 * MINUTE],
            )
            .unwrap();
        let newer = historical(&path, request(AnalysisRange::Hour, anchor)).unwrap();
        assert!(!newer.comparable());
        assert!(newer.can_compare_observed());
        assert!(newer.bursts.is_empty());
        assert_eq!(snapshot.bursts.len(), 1);
        assert!(historical(&path, request(AnalysisRange::FiveMinutes, anchor)).is_err());
    }

    #[test]
    fn burst_threshold_is_inclusive_and_a_tiny_gap_is_not_zero_traffic() {
        let mut snapshot = Snapshot::new(request(AnalysisRange::FiveMinutes, 1_000_000));
        snapshot.covered.fill(60_000);
        let current = snapshot.indices().1;
        let mut trace = Trace::default();
        for index in current - 5..current {
            trace.add(index, bytes(5 * 1024 * 1024 / 2));
        }
        trace.add(current, bytes(15 * 1024 * 1024 / 2));
        snapshot.apps.insert("app".into(), trace);
        snapshot.finish();
        assert_eq!(snapshot.bursts.len(), 1);
        snapshot
            .apps
            .get_mut("app")
            .unwrap()
            .bins
            .get_mut(&current)
            .unwrap()
            .down -= 1;
        snapshot.finish();
        assert!(snapshot.bursts.is_empty());
        snapshot
            .apps
            .get_mut("app")
            .unwrap()
            .bins
            .get_mut(&current)
            .unwrap()
            .down += 1;
        snapshot.covered[current - 1] -= 1;
        snapshot.finish();
        assert!(snapshot.bursts.is_empty());
        assert!(!snapshot.comparable());
        let day = request(AnalysisRange::Day, 1_729_872_123_456);
        let s = Snapshot::new(day);
        assert_eq!(s.request.anchor - s.current_from(), 86_400_000);
    }

    #[test]
    fn read_only_analytics_does_not_wait_for_an_uncommitted_writer() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("traffic.sqlite3");
        let storage = crate::storage::Storage::open(&path).unwrap();
        let tx = storage.db.unchecked_transaction().unwrap();
        tx.execute("INSERT INTO totals VALUES(60000,100,100)", [])
            .unwrap();
        let snapshot = historical(&path, request(AnalysisRange::Hour, 600_000)).unwrap();
        assert_eq!(snapshot.metrics(None).current.total(), 0);
        tx.rollback().unwrap();
    }
}
