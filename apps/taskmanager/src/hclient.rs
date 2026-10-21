//! Talks to the helper (rust-sysmon --api) on 127.0.0.1 with plain HTTP over a local socket.

use crate::hdata::{ActResults, History, Snapshot, StartupData};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const PORT: u16 = 7880;

#[derive(Clone)]
pub struct Client {
    pub token: String,
}

/// Split a raw HTTP/1.0 response into (status code, body).
pub fn split_http(raw: &[u8]) -> Result<(u16, String), String> {
    let text = String::from_utf8_lossy(raw);
    let (head, body) = text.split_once("\r\n\r\n").ok_or("incomplete reply from the helper")?;
    let code = head.lines().next().and_then(|l| l.split_whitespace().nth(1)).and_then(|c| c.parse().ok()).ok_or("bad reply from the helper")?;
    Ok((code, body.to_string()))
}

impl Client {
    pub fn request(&self, method: &str, path: &str, body: &str, timeout: Duration) -> Result<String, String> {
        let addr = SocketAddr::from(([127, 0, 0, 1], PORT));
        let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(800)).map_err(|_| "helper not running".to_string())?;
        s.set_read_timeout(Some(timeout)).map_err(|e| e.to_string())?;
        s.set_write_timeout(Some(Duration::from_secs(2))).map_err(|e| e.to_string())?;
        let req = format!("{method} {path} HTTP/1.0\r\nHost: 127.0.0.1\r\nX-Token: {}\r\nContent-Length: {}\r\n\r\n{body}", self.token, body.len());
        s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
        let mut raw = Vec::new();
        s.read_to_end(&mut raw).map_err(|e| format!("helper did not answer: {e}"))?;
        let (code, body) = split_http(&raw)?;
        match code {
            200 => Ok(body),
            403 => Err("helper refused the token (switch detailed mode off and on again)".into()),
            c => Err(format!("helper error {c}")),
        }
    }

    pub fn snapshot(&self) -> Result<Snapshot, String> {
        let body = self.request("GET", "/v1/snapshot", "", Duration::from_secs(3))?;
        serde_json::from_str(&body).map_err(|e| format!("unreadable snapshot: {e}"))
    }

    pub fn startup(&self) -> Result<StartupData, String> {
        let body = self.request("GET", "/v1/startup", "", Duration::from_secs(40))?;
        serde_json::from_str(&body).map_err(|e| format!("unreadable startup data: {e}"))
    }

    pub fn act(&self, op: &str, pkgs: &[String]) -> Result<ActResults, String> {
        let body = self.request("POST", &format!("/v1/act?op={op}"), &pkgs.join("\n"), Duration::from_secs(60))?;
        serde_json::from_str(&body).map_err(|e| format!("unreadable reply: {e}"))
    }
}

/// Everything the background thread keeps up to date for the screen.
#[derive(Default)]
pub struct Live {
    pub snap: Option<Snapshot>,
    pub hist: History,
    pub error: Option<String>,
    pub startup: Option<StartupData>,
    pub startup_busy: bool,
    pub note: Option<String>,
}

pub type Shared = Arc<Mutex<Live>>;

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// Poll the helper once a second, but only while the screen is being drawn: `last_frame` is
/// stamped by the UI, so a backgrounded app stops asking and the helper goes back to sleep.
pub fn spawn_poller(client: Client, shared: Shared, last_frame: Arc<AtomicU64>, repaint: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(1));
            if now_ms().saturating_sub(last_frame.load(Ordering::Relaxed)) > 3000 {
                continue;
            }
            let result = client.snapshot();
            {
                let mut live = shared.lock().unwrap();
                match result {
                    Ok(s) if s.warming_up || s.cpu.cores.is_empty() => {}
                    Ok(s) => {
                        live.hist.add(&s);
                        live.snap = Some(s);
                        live.error = None;
                    }
                    Err(e) => live.error = Some(e),
                }
            }
            repaint();
        }
    });
}

pub fn spawn_startup_load(client: Client, shared: Shared, repaint: impl Fn() + Send + 'static) {
    {
        let mut live = shared.lock().unwrap();
        if live.startup_busy {
            return;
        }
        live.startup_busy = true;
    }
    std::thread::spawn(move || {
        let result = client.startup();
        let mut live = shared.lock().unwrap();
        live.startup_busy = false;
        match result {
            Ok(d) => live.startup = Some(d),
            Err(e) => live.note = Some(e),
        }
        drop(live);
        repaint();
    });
}

pub fn spawn_act(client: Client, shared: Shared, op: String, pkgs: Vec<String>, repaint: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        let note = match client.act(&op, &pkgs) {
            Ok(r) => {
                let bad: Vec<_> = r.results.iter().filter(|x| !x.ok).collect();
                let ok = r.results.len() - bad.len();
                let mut s = format!("{ok} done, {} refused or failed", bad.len());
                if let Some(first) = bad.first() {
                    s += &format!(" ({}: {})", first.p, first.msg);
                }
                s
            }
            Err(e) => e,
        };
        shared.lock().unwrap().note = Some(note);
        // refresh the list so the new states show
        match client.startup() {
            Ok(d) => shared.lock().unwrap().startup = Some(d),
            Err(e) => shared.lock().unwrap().note = Some(e),
        }
        repaint();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_replies_are_split() {
        let (code, body) = split_http(b"HTTP/1.0 200 OK\r\nContent-Length: 11\r\n\r\n{\"ok\":true}").unwrap();
        assert_eq!((code, body.as_str()), (200, "{\"ok\":true}"));
        let (code, _) = split_http(b"HTTP/1.0 403 Forbidden\r\n\r\n{}").unwrap();
        assert_eq!(code, 403);
        assert!(split_http(b"garbage").is_err());
        assert!(split_http(b"HTTP/1.0 xyz\r\n\r\nbody").is_err());
    }

    #[test]
    fn unreachable_helper_is_reported_plainly() {
        // nothing listens on the helper port in the test environment
        let c = Client { token: "t".into() };
        if let Err(e) = c.request("GET", "/v1/ping", "", Duration::from_millis(300)) {
            assert!(e.contains("helper"), "{e}");
        }
    }
}
