use crate::model::{DiskMetrics, GpuMetrics, Shared, SystemMetrics};
use std::{collections::HashMap, ptr::null_mut};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, IDXGIFactory1,
};
use windows_sys::Win32::{
    Foundation::FILETIME,
    System::{
        Performance::*,
        SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX},
        Threading::GetSystemTimes,
    },
};

// pdh.h format modifier; windows-sys exposes the base formats only.
const PDH_FMT_NOCAP100: u32 = 0x8000;

#[derive(Clone, Copy)]
struct CpuTimes {
    idle: u64,
    kernel: u64,
    user: u64,
}

impl CpuTimes {
    fn read() -> Option<Self> {
        let (mut idle, mut kernel, mut user) = (
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
        );
        if unsafe { GetSystemTimes(&mut idle, &mut kernel, &mut user) } == 0 {
            return None;
        }
        let ticks =
            |time: FILETIME| (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
        Some(Self {
            idle: ticks(idle),
            kernel: ticks(kernel),
            user: ticks(user),
        })
    }

    fn usage(self, previous: Self) -> Option<f64> {
        let idle = self.idle.checked_sub(previous.idle)?;
        let total = self
            .kernel
            .checked_sub(previous.kernel)?
            .checked_add(self.user.checked_sub(previous.user)?)?;
        // Kernel time includes idle time. Reset counters and empty intervals
        // establish a new baseline instead of producing a fabricated percentage.
        if total == 0 || idle > total {
            return None;
        }
        Some((total - idle) as f64 * 100. / total as f64)
    }
}

fn memory() -> Option<(u64, u64)> {
    let mut status = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    if unsafe { GlobalMemoryStatusEx(&mut status) } == 0 || status.ullTotalPhys == 0 {
        return None;
    }
    Some((
        status.ullTotalPhys.saturating_sub(status.ullAvailPhys),
        status.ullTotalPhys,
    ))
}

/// Owned and used entirely on the sampler thread. English paths work on
/// localized Windows; wildcard arrays follow newly created counter instances.
struct Counters {
    query: PDH_HQUERY,
    gpu: PDH_HCOUNTER,
    dedicated: PDH_HCOUNTER,
    shared: PDH_HCOUNTER,
    idle: PDH_HCOUNTER,
    read: PDH_HCOUNTER,
    write: PDH_HCOUNTER,
}

impl Drop for Counters {
    fn drop(&mut self) {
        unsafe {
            PdhCloseQuery(self.query);
        }
    }
}

impl Counters {
    fn retry_missing(&mut self) {
        for (counter, path) in [
            (&mut self.gpu, r"\GPU Engine(*)\Utilization Percentage"),
            (
                &mut self.dedicated,
                r"\GPU Adapter Memory(*)\Dedicated Usage",
            ),
            (&mut self.shared, r"\GPU Adapter Memory(*)\Shared Usage"),
            (&mut self.idle, r"\PhysicalDisk(*)\% Idle Time"),
            (&mut self.read, r"\PhysicalDisk(*)\Disk Read Bytes/sec"),
            (&mut self.write, r"\PhysicalDisk(*)\Disk Write Bytes/sec"),
        ] {
            if counter.is_null() {
                unsafe {
                    PdhAddEnglishCounterW(
                        self.query,
                        crate::native::wide(path).as_ptr(),
                        0,
                        counter,
                    );
                }
            }
        }
    }
    fn new() -> Option<Self> {
        let mut query = null_mut();
        if unsafe { PdhOpenQueryW(std::ptr::null(), 0, &mut query) } != 0 {
            return None;
        }
        let add = |path: &str| {
            let mut counter = null_mut();
            let path = crate::native::wide(path);
            if unsafe { PdhAddEnglishCounterW(query, path.as_ptr(), 0, &mut counter) } == 0 {
                counter
            } else {
                null_mut()
            }
        };
        let result = Self {
            query,
            gpu: add(r"\GPU Engine(*)\Utilization Percentage"),
            dedicated: add(r"\GPU Adapter Memory(*)\Dedicated Usage"),
            shared: add(r"\GPU Adapter Memory(*)\Shared Usage"),
            idle: add(r"\PhysicalDisk(*)\% Idle Time"),
            read: add(r"\PhysicalDisk(*)\Disk Read Bytes/sec"),
            write: add(r"\PhysicalDisk(*)\Disk Write Bytes/sec"),
        };
        unsafe {
            PdhCollectQueryData(query);
        }
        Some(result)
    }

    fn array(counter: PDH_HCOUNTER) -> Vec<(String, f64)> {
        if counter.is_null() {
            return Vec::new();
        }
        let (mut bytes, mut count) = (0, 0);
        let format = PDH_FMT_DOUBLE | PDH_FMT_NOCAP100;
        let status = unsafe {
            PdhGetFormattedCounterArrayW(counter, format, &mut bytes, &mut count, null_mut())
        };
        if status != PDH_MORE_DATA || bytes == 0 {
            return Vec::new();
        }
        // u64 allocation keeps both the item structures and the trailing UTF-16
        // names aligned. Retry if instances change between sizing and reading.
        for _ in 0..3 {
            if bytes > 16 * 1024 * 1024 {
                return Vec::new();
            }
            let mut buffer = vec![0u64; (bytes as usize).div_ceil(8)];
            let allocated = buffer.len() * 8;
            let items = buffer.as_mut_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>();
            let status = unsafe {
                PdhGetFormattedCounterArrayW(counter, format, &mut bytes, &mut count, items)
            };
            if status == PDH_MORE_DATA {
                continue;
            }
            if status != 0
                || count as usize > allocated / std::mem::size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>()
            {
                return Vec::new();
            }
            let start = buffer.as_ptr() as usize;
            let end = start + allocated;
            return unsafe { std::slice::from_raw_parts(items, count as usize) }
                .iter()
                .filter_map(|item| {
                    if !matches!(
                        item.FmtValue.CStatus,
                        PDH_CSTATUS_VALID_DATA | PDH_CSTATUS_NEW_DATA
                    ) {
                        return None;
                    }
                    let value = unsafe { item.FmtValue.Anonymous.doubleValue };
                    if !value.is_finite() || value < 0. {
                        return None;
                    }
                    let address = item.szName as usize;
                    if address < start || address >= end || !address.is_multiple_of(2) {
                        return None;
                    }
                    let text =
                        unsafe { std::slice::from_raw_parts(item.szName, (end - address) / 2) };
                    let len = text.iter().position(|ch| *ch == 0)?;
                    Some((String::from_utf16_lossy(&text[..len]), value))
                })
                .collect();
        }
        Vec::new()
    }

    fn sample(&self, adapters: &[Adapter]) -> (Vec<GpuMetrics>, Vec<DiskMetrics>) {
        if unsafe { PdhCollectQueryData(self.query) } != 0 {
            return (
                adapters.iter().map(|a| a.metric.clone()).collect(),
                Vec::new(),
            );
        }
        // PDH instance casing varies by provider (e.g. 0x00014D29).
        // LUIDs and engine names are Windows identifiers, not case-sensitive text.
        let lower_names = |values: Vec<(String, f64)>| {
            values
                .into_iter()
                .map(|(name, value)| (name.to_ascii_lowercase(), value))
                .collect::<Vec<_>>()
        };
        let engines = lower_names(Self::array(self.gpu));
        let dedicated = lower_names(Self::array(self.dedicated));
        let shared = lower_names(Self::array(self.shared));
        let gpus = adapters
            .iter()
            .map(|adapter| {
                let mut metric = adapter.metric.clone();
                let mut totals = HashMap::<String, f64>::new();
                for (name, value) in &engines {
                    if let Some((_, engine)) = name.split_once(&adapter.counter_key) {
                        *totals.entry(engine.into()).or_default() += value;
                    }
                }
                metric.percent = totals
                    .values()
                    .copied()
                    .reduce(f64::max)
                    .map(|v| v.clamp(0., 100.));
                let counters = if metric.shared_memory {
                    &shared
                } else {
                    &dedicated
                };
                let usage = counters
                    .iter()
                    .filter(|(name, _)| name.contains(&adapter.counter_key))
                    .map(|(_, v)| *v)
                    .reduce(|a, b| a + b);
                metric.memory = usage.map(|used| (used as u64, adapter.capacity));
                metric
            })
            .collect();
        let idle = Self::array(self.idle);
        let reads = Self::array(self.read);
        let writes = Self::array(self.write);
        let mut names = std::collections::BTreeSet::new();
        for (name, _) in idle.iter().chain(&reads).chain(&writes) {
            if name != "_Total" {
                names.insert(name.clone());
            }
        }
        let mut disks: Vec<_> = names
            .into_iter()
            .map(|name| DiskMetrics {
                id: name.split_whitespace().next().unwrap_or(&name).into(),
                name: format!("磁盘 {name}"),
                percent: idle
                    .iter()
                    .find(|(n, _)| n == &name)
                    .map(|(_, v)| (100. - v).clamp(0., 100.)),
                read: reads.iter().find(|(n, _)| n == &name).map(|(_, v)| *v),
                write: writes.iter().find(|(n, _)| n == &name).map(|(_, v)| *v),
            })
            .collect();
        if !disks.is_empty() {
            let aggregate = DiskMetrics {
                id: String::new(),
                name: "所有磁盘".into(),
                percent: disks
                    .iter()
                    .filter_map(|disk| disk.percent)
                    .reduce(f64::max),
                read: disks
                    .iter()
                    .map(|disk| disk.read)
                    .collect::<Option<Vec<_>>>()
                    .map(|v| v.iter().sum()),
                write: disks
                    .iter()
                    .map(|disk| disk.write)
                    .collect::<Option<Vec<_>>>()
                    .map(|v| v.iter().sum()),
            };
            disks.insert(0, aggregate);
        }
        (gpus, disks)
    }
}

struct Adapter {
    metric: GpuMetrics,
    counter_key: String,
    capacity: u64,
}

fn adapters() -> Vec<Adapter> {
    let Ok(factory) = (unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }) else {
        return Vec::new();
    };
    let mut adapters = Vec::new();
    for index in 0..32 {
        let Ok(adapter) = (unsafe { factory.EnumAdapters1(index) }) else {
            break;
        };
        let Ok(desc) = (unsafe { adapter.GetDesc1() }) else {
            continue;
        };
        if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
            continue;
        }
        let len = desc
            .Description
            .iter()
            .position(|v| *v == 0)
            .unwrap_or(desc.Description.len());
        let shared = desc.DedicatedVideoMemory == 0;
        adapters.push(Adapter {
            metric: GpuMetrics {
                id: format!(
                    "{:04x}-{:04x}-{:08x}-{index}",
                    desc.VendorId, desc.DeviceId, desc.SubSysId
                ),
                name: format!(
                    "GPU {index} · {}",
                    String::from_utf16_lossy(&desc.Description[..len])
                ),
                shared_memory: shared,
                ..Default::default()
            },
            counter_key: format!(
                "luid_0x{:08x}_0x{:08x}",
                desc.AdapterLuid.HighPart as u32, desc.AdapterLuid.LowPart
            ),
            capacity: if shared {
                desc.SharedSystemMemory as u64
            } else {
                desc.DedicatedVideoMemory as u64
            },
        });
    }
    adapters
}

pub fn start(shared: Arc<Mutex<Shared>>, stop: Arc<AtomicBool>) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("system-metrics".into())
        .spawn(move || {
            let mut previous = None;
            let mut counters = Counters::new();
            let mut gpu_adapters = adapters();
            let mut tick = 0u64;
            while !stop.load(Ordering::Relaxed) {
                if tick.is_multiple_of(30) {
                    gpu_adapters = adapters();
                    if counters.is_none() {
                        counters = Counters::new();
                    }
                    if let Some(counters) = counters.as_mut() {
                        counters.retry_missing();
                    }
                }
                let now = CpuTimes::read();
                let cpu_percent = now.zip(previous).and_then(|(now, old)| now.usage(old));
                previous = now;
                let (gpus, disks) = counters
                    .as_ref()
                    .map(|c| c.sample(&gpu_adapters))
                    .unwrap_or_else(|| {
                        (
                            gpu_adapters.iter().map(|a| a.metric.clone()).collect(),
                            Vec::new(),
                        )
                    });
                let metrics = SystemMetrics {
                    cpu_percent,
                    memory: memory(),
                    gpus,
                    disks,
                };
                shared.lock().unwrap().system = metrics;
                std::thread::sleep(Duration::from_secs(1));
                tick += 1;
            }
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_usage_excludes_idle_and_rejects_invalid_intervals() {
        let old = CpuTimes {
            idle: 100,
            kernel: 200,
            user: 100,
        };
        assert_eq!(
            CpuTimes {
                idle: 170,
                kernel: 280,
                user: 120
            }
            .usage(old),
            Some(30.)
        );
        assert_eq!(old.usage(old), None);
        assert_eq!(
            CpuTimes {
                idle: 90,
                kernel: 280,
                user: 120
            }
            .usage(old),
            None
        );
        assert_eq!(
            CpuTimes {
                idle: 300,
                kernel: 280,
                user: 120
            }
            .usage(old),
            None
        );
    }

    #[test]
    fn windows_memory_and_cpu_counters_are_available() {
        let (used, total) = memory().unwrap();
        assert!(total > 0 && used <= total);
        assert!(CpuTimes::read().is_some());
    }

    #[test]
    fn windows_gpu_and_disk_counter_arrays_are_readable() {
        let counters = Counters::new().expect("PDH query");
        let adapters = adapters();
        std::thread::sleep(Duration::from_millis(1100));
        let (gpus, disks) = counters.sample(&adapters);
        println!("Windows GPU counters: {gpus:?}; physical disk counters: {disks:?}");
        let engines = Counters::array(counters.gpu);
        let dedicated = Counters::array(counters.dedicated);
        let shared = Counters::array(counters.shared);
        assert_eq!(gpus.len(), adapters.len());
        for (adapter, gpu) in adapters.iter().zip(&gpus) {
            // PDH can report instances for adapters that DXGI does not enumerate.
            let has_counter = |samples: &[(String, f64)]| {
                samples
                    .iter()
                    .any(|(name, _)| name.to_ascii_lowercase().contains(&adapter.counter_key))
            };
            assert!(
                !has_counter(&engines) || gpu.percent.is_some(),
                "missing utilization for {}",
                gpu.name
            );
            let memory = if adapter.metric.shared_memory {
                &shared
            } else {
                &dedicated
            };
            assert!(
                !has_counter(memory) || gpu.memory.is_some(),
                "missing memory usage for {}",
                gpu.name
            );
        }
        if !counters.idle.is_null() {
            assert!(
                disks.iter().any(|disk| disk.percent.is_some()),
                "registered disk counter returned no samples"
            );
        }
        for gpu in gpus {
            assert!(
                gpu.percent
                    .is_none_or(|value| (0. ..=100.).contains(&value))
            );
        }
        for disk in disks {
            assert!(
                disk.percent
                    .is_none_or(|value| (0. ..=100.).contains(&value))
            );
            assert!(
                disk.read.is_none_or(|value| value >= 0.)
                    && disk.write.is_none_or(|value| value >= 0.)
            );
        }
    }
}
