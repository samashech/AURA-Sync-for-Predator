use std::collections::VecDeque;
use std::fs;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::hardware::hwmon;

#[derive(Debug, Clone, Default)]
pub struct SensorData {
    pub cpu_temp: Option<f64>,
    pub gpu_temp: Option<f64>,
    pub system_temp: Option<f64>,
    pub cpu_fan_rpm: Option<u32>,
    pub gpu_fan_rpm: Option<u32>,
    pub cpu_freq_mhz: Option<u32>,
    pub cpu_model: String,
    pub gpu_info: GpuInfo,
    pub nvme0_temp: Option<f64>,
    pub nvme1_temp: Option<f64>,
    pub wifi_temp: Option<f64>,
    pub ram0_temp: Option<f64>,
    pub ram1_temp: Option<f64>,
}

#[derive(Debug, Clone, Default)]
pub struct GpuInfo {
    pub name: String,
    pub temp: Option<f64>,
    pub fan_speed_pct: Option<u32>,
    pub clock_mhz: Option<u32>,
    pub mem_clock_mhz: Option<u32>,
    pub utilization_pct: Option<u32>,
    pub power_watts: Option<f64>,
}

pub fn read_all_sensors() -> SensorData {
    let gpu_info = read_nvidia_gpu_info();
    SensorData {
        cpu_temp: read_cpu_temperature(),
        gpu_temp: gpu_info.temp,
        system_temp: read_system_temperature(),
        cpu_fan_rpm: read_fan("fan1_input"),
        gpu_fan_rpm: read_fan("fan2_input"),
        cpu_freq_mhz: read_cpu_frequency(),
        cpu_model: read_cpu_model(),
        gpu_info,
        nvme0_temp: find_hwmon_temp_by_name("nvme", "temp1_input"),
        nvme1_temp: find_second_hwmon_temp("nvme", "temp1_input"),
        wifi_temp: find_hwmon_temp_by_name("iwlwifi_1", "temp1_input"),
        ram0_temp: find_hwmon_temp_by_name("spd5118", "temp1_input"),
        ram1_temp: find_second_hwmon_temp("spd5118", "temp1_input"),
    }
}

fn read_cpu_model() -> String {
    if let Ok(c) = fs::read_to_string("/proc/cpuinfo") {
        for l in c.lines() {
            if l.starts_with("model name") {
                if let Some(n) = l.split(':').nth(1) { return n.trim().to_string(); }
            }
        }
    }
    "Unknown CPU".into()
}

fn read_cpu_frequency() -> Option<u32> {
    Some(fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq").ok()?.trim().parse::<u32>().ok()? / 1000)
}

fn read_nvidia_gpu_info() -> GpuInfo {
    // Reuse the comprehensive GPU cache instead of maintaining a second
    // nvidia-smi command, TTL and suspended-device policy in parallel.
    let metrics = crate::hardware::gpu::read_gpu_metrics();
    GpuInfo {
        name: metrics.name,
        temp: metrics.live.then_some(metrics.temp),
        fan_speed_pct: if metrics.live {
            metrics.fan_speed_pct
        } else {
            None
        },
        clock_mhz: metrics.live.then_some(metrics.clock_core_mhz),
        mem_clock_mhz: metrics.live.then_some(metrics.clock_mem_mhz),
        utilization_pct: metrics.live.then_some(metrics.util_gpu_pct),
        power_watts: metrics.live.then_some(metrics.power_draw_w),
    }
}

/// CPU temperature with momentary spikes removed - what every part of the app
/// should show and act on. See [`SpikeFilter`].
pub fn read_cpu_temperature() -> Option<f64> {
    static FILTER: Mutex<SpikeFilter> = Mutex::new(SpikeFilter::new());
    let raw = read_raw_cpu_temperature()?;
    let mut filter = FILTER.lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    // No history to outvote a spike with on the first read, or after a long
    // gap between reads: start from the typical core instead of the hottest.
    let reading = if filter.has_history(now) {
        raw
    } else {
        median(read_core_temperatures()).unwrap_or(raw)
    };
    Some(filter.feed(now, reading))
}

/// Every per-core reading `coretemp` publishes ("Core 0", "Core 8", ...).
fn read_core_temperatures() -> Vec<f64> {
    let Some(dirs) = hwmon::index().get("coretemp") else {
        return Vec::new();
    };
    let mut temps = Vec::new();
    for dir in dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for path in entries.flatten().map(|e| e.path()) {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let Some(stem) = name.strip_suffix("_label") else {
                continue;
            };
            let is_core = fs::read_to_string(&path).is_ok_and(|l| l.starts_with("Core"));
            if is_core {
                temps.extend(hwmon::read_temp_milli(&dir.join(format!("{stem}_input"))));
            }
        }
    }
    temps
}

/// The middle value - the lower middle on an even count, so that with just
/// two values, one of which may be a spike, the believable one wins.
fn median(mut values: Vec<f64>) -> Option<f64> {
    values.sort_by(f64::total_cmp);
    values.get(values.len().checked_sub(1)? / 2).copied()
}

fn read_raw_cpu_temperature() -> Option<f64> {
    find_hwmon_label("coretemp", "Package id 0")
        .or_else(|| find_hwmon_temp_by_name("coretemp", "temp1_input"))
        .or_else(|| find_hwmon_temp_by_name("k10temp", "temp1_input"))
}

/// How far back the median looks. Long enough that a lone spike is outvoted
/// at every read rate the app uses (Home every 1.5s, alerts every 5s), short
/// enough that sustained heat still shows within seconds.
const SPIKE_WINDOW: Duration = Duration::from_secs(8);
/// Reads closer together than this count once: `coretemp` caches its value
/// for about a second, so several callers reading in the same second would
/// otherwise record one spike several times and let it win the median.
const SPIKE_MIN_GAP: Duration = Duration::from_secs(1);

/// Median of the last few seconds of readings.
///
/// "Package id 0" is the hottest point on any core at that instant, so on a
/// high-boost chip a brief burst on one core reads as a huge jump. Measured on
/// a PHN16-72 (i7-14700HX) at idle: 62C to 100C from one read to the next and
/// back, against a 30s median of 63C, while the firmware's filtered TCPU zone
/// stayed at 63C. Shown raw, that is a random "99C" on an idle machine, and it
/// would trip the 90C critical alert. A median ignores those but, unlike an
/// average or a cap, is not dragged around by them, and real load (the same
/// machine holds ~97C on a single-core burst) still comes through in full.
struct SpikeFilter {
    samples: VecDeque<(Instant, f64)>,
}

impl SpikeFilter {
    const fn new() -> Self {
        Self { samples: VecDeque::new() }
    }

    /// Whether any reading is still recent enough to count.
    fn has_history(&self, now: Instant) -> bool {
        self.samples
            .back()
            .is_some_and(|(at, _)| now.duration_since(*at) <= SPIKE_WINDOW)
    }

    fn feed(&mut self, now: Instant, raw: f64) -> f64 {
        while self
            .samples
            .front()
            .is_some_and(|(at, _)| now.duration_since(*at) > SPIKE_WINDOW)
        {
            self.samples.pop_front();
        }
        let is_new = self
            .samples
            .back()
            .is_none_or(|(at, _)| now.duration_since(*at) >= SPIKE_MIN_GAP);
        if is_new {
            self.samples.push_back((now, raw));
        }
        median(self.samples.iter().map(|(_, t)| *t).collect()).unwrap_or(raw)
    }
}

/// Lightweight CPU/GPU temperature read for background alerts.
/// CPU is from hwmon (cheap); GPU reuses the cached nvidia-smi result.
pub fn read_critical_temps() -> (Option<f64>, Option<f64>) {
    let cpu = read_cpu_temperature();
    let gpu = read_nvidia_gpu_info().temp;
    (cpu, gpu)
}

fn read_system_temperature() -> Option<f64> {
    find_hwmon_temp_by_name("acpitz", "temp1_input").or_else(|| find_thermal("acpitz"))
}

fn find_hwmon_label(driver: &str, label: &str) -> Option<f64> {
    hwmon::label_temp(driver, label)
}

fn find_hwmon_temp_by_name(driver: &str, file: &str) -> Option<f64> {
    hwmon::first_temp(driver, file)
}

fn find_second_hwmon_temp(driver: &str, file: &str) -> Option<f64> {
    hwmon::nth_temp(driver, file, 1)
}

fn find_thermal(zone_type: &str) -> Option<f64> {
    for e in fs::read_dir("/sys/class/thermal").ok()?.flatten() {
        let p = e.path();
        if !p.file_name()?.to_str()?.starts_with("thermal_zone") { continue; }
        if let Ok(t) = fs::read_to_string(p.join("type")) {
            if t.trim() == zone_type { return hwmon::read_temp_milli(&p.join("temp")); }
        }
    }
    None
}

fn read_fan(file: &str) -> Option<u32> {
    let idx = hwmon::index();
    for name in &["acer", "facer"] {
        if let Some(paths) = idx.get(*name) {
            for p in paths {
                if let Ok(c) = fs::read_to_string(p.join(file)) {
                    if let Ok(v) = c.trim().parse::<u32>() {
                        if v > 0 { return Some(v); }
                    }
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feeds one reading per entry, `step` apart, returning what was reported.
    fn run(readings: &[f64], step: Duration) -> Vec<f64> {
        let mut filter = SpikeFilter::new();
        let start = Instant::now();
        readings
            .iter()
            .enumerate()
            .map(|(i, raw)| filter.feed(start + step * i as u32, *raw))
            .collect()
    }

    const HOME_RATE: Duration = Duration::from_millis(1500);

    #[test]
    fn a_lone_spike_never_shows() {
        let out = run(&[63.0, 62.0, 100.0, 63.0, 64.0, 62.0], HOME_RATE);
        assert!(out.iter().all(|t| *t < 70.0), "{out:?}");
    }

    #[test]
    fn a_spike_on_the_second_read_still_does_not_show() {
        assert_eq!(run(&[63.0, 100.0], HOME_RATE), [63.0, 63.0]);
    }

    #[test]
    fn sustained_heat_comes_through_within_seconds() {
        let out = run(&[63.0, 63.0, 63.0, 97.0, 97.0, 97.0, 97.0, 97.0], HOME_RATE);
        assert_eq!(*out.last().unwrap(), 97.0, "{out:?}");
        // The rise is seen no later than 5 reads (~7.5s) after it starts.
        assert_eq!(out[7], 97.0);
    }

    #[test]
    fn cooling_down_comes_through_too() {
        let out = run(&[97.0, 97.0, 97.0, 97.0, 60.0, 60.0, 60.0, 60.0, 60.0], HOME_RATE);
        assert_eq!(*out.last().unwrap(), 60.0, "{out:?}");
    }

    /// Several callers landing on one cached `coretemp` value must not vote
    /// more than once, or one spike could outvote the real readings.
    #[test]
    fn repeat_reads_of_one_spike_count_once() {
        let mut filter = SpikeFilter::new();
        let t0 = Instant::now();
        filter.feed(t0, 63.0);
        filter.feed(t0 + Duration::from_millis(1500), 63.0);
        for ms in [3000, 3100, 3200, 3300] {
            assert_eq!(filter.feed(t0 + Duration::from_millis(ms), 100.0), 63.0);
        }
    }

    #[test]
    fn history_lapses_with_the_window() {
        let mut filter = SpikeFilter::new();
        let t0 = Instant::now();
        assert!(!filter.has_history(t0));
        filter.feed(t0, 60.0);
        assert!(filter.has_history(t0 + Duration::from_secs(5)));
        assert!(!filter.has_history(t0 + Duration::from_secs(60)));
    }

    #[test]
    fn one_hot_core_does_not_move_the_core_median() {
        assert_eq!(median(vec![62.0, 100.0, 63.0, 61.0, 64.0]), Some(63.0));
        assert_eq!(median(vec![63.0, 100.0]), Some(63.0));
        assert_eq!(median(Vec::new()), None);
    }

    #[test]
    fn readings_older_than_the_window_stop_counting() {
        let mut filter = SpikeFilter::new();
        let t0 = Instant::now();
        filter.feed(t0, 40.0);
        assert_eq!(filter.feed(t0 + Duration::from_secs(60), 70.0), 70.0);
    }

    /// Manual check against the real sensor, not run by the normal suite:
    /// `cargo test --release -- --ignored --nocapture cpu_temperature_live`
    #[test]
    #[ignore]
    fn cpu_temperature_live() {
        let (mut raw_max, mut shown_max) = (0.0f64, 0.0f64);
        for _ in 0..40 {
            let raw = read_raw_cpu_temperature().expect("no CPU sensor");
            let shown = read_cpu_temperature().expect("no CPU sensor");
            println!("raw {raw:5.1}  shown {shown:5.1}");
            raw_max = raw_max.max(raw);
            shown_max = shown_max.max(shown);
            std::thread::sleep(HOME_RATE);
        }
        println!("max raw {raw_max:.1}  max shown {shown_max:.1}");
    }
}
