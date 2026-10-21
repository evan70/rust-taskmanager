//! Parsing and safety rules shared by the PC debloater and the on-phone helper, so both read
//! `dumpsys`/logcat output and decide what may be touched in exactly the same way.

use std::collections::{HashMap, HashSet};

/// Android package names are letters, digits, dots and underscores. Anything else is refused
/// before it can reach a shell command.
pub fn valid_pkg(p: &str) -> bool {
    !p.is_empty() && p.len() < 200 && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
}

pub struct BootStart {
    pub trigger: String,
    pub component: String,
    pub after_s: i64,
}

/// "MM-dd HH:mm:ss.fff" at the start of a logcat line -> seconds since midnight.
fn seconds_of_day(line: &str) -> Option<f64> {
    let t = line.get(6..18)?; // "HH:mm:ss.fff"
    let h: f64 = t.get(0..2)?.parse().ok()?;
    let m: f64 = t.get(3..5)?.parse().ok()?;
    let s: f64 = t.get(6..12)?.parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + s)
}

/// First process each package got within `window` seconds of the BOOT_COMPLETED broadcast,
/// read from the system log's `ActivityManager: Start proc` lines.
pub fn parse_boot_starts(log: &str, window: f64) -> HashMap<String, BootStart> {
    let t0 = log
        .lines()
        .find(|l| l.contains("broadcastIntent-0-android.intent.action.BOOT_COMPLETED"))
        .and_then(seconds_of_day);
    let mut out = HashMap::new();
    let Some(t0) = t0 else { return out };
    for l in log.lines().filter(|l| l.contains("ActivityManager: Start proc ")) {
        let Some(t) = seconds_of_day(l) else { continue };
        let after = t - t0;
        if !(-2.0..=window).contains(&after) {
            continue;
        }
        let Some(rest) = l.split("Start proc ").nth(1) else { continue };
        let Some(name) = rest.split_whitespace().next() else { continue };
        let Some(name) = name.split('/').next().and_then(|n| n.split_once(':')).map(|x| x.1) else { continue };
        let pkg = name.split(':').next().unwrap_or(name).to_string();
        let Some(after_for) = rest.split(" for ").nth(1) else { continue };
        let trigger = after_for.split(" {").next().unwrap_or("").split(" caller=").next().unwrap_or("").trim().to_string();
        let component = after_for
            .split('{')
            .nth(1)
            .and_then(|c| c.split('}').next())
            .map(|c| c.rsplit('.').next().unwrap_or(c).to_string())
            .unwrap_or_default();
        out.entry(pkg).or_insert(BootStart { trigger, component, after_s: after as i64 });
    }
    out
}

/// `dumpsys package` -> package -> (stopped, enabled-state code) for user 0.
pub fn parse_package_flags(dump: &str) -> HashMap<String, (bool, u8)> {
    let mut out = HashMap::new();
    let mut current: Option<String> = None;
    for l in dump.lines() {
        let t = l.trim_start();
        if let Some(rest) = t.strip_prefix("Package [") {
            current = rest.split(']').next().map(str::to_string);
        } else if t.starts_with("User 0:") {
            if let (Some(p), Some(si)) = (&current, t.find("stopped=")) {
                let stopped = t[si + 8..].starts_with("true");
                let enabled = t
                    .find("enabled=")
                    .and_then(|i| t[i + 8..].chars().next())
                    .and_then(|c| c.to_digit(10))
                    .unwrap_or(0) as u8;
                out.entry(p.clone()).or_insert((stopped, enabled));
            }
        }
    }
    out
}

/// `dumpsys meminfo` summary lines ("  620,576K: name (pid 1)") -> package -> MB.
pub fn parse_meminfo(text: &str) -> HashMap<String, u32> {
    let mut out: HashMap<String, u32> = HashMap::new();
    // The dump lists every process again under "by OOM adjustment" and "by category";
    // only the first section ("by process") may be counted.
    let mut in_section = false;
    for l in text.lines() {
        let t = l.trim_start();
        if t.starts_with("Total PSS by process") {
            in_section = true;
            continue;
        }
        if in_section && (t.is_empty() || t.starts_with("Total ")) {
            break;
        }
        if !in_section {
            continue;
        }
        let Some((kb, rest)) = t.split_once("K: ") else { continue };
        let Ok(kb) = kb.replace(',', "").parse::<u64>() else { continue };
        let Some(name) = rest.split_whitespace().next() else { continue };
        let pkg = name.split(':').next().unwrap_or(name);
        *out.entry(pkg.to_string()).or_default() += (kb / 1024) as u32;
    }
    out
}

/// Packages that declare a BOOT_COMPLETED receiver.
pub fn parse_boot_receivers(out: &str) -> HashSet<String> {
    out.lines()
        .filter_map(|l| {
            let t = l.trim();
            let (pkg, rest) = t.split_once('/')?;
            let ok_pkg = !pkg.is_empty() && pkg.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_');
            (ok_pkg && !rest.is_empty() && !t.contains(' ')).then(|| pkg.to_string())
        })
        .collect()
}

/// Why an app must never be stopped or frozen by these tools, if it is one of those apps.
/// `roles` holds the keyboard, launcher, phone and messages apps, found on the device.
pub fn protected_reason(pkg: &str, roles: &HashMap<String, &'static str>) -> Option<&'static str> {
    if pkg == "com.android.deskclock" {
        return Some("alarm clock");
    }
    let system_ish = ["com.android.", "android.", "com.miui.", "com.xiaomi.", "com.mi.", "com.qualcomm.", "com.qti."];
    if system_ish.iter().any(|p| pkg.starts_with(p)) || pkg.starts_with("com.google.android.gms") || pkg == "com.google.android.gsf" {
        return Some("system component");
    }
    roles.get(pkg).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = "\
10-21 11:39:51.354  2234  2409 D SystemServerTiming: broadcastIntent-0-android.intent.action.LOCKED_BOOT_COMPLETED
10-21 11:42:26.076  2234  2393 D SystemServerTiming: broadcastIntent-0-android.intent.action.BOOT_COMPLETED
10-21 11:42:30.985  2234  3940 I ActivityManager: Start proc 10269:com.facebook.katana/u0a311 for content provider {com.facebook.katana/com.facebook.platform.common.provider.PlatformProvider} caller=com.x
10-21 11:42:40.944  2234  2433 I ActivityManager: Start proc 12656:com.example.bank:remote/u0a363 for broadcast {com.example.bank/com.pravera.BootReceiver} caller=android
10-21 11:42:41.944  2234  2433 I ActivityManager: Start proc 12700:com.example.bank/u0a363 for service {com.example.bank/com.pravera.Service} caller=android
10-21 11:45:59.000  2234  2433 I ActivityManager: Start proc 13000:com.late.app/u0a400 for broadcast {com.late.app/com.late.Receiver} caller=android
10-21 11:42:20.000  2234  2433 I ActivityManager: Start proc 9000:com.early.app/u0a401 for broadcast {com.early.app/com.early.Receiver} caller=android
";

    #[test]
    fn boot_starts_are_read_relative_to_boot_completed() {
        let b = parse_boot_starts(LOG, 90.0);
        assert_eq!(b.len(), 2, "late and early starts are outside the window");
        let f = &b["com.facebook.katana"];
        assert_eq!(f.trigger, "content provider");
        assert_eq!(f.component, "PlatformProvider");
        assert_eq!(f.after_s, 4);
        let t = &b["com.example.bank"];
        assert_eq!((t.trigger.as_str(), t.component.as_str()), ("broadcast", "BootReceiver"), "first start wins; :remote maps to the package");
        assert!(parse_boot_starts("no boot marker here", 90.0).is_empty());
    }

    #[test]
    fn package_flags_come_from_the_user_0_line() {
        let dump = "\
Packages:
  Package [com.a.one] (abc):
    userId=10100
    User 0: ceDataInode=1 installed=true hidden=false suspended=false stopped=true notLaunched=false enabled=0 instant=false
  Package [com.b.two] (def):
    User 0: ceDataInode=2 installed=true stopped=false notLaunched=false enabled=3 instant=false
    User 10: installed=true stopped=true enabled=0
";
        let f = parse_package_flags(dump);
        assert_eq!(f["com.a.one"], (true, 0));
        assert_eq!(f["com.b.two"], (false, 3), "disabled-user is state 3; other users are ignored");
    }

    #[test]
    fn meminfo_sums_processes_per_package() {
        let t = "Total PSS by process:\n    620,576K: system (pid 2339)\n    204,800K: com.x.app (pid 5 / activities)\n    102,400K: com.x.app:remote (pid 6)\n\nTotal PSS by OOM adjustment:\n    900,000K: Foreground\n        204,800K: com.x.app (pid 5 / activities)\n\nTotal PSS by category:\n    100,000K: Native\n";
        let m = parse_meminfo(t);
        assert_eq!(m["system"], 606);
        assert_eq!(m["com.x.app"], 300, "later sections must not be added again");
        assert_eq!(m.len(), 2, "category and OOM rows are not processes");
    }

    #[test]
    fn boot_receivers_are_read_from_component_names() {
        let out = "2 receivers:\n  com.example.work/com.example.WorkReceiver\n  com.example.pay/.BootReceiver\nnot a component line\n";
        let r = parse_boot_receivers(out);
        assert!(r.contains("com.example.work") && r.contains("com.example.pay"));
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn protection_rules() {
        let mut roles = HashMap::new();
        roles.insert("com.my.keyboard".to_string(), "your keyboard");
        assert_eq!(protected_reason("com.android.deskclock", &roles), Some("alarm clock"));
        assert_eq!(protected_reason("com.miui.notes", &roles), Some("system component"));
        assert_eq!(protected_reason("com.google.android.gms.location", &roles), Some("system component"));
        assert_eq!(protected_reason("com.my.keyboard", &roles), Some("your keyboard"));
        assert_eq!(protected_reason("com.instagram.android", &roles), None);
    }

    #[test]
    fn package_names_are_validated() {
        assert!(valid_pkg("com.foo.bar_baz1"));
        for bad in ["", "a;b", "a b", "$(x)", "a&&b", "a|b", "a`b"] {
            assert!(!valid_pkg(bad), "{bad:?}");
        }
    }
}
