use anyhow::{Context, Result};
use chrono::{DateTime, Days, Months, TimeZone};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TimePeriod {
    #[default]
    Today,
    Yesterday,
    LastSevenDays,
    LastThreeMonths,
    Custom,
}

impl TimePeriod {
    pub const HISTORY: [Self; 5] = [
        Self::Today,
        Self::Yesterday,
        Self::LastSevenDays,
        Self::LastThreeMonths,
        Self::Custom,
    ];
    pub const ALL: [Self; 4] = [
        Self::Today,
        Self::LastSevenDays,
        Self::LastThreeMonths,
        Self::Custom,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Today => "今天",
            Self::Yesterday => "昨天",
            Self::LastSevenDays => "近7天",
            Self::LastThreeMonths => "近3个月",
            Self::Custom => "自定义",
        }
    }

    pub fn bounds<T: TimeZone>(self, now: DateTime<T>) -> Result<Option<(i64, i64)>> {
        if self == Self::Custom {
            return Ok(None);
        }
        let today = now.date_naive();
        let end = if self == Self::Yesterday {
            today
        } else {
            today.succ_opt().context("无法确定结束日期")?
        };
        let start = match self {
            Self::Today => Some(today),
            Self::Yesterday => today.pred_opt(),
            Self::LastSevenDays => end.checked_sub_days(Days::new(7)),
            Self::LastThreeMonths => end.checked_sub_months(Months::new(3)),
            Self::Custom => unreachable!(),
        }
        .context("无法确定开始日期")?;
        let midnight = |date: chrono::NaiveDate| {
            now.timezone()
                .from_local_datetime(&date.and_hms_opt(0, 0, 0).unwrap())
                .earliest()
                .context("无法确定本地日期边界")
                .map(|date| date.timestamp_millis())
        };
        Ok(Some((midnight(start)?, midnight(end)?)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, Utc};

    #[test]
    fn history_yesterday_crosses_years_without_changing_global_presets() {
        let zone = FixedOffset::east_opt(8 * 3600).unwrap();
        let now = zone.with_ymd_and_hms(2026, 1, 1, 14, 0, 0).unwrap();
        let (from, to) = TimePeriod::Yesterday.bounds(now).unwrap().unwrap();
        assert_eq!(
            from,
            zone.with_ymd_and_hms(2025, 12, 31, 0, 0, 0)
                .unwrap()
                .timestamp_millis()
        );
        assert_eq!(
            to,
            zone.with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
                .unwrap()
                .timestamp_millis()
        );
        assert!(!TimePeriod::ALL.contains(&TimePeriod::Yesterday));
    }

    #[test]
    fn presets_include_today_and_roll_across_months_and_leap_years() {
        let zone = FixedOffset::east_opt(8 * 3600).unwrap();
        let now = zone.with_ymd_and_hms(2026, 10, 25, 16, 20, 0).unwrap();
        let start = |month, day| {
            zone.with_ymd_and_hms(2026, month, day, 0, 0, 0)
                .unwrap()
                .timestamp_millis()
        };
        assert_eq!(
            TimePeriod::Today.bounds(now).unwrap(),
            Some((start(10, 25), start(10, 26)))
        );
        assert_eq!(
            TimePeriod::LastSevenDays.bounds(now).unwrap(),
            Some((start(10, 19), start(10, 26)))
        );
        assert_eq!(
            TimePeriod::LastThreeMonths.bounds(now).unwrap(),
            Some((start(7, 26), start(10, 26)))
        );
        assert_eq!(TimePeriod::Custom.bounds(now).unwrap(), None);
        let leap = Utc.with_ymd_and_hms(2024, 5, 30, 12, 0, 0).unwrap();
        let (from, to) = TimePeriod::LastThreeMonths.bounds(leap).unwrap().unwrap();
        assert_eq!(
            from,
            Utc.with_ymd_and_hms(2024, 2, 29, 0, 0, 0)
                .unwrap()
                .timestamp_millis()
        );
        assert_eq!(
            to,
            Utc.with_ymd_and_hms(2024, 5, 31, 0, 0, 0)
                .unwrap()
                .timestamp_millis()
        );
        let next_year = Utc.with_ymd_and_hms(2026, 1, 2, 1, 0, 0).unwrap();
        assert_eq!(
            TimePeriod::LastSevenDays
                .bounds(next_year)
                .unwrap()
                .unwrap()
                .0,
            Utc.with_ymd_and_hms(2025, 12, 27, 0, 0, 0)
                .unwrap()
                .timestamp_millis()
        );
    }
}
