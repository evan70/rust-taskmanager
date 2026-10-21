//! Detailed mode: the app pairs with this phone's own wireless debugging, connects, and starts a
//! small helper program that can read what ordinary apps cannot (every process's CPU and memory,
//! chip temperatures). It all happens on the phone; no computer and no other app is involved. The
//! work runs on a background thread and reports through `Adv::status()`.

use crate::sys::Sys;
use adbwire::key::AdbKey;
use adbwire::net::pair;
use std::io;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const HELPER_PORT: u16 = crate::hclient::PORT;
const HELPER_FILE: &str = "tm-sysmon";
const KEY_NAME: &str = "task-manager@phone";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Step {
    /// Never set up on this run.
    Idle,
    /// The notification is up, waiting for the code.
    WaitingForCode,
    Pairing,
    Connecting,
    Starting,
    Running,
    Failed,
}

#[derive(Clone, Debug)]
pub struct Status {
    pub step: Step,
    pub message: String,
    /// What happened, step by step, for people who need to report a problem.
    pub log: Vec<String>,
}

enum Cmd {
    /// Show the notification and wait for the pairing code.
    Setup,
    /// The code arrived (from the notification or the app's own box).
    Code(String),
    /// Already paired: connect and (re)start the helper.
    Connect,
    /// Start the helper through Shizuku.
    Shizuku,
}

pub struct Adv {
    shared: Arc<Mutex<Status>>,
    tx: Sender<Cmd>,
}

/// The sender the Java receiver reaches through the native method (one app, one setup at a time).
static CODE_TX: Mutex<Option<Sender<Cmd>>> = Mutex::new(None);

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
/// Called from `PairReceiver.nativeCode` (see sys.rs) with the digits typed into the notification.
pub fn deliver_code(code: &str) {
    if let Some(tx) = CODE_TX.lock().ok().and_then(|g| g.clone()) {
        let _ = tx.send(Cmd::Code(code.to_string()));
    }
}

impl Adv {
    /// `dir` holds the key; `token` is what the helper must be given with every request.
    pub fn new(sys: Sys, dir: PathBuf, token: String, repaint: impl Fn() + Send + 'static) -> Adv {
        let shared = Arc::new(Mutex::new(Status { step: Step::Idle, message: String::new(), log: Vec::new() }));
        let (tx, rx) = mpsc::channel();
        *CODE_TX.lock().unwrap() = Some(tx.clone());
        let worker = Worker { sys, dir, token, shared: shared.clone(), repaint: Box::new(repaint) };
        std::thread::spawn(move || worker.run(rx));
        Adv { shared, tx }
    }

    pub fn status(&self) -> Status {
        self.shared.lock().map(|s| s.clone()).unwrap_or(Status { step: Step::Failed, message: "internal error".into(), log: Vec::new() })
    }

    pub fn setup(&self) {
        let _ = self.tx.send(Cmd::Setup);
    }

    pub fn code(&self, code: &str) {
        let _ = self.tx.send(Cmd::Code(code.to_string()));
    }

    pub fn connect(&self) {
        let _ = self.tx.send(Cmd::Connect);
    }

    pub fn shizuku(&self) {
        let _ = self.tx.send(Cmd::Shizuku);
    }
}

struct Worker {
    sys: Sys,
    dir: PathBuf,
    token: String,
    shared: Arc<Mutex<Status>>,
    repaint: Box<dyn Fn() + Send>,
}

/// The shell line that puts the helper in place and starts it. An older copy is stopped by its
/// process *name* (`pkill -x`): matching on the whole command line (`-f`) would also match the shell
/// running this very line, because the line itself mentions the helper's name, and the shell would
/// kill itself before doing anything (which is exactly what the first version did).
pub fn start_script(lib_dir: &str, token_file: &str) -> String {
    let d = "/data/local/tmp";
    format!(
        "pkill -x {HELPER_FILE}; cp '{lib_dir}/libsysmon.so' {d}/{HELPER_FILE} && chmod 755 {d}/{HELPER_FILE} && (nohup {d}/{HELPER_FILE} --interval 30 --log {d}/{HELPER_FILE}.log --api {HELPER_PORT} --token-file '{token_file}' < /dev/null > /dev/null 2>&1 &); sleep 1; echo started; ls -l {d}/{HELPER_FILE} 2>&1"
    )
}

/// What to ask the phone when the helper did not answer, so the message says why.
pub fn why_not_script() -> String {
    format!("ls -l /data/local/tmp/{HELPER_FILE} 2>&1; ps -A | grep {HELPER_FILE}; tail -n 5 /data/local/tmp/{HELPER_FILE}.log 2>&1")
}

/// Words for people, from what went wrong.
pub fn explain(e: &io::Error) -> String {
    match e.kind() {
        io::ErrorKind::PermissionDenied => "The phone refused: the code was wrong, or this app is no longer paired. Start the setup again.".into(),
        io::ErrorKind::ConnectionRefused => "The phone is not accepting connections. Is Wireless debugging still on?".into(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => "The phone did not answer in time. Keep the Wireless debugging screen open and try again.".into(),
        _ => format!("{e}"),
    }
}

impl Worker {
    fn set(&self, step: Step, message: impl Into<String>) {
        let message = message.into();
        heartnet_kit::diag::log(&format!("detailed mode: {step:?}: {message}"));
        if let Ok(mut s) = self.shared.lock() {
            s.step = step;
            s.message = message.clone();
            if s.log.last() != Some(&message) {
                s.log.push(message);
            }
            if s.log.len() > 40 {
                s.log.remove(0);
            }
        }
        (self.repaint)();
    }

    fn key(&self) -> io::Result<AdbKey> {
        let path = self.dir.join("adb-key.pk8");
        if let Ok(der) = std::fs::read(&path) {
            if let Ok(k) = AdbKey::from_pkcs8(&der, KEY_NAME) {
                return Ok(k);
            }
        }
        self.set(Step::Pairing, "Making this app's key (a few seconds, once)…");
        let k = AdbKey::generate(KEY_NAME).map_err(|e| io::Error::other(e.to_string()))?;
        let _ = std::fs::create_dir_all(&self.dir);
        std::fs::write(&path, &k.pkcs8)?;
        Ok(k)
    }

    fn wait_port(&self, pairing: bool, timeout: Duration) -> Option<u16> {
        let t0 = Instant::now();
        loop {
            let (c, p) = self.sys.nsd_ports();
            let port = if pairing { p } else { c };
            if port != 0 {
                return Some(port);
            }
            if t0.elapsed() > timeout {
                return None;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    fn run(self, rx: Receiver<Cmd>) {
        let mut waiting = false;
        while let Ok(cmd) = rx.recv() {
            match cmd {
                Cmd::Setup => {
                    if let Err(e) = self.key() {
                        self.set(Step::Failed, format!("Could not make a key: {e}"));
                        continue;
                    }
                    waiting = true;
                    self.sys.nsd_start();
                    self.sys.show_pairing("Pairing code needed", "The code is in Settings > Developer options > Wireless debugging > \"Pair device with pairing code\". It shows six digits in a pop-up. Type those six digits here.", true);
                    self.set(Step::WaitingForCode, "Waiting for the pairing code…");
                }
                Cmd::Code(code) => {
                    if !waiting {
                        continue;
                    }
                    waiting = false;
                    self.sys.show_pairing("Pairing…", "Please wait.", false);
                    match self.pair_and_start(&code) {
                        Ok(()) => {
                            self.sys.show_pairing("Detailed mode is on", "Open Task Manager.", false);
                            self.set(Step::Running, "Detailed mode is on.");
                        }
                        Err(msg) => {
                            self.sys.show_pairing("Setup did not finish", &msg, false);
                            self.set(Step::Failed, msg);
                        }
                    }
                    self.sys.nsd_stop();
                }
                Cmd::Shizuku => match self.start_with_shizuku() {
                    Ok(()) => self.set(Step::Running, "Detailed mode is on (through Shizuku)."),
                    Err(msg) => self.set(Step::Failed, msg),
                },
                Cmd::Connect => {
                    self.sys.nsd_start();
                    match self.connect_and_start() {
                        Ok(()) => self.set(Step::Running, "Detailed mode is on."),
                        Err(msg) => self.set(Step::Failed, msg),
                    }
                    self.sys.nsd_stop();
                }
            }
        }
    }

    fn pair_and_start(&self, code: &str) -> Result<(), String> {
        let code = code.trim();
        if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
            return Err("The code is six digits.".into());
        }
        let key = self.key().map_err(|e| explain(&e))?;
        self.set(Step::Pairing, "Looking for the pairing port…");
        let port = self.wait_port(true, Duration::from_secs(6)).ok_or("Could not find the pairing window. Keep \"Pair device with pairing code\" open and enter the code again.")?;
        self.set(Step::Pairing, format!("Pairing (port {port})…"));
        let phone = pair(("127.0.0.1", port), code, &key).map_err(|e| explain(&e))?;
        heartnet_kit::diag::log(&format!("paired with {:?}", phone.text()));
        self.connect_and_start()
    }

    fn connect_and_start(&self) -> Result<(), String> {
        let key = self.key().map_err(|e| explain(&e))?;
        self.set(Step::Connecting, "Looking for the debugging port…");
        let port = self.wait_port(false, Duration::from_secs(10)).ok_or("Wireless debugging does not seem to be on. Turn it on in Developer options (it needs Wi-Fi) and try again.")?;
        self.set(Step::Connecting, format!("Connecting (port {port})…"));
        let mut conn = adbwire::conn::Conn::connect(("127.0.0.1", port), &key).map_err(|e| explain(&e))?;

        self.launch_helper(&mut |script| conn.shell(script).map_err(|e| explain(&e)))
    }

    /// The other way in: Shizuku, if it is installed, running, and this app is allowed to use it.
    fn start_with_shizuku(&self) -> Result<(), String> {
        self.set(Step::Starting, "Checking Shizuku…");
        let mut state = self.sys.shizuku_state();
        match shizuku_step(state) {
            ShizukuStep::Install => return Err("Shizuku is not installed.".into()),
            ShizukuStep::StartIt => return Err("Shizuku is installed but not running. Open it and start it first.".into()),
            ShizukuStep::Allow => {
                self.set(Step::Starting, "Allow Task Manager in the Shizuku window…");
                self.sys.shizuku_request();
                let t0 = Instant::now();
                while state & SHIZUKU_GRANTED == 0 && t0.elapsed() < Duration::from_secs(60) {
                    std::thread::sleep(Duration::from_millis(500));
                    state = self.sys.shizuku_state();
                }
                if state & SHIZUKU_GRANTED == 0 {
                    return Err("Shizuku did not allow Task Manager.".into());
                }
            }
            ShizukuStep::Ready => {}
        }
        self.set(Step::Starting, "Starting the helper through Shizuku…");
        self.launch_helper(&mut |script| {
            let out = self.sys.shizuku_run(script);
            match out.strip_prefix("ERROR:") {
                Some(e) => Err(format!("Shizuku could not run it:{e}")),
                None => Ok(out),
            }
        })
    }

    /// Put the helper in place and start it with `run` (the phone's shell, reached some way), then
    /// check that it answers.
    fn launch_helper(&self, run: &mut dyn FnMut(&str) -> Result<String, String>) -> Result<(), String> {
        self.set(Step::Starting, "Starting the helper…");
        let token_file = self.sys.write_token(&self.token).ok_or("Could not save the helper's token.")?;
        let script = start_script(&self.sys.native_lib_dir(), &token_file);
        let out = run(&script)?;
        heartnet_kit::diag::log(&format!("start script said {out:?}"));
        let client = crate::hclient::Client { token: self.token.clone() };
        for _ in 0..12 {
            if client.request("GET", "/v1/ping", "", Duration::from_secs(1)).is_ok() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        let why = run(&why_not_script()).unwrap_or_default();
        Err(format!("The helper did not start.\nStart line said: {}\nChecking afterwards: {}", out.trim().chars().take(300).collect::<String>(), why.trim().chars().take(400).collect::<String>()))
    }
}

pub const SHIZUKU_INSTALLED: u32 = 1;
pub const SHIZUKU_RUNNING: u32 = 2;
pub const SHIZUKU_GRANTED: u32 = 4;

/// Where the person is with Shizuku, as what to do next.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ShizukuStep {
    Install,
    StartIt,
    Allow,
    Ready,
}

pub fn shizuku_step(state: u32) -> ShizukuStep {
    if state & SHIZUKU_INSTALLED == 0 && state & SHIZUKU_RUNNING == 0 {
        ShizukuStep::Install
    } else if state & SHIZUKU_RUNNING == 0 {
        ShizukuStep::StartIt
    } else if state & SHIZUKU_GRANTED == 0 {
        ShizukuStep::Allow
    } else {
        ShizukuStep::Ready
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_start_line_copies_runs_and_cannot_kill_itself() {
        let s = start_script("/data/app/x/lib/arm64", "/sdcard/Android/data/p/files/token");
        assert!(s.contains("cp '/data/app/x/lib/arm64/libsysmon.so' /data/local/tmp/tm-sysmon"));
        assert!(s.contains("--api 7880"));
        assert!(s.contains("--token-file '/sdcard/Android/data/p/files/token'"));
        // The first version used `pkill -f`, which matched the shell running the line itself (the
        // line mentions the helper's name), so the shell killed itself and nothing was started.
        assert!(!s.contains("pkill -f"), "never match on the whole command line");
        assert!(s.starts_with("pkill -x tm-sysmon;"), "stop an older copy by exact process name");
        assert!(s.contains("echo started"));
        // the helper runs detached (nohup, no input, in a subshell) so it outlives the shell that started it
        assert!(s.contains("(nohup /data/local/tmp/tm-sysmon "));
        assert!(s.contains("< /dev/null > /dev/null 2>&1 &)"));
    }

    #[test]
    fn the_failure_check_looks_at_the_file_the_process_and_the_log() {
        let s = why_not_script();
        assert!(s.contains("ls -l /data/local/tmp/tm-sysmon"));
        assert!(s.contains("ps -A | grep tm-sysmon"));
        assert!(s.contains("tm-sysmon.log"));
    }

    #[test]
    fn shizuku_progress_maps_to_the_next_thing_to_do() {
        assert_eq!(shizuku_step(0), ShizukuStep::Install);
        assert_eq!(shizuku_step(SHIZUKU_INSTALLED), ShizukuStep::StartIt);
        assert_eq!(shizuku_step(SHIZUKU_INSTALLED | SHIZUKU_RUNNING), ShizukuStep::Allow);
        assert_eq!(shizuku_step(SHIZUKU_INSTALLED | SHIZUKU_RUNNING | SHIZUKU_GRANTED), ShizukuStep::Ready);
        // running implies installed even if the package check was blocked
        assert_eq!(shizuku_step(SHIZUKU_RUNNING), ShizukuStep::Allow);
        assert_eq!(shizuku_step(SHIZUKU_RUNNING | SHIZUKU_GRANTED), ShizukuStep::Ready);
    }

    #[test]
    fn failures_are_explained_in_plain_words() {
        assert!(explain(&io::Error::new(io::ErrorKind::PermissionDenied, "x")).contains("code was wrong"));
        assert!(explain(&io::Error::new(io::ErrorKind::ConnectionRefused, "x")).contains("Wireless debugging"));
        assert!(explain(&io::Error::new(io::ErrorKind::TimedOut, "x")).contains("did not answer"));
        assert_eq!(explain(&io::Error::other("odd")), "odd");
    }

    #[test]
    fn a_code_that_is_not_six_digits_is_refused_before_anything_happens() {
        let w = Worker { sys: Sys::default(), dir: std::env::temp_dir().join("tm-adv-test"), token: "t".into(), shared: Arc::new(Mutex::new(Status { step: Step::Idle, message: String::new(), log: vec![] })), repaint: Box::new(|| {}) };
        assert!(w.pair_and_start("12345").unwrap_err().contains("six digits"));
        assert!(w.pair_and_start("12345a").unwrap_err().contains("six digits"));
    }
}
