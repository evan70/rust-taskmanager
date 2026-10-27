//! Local API for the on-phone Task Manager app.
//!
//! An ordinary Android app cannot see other apps' CPU, memory or boot behaviour; this helper runs
//! with shell privileges and can. It listens on 127.0.0.1 only and every request must carry the
//! secret token the Task Manager app reads from its own (private) folder.
//!
//!   GET  /v1/ping
//!   GET  /v1/snapshot      CPU per core, memory, temperatures, battery, busiest processes (1 Hz)
//!   GET  /v1/startup       your apps: what woke at boot, memory, stopped/frozen state
//!   POST /v1/act?op=stop|freeze|unfreeze   (body: one package per line)
//!
//! Sampling only runs while a client has asked for a snapshot in the last few seconds.

use super::{page_kb, pct, read_cpu, CpuTimes, CLK_TCK};
use phone_shared::{parse_boot_receivers, parse_boot_starts, parse_meminfo, parse_package_flags, protected_reason, valid_pkg};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, sleep};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const ACTIVE_FOR: Duration = Duration::from_secs(8);
const MAX_PROCS: usize = 120;

struct State {
    snapshot: Mutex<String>,
    last_request_ms: AtomicU64,
    started: Instant,
    token: String,
}

impl State {
    fn touch(&self) {
        self.last_request_ms.store(self.started.elapsed().as_millis() as u64 + 1, Ordering::Relaxed);
    }
    fn active(&self) -> bool {
        let last = self.last_request_ms.load(Ordering::Relaxed);
        last != 0 && self.started.elapsed().as_millis() as u64 - (last - 1) < ACTIVE_FOR.as_millis() as u64
    }
}

pub fn start(port: u16, token: String) {
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("api: cannot listen on 127.0.0.1:{port}: {e}");
            return;
        }
    };
    let state = Arc::new(State {
        snapshot: Mutex::new(String::from("{}")),
        last_request_ms: AtomicU64::new(0),
        started: Instant::now(),
        token,
    });
    let s = state.clone();
    thread::spawn(move || sampler(s));
    thread::spawn(move || {
        for conn in listener.incoming().flatten() {
            let st = state.clone();
            thread::spawn(move || handle(conn, &st));
        }
    });
}

// ---------- pure helpers (unit tested) ----------

pub fn json_str(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => {}
            c if (c as u32) < 0x20 => o.push(' '),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// `/proc/<pid>/stat` -> (cpu ticks = utime + stime, thread count).
pub fn parse_stat(stat: &str) -> Option<(u64, u32)> {
    let close = stat.rfind(')')?; // the name may contain spaces and parentheses
    let f: Vec<&str> = stat[close + 1..].split_whitespace().collect();
    let ticks = f.get(11)?.parse::<u64>().ok()? + f.get(12)?.parse::<u64>().ok()?;
    let threads = f.get(17)?.parse::<u32>().ok()?;
    Some((ticks, threads))
}

#[derive(Default, Debug, PartialEq)]
pub struct Thermal {
    pub cpu: Option<f64>,
    pub gpu: Option<f64>,
    pub battery: Option<f64>,
    pub skin: Option<f64>,
}

/// Pick the interesting temperatures out of all thermal zones (zone type, degrees C).
pub fn summarize_thermal(zones: &[(String, f64)]) -> Thermal {
    let mut t = Thermal::default();
    let max = |slot: &mut Option<f64>, v: f64| *slot = Some(slot.map_or(v, |m| m.max(v)));
    for (name, c) in zones.iter().filter(|(_, c)| (0.0..150.0).contains(c)) {
        let n = name.to_ascii_lowercase();
        if n.contains("cpu") {
            max(&mut t.cpu, *c);
        } else if n.contains("gpu") {
            max(&mut t.gpu, *c);
        } else if n == "battery" {
            max(&mut t.battery, *c);
        } else if n.contains("skin") || n.contains("shell") {
            max(&mut t.skin, *c);
        }
    }
    t
}

/// `dumpsys battery` -> (level %, state, temperature C). The state is spelled the way `domain`
/// spells it, so the app reads the same word whether the number came from here or from desktop.
pub fn parse_battery(text: &str) -> Option<(i64, &'static str, f64)> {
    let field = |key: &str| -> Option<i64> { text.lines().find_map(|l| l.trim().strip_prefix(key)?.trim().parse().ok()) };
    let level = field("level:")?;
    let state = match field("status:") {
        Some(2) => "charging",
        Some(3) => "discharging",
        Some(4) => "not_charging",
        Some(5) => "full",
        _ => "unknown",
    };
    let temp = field("temperature:").map_or(0.0, |t| t as f64 / 10.0);
    Some((level, state, temp))
}

fn meminfo_mb() -> [u64; 5] {
    // total, available, cached, swap total, swap free (MB)
    let s = fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let mut r = [0u64; 5];
    for l in s.lines() {
        let mut it = l.split_whitespace();
        let (Some(k), Some(v)) = (it.next(), it.next()) else { continue };
        let v: u64 = v.parse().unwrap_or(0) / 1024;
        match k {
            "MemTotal:" => r[0] = v,
            "MemAvailable:" => r[1] = v,
            "Cached:" => r[2] = v,
            "SwapTotal:" => r[3] = v,
            "SwapFree:" => r[4] = v,
            _ => {}
        }
    }
    r
}

// ---------- sampler ----------

struct Sample {
    pid: u32,
    name: String,
    ticks: u64,
    threads: u32,
    rss_mb: u64,
}

fn scan_procs(page_kb: u64) -> Vec<Sample> {
    let mut out = Vec::new();
    let Ok(dir) = fs::read_dir("/proc") else { return out };
    for e in dir.flatten() {
        let Ok(pid) = e.file_name().to_string_lossy().parse::<u32>() else { continue };
        let base = format!("/proc/{pid}");
        let Ok(statm) = fs::read_to_string(format!("{base}/statm")) else { continue };
        let pages: u64 = statm.split_whitespace().nth(1).and_then(|x| x.parse().ok()).unwrap_or(0);
        if pages == 0 {
            continue; // kernel thread
        }
        let Ok(stat) = fs::read_to_string(format!("{base}/stat")) else { continue };
        let Some((ticks, threads)) = parse_stat(&stat) else { continue };
        let cmd = fs::read(format!("{base}/cmdline")).unwrap_or_default();
        let first = cmd.split(|&b| b == 0).next().unwrap_or(&[]);
        let mut name = String::from_utf8_lossy(first).into_owned();
        if name.is_empty() {
            let open = stat.find('(').map_or(0, |i| i + 1);
            let close = stat.rfind(')').unwrap_or(open);
            name = stat[open..close].to_string();
        }
        out.push(Sample { pid, name, ticks, threads, rss_mb: pages * page_kb / 1024 });
    }
    out
}

fn thermal_zones() -> Vec<(String, f64)> {
    let mut zones = Vec::new();
    let Ok(dir) = fs::read_dir("/sys/class/thermal") else { return zones };
    for e in dir.flatten() {
        if !e.file_name().to_string_lossy().starts_with("thermal_zone") {
            continue;
        }
        let p = e.path();
        let ty = fs::read_to_string(p.join("type")).unwrap_or_default().trim().to_string();
        let Some(t) = fs::read_to_string(p.join("temp")).ok().and_then(|s| s.trim().parse::<f64>().ok()) else { continue };
        zones.push((ty, t / 1000.0));
    }
    zones
}

fn opt(v: Option<f64>) -> String {
    v.map_or("null".to_string(), |x| format!("{x:.1}"))
}

fn sampler(state: Arc<State>) {
    let pkb = page_kb();
    let ncores = read_cpu().len().saturating_sub(1);
    let mut prev_cpu: Vec<CpuTimes> = read_cpu();
    let mut prev_procs: HashMap<u32, u64> = HashMap::new();
    let mut prev_at = Instant::now();
    // (level %, state, temperature C)
    let mut battery: Option<(i64, &'static str, f64)> = None;
    let mut battery_at: Option<Instant> = None;
    loop {
        sleep(Duration::from_secs(1));
        if !state.active() {
            // nobody is watching: stay almost free, and forget the baselines
            prev_cpu = read_cpu();
            prev_procs.clear();
            prev_at = Instant::now();
            continue;
        }
        let cur = read_cpu();
        let dt = prev_at.elapsed().as_secs_f64().max(0.05);
        prev_at = Instant::now();

        let cores: Vec<String> = (1..=ncores)
            .map(|i| {
                let p = match (prev_cpu.get(i), cur.get(i)) {
                    (Some(a), Some(b)) => pct(a, b),
                    _ => 0.0,
                };
                let khz = read_khz(i - 1);
                format!("{{\"percent\":{p:.1},\"mhz\":{}}}", khz / 1000)
            })
            .collect();
        let total = match (prev_cpu.first(), cur.first()) {
            (Some(a), Some(b)) => pct(a, b),
            _ => 0.0,
        };
        prev_cpu = cur;

        let [mt, ma, mc, st, sf] = meminfo_mb();
        let th = summarize_thermal(&thermal_zones());
        if battery_at.is_none_or(|t| t.elapsed() > Duration::from_secs(15)) {
            battery_at = Some(Instant::now());
            battery = Command::new("dumpsys").arg("battery").output().ok().and_then(|o| parse_battery(&String::from_utf8_lossy(&o.stdout)));
        }

        let samples = scan_procs(pkb);
        let mut rows: Vec<(f64, &Sample)> = samples
            .iter()
            .map(|s| {
                let delta = prev_procs.get(&s.pid).map_or(0, |&o| s.ticks.saturating_sub(o));
                (delta as f64 * 100.0 / (CLK_TCK * dt), s)
            })
            .collect();
        prev_procs = samples.iter().map(|s| (s.pid, s.ticks)).collect();
        rows.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal).then(b.1.rss_mb.cmp(&a.1.rss_mb)));
        let nprocs = rows.len();
        let procs: Vec<String> = rows
            .iter()
            .take(MAX_PROCS)
            .map(|(c, s)| format!("{{\"pid\":{},\"name\":{},\"cpu_percent\":{c:.1},\"memory_mb\":{},\"threads\":{}}}", s.pid, json_str(&s.name), s.rss_mb, s.threads))
            .collect();

        let load = fs::read_to_string("/proc/loadavg").unwrap_or_default();
        let load: Vec<&str> = load.split_whitespace().take(3).collect();
        let uptime: f64 = fs::read_to_string("/proc/uptime").ok().and_then(|s| s.split_whitespace().next().and_then(|x| x.parse().ok())).unwrap_or(0.0);
        let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis());
        let bat = battery.map_or("null".to_string(), |(level, state, temp)| format!("{{\"percent\":{level},\"state\":\"{state}\",\"temperature_c\":{temp:.1}}}"));

        let json = format!(
            "{{\"t\":{now_ms},\"uptime\":{uptime:.0},\"cpu\":{{\"total_percent\":{total:.1},\"cores\":[{}],\"load_avg\":[{}]}},\
             \"mem\":{{\"total\":{mt},\"avail\":{ma},\"cached\":{mc},\"swap_total\":{st},\"swap_free\":{sf}}},\
             \"temp\":{{\"cpu\":{},\"gpu\":{},\"battery\":{},\"skin\":{}}},\"battery\":{bat},\
             \"nprocs\":{nprocs},\"procs\":[{}]}}",
            cores.join(","),
            load.join(","),
            opt(th.cpu),
            opt(th.gpu),
            opt(th.battery),
            opt(th.skin),
            procs.join(",")
        );
        *state.snapshot.lock().unwrap() = json;
    }
}

fn read_khz(core: usize) -> u64 {
    fs::read_to_string(format!("/sys/devices/system/cpu/cpu{core}/cpufreq/scaling_cur_freq"))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

// ---------- startup list and actions (same rules as the PC debloater) ----------

fn run(prog: &str, args: &[&str]) -> String {
    match Command::new(prog).args(args).output() {
        Ok(o) => {
            let mut s = String::from_utf8_lossy(&o.stdout).into_owned();
            s.push_str(&String::from_utf8_lossy(&o.stderr));
            s
        }
        Err(e) => format!("failed to run {prog}: {e}"),
    }
}

fn third_party() -> HashSet<String> {
    run("cmd", &["package", "list", "packages", "-3"])
        .lines()
        .filter_map(|l| l.trim().strip_prefix("package:"))
        .map(|p| p.trim().to_string())
        .collect()
}

fn role_holders() -> HashMap<String, &'static str> {
    let mut m = HashMap::new();
    let ime = run("settings", &["get", "secure", "default_input_method"]);
    if let Some(p) = ime.trim().split('/').next().filter(|p| p.contains('.')) {
        m.insert(p.to_string(), "your keyboard");
    }
    let home = run("cmd", &["package", "resolve-activity", "--brief", "-a", "android.intent.action.MAIN", "-c", "android.intent.category.HOME"]);
    if let Some(p) = home.lines().filter_map(|l| l.trim().split('/').next()).rfind(|p| p.contains('.') && !p.contains(' ')) {
        m.insert(p.to_string(), "your home screen");
    }
    for (role, why) in [("android.app.role.DIALER", "your phone app"), ("android.app.role.SMS", "your messages app")] {
        for p in run("cmd", &["role", "get-role-holders", "--user", "0", role]).split_whitespace().filter(|p| p.contains('.')) {
            m.insert(p.to_string(), why);
        }
    }
    m
}

fn startup_json() -> String {
    let third = third_party();
    let flags = parse_package_flags(&run("dumpsys", &["package"]));
    let mem = parse_meminfo(&run("dumpsys", &["meminfo"]));
    let recv = parse_boot_receivers(&run("cmd", &["package", "query-receivers", "--brief", "-a", "android.intent.action.BOOT_COMPLETED"]));
    let boot = parse_boot_starts(&run("logcat", &["-b", "system", "-d"]), 90.0);
    let procs = run("ps", &["-A", "-o", "NAME"]);
    let running: HashSet<&str> = procs.lines().map(|l| l.trim().split(':').next().unwrap_or("")).collect();
    let roles = role_holders();
    let mut names: Vec<&String> = third.iter().collect();
    names.sort();
    let rows: Vec<String> = names
        .iter()
        .map(|p| {
            let (stopped, enabled) = flags.get(*p).copied().unwrap_or((false, 0));
            let boot_json = boot.get(*p).map_or("null".to_string(), |b| {
                format!("{{\"how\":{},\"comp\":{},\"after\":{}}}", json_str(&b.trigger), json_str(&b.component), b.after_s)
            });
            format!(
                "{{\"p\":{},\"mb\":{},\"running\":{},\"stopped\":{},\"frozen\":{},\"recv\":{},\"boot\":{},\"prot\":{}}}",
                json_str(p),
                mem.get(*p).copied().unwrap_or(0),
                running.contains(p.as_str()),
                stopped,
                matches!(enabled, 2..=4),
                recv.contains(*p),
                boot_json,
                json_str(protected_reason(p, &roles).unwrap_or(""))
            )
        })
        .collect();
    format!("{{\"boot_log\":{},\"apps\":[{}]}}", !boot.is_empty(), rows.join(","))
}

fn act(op: &str, body: &str) -> String {
    let third = third_party();
    let roles = role_holders();
    let mut results = Vec::new();
    for p in body.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let (ok, msg) = if !valid_pkg(p) {
            (false, "refused: invalid package name".to_string())
        } else if !third.contains(p) {
            (false, "refused: not one of your installed apps".to_string())
        } else if let Some(why) = protected_reason(p, &roles).filter(|_| op != "unfreeze") {
            (false, format!("refused: {why}"))
        } else {
            let out = match op {
                "stop" => run("am", &["force-stop", p]),
                "freeze" => run("pm", &["disable-user", "--user", "0", p]),
                "unfreeze" => run("pm", &["enable", "--user", "0", p]),
                _ => "unknown operation".into(),
            };
            let ok = match op {
                "stop" => out.trim().is_empty(),
                "freeze" | "unfreeze" => out.contains("new state"),
                _ => false,
            };
            (ok, if out.trim().is_empty() { "done".into() } else { out.trim().replace('\n', " ") })
        };
        results.push(format!("{{\"p\":{},\"ok\":{ok},\"msg\":{}}}", json_str(p), json_str(&msg)));
    }
    format!("{{\"results\":[{}]}}", results.join(","))
}

// ---------- http ----------

struct Req {
    method: String,
    path: String,
    query: HashMap<String, String>,
    token: Option<String>,
    body: String,
}

fn read_req(s: &mut TcpStream) -> Option<Req> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let head_end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        if buf.len() > 65536 {
            return None;
        }
        let n = s.read(&mut tmp).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&tmp[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.lines();
    let mut first = lines.next()?.split_whitespace();
    let method = first.next()?.to_string();
    let target = first.next()?;
    let (path, qs) = target.split_once('?').unwrap_or((target, ""));
    let query = qs.split('&').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let mut token = None;
    let mut len = 0usize;
    for l in lines {
        if let Some((k, v)) = l.split_once(':') {
            match k.trim().to_ascii_lowercase().as_str() {
                "x-token" => token = Some(v.trim().to_string()),
                "content-length" => len = v.trim().parse().unwrap_or(0).min(1 << 20),
                _ => {}
            }
        }
    }
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < len {
        let n = s.read(&mut tmp).ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&tmp[..n]);
    }
    Some(Req { method, path: path.to_string(), query, token, body: String::from_utf8_lossy(&body).into_owned() })
}

fn respond(s: &mut TcpStream, code: &str, body: &str) {
    let _ = write!(s, "HTTP/1.0 {code}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nCache-Control: no-store\r\n\r\n{body}", body.len());
}

fn handle(mut s: TcpStream, state: &State) {
    let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
    let Some(req) = read_req(&mut s) else { return };
    if req.token.as_deref() != Some(state.token.as_str()) {
        return respond(&mut s, "403 Forbidden", r#"{"error":"bad token"}"#);
    }
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/v1/ping") => respond(&mut s, "200 OK", r#"{"ok":true}"#),
        ("GET", "/v1/snapshot") => {
            state.touch();
            let mut snap = state.snapshot.lock().unwrap().clone();
            if snap == "{}" {
                snap = r#"{"warming_up":true}"#.into(); // first sample is one second away
            }
            respond(&mut s, "200 OK", &snap);
        }
        ("GET", "/v1/startup") => respond(&mut s, "200 OK", &startup_json()),
        ("POST", "/v1/act") => {
            let op = req.query.get("op").map(String::as_str).unwrap_or("");
            respond(&mut s, "200 OK", &act(op, &req.body));
        }
        _ => respond(&mut s, "404 Not Found", r#"{"error":"not found"}"#),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_lines_with_awkward_names_are_parsed() {
        let line = "4242 (my (weird) app) S 1 2 3 4 5 6 7 8 9 10 111 222 0 0 20 0 17 0 1000";
        // fields after ')': S 1 2 3 4 5 6 7 8 9 10 111 222 0 0 20 0 17 ...  -> utime=111 stime=222 threads=17
        assert_eq!(parse_stat(line), Some((333, 17)));
        assert_eq!(parse_stat("garbage"), None);
        assert_eq!(parse_stat("1 (x) S 1 2"), None, "too short");
    }

    #[test]
    fn thermal_summary_picks_the_hottest_of_each_kind() {
        let z = |n: &str, c: f64| (n.to_string(), c);
        let t = summarize_thermal(&[z("cpu-0-0-usr", 61.0), z("cpuss-1", 73.5), z("gpuss-0", 55.0), z("battery", 38.2), z("skin-msm-therm", 41.0), z("cpu-bad", -300.0), z("cpu-huge", 5000.0), z("ddr", 90.0)]);
        assert_eq!(t, Thermal { cpu: Some(73.5), gpu: Some(55.0), battery: Some(38.2), skin: Some(41.0) });
        assert_eq!(summarize_thermal(&[]), Thermal::default());
    }

    #[test]
    fn battery_dump_is_parsed() {
        let dump = "Current Battery Service state:\n  AC powered: true\n  status: 2\n  level: 61\n  temperature: 343\n";
        assert_eq!(parse_battery(dump), Some((61, "charging", 34.3)));
        assert_eq!(parse_battery("status: 5\nlevel: 100"), Some((100, "full", 0.0)));
        assert_eq!(parse_battery("status: 3\nlevel: 40"), Some((40, "discharging", 0.0)));
        assert_eq!(parse_battery("nothing useful"), None);
    }

    #[test]
    fn json_strings_are_escaped() {
        assert_eq!(json_str("a\"b\\c\nd"), "\"a\\\"b\\\\c\\nd\"");
    }

    #[test]
    fn sampling_only_runs_while_somebody_is_watching() {
        let st = State { snapshot: Mutex::new("{}".into()), last_request_ms: AtomicU64::new(0), started: Instant::now(), token: "t".into() };
        assert!(!st.active(), "never asked yet");
        st.touch();
        assert!(st.active());
    }
}
