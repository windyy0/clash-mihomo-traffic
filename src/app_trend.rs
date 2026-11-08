use crate::model::Bytes;
use anyhow::{Context, Result};
use chrono::{Datelike, Local, Months, NaiveDate};
use std::collections::BTreeMap;

const HOUR: i64 = 3_600_000;
const DAY: i64 = 24 * HOUR;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum BucketSize {
    #[default]
    Hour,
    Day,
    Month,
}
impl BucketSize {
    pub fn for_range(from: i64, to: i64) -> Self {
        match to.saturating_sub(from) {
            span if span <= 2 * DAY => Self::Hour,
            span if span <= 120 * DAY => Self::Day,
            _ => Self::Month,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Hour => "按小时汇总",
            Self::Day => "按天汇总",
            Self::Month => "按月汇总",
        }
    }
    fn start(self, time: i64) -> Result<i64> {
        let local = chrono::DateTime::from_timestamp_millis(time)
            .context("趋势时间超出支持范围")?
            .with_timezone(&Local);
        Ok(match self {
            Self::Hour => {
                let offset = i64::from(local.offset().local_minus_utc()) * 1000;
                (time + offset).div_euclid(HOUR) * HOUR - offset
            }
            Self::Day => midnight(local.date_naive())?,
            Self::Month => midnight(local.date_naive().with_day(1).context("月份无效")?)?,
        })
    }
    fn next(self, time: i64) -> Result<i64> {
        if self == Self::Hour {
            return time.checked_add(HOUR).context("趋势时间溢出");
        }
        let date = chrono::DateTime::from_timestamp_millis(time)
            .context("趋势时间无效")?
            .with_timezone(&Local)
            .date_naive();
        midnight(
            match self {
                Self::Day => date.succ_opt(),
                Self::Month => date.checked_add_months(Months::new(1)),
                Self::Hour => unreachable!(),
            }
            .context("趋势时间溢出")?,
        )
    }
}
fn midnight(date: NaiveDate) -> Result<i64> {
    // Some time zones advance clocks at midnight. Use the first existing hour.
    (0..24)
        .find_map(|hour| {
            date.and_hms_opt(hour, 0, 0)?
                .and_local_timezone(Local)
                .earliest()
                .map(|time| time.timestamp_millis())
        })
        .context("无法确定本地日期起点")
}
#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Bucket {
    pub from: i64,
    pub to: i64,
    pub bytes: Bytes,
}
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Histogram {
    pub size: BucketSize,
    pub buckets: Vec<Bucket>,
}
pub struct Builder {
    from: i64,
    to: i64,
    size: BucketSize,
    samples: BTreeMap<i64, Bytes>,
}
impl Builder {
    pub fn new(from: i64, to: i64, range_to: i64) -> Self {
        Self {
            from,
            to,
            size: BucketSize::for_range(from, range_to),
            samples: BTreeMap::new(),
        }
    }
    pub fn add(&mut self, time: i64, bytes: Bytes) -> Result<()> {
        if time >= self.from && time < self.to {
            self.samples
                .entry(self.size.start(time)?)
                .or_default()
                .add(bytes);
        }
        Ok(())
    }
    pub fn finish(self) -> Result<Histogram> {
        let mut result = Histogram {
            size: self.size,
            buckets: vec![],
        };
        if self.to <= self.from {
            return Ok(result);
        }
        let mut time = self.size.start(self.from)?;
        while time < self.to {
            let next = self.size.next(time)?;
            anyhow::ensure!(
                next > time && result.buckets.len() < 128,
                "柱状图时间范围无效"
            );
            result.buckets.push(Bucket {
                from: time.max(self.from),
                to: next.min(self.to),
                bytes: self.samples.get(&time).copied().unwrap_or_default(),
            });
            time = next;
        }
        Ok(result)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn local(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> i64 {
        NaiveDate::from_ymd_opt(year, month, day)
            .unwrap()
            .and_hms_opt(hour, minute, 0)
            .unwrap()
            .and_local_timezone(Local)
            .earliest()
            .unwrap()
            .timestamp_millis()
    }
    #[test]
    fn hourly_buckets_keep_empty_hours_and_clip_partial_boundaries() {
        let from = local(2024, 2, 28, 9, 25);
        let to = local(2024, 2, 28, 12, 10);
        let mut builder = Builder::new(from, to, to);
        builder
            .add(from - 60_000, Bytes { up: 999, down: 999 })
            .unwrap();
        builder.add(from, Bytes { up: 3, down: 8 }).unwrap();
        builder
            .add(local(2024, 2, 28, 12, 0), Bytes { up: 4, down: 9 })
            .unwrap();
        builder.add(to, Bytes { up: 999, down: 999 }).unwrap();
        let histogram = builder.finish().unwrap();
        assert_eq!(histogram.buckets.len(), 4);
        assert_eq!(histogram.buckets[0].from, from);
        assert_eq!(histogram.buckets[3].to, to);
        assert_eq!(histogram.buckets[1].bytes, Bytes::default());
        let mut total = Bytes::default();
        for bucket in histogram.buckets {
            total.add(bucket.bytes);
        }
        assert_eq!(total, Bytes { up: 7, down: 17 });
    }
    #[test]
    fn calendar_days_cross_leap_day_and_months_remain_bounded() {
        let from = local(2024, 2, 28, 0, 0);
        let to = local(2024, 3, 2, 0, 0);
        let mut builder = Builder::new(from, to, to);
        builder
            .add(local(2024, 2, 29, 23, 59), Bytes { up: 2, down: 7 })
            .unwrap();
        builder
            .add(local(2024, 3, 1, 0, 0), Bytes { up: 3, down: 8 })
            .unwrap();
        let histogram = builder.finish().unwrap();
        assert_eq!(histogram.size, BucketSize::Day);
        assert_eq!(histogram.buckets.len(), 3);
        assert_eq!(histogram.buckets[1].bytes, Bytes { up: 2, down: 7 });
        assert_eq!(histogram.buckets[2].bytes, Bytes { up: 3, down: 8 });
        let long = Builder::new(from, local(2034, 2, 28, 0, 0), local(2034, 2, 28, 0, 0))
            .finish()
            .unwrap();
        assert_eq!(long.size, BucketSize::Month);
        assert!(long.buckets.len() <= 121);
    }
}
