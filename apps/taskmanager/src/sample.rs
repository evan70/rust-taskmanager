//! The numbers behind the screens: reading what Android lets an ordinary app see, turning raw
//! counters into percentages and speeds, and the app lists. Nothing here touches the screen or the
//! phone, so it is all unit tested.

use std::collections::{HashMap, VecDeque};

use domain::{DiskInfo, MemoryInfo, NetworkInfo};

pub const HISTORY_LEN: usize = 90;

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

// ---------- processor ----------

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
    pub mem: MemoryInfo,
    pub cpu_total: Option<f32>,
    /// Per core: (percent busy if known, current MHz if known, max MHz if known).
    pub cores: Vec<(Option<f32>, Option<u32>, Option<u32>)>,
    pub self_cpu: f32,
    pub battery: Option<Battery>,
    pub thermal: i64,
    pub headroom: Option<f64>,
    /// The one volume Android reports to the app: a size and free space, with no name.
    pub storage: DiskInfo,
    /// The phone as a whole: no interface name, just the traffic since boot.
    pub network: NetworkInfo,
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
        push(&mut self.mem, s.mem.used_percent());
        if let Some(c) = s.cpu_total {
            push(&mut self.cpu, c);
        }
        if let Some(t) = s.battery.as_ref().and_then(|b| b.temp_c) {
            push(&mut self.temp, t);
        }
        push(&mut self.down, s.network.received_bytes_per_second as f32);
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

/// Megabytes as bytes, for the memory lines that print with `fmt_bytes`.
pub fn mb_to_bytes(mb: u64) -> u64 {
    mb * 1024 * 1024
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
        let s = Sample { mem: MemoryInfo { total: 100, avail: 50, ..Default::default() }, cpu_total: Some(10.0), ..Default::default() };
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
