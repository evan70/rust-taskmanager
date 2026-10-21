use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{exit, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::{self, sleep};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

mod api;

const CLK_TCK: f64 = 100.0; // USER_HZ is fixed at 100 on Linux

const USAGE: &str = "rust-sysmon [OPTIONS]                      daemon: system stats sampler\n\
rust-sysmon top [--n N] [--sample SECS]   one-shot top processes by RSS and CPU\n\
rust-sysmon restrict PKG                  put PKG in the restricted standby bucket\n\
rust-sysmon undo PKG                      reverse restrict / --act for PKG\n\
\n\
  --interval S       seconds between samples (default 5)\n\
  --log PATH         append to PATH instead of stdout; rotates to PATH.1\n\
  --max-log-kb N     rotate threshold (default 512)\n\
  --slow-every N     read thermal zones every N samples (default 6)\n\
  --battery-every S  seconds between battery reads via dumpsys (default 300, 0=off)\n\
  --watch S          every S seconds log top processes and warn on limits (default 0=off)\n\
  --rss-limit-mb N   warn when a process RSS exceeds N MB (default 800)\n\
  --cpu-limit PCT    warn when a process uses more than PCT of one core (default 40)\n\
  --top-n N          processes listed per watch tick (default 5)\n\
  --api PORT         local API for the Task Manager app on 127.0.0.1:PORT (needs --token-file)\n\
  --token-file PATH  file holding the API's secret (16+ characters)\n\
  --act MODE         dry|bucket|appops: act on apps over a limit for --strikes ticks.\n\
                     Only user-installed apps that are not in the foreground are touched.\n\
                     bucket = restricted standby bucket; appops = RUN_ANY_IN_BACKGROUND ignore\n\
  --strikes N        consecutive watch ticks over a limit before acting (default 3)\n\
  --protect A,B      package names never to act on\n\
  --logcat           log crashes/ANRs (spec: AndroidRuntime:E DEBUG:E ActivityManager:W *:S)\n\
  --logcat-filter S  same, with your own logcat filterspec, e.g. \"MyTag:D *:S\"\n\
  --listen PORT      read-only query server on 127.0.0.1:PORT (stats, top, tail); use\n\
                     adb forward tcp:PORT tcp:PORT, then curl http://localhost:PORT/top\n\
  --once             print one sample (CPU % over 1s) and exit";

struct Config {
    interval: Duration,
    log: Option<String>,
    max_log: u64,
    slow_every: u64,
    battery_every: u64,
    watch: u64,
    rss_limit: u64,
    cpu_limit: f64,
    top_n: usize,
    act: Option<String>,
    strikes: u32,
    protect: Vec<String>,
    logcat: Option<String>,
    listen: Option<u16>,
    api: Option<u16>,
    token_file: Option<String>,
    once: bool,
}

const DEFAULT_LOGCAT: &str = "AndroidRuntime:E DEBUG:E ActivityManager:W *:S";

fn parse_args() -> Config {
    let mut c = Config {
        interval: Duration::from_secs(5),
        log: None,
        max_log: 512 * 1024,
        slow_every: 6,
        battery_every: 300,
        watch: 0,
        rss_limit: 800,
        cpu_limit: 40.0,
        top_n: 5,
        act: None,
        strikes: 3,
        protect: Vec::new(),
        logcat: None,
        listen: None,
        api: None,
        token_file: None,
        once: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = |name: &str| -> String {
            args.next().unwrap_or_else(|| {
                eprintln!("missing value for {name}\n{USAGE}");
                exit(2)
            })
        };
        match a.as_str() {
            "--interval" => {
                let s: f64 = val("--interval").parse().unwrap_or(5.0);
                c.interval = Duration::from_secs_f64(s.max(0.5));
            }
            "--log" => c.log = Some(val("--log")),
            "--max-log-kb" => c.max_log = val("--max-log-kb").parse::<u64>().unwrap_or(512) * 1024,
            "--slow-every" => c.slow_every = val("--slow-every").parse::<u64>().unwrap_or(6).max(1),
            "--battery-every" => c.battery_every = val("--battery-every").parse().unwrap_or(300),
            "--watch" => c.watch = val("--watch").parse().unwrap_or(0),
            "--rss-limit-mb" => c.rss_limit = val("--rss-limit-mb").parse().unwrap_or(800),
            "--cpu-limit" => c.cpu_limit = val("--cpu-limit").parse().unwrap_or(40.0),
            "--top-n" => c.top_n = val("--top-n").parse::<usize>().unwrap_or(5).max(1),
            "--act" => {
                let m = val("--act");
                if !matches!(m.as_str(), "dry" | "bucket" | "appops") {
                    eprintln!("--act must be dry, bucket or appops");
                    exit(2);
                }
                c.act = Some(m);
            }
            "--strikes" => c.strikes = val("--strikes").parse::<u32>().unwrap_or(3).max(1),
            "--protect" => c.protect = val("--protect").split(',').map(str::to_string).collect(),
            "--logcat" => c.logcat = Some(DEFAULT_LOGCAT.to_string()),
            "--logcat-filter" => c.logcat = Some(val("--logcat-filter")),
            "--listen" => c.listen = Some(val("--listen").parse().unwrap_or(7878)),
            "--api" => c.api = Some(val("--api").parse().unwrap_or(7879)),
            "--token-file" => c.token_file = Some(val("--token-file")),
            "--once" => c.once = true,
            _ => {
                eprintln!("{USAGE}");
                exit(if a == "-h" || a == "--help" { 0 } else { 2 });
            }
        }
    }
    c
}

#[derive(Clone, Default)]
struct CpuTimes {
    busy: u64,
    total: u64,
}

fn read_cpu() -> Vec<CpuTimes> {
    let s = fs::read_to_string("/proc/stat").unwrap_or_default();
    let mut out = Vec::new();
    for line in s.lines() {
        if !line.starts_with("cpu") {
            break;
        }
        let f: Vec<u64> = line
            .split_whitespace()
            .skip(1)
            .filter_map(|x| x.parse().ok())
            .collect();
        if f.len() < 5 {
            continue;
        }
        // user nice system idle iowait irq softirq steal
        let total: u64 = f.iter().take(8).sum();
        let idle = f[3] + f[4];
        out.push(CpuTimes { busy: total - idle, total });
    }
    out
}

fn pct(prev: &CpuTimes, cur: &CpuTimes) -> f64 {
    let dt = cur.total.saturating_sub(prev.total);
    if dt == 0 {
        return 0.0;
    }
    cur.busy.saturating_sub(prev.busy) as f64 * 100.0 / dt as f64
}

fn meminfo() -> [u64; 4] {
    // total, available, swap_total, swap_free in MB
    let s = fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let mut r = [0u64; 4];
    for line in s.lines() {
        let mut it = line.split_whitespace();
        let (Some(k), Some(v)) = (it.next(), it.next()) else { continue };
        let v: u64 = v.parse().unwrap_or(0) / 1024;
        match k {
            "MemTotal:" => r[0] = v,
            "MemAvailable:" => r[1] = v,
            "SwapTotal:" => r[2] = v,
            "SwapFree:" => r[3] = v,
            _ => {}
        }
    }
    r
}

fn loadavg() -> String {
    let s = fs::read_to_string("/proc/loadavg").unwrap_or_default();
    s.split_whitespace().take(3).collect::<Vec<_>>().join(",")
}

fn read_i64(path: &str) -> Option<i64> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// Hottest thermal zone, via /sys. Omitted if unreadable.
fn thermal_max() -> String {
    let mut max_c: Option<f64> = None;
    if let Ok(dir) = fs::read_dir("/sys/class/thermal") {
        for e in dir.flatten() {
            if !e.file_name().to_string_lossy().starts_with("thermal_zone") {
                continue;
            }
            if let Some(t) = read_i64(&format!("{}/temp", e.path().display())) {
                // zones report millidegrees; ignore junk readings
                let c = t as f64 / 1000.0;
                if (-20.0..150.0).contains(&c) && max_c.map_or(true, |m| c > m) {
                    max_c = Some(c);
                }
            }
        }
    }
    max_c.map_or(String::new(), |c| format!(" therm_max_c={c:.1}"))
}

/// Battery via `dumpsys battery` (sysfs is not readable by the shell user on Android 16).
fn battery() -> Option<String> {
    let out = Command::new("dumpsys").arg("battery").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let field = |key: &str| -> Option<i64> {
        text.lines()
            .find_map(|l| l.trim().strip_prefix(key)?.trim().parse().ok())
    };
    let level = field("level:")?;
    let mut s = format!("bat={level}%");
    if let Some(t) = field("temperature:") {
        s += &format!(" bat_c={:.1}", t as f64 / 10.0);
    }
    if let Some(v) = field("voltage:") {
        s += &format!(" bat_mv={v}");
    }
    let chg = match field("status:") {
        Some(2) => "charging",
        Some(3) => "discharging",
        Some(4) => "not_charging",
        Some(5) => "full",
        _ => "unknown",
    };
    s += &format!(" chg={chg}");
    Some(s)
}

struct Proc {
    pid: u32,
    name: String,
    rss_mb: u64,
    ticks: u64,
}

struct Row {
    name: String,
    rss_mb: u64,
    cpu: f64,
}

fn page_kb() -> u64 {
    fs::read_to_string("/proc/self/smaps")
        .ok()
        .and_then(|s| {
            s.lines().find_map(|l| {
                l.strip_prefix("KernelPageSize:")?.trim().strip_suffix("kB")?.trim().parse().ok()
            })
        })
        .unwrap_or(4)
}

fn scan(page_kb: u64) -> Vec<Proc> {
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
        // comm may contain spaces/parens, so split after the last ')'
        let Some(close) = stat.rfind(')') else { continue };
        let f: Vec<&str> = stat[close + 1..].split_whitespace().collect();
        let ticks = match (f.get(11), f.get(12)) {
            (Some(u), Some(s)) => u.parse::<u64>().unwrap_or(0) + s.parse::<u64>().unwrap_or(0),
            _ => continue,
        };
        let cmd = fs::read(format!("{base}/cmdline")).unwrap_or_default();
        let first = cmd.split(|&b| b == 0).next().unwrap_or(&[]);
        let mut name = String::from_utf8_lossy(first).into_owned();
        if name.is_empty() {
            let open = stat.find('(').map_or(0, |i| i + 1);
            name = stat[open..close].to_string();
        }
        out.push(Proc { pid, name, rss_mb: pages * page_kb / 1024, ticks });
    }
    out
}

/// Tracks per-pid tick counts between scans so CPU % can be derived.
struct Watch {
    page_kb: u64,
    prev: HashMap<u32, u64>,
    at: Instant,
}

impl Watch {
    fn new() -> Watch {
        let mut w = Watch { page_kb: page_kb(), prev: HashMap::new(), at: Instant::now() };
        w.rows(); // prime so the next call has a baseline
        w
    }

    /// Scan and return rows with CPU % (of one core) since the previous call.
    fn rows(&mut self) -> Vec<Row> {
        let procs = scan(self.page_kb);
        let dt = self.at.elapsed().as_secs_f64().max(0.001);
        self.at = Instant::now();
        let rows = procs
            .iter()
            .map(|p| {
                let delta = self.prev.get(&p.pid).map_or(0, |&o| p.ticks.saturating_sub(o));
                Row {
                    name: p.name.clone(),
                    rss_mb: p.rss_mb,
                    cpu: delta as f64 * 100.0 / (CLK_TCK * dt),
                }
            })
            .collect();
        self.prev = procs.iter().map(|p| (p.pid, p.ticks)).collect();
        rows
    }
}

fn by_rss(rows: &mut [Row]) {
    rows.sort_by(|a, b| b.rss_mb.cmp(&a.rss_mb));
}

fn by_cpu(rows: &mut [Row]) {
    rows.sort_by(|a, b| b.cpu.partial_cmp(&a.cpu).unwrap_or(std::cmp::Ordering::Equal));
}

fn run_top(args: Vec<String>) {
    let (mut n, mut sample) = (15usize, 2.0f64);
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match (a.as_str(), it.next()) {
            ("--n", Some(v)) => n = v.parse().unwrap_or(15),
            ("--sample", Some(v)) => sample = v.parse().unwrap_or(2.0),
            _ => {
                eprintln!("{USAGE}");
                exit(2)
            }
        }
    }
    print!("{}", top_text(n, sample));
}

fn top_text(n: usize, sample: f64) -> String {
    let mut w = Watch::new();
    sleep(Duration::from_secs_f64(sample.max(0.5)));
    let mut rows = w.rows();
    let mut s = format!("{:>7}  {:>6}  PROCESS (by RSS)\n", "RSS_MB", "CPU%");
    by_rss(&mut rows);
    for r in rows.iter().take(n) {
        s += &format!("{:>7}  {:>6.1}  {}\n", r.rss_mb, r.cpu, r.name);
    }
    s += &format!("\n{:>7}  {:>6}  PROCESS (by CPU)\n", "RSS_MB", "CPU%");
    by_cpu(&mut rows);
    for r in rows.iter().take(n) {
        s += &format!("{:>7}  {:>6.1}  {}\n", r.rss_mb, r.cpu, r.name);
    }
    s
}

fn join_top(rows: &[Row], n: usize, f: impl Fn(&Row) -> String) -> String {
    rows.iter()
        .take(n)
        .map(|r| format!("{}={}", r.name, f(r)))
        .collect::<Vec<_>>()
        .join(",")
}

/// Package name of an app process ("com.foo:svc" -> "com.foo"); None for native binaries.
fn pkg_of(proc_name: &str) -> Option<&str> {
    let p = proc_name.split(':').next()?;
    (p.contains('.') && !p.starts_with('/')).then_some(p)
}

fn cmd_ok(args: &[&str]) -> bool {
    Command::new("cmd").args(args).output().map_or(false, |o| o.status.success())
}

fn third_party_pkgs() -> HashSet<String> {
    let out = Command::new("cmd").args(["package", "list", "packages", "-3"]).output();
    let text = out.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    text.lines().filter_map(|l| l.strip_prefix("package:")).map(|p| p.trim().to_string()).collect()
}

fn foreground_pkg() -> Option<String> {
    let out = Command::new("dumpsys").args(["activity", "activities"]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().find(|l| l.contains("ResumedActivity:"))?;
    let mut tokens = line.split_whitespace();
    tokens.find(|t| *t == "u0")?;
    Some(tokens.next()?.split('/').next()?.to_string())
}

fn restrict(pkg: &str, mode: &str) -> bool {
    match mode {
        "bucket" => cmd_ok(&["activity", "set-standby-bucket", pkg, "restricted"]),
        "appops" => cmd_ok(&["appops", "set", pkg, "RUN_ANY_IN_BACKGROUND", "ignore"]),
        _ => true, // dry
    }
}

fn undo(pkg: &str) -> bool {
    let a = cmd_ok(&["activity", "set-bg-restriction-level", "--user", "0", pkg, "adaptive_bucket"]);
    let b = cmd_ok(&["appops", "set", pkg, "RUN_ANY_IN_BACKGROUND", "allow"]);
    a && b
}

#[derive(Default)]
struct ActState {
    strikes: HashMap<String, u32>,
    acted: HashSet<String>,
}

fn watch_lines(w: &mut Watch, cfg: &Config, st: &mut ActState, ts: u64) -> Vec<String> {
    let mut rows = w.rows();
    let mut out = Vec::new();
    let mut over: HashSet<String> = HashSet::new();
    for r in &rows {
        let (rss_hit, cpu_hit) = (r.rss_mb > cfg.rss_limit, r.cpu > cfg.cpu_limit);
        if rss_hit {
            out.push(format!("ts={ts} WARN rss proc={} rss_mb={} limit_mb={}", r.name, r.rss_mb, cfg.rss_limit));
        }
        if cpu_hit {
            out.push(format!("ts={ts} WARN cpu proc={} cpu={:.0}% limit={:.0}%", r.name, r.cpu, cfg.cpu_limit));
        }
        if let (true, Some(p)) = (rss_hit || cpu_hit, pkg_of(&r.name)) {
            over.insert(p.to_string());
        }
    }
    if let Some(mode) = cfg.act.as_deref() {
        st.strikes.retain(|p, _| over.contains(p));
        for p in &over {
            *st.strikes.entry(p.clone()).or_insert(0) += 1;
        }
        let due: Vec<String> = st
            .strikes
            .iter()
            .filter(|(p, n)| **n >= cfg.strikes && !st.acted.contains(*p) && !cfg.protect.contains(*p))
            .map(|(p, _)| p.clone())
            .collect();
        if !due.is_empty() {
            let user_apps = third_party_pkgs();
            let fg = foreground_pkg();
            for p in due {
                // keep the strike count so it acts as soon as the app leaves the foreground
                if !user_apps.contains(&p) || fg.as_deref() == Some(p.as_str()) {
                    continue;
                }
                let ok = restrict(&p, mode);
                out.push(format!("ts={ts} ACT mode={mode} pkg={p} ok={ok}"));
                st.acted.insert(p);
            }
        }
    }
    by_rss(&mut rows);
    out.push(format!("ts={ts} top_rss {}", join_top(&rows, cfg.top_n, |r| format!("{}MB", r.rss_mb))));
    by_cpu(&mut rows);
    out.push(format!("ts={ts} top_cpu {}", join_top(&rows, cfg.top_n, |r| format!("{:.0}%", r.cpu))));
    out
}

enum Sink {
    Stdout,
    File { path: String, f: File, size: u64, max: u64 },
}

impl Sink {
    fn open(path: Option<&str>, max: u64) -> io::Result<Sink> {
        match path {
            None => Ok(Sink::Stdout),
            Some(p) => {
                let f = OpenOptions::new().create(true).append(true).open(p)?;
                let size = f.metadata()?.len();
                Ok(Sink::File { path: p.to_string(), f, size, max })
            }
        }
    }

    fn write_line(&mut self, line: &str) -> io::Result<()> {
        match self {
            Sink::Stdout => {
                let mut o = io::stdout().lock();
                writeln!(o, "{line}")?;
                o.flush()
            }
            Sink::File { path, f, size, max } => {
                if *size + line.len() as u64 + 1 > *max {
                    let _ = fs::rename(&*path, format!("{path}.1"));
                    *f = OpenOptions::new().create(true).append(true).open(&*path)?;
                    *size = 0;
                }
                writeln!(f, "{line}")?;
                *size += line.len() as u64 + 1;
                Ok(())
            }
        }
    }
}

/// State shared between the sampler, logcat and listener threads.
struct Shared {
    sink: Mutex<Sink>,
    latest: Mutex<String>,
}

fn emit(sh: &Shared, line: &str) -> bool {
    sh.sink.lock().map_or(false, |mut s| s.write_line(line).is_ok())
}

fn now_ts() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn spawn_logcat(sh: Arc<Shared>, spec: String) {
    thread::spawn(move || loop {
        let child = Command::new("logcat")
            .args(["-v", "threadtime", "-T", "1"])
            .args(spec.split_whitespace())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        if let Ok(mut ch) = child {
            if let Some(out) = ch.stdout.take() {
                // -T 1 replays one old line; skip it, and skip the "--- beginning of" headers
                let lines = BufReader::new(out).lines().map_while(Result::ok).skip(1);
                for line in lines.filter(|l| !l.starts_with("---")) {
                    let l: String = line.chars().take(300).collect();
                    if !emit(&sh, &format!("ts={} LOG {l}", now_ts())) {
                        exit(1);
                    }
                }
            }
            let _ = ch.wait();
        }
        sleep(Duration::from_secs(5)); // logcat died (e.g. buffer reset); retry
    });
}

fn tail_text(path: Option<&str>, sh: &Shared) -> String {
    let Some(p) = path else {
        return sh.latest.lock().map(|l| l.clone()).unwrap_or_default();
    };
    let text = fs::read_to_string(p).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(20)..].join("\n")
}

fn handle(mut s: TcpStream, sh: &Shared, log: Option<&str>) -> io::Result<()> {
    s.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut buf = [0u8; 512];
    let n = s.read(&mut buf)?;
    let req = String::from_utf8_lossy(&buf[..n]);
    let first = req.lines().next().unwrap_or("");
    let http = first.starts_with("GET ");
    let cmd = if http {
        first.split_whitespace().nth(1).unwrap_or("/").trim_matches('/').split('?').next().unwrap_or("")
    } else {
        first.trim()
    };
    let mut body = match cmd {
        "stats" => sh.latest.lock().map(|l| l.clone()).unwrap_or_default(),
        "top" => top_text(10, 1.0),
        "tail" => tail_text(log, sh),
        _ => "commands: stats, top, tail".to_string(),
    };
    body.push('\n');
    if http {
        write!(s, "HTTP/1.0 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n", body.len())?;
    }
    s.write_all(body.as_bytes())
}

fn spawn_listener(port: u16, sh: Arc<Shared>, log: Option<String>) {
    thread::spawn(move || {
        let l = match TcpListener::bind(("127.0.0.1", port)) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("cannot listen on 127.0.0.1:{port}: {e}");
                return;
            }
        };
        for s in l.incoming().flatten() {
            let _ = handle(s, &sh, log.as_deref());
        }
    });
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    match (argv.first().map(String::as_str), argv.get(1)) {
        (Some("top"), _) => return run_top(argv[1..].to_vec()),
        (Some("restrict"), Some(p)) => {
            let ok = restrict(p, "bucket");
            println!("restrict {p}: {}", if ok { "ok" } else { "failed" });
            exit(if ok { 0 } else { 1 });
        }
        (Some("undo"), Some(p)) => {
            let ok = undo(p);
            println!("undo {p}: {}", if ok { "ok" } else { "failed" });
            exit(if ok { 0 } else { 1 });
        }
        _ => {}
    }
    let cfg = parse_args();
    let sink = Sink::open(cfg.log.as_deref(), cfg.max_log).unwrap_or_else(|e| {
        eprintln!("cannot open log: {e}");
        exit(1)
    });
    let shared = Arc::new(Shared { sink: Mutex::new(sink), latest: Mutex::new(String::new()) });
    if let Some(spec) = cfg.logcat.clone() {
        spawn_logcat(shared.clone(), spec);
    }
    if let Some(port) = cfg.listen {
        spawn_listener(port, shared.clone(), cfg.log.clone());
    }
    if let Some(port) = cfg.api {
        // the API can stop apps, so it never starts without a secret token
        let token = cfg.token_file.as_deref().and_then(|p| fs::read_to_string(p).ok()).map(|t| t.trim().to_string()).filter(|t| t.len() >= 16);
        match token {
            Some(t) => api::start(port, t),
            None => {
                eprintln!("--api needs --token-file pointing at a file with a secret of at least 16 characters");
                exit(2);
            }
        }
    }

    let mut prev = read_cpu();
    let mut n: u64 = 0;
    let mut watch = (cfg.watch > 0).then(Watch::new);
    let mut act_state = ActState::default();
    let mut last_watch = Instant::now();
    let mut last_bat: Option<Instant> = None;
    loop {
        sleep(if cfg.once { Duration::from_secs(1) } else { cfg.interval });
        let cur = read_cpu();
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let all = match (prev.first(), cur.first()) {
            (Some(p), Some(c)) => pct(p, c),
            _ => 0.0,
        };
        let cores: Vec<String> = prev
            .iter()
            .zip(cur.iter())
            .skip(1)
            .map(|(p, c)| format!("{:.0}", pct(p, c)))
            .collect();
        let [total, avail, swt, swf] = meminfo();
        let mut line = format!(
            "ts={ts} cpu={all:.1}% cores={} mem_used_mb={} mem_avail_mb={avail} mem_total_mb={total} swap_used_mb={} load={}",
            cores.join(","),
            total.saturating_sub(avail),
            swt.saturating_sub(swf),
            loadavg(),
        );
        if n % cfg.slow_every == 0 {
            line += &thermal_max();
        }
        let bat_due = cfg.battery_every > 0
            && last_bat.map_or(true, |t| t.elapsed().as_secs() >= cfg.battery_every);
        if bat_due {
            last_bat = Some(Instant::now());
            if let Some(b) = battery() {
                line += " ";
                line += &b;
            }
        }
        if let Ok(mut l) = shared.latest.lock() {
            *l = line.clone();
        }
        let mut lines = vec![line];
        if let Some(w) = watch.as_mut() {
            if last_watch.elapsed().as_secs() >= cfg.watch {
                last_watch = Instant::now();
                lines.extend(watch_lines(w, &cfg, &mut act_state, ts));
            }
        }
        // stdout closed (e.g. adb disconnected) or disk full
        if lines.iter().any(|l| !emit(&shared, l)) {
            exit(1);
        }
        if cfg.once {
            break;
        }
        prev = cur;
        n += 1;
    }
}
