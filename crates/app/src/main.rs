//! squ4t: the tray app.
//!
//! Service.qml and BarWidget.qml of omarchy-squat-break, on Windows: a worker runs one ledger
//! operation at a time (the queue of Service.qml, at most 20, a tick dropped while busy), the
//! tray icon and its tooltip stand in for the bar widget, and the panel is the bar's popout.

#![cfg_attr(not(test), windows_subsystem = "windows")]

mod anim;
mod art;
mod sound;
mod tray;

use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anim::{Animator, WindowMotion};
use art::RingState;
use slint::winit_030::{winit, EventResult, WinitWindowAccessor};
use slint::{Color, ComponentHandle};
use squ4t::store::{default_dir, Lock, Store};
use squ4t::{autostart, countdown, format_number, instance_title, tell_instance, Strings as Catalog, MSG_OPEN};
use squ4t_core::ledger::Report;
use squ4t_core::look::{self, Look, Rgba};
use tray::{Appearance, Event, MenuCommand, MenuState, Tray};

slint::include_modules!();

const PANEL_W: f64 = 320.0;
const PANEL_H: f64 = 378.0;

type Job = (String, Vec<String>);
type Outcome = (String, Result<Report, String>);

struct App {
    ui: Panel,
    tray: Tray,
    text: Catalog,
    jobs: mpsc::Sender<Job>,
    look: Appearance,
    colors: Look,
    /// The panel's scale and the last ring drawn there, so it is redrawn only when it changes.
    scale: f64,
    ring_key: Option<(u32, bool, bool, i32)>,
    anim: Animator,
    report: Option<Report>,
    sampled_at: f64,
    ready: bool,
    failed: bool,
    /// The old ledger could not be brought over: nothing may write a new one in its place.
    blocked: bool,
    in_flight: bool,
    queue: VecDeque<Job>,
    visible: bool,
    hidden_at: Option<Instant>,
    /// What the switch shows; it moves at once on a click, before the ledger answers.
    switch_on: bool,
    ui_test: bool,
    deadline: slint::Timer,
    display: slint::Timer,
    safety: slint::Timer,
    last_tip: String,
    last_icon: Vec<u8>,
    /// The panel is leaving; `close_generation` tells a finished exit from one that was cancelled.
    closing: bool,
    close_generation: u32,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Runs `f` with the app. If the app is already borrowed (a callback fired during another
/// update), `f` runs right after, on the event loop.
fn with_app(f: impl FnOnce(&mut App) + 'static) {
    let pending = APP.with(|cell| match cell.try_borrow_mut() {
        Ok(mut app) => {
            if let Some(app) = app.as_mut() {
                f(app);
            }
            None
        }
        Err(_) => Some(f),
    });
    if let Some(f) = pending {
        slint::Timer::single_shot(Duration::ZERO, move || with_app(f));
    }
}

fn wall_now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

fn color(c: Rgba) -> Color {
    Color::from_argb_u8(c.a, c.r, c.g, c.b)
}

impl App {
    fn busy(&self) -> bool {
        self.in_flight || !self.queue.is_empty()
    }

    /// `Service.request`: a tick is dropped while busy; at most 20 waiting operations.
    fn request(&mut self, action: &str, args: Vec<String>) {
        if self.blocked {
            return;
        }
        if action == "tick" && self.busy() {
            return;
        }
        if self.queue.len() >= 20 {
            return;
        }
        self.queue.push_back((action.to_string(), args));
        self.drain();
        self.refresh_controls();
    }

    fn drain(&mut self) {
        if self.in_flight {
            return;
        }
        if let Some(job) = self.queue.pop_front() {
            self.in_flight = true;
            let _ = self.jobs.send(job);
        }
    }

    fn on_result(&mut self, (action, result): Outcome) {
        self.in_flight = false;
        match result {
            Ok(report) => {
                let alert = report.alert;
                self.sampled_at = wall_now();
                self.report = Some(report);
                self.failed = false;
                self.ready = true;
                if alert {
                    self.notify();
                }
                // Motion.Result at the button that was pressed: Record, or Save settings.
                if action == "add" {
                    sound::recorded();
                }
                if (action == "add" || action == "settings") && self.visible && !self.closing {
                    self.ui.set_result_target(if action == "settings" { 1 } else { 0 });
                    self.anim.confirm(&self.ui);
                    self.run_animations();
                }
            }
            Err(error) => {
                eprintln!("squat ledger: {error}");
                self.failed = true;
            }
        }
        self.drain();
        self.refresh();
    }

    fn remaining(&self) -> i128 {
        match &self.report {
            None => 0,
            Some(r) if r.paused => r.remaining,
            Some(r) => (r.remaining as f64 - (wall_now() - self.sampled_at)).ceil().max(0.0) as i128,
        }
    }

    fn status_text(&self) -> String {
        match &self.report {
            _ if self.failed => self.text.t("error"),
            None => self.text.t("loading"),
            Some(r) if r.paused => self.text.t("paused"),
            Some(r) if r.due => self.text.t("due"),
            Some(_) => self.text.t("next").replace("%1", &format_number((self.remaining() + 59) / 60)),
        }
    }

    /// Everything that follows a new report.
    fn refresh(&mut self) {
        let paused = self.report.as_ref().is_some_and(|r| r.paused);
        if self.ready && self.switch_on == paused {
            // The ledger disagrees with the switch (a click that failed, or a change from the menu or the command line).
            self.switch_on = !paused;
            if self.visible {
                self.anim.change_switch(&self.ui, self.switch_on);
                self.run_animations();
            } else {
                self.anim.place_switch(&self.ui, self.switch_on);
            }
        }
        if let Some(r) = &self.report {
            self.ui.set_today_text(format_number(r.today).into());
            self.ui.set_total_text(format_number(r.total).into());
            self.ui.set_can_undo(r.can_undo);
            self.ui.set_saved_interval(r.interval as i32);
            self.ui.set_saved_suggested(r.suggested as i32);
        }
        self.ui.set_paused(!self.switch_on);
        self.refresh_controls();
        self.tick_display();
        self.schedule_deadline();
        tray::set_menu(MenuState {
            open: self.text.t("menu.open"),
            pause_resume: self.text.t(if paused { "resume" } else { "pause" }),
            snooze: self.text.t("menu.snooze"),
            startup: self.text.t("menu.startup"),
            startup_on: autostart::get().is_some(),
            exit: self.text.t("menu.exit"),
        });
    }

    fn refresh_controls(&self) {
        self.ui.set_can_act(self.ready && !self.failed && !self.busy());
    }

    /// Once a second: the countdown in the status line, the tooltip and the tray icon.
    fn tick_display(&mut self) {
        self.ui.set_status(self.status_text().into());
        self.ui.set_status_error(self.failed);
        let (today, total) = self.report.as_ref().map(|r| (r.today, r.total)).unwrap_or((0, 0));
        let clock = if self.ready { countdown(self.remaining()) } else { "--:--".into() };
        let tip = format!(
            "{} \u{b7} {}\n{}",
            self.text.t("title"),
            clock,
            self.text.t("tooltip").replace("%1", &format_number(today)).replace("%2", &format_number(total)).replace("%3", &self.status_text())
        );
        let (due, paused, interval) = self.report.as_ref().map(|r| (r.due, r.paused, r.interval)).unwrap_or((false, false, 60));
        let left = if self.ready { self.remaining() as f64 / (interval * 60) as f64 } else { 1.0 };
        let ring = RingState { left: left.clamp(0.0, 1.0), due: due && !paused, paused };
        if self.visible {
            let px = (16.0 * self.scale).round() as u32;
            let key = (px, ring.due, ring.paused, (ring.left * 240.0).round() as i32);
            if self.ring_key != Some(key) {
                self.ring_key = Some(key);
                self.ui.global::<Theme>().set_ring(art::ring_image(px, ring, self.colors.text_secondary, self.colors.accent));
            }
        }
        let size = tray::icon_size();
        let icon = art::tray_icon(size, ring, self.look.light_taskbar, self.colors.accent);
        if tip != self.last_tip || icon != self.last_icon {
            self.tray.update(&icon, size, &tip);
            self.last_tip = tip;
            self.last_icon = icon;
        }
    }

    /// The next reminder, timed to its deadline instead of polled every 15 s.
    fn schedule_deadline(&mut self) {
        self.deadline.stop();
        let Some(r) = &self.report else { return };
        if r.paused || self.failed {
            return;
        }
        let left = (r.remaining as f64 - (wall_now() - self.sampled_at)).max(0.0);
        self.deadline.start(slint::TimerMode::SingleShot, Duration::from_secs_f64(left + 0.25), || with_app(|a| a.request("tick", vec![])));
    }

    fn notify(&self) {
        let suggested = self.report.as_ref().map(|r| r.suggested).unwrap_or(20);
        self.tray.notify(&self.text.t("alert.title"), &self.text.tn("alert.body", suggested));
    }

    fn run_animations(&mut self) {
        self.anim.ensure_timer(|| {
            with_app(|a| {
                if !a.anim.frame(&a.ui) {
                    a.anim.stop_timer();
                }
            })
        });
    }

    // ------------------------------------------------------------ appearance

    fn apply_appearance(&mut self) {
        self.look = tray::appearance();
        self.anim.enabled = self.look.animations;
        if !self.anim.enabled {
            self.anim.stop(&self.ui);
        }
        self.colors = look::look(self.look.light_apps, self.look.high_contrast, self.look.system);
        let c = &self.colors;
        let t = self.ui.global::<Theme>();
        t.set_surface(color(c.surface));
        t.set_raised(color(c.raised));
        t.set_raised_hover(color(c.raised_hover));
        t.set_hairline(color(c.hairline));
        t.set_text(color(c.text));
        t.set_text_secondary(color(c.text_secondary));
        t.set_text_muted(color(c.text_muted));
        t.set_primary(color(c.primary));
        t.set_primary_hover(color(c.primary_hover));
        t.set_on_primary(color(c.on_primary));
        t.set_accent(color(c.accent));
        t.set_on_accent(color(c.on_accent));
        t.set_focus(color(c.focus));
        t.set_negative(color(c.negative));
        t.set_motion(self.anim.enabled);
        t.set_check_mark(art::check_mark(c.text, 3.0));
        self.ring_key = None;
        self.last_icon.clear();
        self.tick_display();
    }

    /// The panel's scale: the ring is redrawn at it.
    fn set_scale(&mut self, scale: f64) {
        if (scale - self.scale).abs() > 1e-6 {
            self.scale = scale;
            self.ring_key = None;
        }
    }

    // ------------------------------------------------------------ the panel

    /// `BarWidget.open`: drafts start from the saved settings each time.
    fn open_panel(&mut self) {
        if let Some(r) = &self.report {
            self.ui.set_count(r.suggested as i32);
            self.ui.set_interval_draft(r.interval as i32);
            self.ui.set_suggested_draft(r.suggested as i32);
        }
        self.update_record_text();
        let (x, y, scale) = tray::panel_position(self.tray.icon_rect(), PANEL_W, PANEL_H);
        self.set_scale(scale);
        if self.closing {
            // Reopened while leaving: the exit is cancelled and the entrance starts from where it is.
            self.closing = false;
            self.close_generation += 1;
        } else {
            self.anim.place_switch(&self.ui, self.switch_on);
        }
        self.ui.window().set_position(slint::PhysicalPosition::new(x, y));
        let _ = self.ui.show();
        self.visible = true;
        self.attach_window(x, y, scale, 0);
    }

    /// Takes the window over from winit: no taskbar button, rounded corners, the focus, then the
    /// entrance. Before the event loop runs (the panel opened at start) winit has no window yet,
    /// so this tries again on the next turns of the loop.
    fn attach_window(&mut self, x: i32, y: i32, scale: f64, attempt: u32) {
        let hwnd = self
            .ui
            .window()
            .with_winit_window(|w| {
                use winit::platform::windows::WindowExtWindows;
                w.set_skip_taskbar(true);
                w.focus_window();
                hwnd_of(w)
            })
            .flatten();
        let Some(hwnd) = hwnd else {
            if attempt < 20 {
                slint::Timer::single_shot(Duration::from_millis(10), move || {
                    with_app(move |a| {
                        if a.visible {
                            a.attach_window(x, y, scale, attempt + 1)
                        }
                    })
                });
            }
            return;
        };
        round_corners(hwnd);
        tray::foreground(hwnd);
        // Shown again after a hide: Windows kept none of its pixels.
        anim::repaint_all(&self.ui);
        self.anim.window = Some(WindowMotion { hwnd, x, y, scale });
        self.anim.enter(&self.ui);
        self.run_animations();
    }

    /// Closes with the entrance reversed: the panel sinks 10 units and fades out, then hides.
    fn close_panel(&mut self) {
        if !self.visible || self.closing {
            return;
        }
        self.hidden_at = Some(Instant::now());
        if !self.anim.enabled || self.anim.window.is_none() {
            self.finish_close();
            return;
        }
        self.closing = true;
        self.close_generation += 1;
        let generation = self.close_generation;
        self.anim.exit(&self.ui);
        self.run_animations();
        slint::Timer::single_shot(Duration::from_millis(310), move || {
            with_app(move |a| {
                if a.closing && a.close_generation == generation {
                    a.finish_close();
                }
            })
        });
    }

    fn finish_close(&mut self) {
        let _ = self.ui.hide();
        self.visible = false;
        self.closing = false;
    }

    fn toggle_panel(&mut self) {
        if self.visible && !self.closing {
            self.close_panel();
        } else if self.hidden_at.is_some_and(|t| t.elapsed() < Duration::from_millis(400)) {
            // The click on the icon took the focus away and already closed the panel.
            self.hidden_at = None;
        } else {
            self.open_panel();
        }
    }

    fn update_record_text(&self) {
        self.ui.set_record_text(self.text.tn("record", self.ui.get_count() as i128).into());
    }

    fn toggle_reminders(&mut self) {
        let Some(r) = &self.report else { return };
        let action = if r.paused { "resume" } else { "pause" };
        self.switch_on = r.paused;
        self.ui.set_paused(!self.switch_on);
        if self.visible {
            self.anim.change_switch(&self.ui, self.switch_on);
            self.run_animations();
        }
        self.request(action, vec![]);
    }

    fn on_event(&mut self, event: Event) {
        match event {
            Event::Select | Event::Open => {
                if event == Event::Open {
                    if !self.visible {
                        self.open_panel();
                    }
                } else {
                    self.toggle_panel();
                }
            }
            Event::BalloonClick => {
                if !self.visible {
                    self.open_panel();
                }
            }
            Event::TaskbarCreated => {
                self.tray.add();
                self.last_icon.clear();
                self.tick_display();
            }
            Event::Resumed | Event::TimeChanged | Event::Refresh => self.request("tick", vec![]),
            Event::AppearanceChanged => self.apply_appearance(),
            Event::TestAlert => self.notify(),
            Event::Menu(MenuCommand::Open) => self.open_panel(),
            Event::Menu(MenuCommand::PauseResume) => self.toggle_reminders(),
            Event::Menu(MenuCommand::Snooze) => self.request("snooze", vec![]),
            Event::Menu(MenuCommand::Startup) => {
                let on = autostart::get().is_some();
                let exe = std::env::current_exe().ok();
                let _ = autostart::set(if on { None } else { exe.as_deref() });
                self.refresh();
            }
            Event::Menu(MenuCommand::Exit) | Event::Quit => {
                let _ = slint::quit_event_loop();
            }
        }
    }
}

fn hwnd_of(w: &winit::window::Window) -> Option<windows_sys::Win32::Foundation::HWND> {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match w.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get() as windows_sys::Win32::Foundation::HWND),
        _ => None,
    }
}

/// Windows 11 rounds the popup's corners; earlier versions ignore the request.
fn round_corners(hwnd: windows_sys::Win32::Foundation::HWND) {
    use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND};
    let preference = DWMWCP_ROUND;
    // SAFETY: a valid window and a 4-byte attribute value.
    unsafe { DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE as u32, (&preference as *const i32).cast(), 4) };
}

fn since_process_start_ms() -> u64 {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::SystemInformation::GetSystemTimeAsFileTime;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    let zero = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
    let (mut created, mut exited, mut kernel, mut user, mut now) = (zero, zero, zero, zero, zero);
    // SAFETY: out-parameters for the current process.
    unsafe {
        GetProcessTimes(GetCurrentProcess(), &mut created, &mut exited, &mut kernel, &mut user);
        GetSystemTimeAsFileTime(&mut now);
    }
    let t = |f: FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
    t(now).saturating_sub(t(created)) / 10_000
}

fn main() {
    let startup_test = std::env::args().any(|a| a == "--startup-test");
    let ui_test = startup_test || std::env::args().any(|a| a == "--ui-test");
    let migrated = squ4t::store::migrate_legacy_ledger();
    let store = Store::new(default_dir());
    let blocked = migrated.is_err();
    if let Err(e) = migrated {
        eprintln!("could not bring the ledger over from {}: {e}", squ4t::store::legacy_dir().display());
    }
    if std::fs::create_dir_all(store.dir()).is_err() {
        return;
    }
    // One instance per data folder. A second start opens the running one's panel, as a click on a reminder does.
    let Ok(Some(_instance)) = Lock::try_take(&store.dir().join("instance.lock")) else {
        tell_instance(store.dir(), MSG_OPEN);
        return;
    };
    // The software renderer: no GPU context, the smallest memory footprint (1nstall phase 3).
    if let Err(e) = slint::BackendSelector::new().backend_name("winit".into()).renderer_name("software".into()).select() {
        eprintln!("{e}");
        return;
    }
    let ui = Panel::new().expect("panel");
    let text = Catalog::english();
    ui.set_strings(Strings {
        title: text.t("title").into(),
        today: text.t("today").into(),
        total: text.t("total").into(),
        done: text.t("done").into(),
        undo: text.t("undo").into(),
        snooze: text.t("snooze").into(),
        interval: text.t("interval").into(),
        suggested: text.t("suggested").into(),
        save: text.t("save").into(),
        reminders: text.t("reminders").into(),
        hint: text.t("hint").into(),
    });

    let (jobs, inbox) = mpsc::channel::<Job>();
    let worker_store = store.clone();
    std::thread::spawn(move || {
        let _ = worker_store.remove_stale_temporaries();
        for (action, args) in inbox {
            let result = worker_store.run(&action, &args).map_err(|e| e.to_string());
            let outcome: Outcome = (action, result);
            let _ = slint::invoke_from_event_loop(move || with_app(move |a| a.on_result(outcome)));
        }
    });

    let mut tray = Tray::create(&instance_title(store.dir()), |event| with_app(move |a| a.on_event(event)));
    tray.add();

    let appearance = tray::appearance();
    let app = App {
        colors: look::look(appearance.light_apps, appearance.high_contrast, appearance.system),
        look: appearance,
        scale: 1.0,
        ring_key: None,
        ui: ui.clone_strong(),
        tray,
        text,
        jobs,
        anim: Animator::new(),
        report: None,
        sampled_at: 0.0,
        ready: false,
        failed: blocked,
        blocked,
        in_flight: false,
        queue: VecDeque::new(),
        visible: false,
        hidden_at: None,
        switch_on: true,
        ui_test,
        deadline: slint::Timer::default(),
        display: slint::Timer::default(),
        safety: slint::Timer::default(),
        last_tip: String::new(),
        last_icon: Vec::new(),
        closing: false,
        close_generation: 0,
    };
    APP.with(|cell| *cell.borrow_mut() = Some(app));

    ui.on_close(|| with_app(|a| a.close_panel()));
    ui.on_count_edited(|_| with_app(|a| a.update_record_text()));
    ui.on_record(|count| with_app(move |a| a.request("add", vec![count.to_string()])));
    ui.on_undo(|| with_app(|a| a.request("undo", vec![])));
    ui.on_snooze(|| with_app(|a| a.request("snooze", vec![])));
    ui.on_save(|interval, suggested| with_app(move |a| a.request("settings", vec![interval.to_string(), suggested.to_string()])));
    ui.on_toggle_reminders(|| with_app(|a| a.toggle_reminders()));
    ui.on_switch_press(|down| {
        with_app(move |a| {
            a.anim.press_switch(&a.ui, a.switch_on, down);
            a.run_animations();
        })
    });
    ui.window().on_winit_window_event(|_, event| {
        match event {
            winit::event::WindowEvent::Focused(false) => with_app(|a| {
                if !a.ui_test {
                    a.close_panel();
                }
            }),
            winit::event::WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                let scale = *scale_factor;
                with_app(move |a| a.set_scale(scale));
            }
            _ => {}
        }
        EventResult::Propagate
    });

    with_app(|a| {
        a.apply_appearance();
        a.refresh();
        a.request("tick", vec![]); // Component.onCompleted
        a.display.start(slint::TimerMode::Repeated, Duration::from_secs(1), || with_app(|a| a.tick_display()));
        // A slow safety net for a timer the system did not fire: cheap, and no reminder depends on it.
        a.safety.start(slint::TimerMode::Repeated, Duration::from_secs(300), || with_app(|a| a.request("tick", vec![])));
        if !a.ui_test && !a.text.t("title").is_empty() {
            first_run_autostart(a);
        }
        if a.ui_test {
            a.open_panel();
        }
    });
    if startup_test {
        // The software renderer has no frame callback: two idle turns of the event loop after
        // show, as 1nstall's prototype measured it. Milliseconds since the process was created.
        slint::Timer::single_shot(Duration::ZERO, || {
            slint::Timer::single_shot(Duration::from_millis(1), || {
                let ms = since_process_start_ms();
                let log = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("startup-test.log")));
                if let Some(log) = log {
                    let _ = std::fs::write(log, format!("READY {ms}\n"));
                }
                let _ = slint::quit_event_loop();
            })
        });
    }
    let _ = slint::run_event_loop_until_quit();
    APP.with(|cell| cell.borrow_mut().take());
}

/// The first start registers the app to start with Windows, as enabling the plugin makes it
/// part of the shell. The menu turns it off; the choice is not made again.
fn first_run_autostart(a: &App) {
    let marker = default_dir().join("autostart-chosen");
    if marker.exists() {
        // Already chosen. If it is on but names another copy of the program (moved, or unzipped
        // somewhere else), point it at this one, so the next logon finds a program that exists.
        if let (Some(registered), Ok(exe)) = (autostart::get(), std::env::current_exe()) {
            if registered.trim_matches('"') != exe.to_string_lossy() {
                let _ = autostart::set(Some(&exe));
            }
        }
        return;
    }
    // A first build may have made the choice already: its entry points to an old program file.
    let legacy_on = autostart::remove_legacy();
    let legacy_chosen = squ4t::store::legacy_dir().join("autostart-chosen").exists();
    let wanted = legacy_on || !legacy_chosen;
    if wanted {
        if let Ok(exe) = std::env::current_exe() {
            if autostart::set(Some(&exe)).is_err() {
                return;
            }
        }
    }
    let _ = std::fs::write(&marker, b"");
    let _ = a;
}
