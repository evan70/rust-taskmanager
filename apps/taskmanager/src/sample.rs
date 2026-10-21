//! The numbers behind the screens: reading what Android lets an ordinary app see, turning raw
//! counters into percentages and speeds, and the app lists. Nothing here touches the screen or the
//! phone, so it is all unit tested.

use std::collections::{HashMap, VecDeque};

pub const HISTORY_LEN: usize = 90;
/// Linux counts CPU time in 1/100 s on Android.
const CLOCK_TICKS: f64 = 100.0;

// ---------- key=value text from the Java side ----------

/// "key<TAB>value" lines.
pub fn parse_kv(text: &str) -> HashMap<String, String> {
    text.lines().filter_map(|l| l.split_once('\t')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).collect()
}

pub fn get_i64(m: &HashMap<String, String>, k: &str) -> Option<i64> {
    m.get(k)?.parse().ok()
}

pub fn get_f64(m: &HashMap<String, String>, k: &str) -> Option<f64> {
    m.get(k)?.parse::<f64>().ok().filter(|v| v.is_finite())
}

// ---------- memory ----------

/// Bytes.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct Mem {
    pub total: u64,
    pub avail: u64,
    pub cached: u64,
    pub swap_total: u64,
    pub swap_free: u64,
}

impl Mem {
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.avail)
    }
    pub fn used_pct(&self) -> f32 {
        if self.total == 0 { 0.0 } else { 100.0 * self.used() as f32 / self.total as f32 }
    }
    pub fn swap_used(&self) -> u64 {
        self.swap_total.saturating_sub(self.swap_free)
    }
}

/// /proc/meminfo (values are in kB).
pub fn parse_meminfo(text: &str) -> Option<Mem> {
    let mut m = Mem::default();
    let mut seen = false;
    for line in text.lines() {
        let Some((k, v)) = line.split_once(':') else { continue };
        let kb: u64 = v.split_whitespace().next().and_then(|x| x.parse().ok()).unwrap_or(0);
        let bytes = kb * 1024;
        match k {
            "MemTotal" => {
                m.total = bytes;
                seen = true;
            }
            "MemAvailable" => m.avail = bytes,
            "Cached" => m.cached = bytes,
            "SwapTotal" => m.swap_total = bytes,
            "SwapFree" => m.swap_free = bytes,
            _ => {}
        }
    }
    (seen && m.total > 0).then_some(m)
}

// ---------- processor ----------

#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct CpuTimes {
    pub busy: u64,
    pub total: u64,
}

/// /proc/stat: the first entry is all cores together, then one per core.
pub fn parse_proc_stat(text: &str) -> Vec<CpuTimes> {
    let mut out = Vec::new();
    for line in text.lines() {
        if !line.starts_with("cpu") {
            continue;
        }
        let nums: Vec<u64> = line.split_whitespace().skip(1).take(8).filter_map(|x| x.parse().ok()).collect();
        if nums.len() < 4 {
            continue;
        }
        let total: u64 = nums.iter().sum();
        let idle = nums[3] + nums.get(4).copied().unwrap_or(0);
        out.push(CpuTimes { busy: total.saturating_sub(idle), total });
    }
    out
}

/// Percent busy between two readings; None when no time passed.
pub fn cpu_percent(prev: CpuTimes, cur: CpuTimes) -> Option<f32> {
    let dt = cur.total.checked_sub(prev.total)?;
    if dt == 0 {
        return None;
    }
    let db = cur.busy.saturating_sub(prev.busy);
    Some((100.0 * db as f64 / dt as f64).clamp(0.0, 100.0) as f32)
}

/// Clock ticks this app's own process has used (user + system), from /proc/self/stat.
pub fn parse_self_ticks(text: &str) -> Option<u64> {
    // the name in brackets may contain spaces, so count fields after the last ')'
    let rest = &text[text.rfind(')')? + 1..];
    let f: Vec<&str> = rest.split_whitespace().collect();
    let utime: u64 = f.get(11)?.parse().ok()?;
    let stime: u64 = f.get(12)?.parse().ok()?;
    Some(utime + stime)
}

/// Share of one core used between two readings `dt` seconds apart.
pub fn self_cpu_percent(prev_ticks: u64, cur_ticks: u64, dt: f64) -> f32 {
    if dt <= 0.0 {
        return 0.0;
    }
    (100.0 * cur_ticks.saturating_sub(prev_ticks) as f64 / CLOCK_TICKS / dt) as f32
}

/// "Moderate" and so on, from PowerManager's thermal status (0..=6).
pub fn thermal_label(status: i64) -> &'static str {
    match status {
        0 => "Normal",
        1 => "Light",
        2 => "Moderate",
        3 => "Severe",
        4 => "Critical",
        5 => "Emergency",
        6 => "Shutdown",
        _ => "Unknown",
    }
}

// ---------- battery ----------

#[derive(Clone, Default, Debug, PartialEq)]
pub struct Battery {
    pub level: i32,
    pub charging: bool,
    pub full: bool,
    pub plugged: &'static str,
    pub health: &'static str,
    pub temp_c: Option<f32>,
    pub volt_v: Option<f32>,
    /// Milliamps, always positive; whether it is going in or out comes from `charging`.
    pub current_ma: Option<f32>,
    pub cycles: Option<i32>,
    pub full_in_ms: Option<i64>,
}

impl Battery {
    pub fn from_kv(m: &HashMap<String, String>) -> Option<Battery> {
        let level = get_i64(m, "bat_level")?;
        let scale = get_i64(m, "bat_scale").filter(|s| *s > 0).unwrap_or(100);
        let status = get_i64(m, "bat_status").unwrap_or(1);
        let plugged = match get_i64(m, "bat_plugged").unwrap_or(0) {
            0 => "",
            1 => "AC",
            2 => "USB",
            4 => "Wireless",
            _ => "Dock",
        };
        let health = match get_i64(m, "bat_health").unwrap_or(1) {
            2 => "Good",
            3 => "Overheating",
            4 => "Dead",
            5 => "Over voltage",
            6 => "Failure",
            7 => "Cold",
            _ => "Unknown",
        };
        Some(Battery {
            level: (level * 100 / scale) as i32,
            charging: status == 2,
            full: status == 5,
            plugged,
            health,
            temp_c: get_i64(m, "bat_temp").map(|t| t as f32 / 10.0),
            volt_v: get_i64(m, "bat_voltage").filter(|v| *v > 0).map(|v| v as f32 / 1000.0),
            // some phones report microamps, a few report milliamps; anything beyond 20 A is microamps
            current_ma: get_i64(m, "bat_current_ua").filter(|c| *c != 0 && *c != i64::MIN).map(|c| {
                let v = c.unsigned_abs() as f32;
                if v > 20_000.0 { v / 1000.0 } else { v }
            }),
            cycles: get_i64(m, "bat_cycles").filter(|c| *c >= 0).map(|c| c as i32),
            full_in_ms: get_i64(m, "bat_full_in_ms").filter(|t| *t > 0),
        })
    }

    /// Power in watts, when voltage and current are both known.
    pub fn watts(&self) -> Option<f32> {
        Some(self.volt_v? * self.current_ma? / 1000.0)
    }
}

// ---------- the sample ----------

#[derive(Clone, Default, Debug)]
pub struct Sample {
    pub mem: Mem,
    pub cpu_total: Option<f32>,
    /// Per core: (percent busy if known, current MHz if known, max MHz if known).
    pub cores: Vec<(Option<f32>, Option<u32>, Option<u32>)>,
    pub self_cpu: f32,
    pub battery: Option<Battery>,
    pub thermal: i64,
    pub headroom: Option<f64>,
    pub storage_total: u64,
    pub storage_free: u64,
    pub net_down: f64,
    pub net_up: f64,
    pub net_rx_total: u64,
    pub net_tx_total: u64,
    pub uptime_ms: i64,
    pub power_save: bool,
    pub low_memory: bool,
}

pub fn rate(prev: u64, cur: u64, dt: f64) -> f64 {
    if dt <= 0.0 || cur < prev { 0.0 } else { (cur - prev) as f64 / dt }
}

// ---------- history for the charts ----------

#[derive(Default, Clone)]
pub struct History {
    pub mem: VecDeque<f32>,
    pub cpu: VecDeque<f32>,
    pub temp: VecDeque<f32>,
    pub down: VecDeque<f32>,
}

fn push(q: &mut VecDeque<f32>, v: f32) {
    if q.len() >= HISTORY_LEN {
        q.pop_front();
    }
    q.push_back(v);
}

impl History {
    pub fn add(&mut self, s: &Sample) {
        push(&mut self.mem, s.mem.used_pct());
        if let Some(c) = s.cpu_total {
            push(&mut self.cpu, c);
        }
        if let Some(t) = s.battery.as_ref().and_then(|b| b.temp_c) {
            push(&mut self.temp, t);
        }
        push(&mut self.down, s.net_down as f32);
    }
}

// ---------- formatting ----------

pub fn fmt_bytes(b: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{b} B") } else if v >= 100.0 { format!("{v:.0} {}", U[i]) } else { format!("{v:.1} {}", U[i]) }
}

pub fn fmt_speed(bytes_per_s: f64) -> String {
    format!("{}/s", fmt_bytes(bytes_per_s.max(0.0) as u64))
}

/// "2d 3h", "5h 12m", "3m", "under a minute"
pub fn fmt_duration(ms: i64) -> String {
    let s = (ms.max(0) / 1000) as u64;
    let (d, h, m) = (s / 86_400, s % 86_400 / 3600, s % 3600 / 60);
    if d > 0 {
        format!("{d}d {h}h")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else if m > 0 {
        format!("{m}m")
    } else {
        "under a minute".to_string()
    }
}

pub fn fmt_temp(c: f32, fahrenheit: bool) -> String {
    if fahrenheit { format!("{:.0}\u{b0}F", c * 9.0 / 5.0 + 32.0) } else { format!("{c:.1}\u{b0}C") }
}

// ---------- the lists of apps ----------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AppFilter {
    User,
    System,
    All,
}

/// An app that can start itself when the phone boots.
#[derive(Clone, Debug, PartialEq)]
pub struct BootRow {
    pub pkg: String,
    pub label: String,
    pub system: bool,
}

/// "package<TAB>label<TAB>flags"
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub fn parse_boot_line(line: &str) -> Option<BootRow> {
    let mut it = line.split('\t');
    let (pkg, label, flags) = (it.next()?, it.next()?, it.next()?);
    (!pkg.is_empty()).then(|| BootRow { pkg: pkg.to_string(), label: label.trim().to_string(), system: flags.parse::<u32>().ok().unwrap_or(0) & 1 != 0 })
}

pub fn visible_boot<'a>(rows: &'a [BootRow], filter: AppFilter, query: &str) -> Vec<&'a BootRow> {
    let q = query.trim().to_lowercase();
    let mut v: Vec<&BootRow> = rows
        .iter()
        .filter(|r| match filter {
            AppFilter::User => !r.system,
            AppFilter::System => r.system,
            AppFilter::All => true,
        })
        .filter(|r| q.is_empty() || r.label.to_lowercase().contains(&q) || r.pkg.to_lowercase().contains(&q))
        .collect();
    v.sort_by(|a, b| a.label.to_lowercase().cmp(&b.label.to_lowercase()).then_with(|| a.pkg.cmp(&b.pkg)));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    const MEMINFO: &str = "MemTotal:        5887476 kB\nMemFree:          145108 kB\nMemAvailable:    3048000 kB\nBuffers:            3396 kB\nCached:          3102068 kB\nSwapTotal:       5887472 kB\nSwapFree:        4985048 kB\n";

    #[test]
    fn meminfo_is_read_in_bytes() {
        let m = parse_meminfo(MEMINFO).unwrap();
        assert_eq!(m.total, 5_887_476 * 1024);
        assert_eq!(m.avail, 3_048_000 * 1024);
        assert_eq!(m.swap_used(), (5_887_472 - 4_985_048) * 1024);
        assert!((m.used_pct() - 48.2).abs() < 0.1, "{}", m.used_pct());
        assert!(parse_meminfo("nothing useful").is_none());
        assert!(parse_meminfo("MemTotal: 0 kB").is_none());
    }

    #[test]
    fn proc_stat_gives_total_and_cores() {
        let t = "cpu  100 0 100 700 100 0 0 0 0 0\ncpu0 50 0 50 350 50 0 0 0 0 0\ncpu1 50 0 50 350 50 0 0 0 0 0\nintr 12345\n";
        let v = parse_proc_stat(t);
        assert_eq!(v.len(), 3);
        assert_eq!(v[0], CpuTimes { busy: 200, total: 1000 });
        assert_eq!(v[1].total, 500);
        assert!(parse_proc_stat("garbage\n").is_empty());
    }

    #[test]
    fn cpu_percent_comes_from_the_change_between_readings() {
        let a = CpuTimes { busy: 200, total: 1000 };
        let b = CpuTimes { busy: 300, total: 1200 };
        assert_eq!(cpu_percent(a, b), Some(50.0));
        assert_eq!(cpu_percent(b, b), None, "no time passed");
        assert_eq!(cpu_percent(b, a), None, "counters went backwards");
    }

    #[test]
    fn own_cpu_use_is_read_from_proc_self_stat() {
        // field 14 and 15 (utime, stime) after the bracketed name, which may hold spaces
        let line = "1234 (my app (x)) S 1 1 0 0 -1 4194560 100 0 0 0 150 50 0 0 20 0 10 0 1000 0 0";
        assert_eq!(parse_self_ticks(line), Some(200));
        assert_eq!(parse_self_ticks("nonsense"), None);
        assert!((self_cpu_percent(0, 200, 2.0) - 100.0).abs() < 0.01, "200 ticks in 2 s is one full core");
        assert_eq!(self_cpu_percent(5, 5, 2.0), 0.0);
        assert_eq!(self_cpu_percent(0, 10, 0.0), 0.0);
    }

    #[test]
    fn key_value_text_is_read() {
        let m = parse_kv("a\t1\nb\t2.5\nbad line\nc\tx\nd\tNaN\n");
        assert_eq!(get_i64(&m, "a"), Some(1));
        assert_eq!(get_f64(&m, "b"), Some(2.5));
        assert_eq!(get_i64(&m, "c"), None);
        assert_eq!(get_f64(&m, "d"), None, "NaN is not a reading");
        assert_eq!(get_i64(&m, "zzz"), None);
    }

    #[test]
    fn battery_values_are_made_sensible() {
        let m = parse_kv("bat_level\t57\nbat_scale\t100\nbat_status\t2\nbat_plugged\t1\nbat_health\t2\nbat_temp\t312\nbat_voltage\t4012\nbat_current_ua\t-1500000\nbat_cycles\t88\nbat_full_in_ms\t3600000\n");
        let b = Battery::from_kv(&m).unwrap();
        assert_eq!((b.level, b.charging, b.plugged, b.health), (57, true, "AC", "Good"));
        assert_eq!(b.temp_c, Some(31.2));
        assert_eq!(b.volt_v, Some(4.012));
        assert_eq!(b.current_ma, Some(1500.0), "microamps become milliamps, sign dropped");
        assert_eq!(b.cycles, Some(88));
        assert!((b.watts().unwrap() - 6.018).abs() < 0.01);
        // a phone that already reports milliamps
        let m = parse_kv("bat_level\t5\nbat_current_ua\t450\n");
        assert_eq!(Battery::from_kv(&m).unwrap().current_ma, Some(450.0));
        // a battery with a different scale, and missing extras
        let m = parse_kv("bat_level\t50\nbat_scale\t200\n");
        let b = Battery::from_kv(&m).unwrap();
        assert_eq!(b.level, 25);
        assert_eq!((b.temp_c, b.current_ma, b.cycles, b.watts()), (None, None, None, None));
        assert!(Battery::from_kv(&parse_kv("other\t1")).is_none());
    }

    #[test]
    fn speeds_never_go_negative() {
        assert_eq!(rate(1000, 3000, 2.0), 1000.0);
        assert_eq!(rate(3000, 1000, 2.0), 0.0, "counter reset");
        assert_eq!(rate(1, 2, 0.0), 0.0);
    }

    #[test]
    fn history_keeps_only_the_latest_points() {
        let mut h = History::default();
        let s = Sample { mem: Mem { total: 100, avail: 50, ..Default::default() }, cpu_total: Some(10.0), ..Default::default() };
        for _ in 0..HISTORY_LEN + 20 {
            h.add(&s);
        }
        assert_eq!(h.mem.len(), HISTORY_LEN);
        assert_eq!(h.cpu.len(), HISTORY_LEN);
        assert!(h.temp.is_empty(), "no battery, no temperature points");
        assert_eq!(h.mem.back(), Some(&50.0));
    }

    #[test]
    fn numbers_are_written_for_people() {
        assert_eq!(fmt_bytes(512), "512 B");
        assert_eq!(fmt_bytes(1536), "1.5 KB");
        assert_eq!(fmt_bytes(5 * 1024 * 1024 * 1024), "5.0 GB");
        assert_eq!(fmt_bytes(250 * 1024 * 1024), "250 MB");
        assert_eq!(fmt_speed(2048.0), "2.0 KB/s");
        assert_eq!(fmt_duration(30_000), "under a minute");
        assert_eq!(fmt_duration(180_000), "3m");
        assert_eq!(fmt_duration(3_900_000), "1h 5m");
        assert_eq!(fmt_duration(200_000_000), "2d 7h");
        assert_eq!(fmt_temp(30.0, false), "30.0\u{b0}C");
        assert_eq!(fmt_temp(30.0, true), "86\u{b0}F");
        assert_eq!(thermal_label(0), "Normal");
        assert_eq!(thermal_label(3), "Severe");
        assert_eq!(thermal_label(-1), "Unknown");
    }

    #[test]
    fn boot_apps_are_listed_by_name() {
        let rows: Vec<BootRow> = ["com.z\tZed\t0", "com.a\tapple\t1", "oops"].iter().filter_map(|l| parse_boot_line(l)).collect();
        assert_eq!(rows.len(), 2);
        let all: Vec<_> = visible_boot(&rows, AppFilter::All, "").iter().map(|r| r.label.as_str()).collect();
        assert_eq!(all, ["apple", "Zed"]);
        assert_eq!(visible_boot(&rows, AppFilter::User, "").len(), 1);
        assert_eq!(visible_boot(&rows, AppFilter::All, "zed").len(), 1);
    }
}
