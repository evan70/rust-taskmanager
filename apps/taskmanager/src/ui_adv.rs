//! The screens that need the helper (detailed mode): CPU per core, chip temperatures, every
//! process, the startup manager with Stop and Freeze, and the setup guide. Ported from the first
//! Task Manager, which got the same data through a computer.

use super::*;
use crate::adv::Step;
use crate::hdata::{self, ProcSort, StartupFilter};

impl TaskMgr {
    /// True while the helper is answering.
    pub(crate) fn detailed(&self) -> bool {
        self.live.lock().map(|l| l.snap.is_some() && l.error.is_none()).unwrap_or(false)
    }

    /// A card that offers the setup, shown where detailed data would be.
    pub(crate) fn invite_card(&self, ui: &mut egui::Ui, what: &str) -> bool {
        let mut open = false;
        Self::card(ui, "Detailed mode", |ui| {
            ui.label(what);
            ui.add_space(6.0);
            if ui.add(egui::Button::new(RichText::new("Set up detailed mode").strong()).fill(ORANGE).corner_radius(16.0).min_size(vec2(0.0, 36.0))).clicked() {
                open = true;
            }
            ui.add_space(2.0);
            ui.label(RichText::new("About two minutes, once. Uses only your phone's own Wireless debugging; no computer and no other app.").small().color(MUTED));
        });
        open
    }

    /// CPU, memory and temperatures from the helper (the Overview shows these first when it is on).
    pub(crate) fn detailed_cards(&self, ui: &mut egui::Ui) {
        let (snap, hist) = {
            let l = self.live.lock().unwrap();
            (l.snap.clone(), l.hist.clone())
        };
        let Some(s) = snap else { return };
        let f = self.fahrenheit();

        Self::card(ui, "CPU", |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{:.0}%", s.cpu.total_percent)).size(32.0).strong().color(Self::load_color(s.cpu.total_percent)));
                ui.label(RichText::new(format!("load {}", s.cpu.load_avg.iter().map(|l| format!("{l:.1}")).collect::<Vec<_>>().join(" "))).color(MUTED));
            });
            Self::spark(ui, &hist.cpu, 100.0, 54.0, ORANGE);
            ui.add_space(8.0);
            for (i, c) in s.cpu.cores.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("Core {i}")).size(13.0).color(MUTED));
                    ui.add_space(4.0);
                    ui.allocate_ui(vec2((ui.available_width() - 125.0).max(60.0), 14.0), |ui| Self::bar(ui, c.percent / 100.0, Self::load_color(c.percent), 12.0));
                    ui.label(RichText::new(format!("{:>3.0}%  {:.1} GHz", c.percent, c.mhz as f32 / 1000.0)).size(13.0).monospace());
                });
            }
        });

        Self::card(ui, "Memory", |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{:.0}%", s.mem.used_percent())).size(32.0).strong().color(Self::load_color(s.mem.used_percent())));
                ui.label(RichText::new(format!("{} used of {}", hdata::gb(s.mem.used_mb()), hdata::gb(s.mem.total))).color(MUTED));
            });
            Self::bar(ui, s.mem.used_percent() / 100.0, Self::load_color(s.mem.used_percent()), 10.0);
            ui.add_space(6.0);
            Self::spark(ui, &hist.ram, 100.0, 40.0, GOOD);
            ui.add_space(4.0);
            ui.label(RichText::new(format!("{} free \u{b7} {} cached \u{b7} swap {} of {}", hdata::gb(s.mem.avail), hdata::gb(s.mem.cached), hdata::gb(s.mem.swap_used_mb()), hdata::gb(s.mem.swap_total))).size(13.0).color(MUTED));
        });

        Self::card(ui, "Temperatures", |ui| {
            ui.horizontal_wrapped(|ui| {
                for (name, v) in [("CPU", s.temp.cpu), ("GPU", s.temp.gpu), ("Battery", s.temp.battery.or(s.battery.as_ref().and_then(|b| b.temperature_c))), ("Skin", s.temp.skin)] {
                    if let Some(c) = v {
                        ui.label(RichText::new(name).color(MUTED));
                        let col = match hdata::temp_level(c) {
                            0 => GOOD,
                            1 => WARN,
                            _ => BAD,
                        };
                        ui.label(RichText::new(fmt_temp(c, f)).strong().size(18.0).color(col));
                        ui.add_space(10.0);
                    }
                }
            });
            Self::spark(ui, &hist.temp, 100.0, 40.0, WARN);
        });
        ui.label(RichText::new(format!("{} processes \u{b7} up {}", s.nprocs, fmt_duration((s.uptime * 1000.0) as i64))).size(13.0).color(MUTED));
        ui.add_space(8.0);
    }

    // ---------- the Processes tab ----------

    pub(crate) fn processes_ui(&mut self, ui: &mut egui::Ui) {
        if !self.detailed() {
            let err = self.live.lock().ok().and_then(|l| l.error.clone());
            if self.invite_card(ui, "Every process with its CPU and memory use, chip temperatures and overall CPU load need detailed mode. Android hides them from ordinary apps.") {
                self.screen = Screen::Setup;
            }
            if err.is_some() && self.settings.bool("detailed", false) {
                ui.label(RichText::new("Detailed mode is set up but not running. Switch Wireless debugging on, then tap \"Connect and start\" in its setup.").small().color(MUTED));
            }
            return;
        }
        let snap = self.live.lock().unwrap().snap.clone();
        let Some(s) = snap else { return };
        ui.add(egui::TextEdit::singleline(&mut self.pquery).hint_text("Search processes").desired_width(f32::INFINITY).margin(egui::Margin::symmetric(12, 8)));
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            for (p, name) in [(ProcSort::Cpu, "CPU"), (ProcSort::Ram, "RAM"), (ProcSort::Name, "Name")] {
                if Self::chip(ui, name, self.psort == p) {
                    self.psort = p;
                }
            }
            ui.label(RichText::new(format!("{} total", s.nprocs)).color(MUTED));
        });
        ui.add_space(4.0);
        let rows: Vec<domain::Process> = hdata::sorted_procs(&s.procs, &self.pquery, self.psort).into_iter().cloned().collect();
        let mut open = None;
        let row_h = 52.0;
        egui::ScrollArea::vertical().auto_shrink([false; 2]).id_salt("procs").show_rows(ui, row_h, rows.len(), |ui, range| {
            for i in range {
                let p = &rows[i];
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h - 4.0), Sense::click());
                let picked = self.expanded == Some(p.pid);
                ui.painter().rect_filled(rect, 10.0, if picked { Color32::from_gray(48) } else if resp.is_pointer_button_down_on() { Color32::from_gray(44) } else { CARD });
                let mid = rect.center().y;
                ui.painter().text(pos2(rect.left() + 12.0, mid), egui::Align2::LEFT_CENTER, hdata::display_name(&p.name), egui::FontId::proportional(15.0), Color32::from_gray(235));
                ui.painter().text(pos2(rect.right() - 12.0, mid), egui::Align2::RIGHT_CENTER, format!("{} MB", p.memory_mb), egui::FontId::monospace(14.0), MUTED);
                ui.painter().text(pos2(rect.right() - 92.0, mid), egui::Align2::RIGHT_CENTER, format!("{:.1}%", p.cpu_percent), egui::FontId::monospace(14.0), Self::load_color(p.cpu_percent * 2.0));
                if resp.clicked() {
                    open = Some(p.pid);
                }
            }
        });
        if let Some(pid) = open {
            self.expanded = Some(pid);
            self.confirm = None;
        }
    }

    /// The panel at the bottom for the tapped process. It stays where it is while the numbers
    /// behind it change, so the buttons can actually be hit.
    pub(crate) fn process_sheet_ui(&mut self, ctx: &Context, bottom: f32) {
        let Some(pid) = self.expanded else { return };
        let snap = self.live.lock().ok().and_then(|l| l.snap.clone());
        let proc = snap.as_ref().and_then(|s| s.procs.iter().find(|p| p.pid == pid)).cloned();
        let screen = ctx.viewport_rect();
        let mut close = false;
        egui::Area::new(egui::Id::new("proc-scrim")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
            let (rect, resp) = ui.allocate_exact_size(screen.size(), Sense::click());
            ui.painter().rect_filled(rect, 0.0, Color32::from_black_alpha(140));
            if resp.clicked() {
                close = true;
            }
        });
        let mut stop: Option<String> = None;
        egui::Area::new(egui::Id::new("proc-sheet")).order(egui::Order::Tooltip).anchor(egui::Align2::LEFT_BOTTOM, vec2(0.0, -bottom)).show(ctx, |ui| {
            egui::Frame::new().fill(CARD).corner_radius(egui::CornerRadius { nw: 20, ne: 20, sw: 0, se: 0 }).inner_margin(egui::Margin::same(16)).show(ui, |ui| {
                ui.set_width(screen.width() - 32.0);
                match &proc {
                    None => {
                        ui.label(RichText::new("This process has ended.").size(16.0));
                        ui.add_space(8.0);
                        if ui.add_sized(vec2(ui.available_width(), 44.0), egui::Button::new("Close").corner_radius(14.0)).clicked() {
                            close = true;
                        }
                    }
                    Some(p) => {
                        ui.label(RichText::new(hdata::display_name(&p.name)).size(19.0).strong());
                        ui.label(RichText::new(&p.name).small().color(MUTED));
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("{:.1}% CPU", p.cpu_percent)).strong().color(Self::load_color(p.cpu_percent * 2.0)));
                            ui.label(RichText::new(format!("{} MB", p.memory_mb)).strong());
                            ui.label(RichText::new(format!("{} threads \u{b7} pid {}", p.threads, p.pid)).color(MUTED));
                        });
                        ui.add_space(10.0);
                        match hdata::package_of(&p.name) {
                            Some(pkg) => {
                                let key = format!("stop:{pkg}");
                                let sure = self.confirm.as_ref().is_some_and(|(k, t)| *k == key && t.elapsed() < Duration::from_secs(6));
                                let label = if sure { "Tap again to stop it" } else { "Stop this app" };
                                let b = egui::Button::new(RichText::new(label).size(16.0).strong().color(Color32::WHITE)).fill(if sure { BAD } else { Color32::from_rgb(120, 50, 45) }).corner_radius(14.0);
                                if ui.add_sized(vec2(ui.available_width(), 48.0), b).clicked() {
                                    if sure {
                                        stop = Some(pkg.to_string());
                                    } else {
                                        self.confirm = Some((key, Instant::now()));
                                    }
                                }
                                ui.label(RichText::new("Only your own apps can be stopped. System apps are refused, and the result is shown here.").small().color(MUTED));
                            }
                            None => {
                                ui.label(RichText::new("This is part of the system, not an app, so it can't be stopped from here.").color(MUTED));
                            }
                        }
                        ui.add_space(6.0);
                        if ui.add_sized(vec2(ui.available_width(), 40.0), egui::Button::new("Close").corner_radius(14.0)).clicked() {
                            close = true;
                        }
                    }
                }
            });
        });
        if self.confirm.is_some() {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
        if let Some(pkg) = stop {
            self.selected = HashSet::from([pkg]);
            self.run_act("stop");
            close = true;
        }
        if close {
            self.expanded = None;
            self.confirm = None;
        }
    }

    // ---------- the Startup tab, with the helper ----------

    pub(crate) fn startup_detailed_ui(&mut self, ui: &mut egui::Ui) {
        if !self.startup_requested {
            self.startup_requested = true;
            self.reload_startup();
        }
        let (data, busy) = {
            let l = self.live.lock().unwrap();
            (l.startup.clone(), l.startup_busy)
        };
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.hsquery).hint_text("Search apps").desired_width((ui.available_width() - 92.0).max(80.0)).margin(egui::Margin::symmetric(12, 8)));
            if ui.add_enabled(!busy, egui::Button::new("Refresh").corner_radius(16.0).min_size(vec2(0.0, 32.0))).clicked() {
                self.reload_startup();
            }
        });
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            for (f, name) in [(StartupFilter::Woke, "Woke at boot"), (StartupFilter::CanWake, "Can wake"), (StartupFilter::Running, "Running"), (StartupFilter::Off, "Stopped / frozen"), (StartupFilter::All, "All")] {
                if Self::chip(ui, name, self.hsfilter == f) {
                    self.hsfilter = f;
                }
            }
        });
        let Some(d) = data else {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| ui.label(if busy { "Reading startup data (a few seconds)\u{2026}" } else { "No data yet." }));
            return;
        };
        let woke = d.apps.iter().filter(|a| a.boot.is_some()).count();
        let woke_mb: u32 = d.apps.iter().filter(|a| a.boot.is_some()).map(|a| a.mb).sum();
        ui.label(
            RichText::new(if woke > 0 { format!("{woke} of your apps woke at the last boot and hold about {woke_mb} MB now.") } else { "No boot data in the phone's log right now (it only keeps the latest start).".to_string() })
                .size(13.0)
                .color(MUTED),
        );
        let rows = hdata::startup_sorted(&d.apps, self.hsfilter, &self.hsquery);
        egui::ScrollArea::vertical().auto_shrink([false; 2]).id_salt("startup-detailed").show(ui, |ui| {
            for a in &rows {
                egui::Frame::new().fill(CARD).corner_radius(10.0).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let mut on = self.selected.contains(&a.p);
                        if ui.add_enabled(a.prot.is_empty(), egui::Checkbox::without_text(&mut on)).changed() {
                            if on {
                                self.selected.insert(a.p.clone());
                            } else {
                                self.selected.remove(&a.p);
                            }
                        }
                        ui.vertical(|ui| {
                            ui.label(RichText::new(&a.p).strong().size(14.0));
                            let why = match (&a.boot, a.recv) {
                                (Some(b), _) => format!("woke at +{}s \u{b7} {}: {}", b.after, b.how, b.comp),
                                (None, true) => "can wake (has a boot receiver)".to_string(),
                                _ => String::new(),
                            };
                            let state = if a.frozen { "frozen" } else if a.stopped { "stopped" } else if a.running { "running" } else { "idle" };
                            let mut line = state.to_string();
                            if !why.is_empty() {
                                line += &format!(" \u{b7} {why}");
                            }
                            if !a.prot.is_empty() {
                                line += &format!(" \u{b7} protected: {}", a.prot);
                            }
                            ui.label(RichText::new(line).size(12.0).color(MUTED));
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if a.mb > 0 {
                                ui.label(RichText::new(format!("{} MB", a.mb)).monospace().color(MUTED));
                            }
                        });
                    });
                });
                ui.add_space(3.0);
            }
            ui.add_space(80.0); // room for the action bar
        });
    }

    /// The Stop / Freeze / Unfreeze bar that appears when apps are ticked.
    pub(crate) fn startup_actions(&mut self, ctx: &Context, bottom: f32) {
        if self.tab != Tab::Startup || self.selected.is_empty() || !self.detailed() {
            return;
        }
        let mut pressed: Option<&'static str> = None;
        egui::Area::new(egui::Id::new("startup-actions")).anchor(egui::Align2::CENTER_BOTTOM, vec2(0.0, -(bottom + 10.0))).order(egui::Order::Foreground).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(format!("{} selected", self.selected.len()));
                    for (op, label, risky) in [("stop", "Stop", true), ("freeze", "Freeze", true), ("unfreeze", "Unfreeze", false)] {
                        let sure = self.confirm.as_ref().is_some_and(|(k, t)| k == op && t.elapsed() < Duration::from_secs(4));
                        let text = if sure { "Again to confirm".to_string() } else { label.to_string() };
                        if ui.button(RichText::new(text).color(if risky { BAD } else { Color32::WHITE })).clicked() {
                            if sure || !risky {
                                pressed = Some(op);
                            } else {
                                self.confirm = Some((op.to_string(), Instant::now()));
                            }
                        }
                    }
                });
            });
        });
        if let Some(op) = pressed {
            self.confirm = None;
            self.run_act(op);
        }
    }

    // ---------- the setup guide ----------

    pub(crate) fn setup_ui(&mut self, ui: &mut egui::Ui) {
        let st = self.adv.status();
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.add(egui::Button::new(RichText::new("\u{2039}").size(26.0)).frame(false).min_size(vec2(40.0, 40.0))).clicked() {
                self.screen = Screen::Main;
            }
            ui.label(RichText::new("Detailed mode").size(24.0).strong());
        });
        egui::ScrollArea::vertical().id_salt("setup").auto_shrink([false; 2]).show(ui, |ui| {
            ui.label(RichText::new("Shows every app's CPU and memory, chip temperatures and the boot history, like the first Task Manager did with a computer. This time the phone does it by itself, using Android's own Wireless debugging. Nothing leaves the phone.").color(MUTED));
            ui.add_space(10.0);

            // what is really happening beats what the last attempt said
            let helper_answers = self.detailed();
            let (color, title) = match st.step {
                _ if helper_answers => (GOOD, "On"),
                Step::Running => (GOOD, "On"),
                Step::Failed => (BAD, "Not working"),
                Step::Idle => (MUTED, "Off"),
                _ => (WARN, "Setting up"),
            };
            Self::card(ui, "Status", |ui| {
                ui.label(RichText::new(title).size(20.0).strong().color(color));
                if helper_answers {
                    ui.label("The helper is answering. Overview, Processes and Startup now show the detailed numbers.");
                } else if !st.message.is_empty() {
                    ui.label(&st.message);
                }
            });

            // Shizuku, for people who already use it
            if self.shizuku_at.is_none_or(|t| t.elapsed() > Duration::from_millis(1000)) {
                self.shizuku = self.sys.shizuku_state();
                self.shizuku_at = Some(Instant::now());
            }
            ui.ctx().request_repaint_after(Duration::from_millis(1000));
            let step = crate::adv::shizuku_step(self.shizuku);
            let busy = matches!(st.step, Step::Pairing | Step::Connecting | Step::Starting);
            // once it has worked, 'Connect and start' is the thing to tap; before that, 'Start pairing'
            let paired = self.settings.bool("detailed", false);
            let mut shizuku_action = false;
            Self::card(ui, "Quickest, if you use Shizuku", |ui| {
                let (text, button) = match step {
                    crate::adv::ShizukuStep::Install => ("Shizuku is a free app that lets other apps use the same access. If you have it, setup is one tap. If not, skip to the steps below, which need nothing else.", "Get Shizuku"),
                    crate::adv::ShizukuStep::StartIt => ("Shizuku is installed but not running. Open it and start it (it explains how), then come back.", "Open Shizuku"),
                    crate::adv::ShizukuStep::Allow => ("Shizuku is running. Task Manager needs your permission to use it.", "Allow Task Manager"),
                    crate::adv::ShizukuStep::Ready => ("Shizuku is ready.", "Start with Shizuku"),
                };
                ui.label(text);
                ui.add_space(6.0);
                if ui.add_enabled(!busy, egui::Button::new(RichText::new(button).strong()).fill(if step == crate::adv::ShizukuStep::Ready { ORANGE } else { CARD }).corner_radius(16.0).min_size(vec2(0.0, 36.0))).clicked() {
                    shizuku_action = true;
                }
            });
            if shizuku_action {
                match step {
                    crate::adv::ShizukuStep::Install | crate::adv::ShizukuStep::StartIt => self.sys.shizuku_open(),
                    crate::adv::ShizukuStep::Allow => self.sys.shizuku_request(),
                    crate::adv::ShizukuStep::Ready => self.adv.shizuku(),
                }
            }

            Self::card(ui, "1. Turn on Wireless debugging", |ui| {
                ui.label("In Settings: About phone, tap Build number seven times to get Developer options. Join any Wi-Fi network (it does not need internet). Then open Developer options and switch on Wireless debugging.");
                ui.add_space(6.0);
                if ui.add(egui::Button::new("Open Developer options").corner_radius(16.0).min_size(vec2(0.0, 34.0))).clicked() {
                    self.sys.open_developer_options();
                }
            });

            Self::card(ui, "2. Pair with a code", |ui| {
                ui.label("Tap the button below. A notification appears. Then, in Wireless debugging, tap \"Pair device with pairing code\". Without leaving that window, pull down the notification shade and type the six digits into the Task Manager notification.");
                ui.add_space(6.0);
                let busy = matches!(st.step, Step::Pairing | Step::Connecting | Step::Starting);
                if ui.add_enabled(!busy, egui::Button::new(RichText::new(if st.step == Step::WaitingForCode { "Show the notification again" } else { "Start pairing" }).strong()).fill(if paired { CARD } else { ORANGE }).corner_radius(16.0).min_size(vec2(0.0, 38.0))).clicked() {
                    if self.sys.can_notify() {
                        self.adv.setup();
                    } else {
                        self.sys.request_notify_permission();
                        self.say("Allow notifications, then tap Start pairing again.");
                    }
                }
                ui.add_space(8.0);
                ui.label(RichText::new("Or type the code here if you use split screen:").small().color(MUTED));
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.code_input).hint_text("123456").desired_width(110.0).margin(egui::Margin::symmetric(10, 8)));
                    if ui.add_enabled(self.code_input.trim().len() == 6, egui::Button::new(RichText::new("Pair").strong()).fill(if self.code_input.trim().len() == 6 { ORANGE } else { CARD }).corner_radius(16.0).min_size(vec2(0.0, 32.0))).clicked() {
                        let c = self.code_input.trim().to_string();
                        self.code_input.clear();
                        self.adv.code(&c);
                    }
                });
            });

            Self::card(ui, "Already paired?", |ui| {
                ui.label("After a restart, switch Wireless debugging on again and tap this. No code is needed.");
                ui.add_space(6.0);
                if ui.add_enabled(!busy, egui::Button::new(RichText::new("Connect and start").strong()).fill(if paired { ORANGE } else { CARD }).corner_radius(16.0).min_size(vec2(0.0, 36.0))).clicked() {
                    self.adv.connect();
                }
            });

            if !st.log.is_empty() {
                ui.collapsing("Details", |ui| {
                    for l in &st.log {
                        ui.label(RichText::new(l).small().color(MUTED));
                    }
                });
            }
            ui.add_space(30.0);
        });
    }
}
