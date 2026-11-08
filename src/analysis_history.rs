use super::*;

/// Frozen aggregates only: raw application minutes never escape the read worker.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
struct Aggregate {
    metrics: Metrics,
    buckets: BTreeMap<i64, Vec<[Bytes; 2]>>,
}
impl Aggregate {
    fn new(request: &AnalysisRequest) -> Self {
        Self {
            buckets: request
                .bucket_sizes()
                .into_iter()
                .map(|size| {
                    let step = size.millis();
                    (
                        step,
                        vec![
                            [Bytes::default(); 2];
                            ((request.duration() + step - 1) / step) as usize
                        ],
                    )
                })
                .collect(),
            ..Default::default()
        }
    }
    fn add(&mut self, request: &AnalysisRequest, time: i64, bytes: Bytes, valid: bool) {
        let w = request.windows.unwrap();
        let (previous, from) = if time >= w.from && time < w.to {
            (false, w.from)
        } else if time >= w.previous_from && time < w.previous_to {
            (true, w.previous_from)
        } else {
            return;
        };
        let (total, peak) = if previous {
            (&mut self.metrics.previous, &mut self.metrics.previous_peak)
        } else {
            (&mut self.metrics.current, &mut self.metrics.peak)
        };
        total.add(bytes);
        if valid && peak.is_none_or(|p| bytes.total() > p.bytes.total()) {
            *peak = Some(Peak { time, bytes });
        }
        for (&step, buckets) in &mut self.buckets {
            buckets[((time - from) / step) as usize][usize::from(previous)].add(bytes);
        }
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(super) struct HistoricalSnapshot {
    pub coverage: [i64; 2],
    total: Aggregate,
    apps: BTreeMap<String, Aggregate>,
    coverage_buckets: BTreeMap<i64, Vec<[u64; 2]>>,
}
impl HistoricalSnapshot {
    pub(super) fn metrics(&self, key: Option<&str>) -> Metrics {
        key.map_or_else(
            || self.total.metrics.clone(),
            |key| {
                self.apps
                    .get(key)
                    .map(|a| a.metrics.clone())
                    .unwrap_or_default()
            },
        )
    }
    pub(super) fn histogram(
        &self,
        request: &AnalysisRequest,
        size: BucketSize,
        app: Option<&str>,
    ) -> Vec<ComparisonBucket> {
        let size = if request.bucket_sizes().contains(&size) {
            size
        } else {
            request.default_bucket()
        };
        let step = size.millis();
        let coverage = &self.coverage_buckets[&step];
        let aggregate = app.map_or(Some(&self.total), |key| self.apps.get(key));
        let w = request.windows.unwrap();
        coverage
            .iter()
            .enumerate()
            .map(|(index, covered)| {
                let bytes = aggregate
                    .map(|a| a.buckets[&step][index])
                    .unwrap_or_default();
                ComparisonBucket {
                    from: w.from + index as i64 * step,
                    previous_from: w.previous_from + index as i64 * step,
                    duration: step.min(request.duration() - index as i64 * step),
                    current: BucketValue {
                        bytes: bytes[0],
                        covered_ms: covered[0],
                    },
                    previous: BucketValue {
                        bytes: bytes[1],
                        covered_ms: covered[1],
                    },
                }
            })
            .collect()
    }
}

fn contains(spans: &[(i64, i64)], time: i64) -> bool {
    let index = spans.partition_point(|&(from, _)| from <= time);
    index > 0 && time + MINUTE <= spans[index - 1].1
}

pub(super) fn read(path: &Path, request: AnalysisRequest) -> Result<Snapshot> {
    let w = request.windows.context("缺少历史分析时间窗口")?;
    anyhow::ensure!(
        w.to > w.from
            && w.to - w.from == w.previous_to - w.previous_from
            && w.previous_to <= w.from,
        "对比时间窗口无效"
    );
    let mut db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    db.busy_timeout(Duration::from_secs(5))?;
    let tx = db.transaction()?;
    let earliest: Option<i64> = tx.query_row("SELECT MIN(time) FROM totals", [], |r| r.get(0))?;
    let checkpoint: Option<String> = tx
        .query_row("SELECT value FROM meta WHERE key='checkpoint'", [], |r| {
            r.get(0)
        })
        .optional()?;
    let latest = checkpoint
        .map(|s| serde_json::from_str::<crate::accounting::Ledger>(&s))
        .transpose()?
        .map(|l| l.time);
    let mut gaps: Vec<(i64, i64)> = Vec::new();
    for gap in tx
        .prepare("SELECT started,ended FROM gaps WHERE started<?2 AND ended>?1 ORDER BY started")?
        .query_map(
            params![w.previous_from.min(w.from - 5 * MINUTE), w.to],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
        )?
    {
        let (from, to) = gap?;
        if let Some(last) = gaps.last_mut()
            && last.1 >= from
        {
            last.1 = last.1.max(to);
        } else {
            gaps.push((from, to));
        }
    }
    let mut data = HistoricalSnapshot {
        coverage: [0; 2],
        total: Aggregate::new(&request),
        apps: BTreeMap::new(),
        coverage_buckets: request
            .bucket_sizes()
            .into_iter()
            .map(|s| {
                (
                    s.millis(),
                    vec![[0; 2]; ((request.duration() + s.millis() - 1) / s.millis()) as usize],
                )
            })
            .collect(),
    };
    let mut valid_spans: Vec<(i64, i64)> = Vec::new();
    {
        let mut stmt = tx.prepare("SELECT time,up,down FROM totals WHERE (time>=?1 AND time<?2) OR (time>=?3 AND time<?4) ORDER BY time")?;
        for row in stmt.query_map(
            params![w.previous_from, w.previous_to, w.from - 5 * MINUTE, w.to],
            |r| Ok((r.get::<_, i64>(0)?, sql_bytes(r, 1)?)),
        )? {
            let (time, bytes) = row?;
            let gap = gaps.partition_point(|&(from, _)| from < time + MINUTE);
            let valid = earliest.is_some_and(|first| time > first)
                && latest.is_some_and(|last| last >= time + MINUTE)
                && (gap == 0 || gaps[gap - 1].1 <= time);
            if valid {
                if let Some(last) = valid_spans.last_mut()
                    && last.1 == time
                {
                    last.1 += MINUTE;
                } else {
                    valid_spans.push((time, time + MINUTE));
                }
            }
            data.total.add(&request, time, bytes, valid);
            let period = if time >= w.from && time < w.to {
                Some((0, w.from))
            } else if time >= w.previous_from && time < w.previous_to {
                Some((1, w.previous_from))
            } else {
                None
            };
            if valid && let Some((period, from)) = period {
                data.coverage[period] += MINUTE;
                for (&step, buckets) in &mut data.coverage_buckets {
                    buckets[((time - from) / step) as usize][period] += MINUTE as u64;
                }
            }
        }
    }
    let zero_peak = |from, to| {
        valid_spans.iter().find_map(|&(a, b)| {
            let time = a.max(from);
            (time < b.min(to)).then_some(Peak {
                time,
                bytes: Bytes::default(),
            })
        })
    };
    let mut bursts = Vec::new();
    {
        let mut stmt = tx.prepare("SELECT d.name,f.time,f.up,f.down FROM application_minutes f JOIN dimensions d ON d.id=f.app WHERE (f.time>=?1 AND f.time<?2) OR (f.time>=?3 AND f.time<?4) ORDER BY d.name,f.time")?;
        let mut key_before = String::new();
        let mut baseline: VecDeque<(i64, Bytes)> = VecDeque::new();
        for row in stmt.query_map(
            params![w.previous_from, w.previous_to, w.from - 5 * MINUTE, w.to],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    sql_bytes(r, 2)?,
                ))
            },
        )? {
            let (key, time, bytes) = row?;
            if key != key_before {
                baseline.clear();
                key_before.clone_from(&key);
            }
            while baseline
                .front()
                .is_some_and(|&(t, _)| t < time - 5 * MINUTE)
            {
                baseline.pop_front();
            }
            let app = data
                .apps
                .entry(key.clone())
                .or_insert_with(|| Aggregate::new(&request));
            app.add(&request, time, bytes, contains(&valid_spans, time));
            if time >= w.from && (0..=5).all(|i| contains(&valid_spans, time - i * MINUTE)) {
                let sum: u128 = baseline.iter().map(|(_, b)| b.total() as u128).sum();
                let value = bytes.total() as u128 * 5;
                if value >= sum * 3 && value.saturating_sub(sum) >= BURST_INCREASE * 5 {
                    bursts.push(Burst {
                        app: key,
                        time,
                        bytes,
                        baseline: (sum / 5).min(u64::MAX as u128) as u64,
                    });
                }
            }
            baseline.push_back((time, bytes));
        }
    }
    for app in data.apps.values_mut() {
        if app.metrics.peak.is_none() {
            app.metrics.peak = zero_peak(w.from, w.to);
        }
        if app.metrics.previous_peak.is_none() {
            app.metrics.previous_peak = zero_peak(w.previous_from, w.previous_to);
        }
    }
    let mut snapshot = Snapshot::new(request);
    snapshot.rows = data
        .apps
        .iter()
        .filter(|(_, a)| a.metrics.current.total() > 0 || a.metrics.previous.total() > 0)
        .map(|(key, a)| AppComparison {
            key: key.clone(),
            metrics: a.metrics.clone(),
        })
        .collect();
    snapshot.rows.sort_by(|a, b| {
        b.metrics
            .delta()
            .unsigned_abs()
            .cmp(&a.metrics.delta().unsigned_abs())
            .then_with(|| a.key.cmp(&b.key))
    });
    snapshot.apps = data
        .apps
        .keys()
        .map(|key| (key.clone(), Trace::default()))
        .collect();
    bursts.sort_by(|a, b| {
        b.time
            .cmp(&a.time)
            .then_with(|| b.increase().cmp(&a.increase()))
            .then_with(|| a.app.cmp(&b.app))
    });
    snapshot.bursts = bursts;
    snapshot.historical = Some(data);
    tx.commit().context("读取历史分析快照失败")?;
    Ok(snapshot)
}

#[cfg(test)]
pub(super) fn fixture(request: AnalysisRequest) -> Snapshot {
    let w = request.windows.unwrap();
    let mut data = HistoricalSnapshot {
        coverage: [request.duration(); 2],
        total: Aggregate::new(&request),
        apps: BTreeMap::new(),
        coverage_buckets: request
            .bucket_sizes()
            .into_iter()
            .map(|s| {
                let step = s.millis();
                (
                    step,
                    (0..(request.duration() + step - 1) / step)
                        .map(|i| [step.min(request.duration() - i * step) as u64; 2])
                        .collect(),
                )
            })
            .collect(),
    };
    for index in 0..22 {
        let key = format!(r"C:\历史应用\long folder with spaces\app-{index:02}.exe");
        let mut app = Aggregate::new(&request);
        app.metrics = Metrics {
            current: Bytes {
                down: (22 - index) * 10_000_000,
                up: 1_000_000,
            },
            previous: Bytes {
                down: 10_000_000,
                up: 1_000_000,
            },
            peak: Some(Peak {
                time: w.from,
                bytes: Bytes {
                    down: 1_000_000,
                    up: 0,
                },
            }),
            previous_peak: Some(Peak {
                time: w.previous_from,
                bytes: Bytes {
                    down: 100_000,
                    up: 0,
                },
            }),
        };
        data.total.metrics.current.add(app.metrics.current);
        data.total.metrics.previous.add(app.metrics.previous);
        for (&step, bins) in &mut app.buckets {
            for (i, bin) in bins.iter_mut().enumerate() {
                let from = i as i64 * step;
                let to = (from + step).min(request.duration());
                *bin = [
                    portion(app.metrics.current, 0, request.duration(), from, to),
                    portion(app.metrics.previous, 0, request.duration(), from, to),
                ];
                for (period, bytes) in bin.iter().enumerate() {
                    data.total.buckets.get_mut(&step).unwrap()[i][period].add(*bytes);
                }
            }
        }
        data.apps.insert(key, app);
    }
    let mut snapshot = Snapshot::new(request);
    snapshot.rows = data
        .apps
        .iter()
        .map(|(key, app)| AppComparison {
            key: key.clone(),
            metrics: app.metrics.clone(),
        })
        .collect();
    snapshot.apps = data
        .apps
        .keys()
        .map(|key| (key.clone(), Trace::default()))
        .collect();
    snapshot.historical = Some(data);
    snapshot
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn historical_windows_compare_the_elapsed_part_of_the_previous_cycle() {
        let from = 10_000 * MINUTE;
        let to = from + 1440 * MINUTE;
        let request =
            AnalysisRequest::historical(1, 7, from, to, from + 840 * MINUTE + 999).unwrap();
        assert_eq!(
            request.windows,
            Some(ComparisonWindows {
                from,
                to: from + 840 * MINUTE,
                previous_from: from - 1440 * MINUTE,
                previous_to: from - 600 * MINUTE,
            })
        );
        assert_eq!(request.generation, 7);
        let complete = AnalysisRequest::historical(2, 7, from, to, to + 1)
            .unwrap()
            .windows
            .unwrap();
        assert_eq!(complete.previous_to, from);
        assert!(AnalysisRequest::historical(1, 0, from, to, from - 1).is_err());
        assert!(AnalysisRequest::historical(1, 0, from, to, from + MINUTE - 1).is_err());
        assert!(AnalysisRequest::historical(1, 0, from + 1, to, to).is_err());
        assert!(AnalysisRequest::historical(1, 0, from, from, to).is_err());
        assert!(
            AnalysisRequest::historical(1, 0, from, from + 3661 * 86_400_000_i64, i64::MAX)
                .is_err()
        );
    }

    fn database() -> (tempfile::TempDir, crate::storage::Storage) {
        let dir = tempfile::tempdir().unwrap();
        let storage = crate::storage::Storage::open(&dir.path().join("traffic.sqlite3")).unwrap();
        storage.db.execute_batch("INSERT INTO dimensions VALUES(1,'app','a'); INSERT INTO dimensions VALUES(2,'app','previous-only');").unwrap();
        for minute in 0..270 {
            let bytes = if minute == 203 { 10_000_000 } else { minute };
            storage
                .db
                .execute(
                    "INSERT INTO totals VALUES(?1,2,?2)",
                    params![minute * MINUTE, bytes],
                )
                .unwrap();
            storage
                .db
                .execute(
                    "INSERT INTO app_minutes VALUES(?1,1,2,?2)",
                    params![minute * MINUTE, bytes],
                )
                .unwrap();
        }
        storage
            .db
            .execute("INSERT INTO app_minutes VALUES(?1,2,0,7)", [142 * MINUTE])
            .unwrap();
        let ledger = crate::accounting::Ledger {
            time: 270 * MINUTE,
            ..Default::default()
        };
        storage
            .db
            .execute(
                "INSERT INTO meta VALUES('checkpoint',?1)",
                [serde_json::to_string(&ledger).unwrap()],
            )
            .unwrap();
        (dir, storage)
    }

    #[test]
    fn streamed_history_preserves_both_windows_tail_buckets_and_minute_bursts() {
        let (dir, storage) = database();
        // The current cycle has elapsed seven of sixty minutes; the periods are not adjacent.
        let request =
            AnalysisRequest::historical(1, 0, 200 * MINUTE, 260 * MINUTE, 207 * MINUTE).unwrap();
        let snapshot = historical(&dir.path().join("traffic.sqlite3"), request.clone()).unwrap();
        assert!(snapshot.comparable());
        assert_eq!(snapshot.coverage(false), 7 * MINUTE);
        let metrics = snapshot.metrics(Some("a"));
        assert_eq!(
            metrics.current.down,
            10_000_000 + 200 + 201 + 202 + 204 + 205 + 206
        );
        assert_eq!(metrics.previous.down, (140..147).sum::<u64>());
        assert_eq!(metrics.peak.unwrap().time, 203 * MINUTE);
        assert_eq!(snapshot.rows.len(), 2);
        assert_eq!(snapshot.metrics(Some("previous-only")).current.total(), 0);
        assert_eq!(snapshot.metrics(Some("previous-only")).previous.down, 7);
        assert_eq!(snapshot.bursts.len(), 1);
        assert!(snapshot.covered.is_empty());
        assert!(snapshot.apps.values().all(|trace| trace.bins.is_empty()));
        for size in request.bucket_sizes() {
            let bins = snapshot.histogram(size, Some("a"));
            assert_eq!(
                bins.iter().map(|b| b.current.bytes.down).sum::<u64>(),
                metrics.current.down
            );
            assert_eq!(
                bins.iter().map(|b| b.previous.bytes.down).sum::<u64>(),
                metrics.previous.down
            );
            assert_eq!(
                bins.iter().map(|b| b.duration).sum::<i64>(),
                request.duration()
            );
            assert!(bins.iter().all(|b| b.comparable()));
        }
        let bins = snapshot.histogram(BucketSize::Minutes5, None);
        assert_eq!(bins.last().unwrap().duration, 2 * MINUTE);
        storage
            .db
            .execute(
                "INSERT INTO gaps(started,ended,up,down,reason) VALUES(?1,?2,0,0,'gap')",
                params![202 * MINUTE, 202 * MINUTE + 1],
            )
            .unwrap();
        let partial = historical(&dir.path().join("traffic.sqlite3"), request).unwrap();
        assert!(!partial.comparable());
        assert!(partial.can_compare_observed());
        assert_eq!(partial.coverage(false), 6 * MINUTE);
        assert_eq!(partial.metrics(Some("a")).current, metrics.current);
        assert!(partial.bursts.is_empty());
        assert_eq!(snapshot.bursts.len(), 1); // Existing results remain frozen.
    }

    #[test]
    fn long_ranges_allocate_buckets_instead_of_application_minutes() {
        let duration = 3660 * 86_400_000_i64;
        let request =
            AnalysisRequest::historical(1, 0, duration, 2 * duration, 2 * duration).unwrap();
        let aggregate = Aggregate::new(&request);
        assert!(aggregate.buckets.values().all(|b| b.len() <= 180));
        assert!(aggregate.buckets[&request.default_bucket().millis()].len() <= 60);
        let coverage = BucketValue {
            covered_ms: BucketSize::Days90.millis() as u64,
            ..Default::default()
        };
        assert!(coverage.complete(BucketSize::Days90.millis()));
    }

    #[test]
    fn empty_history_is_missing_and_a_writer_does_not_change_the_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("traffic.sqlite3");
        let storage = crate::storage::Storage::open(&path).unwrap();
        let request =
            AnalysisRequest::historical(1, 0, 10 * MINUTE, 20 * MINUTE, 30 * MINUTE).unwrap();
        let tx = storage.db.unchecked_transaction().unwrap();
        tx.execute("INSERT INTO totals VALUES(?1,100,100)", [12 * MINUTE])
            .unwrap();
        let snapshot = historical(&path, request).unwrap();
        assert!(!snapshot.can_compare_observed());
        assert_eq!(snapshot.metrics(None).current.total(), 0);
        assert!(
            snapshot
                .histogram(BucketSize::Minute, None)
                .iter()
                .all(|b| !b.current.observed())
        );
        tx.rollback().unwrap();
    }
}
