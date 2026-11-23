//! What the system says about the game's process and the device, for the Stats overlay and
//! `sunrise-ctl perf`: the process's memory (/proc/self), the native heap and the thermal
//! throttling level (probes a platform registers: sunrise-android's mallinfo and AThermal),
//! and the CPU, GPU and battery temperatures and the fastest core's clock where sysfs lets an
//! app read them (Linux and Android; elsewhere they stay unknown).
//!
//! A thread samples once a second, started by the first `latest`: reading smaps_rollup walks
//! every mapping, too slow for the frame. It rests while nobody asks (the overlay hidden, no
//! `perf`), so a look at the numbers does not cost the game a share of a core for good.
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Platform probes: each returns None when it cannot tell.
pub struct Probes {
    /// Bytes the native allocator has handed out.
    pub heap_bytes: fn() -> Option<u64>,
    /// The thermal status: 0 none, 1 light, 2 moderate, 3 severe, 4 critical, 5 emergency,
    /// 6 shutdown (Android's AThermalStatus).
    pub thermal_status: fn() -> Option<i32>,
    /// How close the device is to throttling in the next seconds: 1.0 is the throttling point.
    pub thermal_headroom: fn() -> Option<f32>,
}

static PROBES: OnceLock<Probes> = OnceLock::new();

/// Registers the platform's probes (before the first sample).
pub fn set_probes(probes: Probes) {
    let _ = PROBES.set(probes);
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SystemSample {
    pub rss_bytes: Option<u64>,
    /// Proportional set size: shared pages split among the processes sharing them.
    pub pss_bytes: Option<u64>,
    pub heap_bytes: Option<u64>,
    pub thermal_status: Option<i32>,
    pub thermal_headroom: Option<f32>,
    pub cpu_celsius: Option<f32>,
    pub gpu_celsius: Option<f32>,
    pub battery_celsius: Option<f32>,
    /// The fastest core: its current and its highest clock, in MHz.
    pub cpu_mhz: Option<[u32; 2]>,
}

/// AThermalStatus as a word.
pub fn thermal_name(status: i32) -> &'static str {
    match status {
        0 => "none",
        1 => "light",
        2 => "moderate",
        3 => "severe",
        4 => "critical",
        5 => "emergency",
        6 => "shutdown",
        _ => "unknown",
    }
}

#[derive(Default)]
pub struct Sampler {
    latest: Arc<Mutex<SystemSample>>,
    /// Someone read a sample since the thread last looked.
    wanted: Arc<AtomicBool>,
    started: OnceLock<()>,
}

impl Sampler {
    /// The last sample (all unknown until the first second has passed).
    pub fn latest(&self) -> SystemSample {
        self.wanted.store(true, Ordering::Relaxed);
        self.started.get_or_init(|| {
            let (latest, wanted) = (self.latest.clone(), self.wanted.clone());
            let _ = std::thread::Builder::new()
                .name("sunrise-stats".into())
                .spawn(move || {
                    let zones = Zones::find(Path::new("/sys/class/thermal"));
                    let cpus = fastest_core(Path::new("/sys/devices/system/cpu"));
                    let mut last: Option<Instant> = None;
                    loop {
                        // Asked for within the last second or so: at most a sample a second.
                        if last.is_none_or(|t| t.elapsed() >= Duration::from_secs(1))
                            && wanted.swap(false, Ordering::Relaxed)
                        {
                            let sample = sample(&zones, cpus.as_deref());
                            if let Ok(mut l) = latest.lock() {
                                *l = sample;
                            }
                            last = Some(Instant::now());
                        }
                        std::thread::sleep(Duration::from_millis(250));
                    }
                });
        });
        self.latest.lock().map(|l| l.clone()).unwrap_or_default()
    }
}

fn sample(zones: &Zones, cpu: Option<&Path>) -> SystemSample {
    let probes = PROBES.get();
    let status = fs::read_to_string("/proc/self/status").unwrap_or_default();
    let rollup = fs::read_to_string("/proc/self/smaps_rollup").unwrap_or_default();
    let khz = |name: &str| {
        cpu.and_then(|c| fs::read_to_string(c.join(name)).ok())
            .and_then(|t| t.trim().parse::<u32>().ok())
    };
    SystemSample {
        rss_bytes: kib_field(&status, "VmRSS:").map(|k| k * 1024),
        pss_bytes: kib_field(&rollup, "Pss:").map(|k| k * 1024),
        heap_bytes: probes.and_then(|p| (p.heap_bytes)()),
        thermal_status: probes.and_then(|p| (p.thermal_status)()),
        thermal_headroom: probes.and_then(|p| (p.thermal_headroom)()),
        cpu_celsius: zones.hottest(&zones.cpu),
        gpu_celsius: zones.hottest(&zones.gpu),
        battery_celsius: zones.hottest(&zones.battery),
        cpu_mhz: khz("scaling_cur_freq")
            .zip(khz("cpuinfo_max_freq"))
            .map(|(cur, max)| [cur / 1000, max / 1000]),
    }
}

/// How much a device can take, for the settings a phone's first start picks
/// (`graphics::apply_mobile_defaults`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    Low,
    Mid,
    High,
}

impl Tier {
    /// From the memory and the fastest core's highest clock, which stand for a phone's chip, and
    /// so its GPU, well enough: Snapdragon 8 Gen 2 has a 3.2 GHz core and 8 to 16 GB, 7 Gen 1
    /// 2.4 GHz and 6 to 8 GB, 695 2.2 GHz and 4 to 6 GB. The kernel keeps some memory back from
    /// `MemTotal`: an 8 GB phone reports about 7.4 GB.
    pub fn of(ram_bytes: u64, fastest_mhz: u32) -> Self {
        let gib = ram_bytes as f64 / f64::from(1u32 << 30);
        if fastest_mhz >= 2900 && gib >= 7.0 {
            Self::High
        } else if fastest_mhz >= 2350 && gib >= 5.0 {
            Self::Mid
        } else {
            Self::Low
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Mid => "mid",
            Self::High => "high",
        }
    }
    /// This device's, where /proc and sysfs tell (Linux and Android).
    pub fn here() -> Option<Self> {
        let meminfo = fs::read_to_string("/proc/meminfo").ok()?;
        let ram = kib_field(&meminfo, "MemTotal:")? * 1024;
        let core = fastest_core(Path::new("/sys/devices/system/cpu"))?;
        let khz: u32 = fs::read_to_string(core.join("cpuinfo_max_freq"))
            .ok()?
            .trim()
            .parse()
            .ok()?;
        Some(Self::of(ram, khz / 1000))
    }
}

/// What this process holds and what the device has left, for the log at the steps of a start and
/// when the graphics device is lost: a phone that runs out ends the game with no crash of its own
/// (the system kills it, or the driver loses the device), and a 32-bit game can also run out of
/// address space with memory to spare. None where /proc says nothing (not Linux or Android).
pub fn memory_note() -> Option<String> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    let meminfo = fs::read_to_string("/proc/meminfo").unwrap_or_default();
    memory_text(&status, &meminfo)
}

/// "process 412 MiB, address space 1830 MiB, device 640 of 1880 MiB free".
fn memory_text(status: &str, meminfo: &str) -> Option<String> {
    let mib = |kib: u64| kib / 1024;
    let mut text = format!("process {} MiB", mib(kib_field(status, "VmRSS:")?));
    if let Some(size) = kib_field(status, "VmSize:") {
        text += &format!(", address space {} MiB", mib(size));
    }
    if let (Some(free), Some(total)) = (
        kib_field(meminfo, "MemAvailable:"),
        kib_field(meminfo, "MemTotal:"),
    ) {
        text += &format!(", device {} of {} MiB free", mib(free), mib(total));
    }
    Some(text)
}

/// A `Name:   1234 kB` line's number.
fn kib_field(text: &str, name: &str) -> Option<u64> {
    text.lines()
        .find_map(|l| l.strip_prefix(name))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|n| n.parse().ok())
}

/// The thermal zones that read as the CPU, the GPU and the battery, by their type names
/// (Qualcomm's `cpu-1-0`, `cpuss-0`, `gpuss-0`, `battery`; others' `cpu_therm`, `gpu`...).
#[derive(Default)]
struct Zones {
    cpu: Vec<PathBuf>,
    gpu: Vec<PathBuf>,
    battery: Vec<PathBuf>,
}

impl Zones {
    fn find(root: &Path) -> Self {
        let mut zones = Zones::default();
        let Ok(entries) = fs::read_dir(root) else {
            return zones;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = fs::read_to_string(path.join("type")) else {
                continue;
            };
            // Only zones this process may read count.
            if fs::read_to_string(path.join("temp")).is_err() {
                continue;
            }
            let kind = kind.trim().to_ascii_lowercase();
            let temp = path.join("temp");
            if kind.starts_with("cpu") {
                zones.cpu.push(temp);
            } else if kind.starts_with("gpu") {
                zones.gpu.push(temp);
            } else if kind == "battery" || kind.starts_with("batt") {
                zones.battery.push(temp);
            }
        }
        zones
    }
    /// The hottest zone in °C (sysfs gives millidegrees; unplugged sensors read -273).
    fn hottest(&self, temps: &[PathBuf]) -> Option<f32> {
        temps
            .iter()
            .filter_map(|t| fs::read_to_string(t).ok()?.trim().parse::<i64>().ok())
            .filter(|&m| m > 0)
            .max()
            .map(|m| m as f32 / 1000.0)
    }
}

/// The cpufreq folder of the core with the highest maximum clock.
fn fastest_core(root: &Path) -> Option<PathBuf> {
    fs::read_dir(root)
        .ok()?
        .flatten()
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            name.strip_prefix("cpu")
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        })
        .map(|e| e.path().join("cpufreq"))
        .filter_map(|p| {
            let max = fs::read_to_string(p.join("cpuinfo_max_freq")).ok()?;
            Some((max.trim().parse::<u64>().ok()?, p))
        })
        .max_by_key(|(max, _)| *max)
        .map(|(_, p)| p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiers_follow_the_chip() {
        const GIB: u64 = 1 << 30;
        // As MemTotal reads them: a little under the box's number.
        assert_eq!(Tier::of(11 * GIB, 3187), Tier::High);
        assert_eq!(Tier::of(15 * GIB / 2, 3000), Tier::High);
        assert_eq!(Tier::of(11 * GIB, 2400), Tier::Mid);
        assert_eq!(Tier::of(11 * GIB / 2, 2850), Tier::Mid);
        assert_eq!(Tier::of(15 * GIB / 4, 3187), Tier::Low);
        assert_eq!(Tier::of(11 * GIB / 2, 2200), Tier::Low);
    }

    #[test]
    fn proc_fields_are_read_in_kib() {
        let status = "Name:\tsunrise\nVmHWM:\t    3840 kB\nVmRSS:\t  828000 kB\n";
        assert_eq!(kib_field(status, "VmRSS:"), Some(828_000));
        assert_eq!(
            kib_field("Rss: 5012 kB\nPss: 1092 kB\n", "Pss:"),
            Some(1092)
        );
        assert_eq!(kib_field(status, "Pss:"), None);
    }

    #[test]
    fn the_memory_note_says_the_process_its_address_space_and_the_device() {
        let status = "Name:\tsunrise\nVmSize:\t 1874000 kB\nVmRSS:\t  421888 kB\n";
        let meminfo =
            "MemTotal:        1925120 kB\nMemFree:  80000 kB\nMemAvailable:     655360 kB\n";
        assert_eq!(
            memory_text(status, meminfo).as_deref(),
            Some("process 412 MiB, address space 1830 MiB, device 640 of 1880 MiB free")
        );
        // What the system doesn't say is left out; with no RSS there is nothing to say.
        assert_eq!(
            memory_text("VmRSS:\t 2048 kB\n", "").as_deref(),
            Some("process 2 MiB")
        );
        assert_eq!(memory_text("Name:\tsunrise\n", meminfo), None);
    }

    #[test]
    fn zones_are_sorted_by_their_type_and_the_hottest_wins() {
        let root = std::env::temp_dir().join(format!("sunrise-zones-{}", std::process::id()));
        for (i, (kind, temp)) in [
            ("cpu-1-0", "54700"),
            ("cpuss-0", "56100"),
            ("gpuss-0", "49800"),
            ("battery", "36400"),
            ("pa", "44000"),
            ("cpu-9-9", "-273000"),
        ]
        .iter()
        .enumerate()
        {
            let zone = root.join(format!("thermal_zone{i}"));
            fs::create_dir_all(&zone).unwrap();
            fs::write(zone.join("type"), format!("{kind}\n")).unwrap();
            fs::write(zone.join("temp"), format!("{temp}\n")).unwrap();
        }
        let zones = Zones::find(&root);
        assert_eq!(zones.cpu.len(), 3);
        assert_eq!(zones.hottest(&zones.cpu), Some(56.1));
        assert_eq!(zones.hottest(&zones.gpu), Some(49.8));
        assert_eq!(zones.hottest(&zones.battery), Some(36.4));
        fs::remove_dir_all(root).unwrap();
    }
}
