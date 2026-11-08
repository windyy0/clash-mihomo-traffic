use crate::model::{Query, Report};
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

const LIMIT: usize = 16;
#[derive(Default)]
pub struct DetailCache {
    revision: u64,
    entries: VecDeque<(Query, Report, Instant)>,
}
pub struct CachedDetail {
    pub report: Report,
    pub fresh: bool,
}
fn key(query: &Query) -> Query {
    Query {
        history: query.history,
        history_context: None,
        app: query.app.clone(),
        from: query.from,
        to: query.to,
        group_apps: query.group_apps,
        group_domains: false,
        dimension: "domain".into(),
        search: String::new(),
        page: 0,
        page_size: 50,
        ascending: false,
        sort: crate::model::TrafficSort::Total,
    }
}
pub fn matches(report: &Report, query: &Query) -> bool {
    report
        .query
        .as_ref()
        .is_some_and(|stored| key(stored) == key(query))
}
impl DetailCache {
    pub fn set_revision(&mut self, revision: u64) -> bool {
        if self.revision == revision {
            return false;
        }
        self.revision = revision;
        self.entries.clear();
        true
    }
    pub fn remember(&mut self, report: &Report) {
        if !report.error.is_empty() {
            return;
        }
        let Some(query) = report.query.as_ref().filter(|query| !query.app.is_empty()) else {
            return;
        };
        let key = key(query);
        if let Some(index) = self
            .entries
            .iter()
            .position(|(stored, _, _)| *stored == key)
        {
            self.entries.remove(index);
        }
        self.entries
            .push_back((key, report.clone(), Instant::now()));
        while self.entries.len() > LIMIT {
            self.entries.pop_front();
        }
    }
    pub fn get(&mut self, query: &Query) -> Option<CachedDetail> {
        let key = key(query);
        let index = self
            .entries
            .iter()
            .position(|(stored, _, _)| *stored == key)?;
        let (key, report, updated) = self.entries.remove(index)?;
        let settled = key.to < chrono::Utc::now().timestamp_millis() - 60_000;
        let fresh = settled || updated.elapsed() < Duration::from_secs(2);
        self.entries.push_back((key, report.clone(), updated));
        Some(CachedDetail { report, fresh })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn report(app: &str) -> Report {
        Report {
            query: Some(Query {
                app: app.into(),
                ..Default::default()
            }),
            ..Default::default()
        }
    }
    #[test]
    fn cache_reuses_dimensions_but_separates_dates_process_scope_and_restore() {
        let mut cache = DetailCache::default();
        let original = report("app.exe");
        cache.remember(&original);
        let mut query = original.query.clone().unwrap();
        query.dimension = "source".into();
        assert!(cache.get(&query).unwrap().fresh);
        query.group_apps = !query.group_apps;
        assert!(cache.get(&query).is_none());
        query.group_apps = !query.group_apps;
        query.from += 60_000;
        assert!(cache.get(&query).is_none());
        query.from -= 60_000;
        assert!(cache.set_revision(1));
        assert!(cache.get(&query).is_none());
        cache.remember(&original);
        assert!(!cache.set_revision(1));
        assert!(cache.get(&query).is_some());
    }
    #[test]
    fn cache_is_bounded_keeps_recent_entries_and_does_not_cache_errors() {
        let mut cache = DetailCache::default();
        for index in 0..LIMIT {
            cache.remember(&report(&format!("{index}.exe")));
        }
        let first = report("0.exe").query.unwrap();
        assert!(cache.get(&first).is_some());
        cache.remember(&report("new.exe"));
        assert!(cache.get(&first).is_some());
        assert!(cache.get(&report("1.exe").query.unwrap()).is_none());
        let mut failed = report("failure.exe");
        failed.error = "disk error".into();
        cache.remember(&failed);
        assert!(cache.get(failed.query.as_ref().unwrap()).is_none());
        let (key, _, time) = cache.entries.back_mut().unwrap();
        key.to = chrono::Utc::now().timestamp_millis() + 60_000;
        *time = Instant::now() - Duration::from_secs(3);
        let query = key.clone();
        assert!(!cache.get(&query).unwrap().fresh);
    }
}
