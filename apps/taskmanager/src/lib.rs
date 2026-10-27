//! Task Manager: how busy the phone is (memory, processor, battery, storage, network), which apps
//! you use and how big they are, and which apps start themselves at boot. It uses only what Android
//! lets an ordinary app see, so it needs no helper, no computer and no network.

#![allow(deprecated)]

mod adv;
mod hclient;
mod hdata;
mod sample;
mod sys;
mod ui_adv;

use eframe::egui::{self, Color32, Context, RichText, Sense, Stroke, pos2, vec2};
use heartnet_kit::settings::Settings;
use heartnet_kit::theme::{BG, CARD, MUTED, ORANGE};
use domain::{DiskInfo, MemoryInfo};
use linux_proc::{CpuTimes, cpu_percent, cpu_times, meminfo, self_cpu_percent};
use sample::*;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use sys::{Sys, read_file};

const GOOD: Color32 = Color32::from_rgb(90, 200, 130);
const WARN: Color32 = Color32::from_rgb(240, 190, 70);
const BAD: Color32 = Color32::from_rgb(240, 100, 90);
const REFRESH_MS: [u64; 3] = [1000, 2000, 5000];
const ICON_PX: usize = 96;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Overview,
    Processes,
    Startup,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Main,
    Settings,
    Diagnostics,
    Setup,
}

/// The counters from the previous reading, so changes can become percentages and speeds.
struct Prev {
    at: Instant,
    cpu: Vec<CpuTimes>,
    self_ticks: u64,
    rx: u64,
    tx: u64,
}

pub struct TaskMgr {
    sys: Sys,
    ctx: Context,
    settings: Settings,
    tab: Tab,
    screen: Screen,
    // live numbers
    sample: Sample,
    kv: HashMap<String, String>,
    device: HashMap<String, String>,
    history: History,
    prev: Option<Prev>,
    last_sample: Option<Instant>,
    // icons
    icons: HashMap<String, Option<egui::TextureHandle>>,
    // startup
    boots: Vec<BootRow>,
    boots_loaded: bool,
    bfilter: AppFilter,
    bquery: String,
    // detailed mode (the helper)
    adv: adv::Adv,
    live: hclient::Shared,
    client: hclient::Client,
    last_frame: Arc<AtomicU64>,
    psort: hdata::ProcSort,
    pquery: String,
    expanded: Option<u32>,
    hsfilter: hdata::StartupFilter,
    hsquery: String,
    selected: HashSet<String>,
    confirm: Option<(String, Instant)>,
    startup_requested: bool,
    code_input: String,
    shizuku: u32,
    shizuku_at: Option<Instant>,
    // misc
    had_focus: bool,
    notice: Option<(String, Instant)>,
    insets: (f32, f32),
    #[cfg(target_os = "android")]
    android: Option<android_activity::AndroidApp>,
}

/// The secret the app and the helper share (kept in the app's own folder).
fn load_or_make_token(dir: &std::path::Path) -> String {
    let path = dir.join("helper-token");
    if let Ok(t) = std::fs::read_to_string(&path) {
        let t = t.trim().to_string();
        if t.len() >= 16 {
            return t;
        }
    }
    use rand::RngCore;
    let mut b = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut b);
    let token: String = b.iter().map(|x| format!("{x:02x}")).collect();
    let _ = std::fs::create_dir_all(dir);
    let _ = std::fs::write(&path, &token);
    token
}

/// Where the desktop preview keeps its settings, logs and token. On a phone those live in the folder
/// Android gives the activity, so this only exists off Android.
#[cfg(not(target_os = "android"))]
fn data_dir() -> PathBuf {
    std::env::var_os("TASKMGR_DIR").map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("taskmgr-data"))
}

impl TaskMgr {
    #[cfg(not(target_os = "android"))]
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        Self::build(cc, data_dir(), Sys)
    }

    fn build(cc: &eframe::CreationContext<'_>, dir: PathBuf, sys: Sys) -> Self {
        heartnet_kit::theme::apply(&cc.egui_ctx);
        heartnet_kit::diag::init(dir.join("logs"));
        heartnet_kit::diag::log("task manager start");
        let device = parse_kv(&sys.device_text());
        let settings = Settings::load(&dir, "settings.txt");
        // desktop preview: TASKMGR_TAB=apps|startup opens that tab first
        let tab = match std::env::var("TASKMGR_TAB").as_deref() {
            Ok("processes") => Tab::Processes,
            Ok("startup") => Tab::Startup,
            _ => Tab::Overview,
        };
        let token = load_or_make_token(&dir);
        let client = hclient::Client { token: token.clone() };
        let live: hclient::Shared = Default::default();
        let last_frame = Arc::new(AtomicU64::new(hclient::now_ms()));
        let repaint = cc.egui_ctx.clone();
        hclient::spawn_poller(client.clone(), live.clone(), last_frame.clone(), move || repaint.request_repaint());
        let repaint = cc.egui_ctx.clone();
        let adv = adv::Adv::new(sys.clone(), dir.clone(), token, move || repaint.request_repaint());
        // set up before: try to reconnect once, quietly (it needs Wireless debugging to be on)
        if settings.bool("detailed", false) {
            adv.connect();
        }
        Self {
            sys,
            ctx: cc.egui_ctx.clone(),
            settings,
            tab,
            screen: Screen::Main,
            sample: Sample::default(),
            kv: HashMap::new(),
            device,
            history: History::default(),
            prev: None,
            last_sample: None,
            icons: HashMap::new(),
            boots: Vec::new(),
            boots_loaded: false,
            bfilter: AppFilter::User,
            bquery: String::new(),
            adv,
            live,
            client,
            last_frame,
            psort: hdata::ProcSort::Cpu,
            pquery: String::new(),
            // desktop preview only: TASKMGR_OPEN=<pid> opens that process's panel first
            expanded: std::env::var("TASKMGR_OPEN").ok().and_then(|v| v.parse().ok()),
            hsfilter: hdata::StartupFilter::Woke,
            hsquery: String::new(),
            selected: HashSet::new(),
            confirm: None,
            startup_requested: false,
            code_input: String::new(),
            shizuku: 0,
            shizuku_at: None,
            had_focus: true,
            notice: None,
            insets: (0.0, 0.0),
            #[cfg(target_os = "android")]
            android: None,
        }
    }

    #[cfg(target_os = "android")]
    pub fn with_android(mut self, app: android_activity::AndroidApp) -> Self {
        heartnet_kit::android::install_lifecycle(&app, &self.ctx);
        self.android = Some(app);
        self
    }

    fn say(&mut self, msg: impl Into<String>) {
        self.notice = Some((msg.into(), Instant::now()));
    }

    fn insets(&self, _ctx: &Context) -> (f32, f32) {
        #[cfg(target_os = "android")]
        if let Some(app) = &self.android {
            return heartnet_kit::android::insets(app, _ctx);
        }
        (0.0, 0.0)
    }

    fn fahrenheit(&self) -> bool {
        self.settings.bool("fahrenheit", false)
    }

    fn refresh(&self) -> Duration {
        Duration::from_millis(REFRESH_MS[self.settings.int("refresh", 1).clamp(0, 2) as usize])
    }

    fn reload_startup(&self) {
        let ctx = self.ctx.clone();
        hclient::spawn_startup_load(self.client.clone(), self.live.clone(), move || ctx.request_repaint());
    }

    fn run_act(&mut self, op: &str) {
        let pkgs: Vec<String> = self.selected.iter().cloned().collect();
        if pkgs.is_empty() {
            return;
        }
        let ctx = self.ctx.clone();
        hclient::spawn_act(self.client.clone(), self.live.clone(), op.to_string(), pkgs, move || ctx.request_repaint());
        self.selected.clear();
        self.say("Working...");
    }

    // ---------- taking a reading ----------

    fn take_sample(&mut self) {
        let now = Instant::now();
        let kv = parse_kv(&self.sys.sample_text());
        let mut s = Sample {
            // The kernel's own file first; on a phone that hides it, whatever the Java side could
            // measure (both are byte counts).
            mem: meminfo()
                .map(|mem| MemoryInfo::from_bytes(mem.total, mem.avail, mem.cached, mem.swap_total, mem.swap_free))
                .unwrap_or_else(|| MemoryInfo::from_bytes(
                    get_i64(&kv, "mem_total").unwrap_or(0).max(0) as u64,
                    get_i64(&kv, "mem_avail").unwrap_or(0).max(0) as u64,
                    0,
                    0,
                    0,
                )),
            ..Default::default()
        };
        s.low_memory = get_i64(&kv, "mem_low") == Some(1);
        s.battery = battery_from_kv(&kv);
        s.thermal = get_i64(&kv, "thermal").unwrap_or(-1);
        s.headroom = get_f64(&kv, "headroom");
        s.storage = DiskInfo::from_bytes(
            get_i64(&kv, "storage_total").unwrap_or(0).max(0) as u64,
            get_i64(&kv, "storage_free").unwrap_or(0).max(0) as u64,
        );
        s.uptime_ms = get_i64(&kv, "uptime_ms").unwrap_or(0);
        s.power_save = get_i64(&kv, "power_save") == Some(1);
        s.network.total_received_bytes = get_i64(&kv, "net_rx").unwrap_or(0).max(0) as u64;
        s.network.total_transmitted_bytes = get_i64(&kv, "net_tx").unwrap_or(0).max(0) as u64;

        let cpu_now = cpu_times();
        let self_ticks = linux_proc::self_ticks().unwrap_or(0);
        let ncores = get_i64(&self.device, "cores").unwrap_or(0).clamp(0, 32) as usize;
        let ncores = if ncores == 0 { cpu_now.len().saturating_sub(1) } else { ncores };
        if let Some(prev) = &self.prev {
            let dt = now.duration_since(prev.at).as_secs_f64();
            s.network.received_bytes_per_second = rate(prev.rx, s.network.total_received_bytes, dt) as u64;
            s.network.transmitted_bytes_per_second = rate(prev.tx, s.network.total_transmitted_bytes, dt) as u64;
            s.self_cpu = self_cpu_percent(prev.self_ticks, self_ticks, dt);
            if let (Some(p), Some(c)) = (prev.cpu.first(), cpu_now.first()) {
                s.cpu_total = cpu_percent(*p, *c);
            }
        }
        for i in 0..ncores {
            let load = match (self.prev.as_ref().and_then(|p| p.cpu.get(i + 1)), cpu_now.get(i + 1)) {
                (Some(p), Some(c)) => cpu_percent(*p, *c),
                _ => None,
            };
            let mhz = |file: &str| read_file(&format!("/sys/devices/system/cpu/cpu{i}/cpufreq/{file}")).and_then(|t| t.trim().parse::<u32>().ok()).map(|khz| khz / 1000);
            s.cores.push((load, mhz("scaling_cur_freq"), mhz("cpuinfo_max_freq")));
        }

        self.prev = Some(Prev { at: now, cpu: cpu_now, self_ticks, rx: s.network.total_received_bytes, tx: s.network.total_transmitted_bytes });
        self.history.add(&s);
        self.sample = s;
        self.kv = kv;
        self.last_sample = Some(now);
    }

    // ---------- icons ----------



    fn icon_for(&mut self, ctx: &Context, pkg: &str) -> Option<egui::TextureHandle> {
        if !self.icons.contains_key(pkg) {
            let tex = self.sys.icon(pkg, ICON_PX).map(|img| ctx.load_texture(format!("icon-{pkg}"), img, egui::TextureOptions::LINEAR));
            self.icons.insert(pkg.to_string(), tex);
        }
        self.icons.get(pkg).cloned().flatten()
    }

    // ---------- drawing helpers ----------

    fn card(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
        egui::Frame::new().fill(CARD).corner_radius(14.0).inner_margin(egui::Margin::same(14)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).size(13.0).color(MUTED));
            ui.add_space(4.0);
            add(ui);
        });
        ui.add_space(10.0);
    }

    fn spark(ui: &mut egui::Ui, values: &VecDeque<f32>, max: f32, height: f32, color: Color32) {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 6.0, Color32::from_gray(16));
        for frac in [0.25, 0.5, 0.75] {
            let y = rect.bottom() - rect.height() * frac;
            p.line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], Stroke::new(1.0, Color32::from_gray(30)));
        }
        if values.len() < 2 || max <= 0.0 {
            return;
        }
        let n = HISTORY_LEN as f32 - 1.0;
        let offset = HISTORY_LEN - values.len();
        let pts: Vec<_> = values.iter().enumerate().map(|(i, v)| pos2(rect.left() + rect.width() * (offset + i) as f32 / n, rect.bottom() - rect.height() * (v / max).clamp(0.0, 1.0))).collect();
        p.add(egui::Shape::line(pts, Stroke::new(2.0, color)));
    }

    fn bar(ui: &mut egui::Ui, frac: f32, color: Color32, height: f32) {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
        ui.painter().rect_filled(rect, height / 2.0, Color32::from_gray(38));
        let mut fill = rect;
        fill.set_width(rect.width() * frac.clamp(0.0, 1.0));
        ui.painter().rect_filled(fill, height / 2.0, color);
    }

    fn load_color(pct: f32) -> Color32 {
        if pct < 60.0 { GOOD } else if pct < 85.0 { WARN } else { BAD }
    }

    fn chip(ui: &mut egui::Ui, text: &str, on: bool) -> bool {
        let b = egui::Button::new(RichText::new(text).size(14.0)).fill(if on { ORANGE } else { CARD }).corner_radius(16.0).min_size(vec2(0.0, 32.0));
        ui.add(b).clicked()
    }

    fn row(ui: &mut egui::Ui, label: &str, value: &str) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(label).color(MUTED));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(value));
        });
    }

    // ---------- the Overview tab ----------

    fn overview(&mut self, ui: &mut egui::Ui) {
        let s = self.sample.clone();
        let f = self.fahrenheit();
        let detailed = self.detailed();
        let mut open_setup = false;
        egui::ScrollArea::vertical().id_salt("overview").auto_shrink([false; 2]).show(ui, |ui| {
            if detailed {
                self.detailed_cards(ui);
            } else if self.invite_card(ui, "See overall CPU load, every core, chip temperatures and each app's CPU and memory. Android hides these from ordinary apps, so this needs a one-time setup.") {
                open_setup = true;
            }
            if !detailed {
            Self::card(ui, "Memory", |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{:.0}%", s.mem.used_percent())).size(30.0).strong().color(Self::load_color(s.mem.used_percent())));
                    ui.label(RichText::new(format!("{} used of {}", fmt_bytes(mb_to_bytes(s.mem.used_mb())), fmt_bytes(mb_to_bytes(s.mem.total)))).color(MUTED));
                });
                Self::bar(ui, s.mem.used_percent() / 100.0, Self::load_color(s.mem.used_percent()), 8.0);
                ui.add_space(6.0);
                Self::spark(ui, &self.history.mem, 100.0, 54.0, ORANGE);
                ui.add_space(4.0);
                Self::row(ui, "Available", &fmt_bytes(mb_to_bytes(s.mem.avail)));
                if s.mem.swap_total > 0 {
                    Self::row(ui, "Swap in use", &format!("{} of {}", fmt_bytes(mb_to_bytes(s.mem.swap_used_mb())), fmt_bytes(mb_to_bytes(s.mem.swap_total))));
                }
                if s.low_memory {
                    ui.label(RichText::new("Android says memory is low").color(BAD));
                }
            });

            Self::card(ui, "Processor", |ui| {
                match s.cpu_total {
                    Some(c) => {
                        ui.label(RichText::new(format!("{c:.0}%")).size(30.0).strong().color(Self::load_color(c)));
                        Self::spark(ui, &self.history.cpu, 100.0, 54.0, ORANGE);
                    }
                    None => {
                        ui.label(RichText::new("Overall load is hidden by Android on this phone").color(MUTED));
                    }
                }
                ui.add_space(6.0);
                for (i, (load, mhz, max)) in s.cores.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("Core {i}")).monospace().color(MUTED));
                        let text = match (mhz, load) {
                            (Some(m), Some(l)) => format!("{m} MHz  {l:.0}%"),
                            (Some(m), None) => format!("{m} MHz"),
                            (None, Some(l)) => format!("{l:.0}%"),
                            (None, None) => "off or hidden".to_string(),
                        };
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(RichText::new(text).monospace()));
                    });
                    let frac = match (load, mhz, max) {
                        (Some(l), _, _) => l / 100.0,
                        (None, Some(m), Some(x)) if *x > 0 => *m as f32 / *x as f32,
                        _ => 0.0,
                    };
                    Self::bar(ui, frac, ORANGE, 5.0);
                    ui.add_space(3.0);
                }
                ui.add_space(4.0);
                if s.thermal >= 0 {
                    let c = if s.thermal == 0 { GOOD } else if s.thermal <= 2 { WARN } else { BAD };
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Heat").color(MUTED));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(RichText::new(thermal_label(s.thermal)).color(c)));
                    });
                }
                Self::row(ui, "This app", &format!("{:.1}% of one core", s.self_cpu));
            });

            }
            if let Some(b) = &s.battery {
                Self::card(ui, "Battery", |ui| {
                    let low = b.percent <= 15.0 && !b.is_charging();
                    let state = if b.is_charging() && !b.plugged.label().is_empty() {
                        format!("{} ({})", b.state.label(), b.plugged.label())
                    } else {
                        b.state.label().to_string()
                    };
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("{:.0}%", b.percent)).size(30.0).strong().color(if low { BAD } else { GOOD }));
                        ui.label(RichText::new(state).color(MUTED));
                    });
                    Self::bar(ui, b.percent / 100.0, if low { BAD } else { GOOD }, 8.0);
                    ui.add_space(6.0);
                    if let Some(t) = b.temperature_c {
                        Self::row(ui, "Temperature", &fmt_temp(t, f));
                        Self::spark(ui, &self.history.temp, 60.0, 40.0, WARN);
                    }
                    if let Some(ma) = b.current_ma {
                        Self::row(ui, if b.is_charging() { "Charging current" } else { "Drawing" }, &format!("{ma} mA"));
                    }
                    if let Some(w) = b.watts() {
                        Self::row(ui, "Power", &format!("{w:.1} W"));
                    }
                    if let Some(mv) = b.voltage_mv {
                        Self::row(ui, "Voltage", &format!("{:.2} V", mv as f32 / 1000.0));
                    }
                    Self::row(ui, "Health", b.health.label());
                    if let Some(c) = b.cycles {
                        Self::row(ui, "Charge cycles", &c.to_string());
                    }
                    if let (true, Some(ms)) = (b.is_charging(), b.full_in_ms) {
                        Self::row(ui, "Full in", &fmt_duration(ms as i64));
                    }
                    if s.power_save {
                        ui.label(RichText::new("Battery saver is on").color(WARN));
                    }
                });
            }

            Self::card(ui, "Storage", |ui| {
                let disk = &s.storage;
                let pct = disk.used_percent();
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{pct:.0}%")).size(30.0).strong().color(if pct > 90.0 { BAD } else { GOOD }));
                    ui.label(RichText::new(format!("{} used of {}", hdata::gb(disk.used_mb()), hdata::gb(disk.total_mb))).color(MUTED));
                });
                Self::bar(ui, pct / 100.0, if pct > 90.0 { BAD } else { GOOD }, 8.0);
                ui.add_space(4.0);
                Self::row(ui, "Free", &hdata::gb(disk.available_mb));
            });

            Self::card(ui, "Network", |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("\u{2193} {}", fmt_speed(s.network.received_bytes_per_second as f64))).size(18.0).strong());
                    ui.label(RichText::new(format!("\u{2191} {}", fmt_speed(s.network.transmitted_bytes_per_second as f64))).size(18.0).strong().color(MUTED));
                });
                let top = self.history.down.iter().cloned().fold(100_000.0f32, f32::max);
                Self::spark(ui, &self.history.down, top, 40.0, ORANGE);
                ui.add_space(4.0);
                Self::row(ui, "Received since start", &fmt_bytes(s.network.total_received_bytes));
                Self::row(ui, "Sent since start", &fmt_bytes(s.network.total_transmitted_bytes));
            });

            Self::card(ui, "This phone", |ui| {
                for (label, key) in [("Model", "model"), ("Android", "android"), ("Kernel", "kernel"), ("Processor type", "abi")] {
                    if let Some(v) = self.device.get(key) {
                        Self::row(ui, label, v);
                    }
                }
                if let Some(c) = get_i64(&self.device, "cores") {
                    Self::row(ui, "Cores", &c.to_string());
                }
                if let Some(r) = get_f64(&self.device, "refresh") {
                    Self::row(ui, "Screen", &format!("{r:.0} Hz"));
                }
                Self::row(ui, "On for", &fmt_duration(s.uptime_ms));
            });
            ui.add_space(30.0);
        });
        if open_setup {
            self.screen = Screen::Setup;
        }
    }

    // ---------- the Apps tab ----------



    // ---------- the Startup tab ----------

    fn startup_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if !self.boots_loaded {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| ui.label(RichText::new("Loading…").color(MUTED)));
            return;
        }
        ui.label(RichText::new("These apps ask Android to start them when the phone turns on. Android may still hold some back to save battery.").small().color(MUTED));
        ui.add_space(6.0);
        ui.add(egui::TextEdit::singleline(&mut self.bquery).hint_text("Search").desired_width(f32::INFINITY).margin(egui::Margin::symmetric(12, 8)));
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            for (f, name) in [(AppFilter::User, "Your apps"), (AppFilter::System, "System"), (AppFilter::All, "All")] {
                if Self::chip(ui, name, self.bfilter == f) {
                    self.bfilter = f;
                }
            }
            if ui.add(egui::Button::new("Battery settings").corner_radius(16.0).min_size(vec2(0.0, 32.0))).clicked() {
                self.sys.battery_settings();
            }
        });
        ui.add_space(6.0);
        let list: Vec<BootRow> = visible_boot(&self.boots, self.bfilter, &self.bquery).into_iter().cloned().collect();
        if list.is_empty() {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| ui.label(RichText::new("Nothing here.").color(MUTED)));
            return;
        }
        let mut info = None;
        let row_h = 64.0;
        egui::ScrollArea::vertical().id_salt("startup").auto_shrink([false; 2]).show_rows(ui, row_h, list.len(), |ui, range| {
            for i in range {
                let r = &list[i];
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h - 4.0), Sense::click());
                ui.painter().rect_filled(rect, 12.0, if resp.is_pointer_button_down_on() { Color32::from_gray(44) } else { CARD });
                let icon_rect = egui::Rect::from_min_size(rect.left_top() + vec2(10.0, 9.0), vec2(40.0, 40.0));
                match self.icon_for(&ctx, &r.pkg) {
                    Some(tex) => {
                        ui.painter().image(tex.id(), icon_rect, egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                    }
                    None => {
                        ui.painter().rect_filled(icon_rect, 10.0, Color32::from_gray(50));
                    }
                }
                ui.painter().text(pos2(rect.left() + 62.0, rect.top() + 11.0), egui::Align2::LEFT_TOP, &r.label, egui::FontId::proportional(16.0), Color32::from_gray(235));
                ui.painter().text(pos2(rect.left() + 62.0, rect.top() + 35.0), egui::Align2::LEFT_TOP, if r.system { "System app  \u{b7}  tap for details" } else { "Tap for details" }, egui::FontId::proportional(13.0), MUTED);
                if resp.clicked() {
                    info = Some(r.pkg.clone());
                }
            }
        });
        if let Some(p) = info {
            self.sys.app_info(&p);
        }
    }

    // ---------- settings and diagnostics ----------

    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.add(egui::Button::new(RichText::new("\u{2039}").size(26.0)).frame(false).min_size(vec2(40.0, 40.0))).clicked() {
                self.screen = Screen::Main;
            }
            ui.label(RichText::new("Settings").size(24.0).strong());
        });
        ui.add_space(10.0);
        ui.label(RichText::new("How often to update").size(16.0).strong());
        ui.add_space(4.0);
        let cur = self.settings.int("refresh", 1).clamp(0, 2);
        ui.horizontal(|ui| {
            for (i, name) in ["1 second", "2 seconds", "5 seconds"].iter().enumerate() {
                if Self::chip(ui, name, cur == i as i64) {
                    self.settings.set("refresh", i);
                }
            }
        });
        ui.add_space(14.0);
        ui.label(RichText::new("Temperature").size(16.0).strong());
        ui.add_space(4.0);
        let f = self.fahrenheit();
        ui.horizontal(|ui| {
            if Self::chip(ui, "\u{b0}C", !f) {
                self.settings.set("fahrenheit", false);
            }
            if Self::chip(ui, "\u{b0}F", f) {
                self.settings.set("fahrenheit", true);
            }
        });
        ui.add_space(14.0);
        ui.label(RichText::new("Detailed mode").size(16.0).strong());
        ui.label(RichText::new("CPU load, every app's CPU and memory, chip temperatures, startup history.").small().color(MUTED));
        if ui.add(egui::Button::new("Set up or check detailed mode").corner_radius(16.0).min_size(vec2(0.0, 34.0))).clicked() {
            self.screen = Screen::Setup;
        }
        ui.add_space(14.0);
        if ui.add(egui::Button::new("What can this phone show?").corner_radius(16.0).min_size(vec2(0.0, 34.0))).clicked() {
            self.screen = Screen::Diagnostics;
        }
        ui.add_space(20.0);
        ui.label(RichText::new("Privacy").size(16.0).strong());
        ui.label(RichText::new("Nothing leaves your phone. This app has no network access and no account.").small().color(MUTED));
    }

    fn diagnostics_ui(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.add(egui::Button::new(RichText::new("\u{2039}").size(26.0)).frame(false).min_size(vec2(40.0, 40.0))).clicked() {
                self.screen = Screen::Settings;
            }
            ui.label(RichText::new("What this phone shows").size(22.0).strong());
        });
        ui.label(RichText::new("Android hides some readings from apps, and phone makers differ. A cross means this phone does not allow it.").small().color(MUTED));
        ui.add_space(8.0);
        let kv = &self.kv;
        let readable = |p: &str| read_file(p).is_some();
        let thermal_zones = (0..60).filter(|i| readable(&format!("/sys/class/thermal/thermal_zone{i}/temp"))).count();
        let checks: Vec<(&str, bool, String)> = vec![
            ("Memory use", readable("/proc/meminfo") || kv.contains_key("mem_total"), "total, available, swap".into()),
            ("Overall processor load", readable("/proc/stat"), "needs /proc/stat".into()),
            ("Processor speed per core", readable("/sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq"), "current MHz".into()),
            ("Chip temperatures", thermal_zones > 0, format!("{thermal_zones} sensors readable")),
            ("Heat level", kv.contains_key("thermal"), "Android's thermal status".into()),
            ("Heat headroom", kv.contains_key("headroom"), "how close to throttling".into()),
            ("Battery temperature", kv.contains_key("bat_temp"), "".into()),
            ("Battery current", kv.contains_key("bat_current_ua"), "how fast it charges or drains".into()),
            ("Charge cycles", kv.contains_key("bat_cycles"), "Android 14 and newer".into()),
            ("Network totals", kv.contains_key("net_rx"), "".into()),
        ];
        egui::ScrollArea::vertical().id_salt("diag").auto_shrink([false; 2]).show(ui, |ui| {
            for (name, ok, note) in checks {
                egui::Frame::new().fill(CARD).corner_radius(10.0).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(if ok { "\u{2714}" } else { "\u{2716}" }).color(if ok { GOOD } else { BAD }).size(16.0));
                        ui.vertical(|ui| {
                            ui.label(name);
                            if !note.is_empty() {
                                ui.label(RichText::new(note).small().color(MUTED));
                            }
                        });
                    });
                });
                ui.add_space(4.0);
            }
        });
    }

    fn go_back(&mut self, ctx: &Context) {
        if self.screen == Screen::Diagnostics {
            self.screen = Screen::Settings;
        } else if self.screen == Screen::Settings || self.screen == Screen::Setup {
            self.screen = Screen::Main;
        } else if self.tab != Tab::Overview {
            self.tab = Tab::Overview;
        } else {
            heartnet_kit::quit(ctx);
        }
    }
}

impl eframe::App for TaskMgr {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if ctx.input(|i| i.key_pressed(egui::Key::Escape) || i.key_pressed(egui::Key::BrowserBack)) {
            self.go_back(&ctx);
        }

        // coming back from the system's settings page: look at the access again
        let focused = ctx.input(|i| i.focused);
        if focused && !self.had_focus {
            self.boots_loaded = false;
        }
        self.had_focus = focused;

        self.last_frame.store(hclient::now_ms(), Ordering::Relaxed);
        // what the helper said about the last Stop / Freeze, so the person sees the result
        if let Some(note) = self.live.lock().ok().and_then(|mut l| l.note.take()) {
            self.say(note);
        }
        if self.adv.status().step == adv::Step::Running && !self.settings.bool("detailed", false) {
            self.settings.set("detailed", true);
        }
        if self.screen == Screen::Main {
            match self.tab {
                Tab::Startup if !self.boots_loaded => {
                    self.boots = self.sys.boot_apps();
                    self.boots_loaded = true;
                }
                _ => {}
            }
        }
        // the Overview is live; the other tabs sit still
        if self.screen == Screen::Main && self.tab == Tab::Overview {
            if self.last_sample.is_none_or(|t| t.elapsed() >= self.refresh()) {
                self.take_sample();
            }
            ctx.request_repaint_after(self.refresh());
        }

        let (top, bottom) = self.insets(&ctx);
        self.insets = (top, bottom);
        let margin = egui::Margin { left: 14, right: 14, top: top.round() as i8, bottom: bottom.round() as i8 };
        egui::CentralPanel::default().frame(egui::Frame::new().fill(BG).inner_margin(margin)).show(ui, |ui| match self.screen {
            Screen::Settings => self.settings_ui(ui),
            Screen::Diagnostics => self.diagnostics_ui(ui),
            Screen::Setup => self.setup_ui(ui),
            Screen::Main => {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Task Manager").size(22.0).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(egui::Button::new(RichText::new("Settings").size(14.0)).corner_radius(16.0).min_size(vec2(0.0, 32.0))).clicked() {
                            self.screen = Screen::Settings;
                        }
                    });
                });
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    for (t, name) in [(Tab::Overview, "Overview"), (Tab::Processes, "Processes"), (Tab::Startup, "Startup")] {
                        if Self::chip(ui, name, self.tab == t) {
                            self.tab = t;
                        }
                    }
                });
                ui.add_space(8.0);
                match self.tab {
                    Tab::Overview => self.overview(ui),
                    Tab::Processes => self.processes_ui(ui),
                    Tab::Startup => {
                        if self.detailed() {
                            self.startup_detailed_ui(ui)
                        } else {
                            self.startup_ui(ui)
                        }
                    }
                }
            }
        });
        if self.screen == Screen::Main {
            self.startup_actions(&ctx, bottom);
        }
        if self.screen == Screen::Main && self.tab == Tab::Processes {
            self.process_sheet_ui(&ctx, bottom);
        }
        if let Some((msg, at)) = self.notice.clone() {
            if at.elapsed() > Duration::from_secs(6) {
                self.notice = None;
            } else {
                let screen = ctx.viewport_rect();
                egui::Area::new(egui::Id::new(("notice", 1u8))).order(egui::Order::Tooltip).fixed_pos(pos2(screen.left() + 16.0, screen.bottom() - 90.0 - bottom)).show(&ctx, |ui| {
                    egui::Frame::new().fill(Color32::from_gray(52)).corner_radius(14.0).inner_margin(egui::Margin::symmetric(14, 10)).show(ui, |ui| ui.label(msg));
                });
                ctx.request_repaint_after(Duration::from_millis(500));
            }
        }
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: android_activity::AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info));
    let handle = app.clone();
    let dir = heartnet_kit::android::files_dir(&app);
    let options = eframe::NativeOptions { android_app: Some(app), ..Default::default() };
    if let Err(e) = eframe::run_native("Task Manager", options, Box::new(move |cc| Ok(Box::new(TaskMgr::build(cc, dir.clone(), Sys::new(handle.clone())).with_android(handle.clone()))))) {
        log::error!("task manager exited: {e}");
    }
}
