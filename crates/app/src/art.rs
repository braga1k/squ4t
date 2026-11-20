//! The few pictures squ4t draws itself: the countdown ring (in the tray and, as the signature,
//! beside the status line) and the check that answers Record and Save. Slint 1.13's software
//! renderer draws no paths, so these are rasterised here, with 4 x 4 samples per pixel.

use slint::{Image, Rgba8Pixel, SharedPixelBuffer};
use squ4t_core::look::Rgba;

/// What the ring shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RingState {
    /// The share of the interval still to go, 0..1: the ring empties clockwise from twelve.
    pub left: f64,
    pub due: bool,
    pub paused: bool,
}

/// Coverage-weighted colour of one sample, or none.
fn ring_sample(x: f64, y: f64, s: f64, state: RingState, ink: Rgba, accent: Rgba) -> Option<(Rgba, f64)> {
    let (cx, cy) = (s / 2.0, s / 2.0);
    let (dx, dy) = (x - cx, y - cy);
    let r = (dx * dx + dy * dy).sqrt();
    let outer = s * 0.46;
    let inner = outer - (s * 0.14).max(1.6);
    if state.due {
        // A break is due: a filled ember disc with a light centre.
        return if r <= s * 0.13 {
            Some((Rgba::rgb(255, 255, 255), 1.0))
        } else if r <= outer {
            Some((accent, 1.0))
        } else {
            None
        };
    }
    if r <= outer && r >= inner {
        if state.paused {
            return Some((ink, 0.45));
        }
        let angle = (dx.atan2(-dy) / std::f64::consts::TAU).rem_euclid(1.0);
        return Some(if angle <= state.left { (ink, 1.0) } else { (ink, 0.25) });
    }
    if state.paused && r < inner {
        let bar = dx.abs() >= s * 0.06 && dx.abs() <= s * 0.15 && dy.abs() <= s * 0.16;
        return bar.then_some((ink, 0.85));
    }
    None
}

/// Premultiplied BGRA rows, top-down, for a Windows icon.
pub fn tray_icon(size: u32, state: RingState, light_taskbar: bool, accent: Rgba) -> Vec<u8> {
    let ink = if light_taskbar { Rgba::hex(0x171717) } else { Rgba::hex(0xEDEDED) };
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    raster(size, |x, y| ring_sample(x, y, size as f64, state, ink, accent), |r, g, b, a| out.extend_from_slice(&[b, g, r, a]), true);
    out
}

/// The same ring as an image for the panel, in the panel's own ink.
pub fn ring_image(size_px: u32, state: RingState, ink: Rgba, accent: Rgba) -> Image {
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(size_px.max(1), size_px.max(1));
    let pixels = buffer.make_mut_slice();
    let mut i = 0;
    raster(
        size_px,
        |x, y| ring_sample(x, y, size_px as f64, state, ink, accent),
        |r, g, b, a| {
            pixels[i] = Rgba8Pixel { r, g, b, a };
            i += 1;
        },
        false,
    );
    Image::from_rgba8(buffer)
}

/// The cog that opens the settings: eight teeth round a ring, in one ink.
pub fn gear_image(size_px: u32, ink: Rgba) -> Image {
    let size = size_px.max(1);
    let s = size as f64;
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(size, size);
    let pixels = buffer.make_mut_slice();
    let mut i = 0;
    raster(
        size,
        |x, y| {
            let (dx, dy) = (x - s / 2.0, y - s / 2.0);
            let r = (dx * dx + dy * dy).sqrt();
            let turn = (dy.atan2(dx) / std::f64::consts::TAU * 8.0).rem_euclid(1.0);
            let tooth = (turn - 0.5).abs() < 0.24;
            let outer = if tooth { s * 0.47 } else { s * 0.35 };
            (r <= outer && r >= s * 0.19).then_some((ink, 1.0))
        },
        |r, g, b, a| {
            pixels[i] = Rgba8Pixel { r, g, b, a };
            i += 1;
        },
        false,
    );
    Image::from_rgba8(buffer)
}

/// Supersamples `sample` over a square; `premultiplied` chooses the output form.
fn raster(size: u32, sample: impl Fn(f64, f64) -> Option<(Rgba, f64)>, mut put: impl FnMut(u8, u8, u8, u8), premultiplied: bool) {
    const SS: usize = 4;
    let n = (SS * SS) as f64;
    for py in 0..size {
        for px in 0..size {
            let mut acc = [0.0f64; 4];
            for sy in 0..SS {
                for sx in 0..SS {
                    let x = px as f64 + (sx as f64 + 0.5) / SS as f64;
                    let y = py as f64 + (sy as f64 + 0.5) / SS as f64;
                    if let Some((c, a)) = sample(x, y) {
                        acc[0] += c.r as f64 * a;
                        acc[1] += c.g as f64 * a;
                        acc[2] += c.b as f64 * a;
                        acc[3] += a;
                    }
                }
            }
            let alpha = acc[3] / n;
            let channel = |v: f64| {
                let premul = v / n;
                let v = if premultiplied || alpha <= 0.0 { premul } else { premul / alpha };
                v.round().clamp(0.0, 255.0) as u8
            };
            put(channel(acc[0]), channel(acc[1]), channel(acc[2]), (alpha * 255.0).round() as u8);
        }
    }
}

/// The check: "M 0,5 L 4,9 12,0" stroked 2 units wide in a 12 x 10 box, at `scale` pixels per unit.
pub fn check_mark(ink: Rgba, scale: f64) -> Image {
    let points = [(0.0, 5.0), (4.0, 9.0), (12.0, 0.0)];
    let (w, h) = ((12.0 * scale) as u32, (10.0 * scale) as u32);
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(w.max(1), h.max(1));
    let width = buffer.width() as usize;
    for (i, p) in buffer.make_mut_slice().iter_mut().enumerate() {
        let (px, py) = (((i % width) as f64 + 0.5) / scale, ((i / width) as f64 + 0.5) / scale);
        let mut d = f64::MAX;
        for seg in points.windows(2) {
            let ((ax, ay), (bx, by)) = (seg[0], seg[1]);
            let (vx, vy) = (bx - ax, by - ay);
            let t = (((px - ax) * vx + (py - ay) * vy) / (vx * vx + vy * vy)).clamp(0.0, 1.0);
            d = d.min(((px - ax - t * vx).powi(2) + (py - ay - t * vy).powi(2)).sqrt());
        }
        // Half the stroke is 1 unit; half a pixel of softening either side.
        let coverage = ((1.0 - d) * scale + 0.5).clamp(0.0, 1.0);
        *p = Rgba8Pixel { r: ink.r, g: ink.g, b: ink.b, a: (coverage * ink.a as f64).round() as u8 };
    }
    Image::from_rgba8(buffer)
}
