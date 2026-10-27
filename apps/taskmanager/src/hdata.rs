//! What the helper sends, plus the small pieces of logic around it (history, filters, settings).

use domain::{BatteryInfo, CpuInfo, MemoryInfo, Process, ThermalZone, celsius_of};
use serde::Deserialize;
use std::collections::VecDeque;

#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct Snapshot {
    pub warming_up: bool,
    pub uptime: f64,
    pub cpu: CpuInfo,
    pub mem: MemoryInfo,
    /// The sensors the helper could read, by the names it gives them (`cpu`, `gpu`, `battery`, `skin`).
    pub thermals: Vec<ThermalZone>,
    pub battery: Option<BatteryInfo>,
    pub nprocs: u32,
    pub procs: Vec<Process>,
}

#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct BootInfo {
    pub how: String,
    pub comp: String,
    pub after: i64,
}

#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct App {
    pub p: String,
    pub mb: u32,
    pub running: bool,
    pub stopped: bool,
    pub frozen: bool,
    pub recv: bool,
    pub boot: Option<BootInfo>,
    pub prot: String,
}

#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct StartupData {
    pub boot_log: bool,
    pub apps: Vec<App>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct ActResult {
    pub p: String,
    pub ok: bool,
    pub msg: String,
}

#[derive(Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct ActResults {
    pub results: Vec<ActResult>,
}

// ---------- history for the charts ----------

pub const HISTORY_LEN: usize = 90;

#[derive(Default, Clone)]
pub struct History {
    pub cpu: VecDeque<f32>,
    pub ram: VecDeque<f32>,
    pub temp: VecDeque<f32>,
    pub cores: Vec<VecDeque<f32>>,
}

fn push(q: &mut VecDeque<f32>, v: f32) {
    if q.len() >= HISTORY_LEN {
        q.pop_front();
    }
    q.push_back(v);
}

impl History {
    pub fn add(&mut self, s: &Snapshot) {
        push(&mut self.cpu, s.cpu.total_percent);
        push(&mut self.ram, s.mem.used_percent());
        if let Some(t) = celsius_of(&s.thermals, "cpu").or_else(|| celsius_of(&s.thermals, "battery")) {
            push(&mut self.temp, t);
        }
        if self.cores.len() != s.cpu.cores.len() {
            self.cores = vec![VecDeque::new(); s.cpu.cores.len()];
        }
        for (q, c) in self.cores.iter_mut().zip(&s.cpu.cores) {
            push(q, c.percent);
        }
    }
}

// ---------- process list ----------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProcSort {
    Cpu,
    Ram,
    Name,
}

/// Processes matching `query`, sorted. The helper already sends them busiest-first.
pub fn sorted_procs<'a>(procs: &'a [Process], query: &str, sort: ProcSort) -> Vec<&'a Process> {
    let q = query.trim().to_lowercase();
    let mut v: Vec<&Process> = procs.iter().filter(|p| q.is_empty() || p.name.to_lowercase().contains(&q)).collect();
    match sort {
        ProcSort::Cpu => v.sort_by(|a, b| b.cpu_percent.partial_cmp(&a.cpu_percent).unwrap_or(std::cmp::Ordering::Equal).then(b.memory_mb.cmp(&a.memory_mb))),
        ProcSort::Ram => v.sort_by_key(|p| std::cmp::Reverse(p.memory_mb)),
        ProcSort::Name => v.sort_by_key(|p| p.name.to_lowercase()),
    }
    v
}

/// "com.instagram.android:remote" -> "instagram.android"; "/system/bin/surfaceflinger" -> "surfaceflinger".
pub fn display_name(name: &str) -> String {
    let base = name.split(':').next().unwrap_or(name);
    if let Some(s) = base.strip_prefix('/') {
        return s.rsplit('/').next().unwrap_or(s).to_string();
    }
    base.strip_prefix("com.").unwrap_or(base).to_string()
}

/// The package a process belongs to, if its name looks like one (so a Stop button makes sense).
pub fn package_of(name: &str) -> Option<&str> {
    let base = name.split(':').next()?;
    (base.contains('.') && !base.starts_with('/') && base.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')).then_some(base)
}

// ---------- startup list filters ----------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StartupFilter {
    Woke,
    CanWake,
    Running,
    Off,
    All,
}

pub fn startup_matches(a: &App, f: StartupFilter, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    if !q.is_empty() && !a.p.to_lowercase().contains(&q) {
        return false;
    }
    match f {
        StartupFilter::Woke => a.boot.is_some(),
        StartupFilter::CanWake => a.recv,
        StartupFilter::Running => a.running,
        StartupFilter::Off => a.stopped || a.frozen,
        StartupFilter::All => true,
    }
}

/// Apps that woke at boot first, then by memory.
pub fn startup_sorted<'a>(apps: &'a [App], f: StartupFilter, query: &str) -> Vec<&'a App> {
    let mut v: Vec<&App> = apps.iter().filter(|a| startup_matches(a, f, query)).collect();
    v.sort_by(|a, b| b.boot.is_some().cmp(&a.boot.is_some()).then(b.mb.cmp(&a.mb)).then(a.p.cmp(&b.p)));
    v
}

// ---------- settings of the status-bar numbers (not part of this version yet) ----------

#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusSettings {
    pub enabled: bool,
    pub cpu: bool,
    pub ram: bool,
    pub temp: bool,
    pub temp_is_battery: bool,
    pub at_boot: bool,
}

#[allow(dead_code)]
impl Default for StatusSettings {
    fn default() -> Self {
        Self { enabled: false, cpu: true, ram: true, temp: true, temp_is_battery: false, at_boot: false }
    }
}

#[allow(dead_code)]
impl StatusSettings {
    pub fn parse(text: &str) -> Self {
        let mut s = Self::default();
        for l in text.lines() {
            let Some((k, v)) = l.trim().split_once('=') else { continue };
            let on = v.trim() == "1";
            match k.trim() {
                "enabled" => s.enabled = on,
                "cpu" => s.cpu = on,
                "ram" => s.ram = on,
                "temp" => s.temp = on,
                "temp_source" => s.temp_is_battery = v.trim() == "battery",
                "boot" => s.at_boot = on,
                _ => {}
            }
        }
        s
    }

    pub fn to_text(&self) -> String {
        let b = |x: bool| if x { "1" } else { "0" };
        format!(
            "enabled={}\ncpu={}\nram={}\ntemp={}\ntemp_source={}\nboot={}\n",
            b(self.enabled),
            b(self.cpu),
            b(self.ram),
            b(self.temp),
            if self.temp_is_battery { "battery" } else { "cpu" },
            b(self.at_boot)
        )
    }
}

/// 0 = cool, 1 = warm, 2 = hot, 3 = too hot (for colouring temperatures).
pub fn temp_level(c: f32) -> u8 {
    match c {
        c if c < 45.0 => 0,
        c if c < 60.0 => 1,
        c if c < 75.0 => 2,
        _ => 3,
    }
}

pub fn gb(mb: u64) -> String {
    format!("{:.1} GB", mb as f64 / 1024.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SNAP: &str = r#"{"t":1,"uptime":1206,"cpu":{"total_percent":26.0,"cores":[{"percent":65.0,"mhz":2016},{"percent":3.0,"mhz":633}],"load_avg":[4.75,5.17,4.04]},
        "mem":{"total":11272,"avail":6114,"cached":4962,"swap_total":12287,"swap_free":11330},
        "thermals":[{"name":"cpu","celsius":46.9},{"name":"gpu","celsius":52.9},{"name":"battery","celsius":37.7}],"battery":{"percent":100,"state":"charging","temperature_c":37.8},
        "nprocs":278,"procs":[{"pid":1,"name":"com.miui.gallery","cpu_percent":85.6,"memory_mb":640,"threads":30},{"pid":2,"name":"/system/bin/surfaceflinger","cpu_percent":3.0,"memory_mb":64,"threads":12}]}"#;

    #[test]
    fn a_real_snapshot_is_parsed() {
        let s: Snapshot = serde_json::from_str(SNAP).unwrap();
        assert_eq!(s.cpu.cores, vec![domain::Core { percent: 65.0, mhz: 2016 }, domain::Core { percent: 3.0, mhz: 633 }]);
        assert_eq!(s.mem.used_mb(), 5158);
        assert!((s.mem.used_percent() - 45.8).abs() < 0.1);
        assert_eq!(s.mem.swap_used_mb(), 957);
        assert_eq!((celsius_of(&s.thermals, "cpu"), celsius_of(&s.thermals, "skin")), (Some(46.9), None));
        assert_eq!(s.battery.as_ref().unwrap().state, domain::BatteryState::Charging);
        assert_eq!(s.battery.as_ref().unwrap().temperature_c, Some(37.8));
        assert_eq!((s.cpu.total_percent, s.cpu.load_avg[0]), (26.0, 4.75));
        assert_eq!(s.procs.len(), 2);
    }

    #[test]
    fn warming_up_and_unknown_fields_do_not_break_parsing() {
        let s: Snapshot = serde_json::from_str(r#"{"warming_up":true}"#).unwrap();
        assert!(s.warming_up && s.procs.is_empty());
        let s: Snapshot = serde_json::from_str(r#"{"cpu":{"total_percent":5.0},"new_field":123}"#).unwrap();
        assert_eq!(s.cpu.total_percent, 5.0);
    }

    #[test]
    fn history_keeps_only_the_latest_samples() {
        let s: Snapshot = serde_json::from_str(SNAP).unwrap();
        let mut h = History::default();
        for _ in 0..(HISTORY_LEN + 20) {
            h.add(&s);
        }
        assert_eq!(h.cpu.len(), HISTORY_LEN);
        assert_eq!(h.cores.len(), 2);
        assert_eq!(h.cores[0].len(), HISTORY_LEN);
        assert_eq!(h.temp.back(), Some(&46.9));
    }

    #[test]
    fn process_list_filters_and_sorts() {
        let s: Snapshot = serde_json::from_str(SNAP).unwrap();
        assert_eq!(sorted_procs(&s.procs, "", ProcSort::Cpu)[0].name, "com.miui.gallery");
        assert_eq!(sorted_procs(&s.procs, "SURFACE", ProcSort::Name).len(), 1);
        assert_eq!(sorted_procs(&s.procs, "", ProcSort::Ram)[0].memory_mb, 640);
        assert!(sorted_procs(&s.procs, "nothing", ProcSort::Cpu).is_empty());
    }

    #[test]
    fn names_are_shortened_and_packages_recognised() {
        assert_eq!(display_name("com.instagram.android:remote"), "instagram.android");
        assert_eq!(display_name("/system/bin/surfaceflinger"), "surfaceflinger");
        assert_eq!(display_name("system_server"), "system_server");
        assert_eq!(package_of("com.instagram.android:remote"), Some("com.instagram.android"));
        assert_eq!(package_of("/system/bin/logd"), None);
        assert_eq!(package_of("zygote64"), None);
        assert_eq!(package_of("bad name.x;y"), None);
    }

    #[test]
    fn startup_filters_and_order() {
        let app = |p: &str, mb: u32, boot: bool, stopped: bool| App { p: p.into(), mb, stopped, boot: boot.then(|| BootInfo { how: "broadcast".into(), comp: "R".into(), after: 3 }), ..Default::default() };
        let apps = vec![app("a.small", 10, true, false), app("b.big", 300, true, false), app("c.idle", 500, false, true)];
        let woke = startup_sorted(&apps, StartupFilter::Woke, "");
        assert_eq!(woke.iter().map(|a| a.p.as_str()).collect::<Vec<_>>(), ["b.big", "a.small"]);
        assert_eq!(startup_sorted(&apps, StartupFilter::Off, "").len(), 1);
        assert_eq!(startup_sorted(&apps, StartupFilter::All, "big").len(), 1);
        let all = startup_sorted(&apps, StartupFilter::All, "");
        assert_eq!(all[0].p, "b.big", "boot wakers first, biggest first");
        assert_eq!(all[2].p, "c.idle");
    }

    #[test]
    fn status_settings_round_trip() {
        let s = StatusSettings { enabled: true, cpu: false, ram: true, temp: true, temp_is_battery: true, at_boot: true };
        assert_eq!(StatusSettings::parse(&s.to_text()), s);
        assert_eq!(StatusSettings::parse(""), StatusSettings::default());
        assert!(!StatusSettings::default().enabled && !StatusSettings::default().at_boot, "off by default");
        // the Java service reads these exact keys
        for key in ["enabled=", "cpu=", "ram=", "temp=", "temp_source=", "boot="] {
            assert!(s.to_text().contains(key), "{key}");
        }
    }

    #[test]
    fn temperature_levels() {
        assert_eq!([30.0, 50.0, 70.0, 80.0].map(temp_level), [0, 1, 2, 3]);
        assert_eq!(gb(2048), "2.0 GB");
    }
}
