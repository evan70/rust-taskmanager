//! Linux' own counters: the files under `/proc` that the kernel writes for everyone, read and
//! turned into numbers. The Android app (what it can see without the helper) and the desktop CLI
//! read the same files, so the reading and the parsing live here instead of in two places.
//!
//! Everything here is best effort by design: a phone that hides `/proc/meminfo` writes the app's
//! fallback path, so the readers return `None` (or an empty list) rather than an error.

use std::fs;

pub const MEMINFO: &str = "/proc/meminfo";
pub const PROC_STAT: &str = "/proc/stat";
pub const SELF_STAT: &str = "/proc/self/stat";
pub const LOADAVG: &str = "/proc/loadavg";

/// Linux counts CPU time in 1/100 s.
pub const CLOCK_TICKS: f64 = 100.0;

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
        if self.total == 0 {
            0.0
        } else {
            100.0 * self.used() as f32 / self.total as f32
        }
    }

    pub fn swap_used(&self) -> u64 {
        self.swap_total.saturating_sub(self.swap_free)
    }
}

/// `/proc/meminfo` (the values are in kB).
pub fn parse_meminfo(text: &str) -> Option<Mem> {
    let mut m = Mem::default();
    let mut seen = false;
    for line in text.lines() {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let kb: u64 = v
            .split_whitespace()
            .next()
            .and_then(|x| x.parse().ok())
            .unwrap_or(0);
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

pub fn meminfo() -> Option<Mem> {
    read(MEMINFO).and_then(|text| parse_meminfo(&text))
}

// ---------- processor ----------

#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct CpuTimes {
    pub busy: u64,
    pub total: u64,
}

/// `/proc/stat`: the first entry is all cores together, then one per core.
pub fn parse_proc_stat(text: &str) -> Vec<CpuTimes> {
    let mut out = Vec::new();
    for line in text.lines() {
        if !line.starts_with("cpu") {
            continue;
        }
        let nums: Vec<u64> = line
            .split_whitespace()
            .skip(1)
            .take(8)
            .filter_map(|x| x.parse().ok())
            .collect();
        if nums.len() < 4 {
            continue;
        }
        let total: u64 = nums.iter().sum();
        let idle = nums[3] + nums.get(4).copied().unwrap_or(0);
        out.push(CpuTimes {
            busy: total.saturating_sub(idle),
            total,
        });
    }
    out
}

/// Percent busy between two readings; `None` when no time passed.
pub fn cpu_percent(prev: CpuTimes, cur: CpuTimes) -> Option<f32> {
    let dt = cur.total.checked_sub(prev.total)?;
    if dt == 0 {
        return None;
    }
    let db = cur.busy.saturating_sub(prev.busy);
    Some((100.0 * db as f64 / dt as f64).clamp(0.0, 100.0) as f32)
}

/// The machine as a whole, then one entry per core.
pub fn cpu_times() -> Vec<CpuTimes> {
    read(PROC_STAT).map(|text| parse_proc_stat(&text)).unwrap_or_default()
}

// ---------- this process ----------

/// Clock ticks this process has used (user + system), from `/proc/self/stat`.
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

pub fn self_ticks() -> Option<u64> {
    read(SELF_STAT).and_then(|text| parse_self_ticks(&text))
}

// ---------- load average ----------

/// `/proc/loadavg`: the 1, 5 and 15 minute averages, the first three fields.
pub fn parse_load_average(text: &str) -> Option<[f32; 3]> {
    let mut fields = text.split_whitespace();
    let one = fields.next()?.parse().ok()?;
    let five = fields.next()?.parse().ok()?;
    let fifteen = fields.next()?.parse().ok()?;
    Some([one, five, fifteen])
}

pub fn load_average() -> Option<[f32; 3]> {
    read(LOADAVG).and_then(|text| parse_load_average(&text))
}

fn read(path: &str) -> Option<String> {
    fs::read_to_string(path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MEMINFO_TEXT: &str = "MemTotal:        5887476 kB\nMemFree:          145108 kB\nMemAvailable:    3048000 kB\nBuffers:            3396 kB\nCached:          3102068 kB\nSwapTotal:       5887472 kB\nSwapFree:        4985048 kB\n";

    #[test]
    fn meminfo_is_read_in_bytes() {
        let m = parse_meminfo(MEMINFO_TEXT).unwrap();
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
        assert!(
            (self_cpu_percent(0, 200, 2.0) - 100.0).abs() < 0.01,
            "200 ticks in 2 s is one full core"
        );
        assert_eq!(self_cpu_percent(5, 5, 2.0), 0.0);
        assert_eq!(self_cpu_percent(0, 10, 0.0), 0.0);
    }

    #[test]
    fn the_load_average_is_the_first_three_fields() {
        assert_eq!(
            parse_load_average("0.85 0.58 0.75 2/2345 197868\n"),
            Some([0.85, 0.58, 0.75])
        );
        assert_eq!(parse_load_average("nonsense"), None);
    }
}
