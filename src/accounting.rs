use crate::model::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const MINUTE: i64 = 60_000;
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Seen {
    pub bytes: Bytes,
    pub start: String,
    pub time: i64,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Ledger {
    pub core: String,
    pub epoch: String,
    pub time: i64,
    pub sequence: u64,
    pub bytes: Bytes,
    pub seen: HashMap<String, Seen>,
    pub epoch_start: i64,
}
#[derive(Clone)]
pub struct Fact {
    pub time: i64,
    pub app: String,
    pub domain: String,
    pub route: String,
    pub source: String,
    pub bytes: Bytes,
}
#[derive(Clone)]
pub struct OwnerFact {
    pub fact: Fact,
    pub owner: crate::attribution::Owner,
}
#[derive(Default)]
pub struct Batch {
    pub sample_from: Option<i64>,
    pub sample_duration_ms: Option<u64>,
    pub owners: Vec<OwnerFact>,
    pub identities: HashMap<String, crate::attribution::Owner>,
    pub application_rates: HashMap<String, Bytes>,
    pub totals: Vec<(i64, Bytes)>,
    pub facts: Vec<Fact>,
    pub gap: Option<(i64, i64, Bytes, String)>,
    pub event: Option<String>,
    pub app_rates: HashMap<String, Bytes>,
    pub rate: Bytes,
}
/// Apportion an observed interval over minute boundaries, preserving every byte.
/// This is sampling precision, not a claim about packet timestamps.
pub fn split(from: i64, to: i64, bytes: Bytes) -> Vec<(i64, Bytes)> {
    if to <= from {
        return vec![(to.div_euclid(MINUTE) * MINUTE, bytes)];
    }
    let duration = (to - from) as u128;
    let mut out = vec![];
    let mut cursor = from;
    let mut assigned = Bytes::default();
    while cursor < to {
        let bucket = cursor.div_euclid(MINUTE) * MINUTE;
        let end = (bucket + MINUTE).min(to);
        let cumulative = Bytes {
            up: (bytes.up as u128 * (end - from) as u128 / duration) as u64,
            down: (bytes.down as u128 * (end - from) as u128 / duration) as u64,
        };
        out.push((bucket, cumulative.delta(assigned).unwrap()));
        assigned = cumulative;
        cursor = end;
    }
    out
}
impl Ledger {
    pub fn ingest(&mut self, snap: &Snapshot, core: &str, now: i64, interrupted: bool) -> Batch {
        let mut batch = Batch::default();
        let current = snap.bytes();
        let changed = self.time == 0 || self.core != core || current.delta(self.bytes).is_none();
        let elapsed = now - self.time;
        let continuous = !changed && !interrupted && (1..=3000).contains(&elapsed);
        if changed {
            batch.event = Some(
                if self.time == 0 {
                    "开始监控，已建立总量基线"
                } else {
                    "内核运行实例或计数发生变化，已重新建立基线"
                }
                .into(),
            );
            if self.time > 0 {
                batch.gap = Some((
                    self.time,
                    now,
                    Bytes::default(),
                    "内核变化，缺口不可恢复".into(),
                ));
            }
            self.epoch = uuid::Uuid::new_v4().to_string();
            self.epoch_start = now;
            self.seen.clear();
        } else if !continuous {
            batch.gap = Some((
                self.time,
                now,
                current.delta(self.bytes).unwrap(),
                "跨缺口恢复：时间和应用归属不可确定".into(),
            ));
            self.seen.clear();
            self.epoch_start = now;
        } else {
            let delta = current.delta(self.bytes).unwrap();
            batch.sample_from = Some(self.time);
            batch.rate = delta;
            batch.totals = split(self.time, now, delta);
        }
        for c in snap.connections.as_deref().unwrap_or_default() {
            if let Some(attribution) = &c.attribution {
                batch
                    .identities
                    .insert(attribution.owner.key.clone(), attribution.owner.clone());
            }
            if continuous {
                let increment =
                    if let Some(old) = self.seen.get(&c.id).filter(|s| s.start == c.start) {
                        c.bytes().delta(old.bytes).unwrap_or_default()
                    } else if c.started() >= self.epoch_start && c.started() >= self.time {
                        // New connections can have sent data before their first observed snapshot.
                        c.bytes()
                    } else {
                        Bytes::default()
                    };
                if increment.total() > 0 {
                    let app = c.app();
                    batch
                        .application_rates
                        .entry(c.application())
                        .or_default()
                        .add(increment);
                    batch
                        .app_rates
                        .entry(app.clone())
                        .or_default()
                        .add(increment);
                    for (time, bytes) in split(self.time, now, increment) {
                        let fact = Fact {
                            time,
                            app: app.clone(),
                            domain: c.domain(),
                            route: c.route(),
                            source: c.metadata.source_ip.clone(),
                            bytes,
                        };
                        if let Some(attribution) = &c.attribution
                            && attribution.owner.key != app
                            // The package path already maps to this owner in the ledger.
                            && !(attribution.owner.key.starts_with(crate::attribution::PACKAGE_KEY_PREFIX)
                                && attribution.owner.icon == app)
                        {
                            batch.owners.push(OwnerFact {
                                fact: fact.clone(),
                                owner: attribution.owner.clone(),
                            });
                        }
                        batch.facts.push(fact);
                    }
                }
            }
            self.seen.insert(
                c.id.clone(),
                Seen {
                    bytes: c.bytes(),
                    start: c.start.clone(),
                    time: now,
                },
            );
        }
        self.seen.retain(|_, s| s.time == now);
        self.time = now;
        self.bytes = current;
        self.core = core.into();
        self.sequence += 1;
        batch
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preexisting_and_reappearing_connections_do_not_double_count() {
        let mut l = Ledger::default();
        let c = Connection {
            id: "x".into(),
            upload: 10,
            download: 20,
            start: chrono::DateTime::from_timestamp_millis(500)
                .unwrap()
                .to_rfc3339(),
            ..Default::default()
        };
        let mut s = snap(100, 200);
        s.connections = Some(vec![c.clone()]);
        assert!(l.ingest(&s, "core", 1000, false).facts.is_empty());
        let mut c = c;
        c.upload = 15;
        c.download = 25;
        s.upload_total += 5;
        s.download_total += 5;
        s.connections = Some(vec![c.clone()]);
        assert_eq!(
            l.ingest(&s, "core", 2000, false)
                .facts
                .iter()
                .map(|f| f.bytes.total())
                .sum::<u64>(),
            10
        );
        s.connections = Some(vec![]);
        l.ingest(&s, "core", 3000, false);
        s.connections = Some(vec![c]);
        assert!(l.ingest(&s, "core", 4000, false).facts.is_empty());
    }
    fn snap(up: u64, down: u64) -> Snapshot {
        Snapshot {
            upload_total: up,
            download_total: down,
            connections: Some(vec![]),
        }
    }
    #[test]
    fn baseline_and_reset_are_not_traffic() {
        let mut l = Ledger::default();
        assert!(
            l.ingest(&snap(100, 200), "a", 1000, false)
                .totals
                .is_empty()
        );
        assert_eq!(
            l.ingest(&snap(110, 230), "a", 2000, false).rate,
            Bytes { up: 10, down: 30 }
        );
        assert!(l.ingest(&snap(1, 2), "a", 3000, false).totals.is_empty());
        assert!(l.ingest(&snap(50, 50), "b", 4000, false).totals.is_empty());
    }
    #[test]
    fn gap_is_not_spread_into_minutes() {
        let mut l = Ledger::default();
        l.ingest(&snap(10, 20), "a", 1000, false);
        let b = l.ingest(&snap(50, 100), "a", 90_000, true);
        assert!(b.totals.is_empty());
        assert_eq!(b.gap.unwrap().2, Bytes { up: 40, down: 80 });
    }
    #[test]
    fn buckets_preserve_integer_bytes() {
        let b = split(59_500, 60_500, Bytes { up: 3, down: 9 });
        assert_eq!(b.len(), 2);
        assert_eq!(b[0].1, Bytes { up: 1, down: 4 });
        assert_eq!(b[1].1, Bytes { up: 2, down: 5 });
    }
    #[test]
    fn new_connection_and_unknown_process() {
        let mut l = Ledger::default();
        l.ingest(&snap(0, 0), "a", 1000, false);
        let mut s = snap(10, 30);
        s.connections = Some(vec![Connection {
            id: "x".into(),
            upload: 10,
            download: 30,
            start: chrono::DateTime::from_timestamp_millis(1500)
                .unwrap()
                .to_rfc3339(),
            ..Default::default()
        }]);
        let b = l.ingest(&s, "a", 2000, false);
        assert_eq!(b.facts[0].app, "未知进程");
        assert_eq!(b.facts[0].bytes.total(), 40);
        assert!(l.ingest(&s, "a", 3000, false).facts.is_empty());
    }
    #[test]
    fn restart_checkpoint_preserves_baseline() {
        let mut l = Ledger::default();
        l.ingest(&snap(10, 20), "a", 1000, false);
        let mut restored: Ledger =
            serde_json::from_str(&serde_json::to_string(&l).unwrap()).unwrap();
        assert_eq!(
            restored
                .ingest(&snap(20, 50), "a", 2000, true)
                .gap
                .unwrap()
                .2
                .total(),
            40
        );
    }
}
