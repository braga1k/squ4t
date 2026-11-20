//! Motion: the switch knob's flight (1nstall's motion.cs, Firm feel: no bounce), the check that
//! answers Record and Save, and the window rising in and sinking out. Every eased segment uses
//! the one curve of the interface, `motion::ease`. A frame timer runs only while something
//! moves; with motion off in Windows everything lands at once.

use std::time::{Duration, Instant};

use squ4t_core::motion::{self, Feel, Pose};

use crate::Panel;

/// The switch feel: 1nstall's Firm preset, no stretch, squash or bounce.
pub const SWITCH: Feel = Feel::FIRM;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prop {
    KnobX,
    KnobStretch,
    KnobHeight,
    Fill,
    Ink,
    CheckY,
    CheckOpacity,
    WindowY,
    WindowOpacity,
}

const ALL: [Prop; 9] = [Prop::KnobX, Prop::KnobStretch, Prop::KnobHeight, Prop::Fill, Prop::Ink, Prop::CheckY, Prop::CheckOpacity, Prop::WindowY, Prop::WindowOpacity];

enum Shape {
    /// Evenly spaced linear keyframes over the duration (the knob's flight).
    Linear(Vec<f64>),
    /// `(ms, value)` keys, each segment eased.
    Keys(Vec<(f64, f64)>),
}

struct Track {
    prop: Prop,
    start: Instant,
    duration: f64,
    shape: Shape,
}

impl Track {
    fn value(&self, elapsed_ms: f64) -> f64 {
        match &self.shape {
            Shape::Linear(values) => motion::sample_linear(values, elapsed_ms / self.duration),
            Shape::Keys(keys) => motion::sample_keys(keys, elapsed_ms),
        }
    }
    fn end(&self) -> f64 {
        self.value(f64::INFINITY)
    }
}

/// The open panel window: where it rests and how many pixels a unit is. The entrance and the
/// exit move and fade the whole window.
#[derive(Clone, Copy)]
pub struct WindowMotion {
    pub hwnd: windows_sys::Win32::Foundation::HWND,
    pub x: i32,
    pub y: i32,
    pub scale: f64,
}

fn index(p: Prop) -> usize {
    ALL.iter().position(|q| *q == p).expect("known property")
}

fn apply(ui: &Panel, window: Option<WindowMotion>, p: Prop, v: f64) {
    let f = v as f32;
    match p {
        Prop::KnobX => ui.set_knob_x(f),
        Prop::KnobStretch => ui.set_knob_stretch(f),
        Prop::KnobHeight => ui.set_knob_height(f),
        Prop::Fill => ui.set_fill_opacity(f),
        Prop::Ink => ui.set_ink_opacity(f),
        Prop::CheckY => ui.set_check_y(f),
        Prop::CheckOpacity => ui.set_check_opacity(f),
        Prop::WindowY => {
            if let Some(w) = window {
                crate::tray::move_window(w.hwnd, w.x, w.y + (v * w.scale).round() as i32);
            }
        }
        Prop::WindowOpacity => {
            if let Some(w) = window {
                if crate::tray::window_alpha(w.hwnd, v) {
                    repaint_all(ui);
                }
            }
        }
    }
}

/// Makes the whole panel dirty, so the next frame paints and presents every pixel.
pub fn repaint_all(ui: &Panel) {
    ui.set_repaint(ui.get_repaint().wrapping_add(1));
}

pub struct Animator {
    tracks: Vec<Track>,
    values: [f64; 9],
    pub enabled: bool,
    pub window: Option<WindowMotion>,
    timer: slint::Timer,
}

impl Animator {
    pub fn new() -> Animator {
        let mut values = [0.0; 9];
        values[index(Prop::KnobHeight)] = 1.0;
        values[index(Prop::WindowOpacity)] = 1.0;
        Animator { tracks: Vec::new(), values, enabled: true, window: None, timer: slint::Timer::default() }
    }

    pub fn get(&self, p: Prop) -> f64 {
        self.values[index(p)]
    }

    pub fn set(&mut self, ui: &Panel, p: Prop, v: f64) {
        self.tracks.retain(|t| t.prop != p);
        self.values[index(p)] = v;
        apply(ui, self.window, p, v);
    }

    fn start(&mut self, ui: &Panel, prop: Prop, duration_ms: f64, shape: Shape) {
        self.tracks.retain(|t| t.prop != prop);
        let track = Track { prop, start: Instant::now(), duration: duration_ms.max(1.0), shape };
        if !self.enabled {
            let end = track.end();
            self.set(ui, prop, end);
            return;
        }
        let first = track.value(0.0);
        self.values[index(prop)] = first;
        apply(ui, self.window, prop, first);
        self.tracks.push(track);
    }

    fn keys(&mut self, ui: &Panel, prop: Prop, keys: Vec<(f64, f64)>) {
        let duration = keys.last().map(|k| k.0).unwrap_or(0.0);
        self.start(ui, prop, duration, Shape::Keys(keys));
    }

    /// Advances every track; true while something still moves.
    pub fn frame(&mut self, ui: &Panel) -> bool {
        let now = Instant::now();
        let mut values = self.values;
        let window = self.window;
        self.tracks.retain(|t| {
            let elapsed = now.duration_since(t.start).as_secs_f64() * 1000.0;
            let v = t.value(elapsed);
            values[index(t.prop)] = v;
            apply(ui, window, t.prop, v);
            elapsed < t.duration
        });
        self.values = values;
        !self.tracks.is_empty()
    }

    /// Lands every track on its last value (motion switched off mid-flight).
    pub fn stop(&mut self, ui: &Panel) {
        for t in std::mem::take(&mut self.tracks) {
            let end = t.end();
            self.values[index(t.prop)] = end;
            apply(ui, self.window, t.prop, end);
        }
    }

    /// Starts the frame timer if it is not running. `tick` must call [`Animator::frame`].
    pub fn ensure_timer(&mut self, tick: impl FnMut() + 'static) {
        if !self.tracks.is_empty() && !self.timer.running() {
            self.timer.start(slint::TimerMode::Repeated, Duration::from_millis(15), tick);
        }
    }
    pub fn stop_timer(&self) {
        self.timer.stop();
    }

    // ------------------------------------------------------------ the switch

    pub fn pose(&self) -> Pose {
        Pose::new(self.get(Prop::KnobX), self.get(Prop::KnobStretch), self.get(Prop::KnobHeight))
    }

    /// At rest, no motion.
    pub fn place_switch(&mut self, ui: &Panel, on: bool) {
        self.set(ui, Prop::KnobX, if on { 16.0 } else { 0.0 });
        self.set(ui, Prop::KnobStretch, 0.0);
        self.set(ui, Prop::KnobHeight, 1.0);
        self.set(ui, Prop::Fill, if on { 1.0 } else { 0.0 });
        self.set(ui, Prop::Ink, if on { 1.0 } else { 0.0 });
    }

    fn fly(&mut self, ui: &Panel, target: f64) {
        let from = self.pose();
        let poses: Vec<Pose> = if self.enabled { motion::flight(from, target, SWITCH).to_vec() } else { vec![from, Pose::new(target, 0.0, 1.0)] };
        let d = SWITCH.duration_ms as f64;
        self.start(ui, Prop::KnobX, d, Shape::Linear(poses.iter().map(|p| p.x).collect()));
        self.start(ui, Prop::KnobStretch, d, Shape::Linear(poses.iter().map(|p| p.stretch).collect()));
        self.start(ui, Prop::KnobHeight, d, Shape::Linear(poses.iter().map(|p| p.height).collect()));
    }

    /// The track fills with the accent and the knob crosses, 160 ms for the colour.
    pub fn change_switch(&mut self, ui: &Panel, on: bool) {
        let v = if on { 1.0 } else { 0.0 };
        self.keys(ui, Prop::Fill, vec![(0.0, self.get(Prop::Fill)), (160.0, v)]);
        self.keys(ui, Prop::Ink, vec![(0.0, self.get(Prop::Ink)), (160.0, v)]);
        self.fly(ui, v * 16.0);
    }

    /// A press leans the knob 1.7 units toward the other side; release returns it.
    pub fn press_switch(&mut self, ui: &Panel, on: bool, down: bool) {
        let rest = if on { 16.0 } else { 0.0 };
        if !down {
            self.fly(ui, rest);
            return;
        }
        if !self.enabled {
            return;
        }
        let direction = if on { -1.0 } else { 1.0 };
        self.keys(ui, Prop::KnobX, vec![(0.0, self.get(Prop::KnobX)), (120.0, rest + direction * 1.7)]);
    }

    // ------------------------------------------------------------ results and the window

    /// The check: rises 4 units into place and fades in, holds, then fades out.
    pub fn confirm(&mut self, ui: &Panel) {
        self.keys(ui, Prop::CheckY, vec![(0.0, 4.0), (300.0, 0.0)]);
        self.keys(ui, Prop::CheckOpacity, vec![(0.0, 0.0), (160.0, 1.0), (800.0, 1.0), (1000.0, 0.0)]);
    }

    /// The window rises 10 units into place as it fades in.
    pub fn enter(&mut self, ui: &Panel) {
        self.keys(ui, Prop::WindowY, vec![(0.0, 10.0), (300.0, 0.0)]);
        self.keys(ui, Prop::WindowOpacity, vec![(0.0, 0.18), (260.0, 1.0)]);
    }

    /// The entrance reversed: 10 units down and out, from wherever it is now.
    pub fn exit(&mut self, ui: &Panel) {
        let (y, opacity) = (self.get(Prop::WindowY), self.get(Prop::WindowOpacity));
        self.keys(ui, Prop::WindowY, vec![(0.0, y), (300.0, 10.0)]);
        self.keys(ui, Prop::WindowOpacity, vec![(0.0, opacity), (260.0, 0.0)]);
    }
}
