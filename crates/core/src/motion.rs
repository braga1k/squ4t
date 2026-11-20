//! The 1nstall motion vocabulary: a port of `ElasticFeel`, `ElasticMotion` and the keyframe
//! shapes of `Motion` in 1nstall's src/motion.cs (4e4dc64). Same operations, same order.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Feel {
    pub stretch: f64,
    pub squash: f64,
    pub bounce: f64,
    pub duration_ms: u32,
}

impl Feel {
    pub const BALANCED: Feel = Feel { stretch: 0.2, squash: 0.1, bounce: 0.3, duration_ms: 720 };
    pub const FIRM: Feel = Feel { stretch: 0.0, squash: 0.0, bounce: 0.0, duration_ms: 220 };
    pub const ELASTIC: Feel = Feel { stretch: 1.0, squash: 0.22, bounce: 0.8, duration_ms: 1100 };
}

/// Knob position, the capsule's extra length, and its height scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub x: f64,
    pub stretch: f64,
    pub height: f64,
}

impl Pose {
    pub const fn new(x: f64, stretch: f64, height: f64) -> Pose {
        Pose { x, stretch, height }
    }
}

fn response(t: f64, bounce: f64) -> f64 {
    let damping = 1.0 - 0.8 * bounce;
    let w = 10.0 / damping;
    if bounce == 0.0 {
        return 1.0 - (1.0 + w * t) * (-w * t).exp();
    }
    let wd = w * (1.0 - damping * damping).sqrt();
    1.0 - (-damping * w * t).exp() * ((wd * t).cos() + damping * w / wd * (wd * t).sin())
}

/// A damped spring from 0 to 1 over t in [0, 1], normalised to land exactly on 1.
pub fn spring(t: f64, bounce: f64) -> f64 {
    if t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return 1.0;
    }
    response(t, bounce) / response(1.0, bounce)
}

fn speed(t: f64, bounce: f64) -> f64 {
    let lo = (t - 0.0001).max(0.0);
    let hi = (t + 0.0001).min(1.0);
    (spring(hi, bounce) - spring(lo, bounce)) / (hi - lo)
}

#[allow(clippy::too_many_arguments)]
fn sample(from: Pose, target: f64, feel: Feel, bounce: f64, peak: f64, length: f64, t: f64) -> Pose {
    if t == 0.0 {
        return from;
    }
    if t == 1.0 {
        return Pose::new(target, 0.0, 1.0);
    }
    let forward = target >= from.x + from.stretch / 2.0;
    let tail = if forward { from.x } else { from.x + from.stretch };
    let p = spring(t, bounce);
    let x = tail + (target - tail) * p;
    let amount = (feel.stretch * 14.0).min((target - tail).abs() * 0.8);
    let mut stretch = amount * speed(t, bounce).abs() / peak * length;
    let remaining = (target - x).abs() * 0.6;
    // Smooth minimum rounds the landing without a second animation or a corner.
    let soft = stretch * remaining / (stretch * stretch + remaining * remaining + 1e-12).sqrt();
    stretch += (soft - stretch) * (1.0 - feel.bounce / 0.5).max(0.0);
    let rest = 1.0 - spring(t, 0.0);
    // An interrupted capsule hands its existing length over to the new flight.
    stretch = stretch * (1.0 - (from.stretch / amount.max(0.0001)).min(1.0) * rest) + from.stretch * rest;
    let mut height = 1.0 - feel.squash * (if amount > 0.0 { stretch / amount.max(from.stretch) } else { 0.0 }) + feel.squash * 2.0 * (spring(t, feel.bounce) - 1.0).max(0.0);
    height += (from.height - 1.0 + feel.squash * (if amount > 0.0 { from.stretch / amount.max(from.stretch) } else { 0.0 })) * rest;
    Pose::new(if forward { x } else { x - stretch }, stretch, height.max(0.1))
}

/// 41 linear keyframes of a knob flight, bounded to the switch's track.
pub fn flight(from: Pose, target: f64, feel: Feel) -> [Pose; 41] {
    let mut bounce = feel.bounce;
    let mut peak = 1.0;
    let mut length = 1.0;
    // Check edges at a denser resolution than the compositor's keyframes.
    for _ in 0..100 {
        peak = 0.0;
        for i in 0..=400 {
            peak = f64::max(peak, speed(i as f64 / 400.0, bounce).abs());
        }
        let mut fits = true;
        for i in 0..=400 {
            let pose = sample(from, target, feel, bounce, peak, length, i as f64 / 400.0);
            if pose.x < -3.00001 || pose.x + pose.stretch > 19.00001 {
                fits = false;
                break;
            }
        }
        if fits {
            break;
        }
        if bounce > 0.0 {
            bounce = (bounce - 0.025).max(0.0);
        } else {
            // Reversing an already long capsule can leave less room than a resting knob.
            length *= 0.8;
        }
    }
    std::array::from_fn(|i| sample(from, target, feel, bounce, peak, length, i as f64 / 40.0))
}

/// Linear keyframes evenly spread over a duration, like `Motion.Animate`.
pub fn sample_linear(values: &[f64], t: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    if t <= 0.0 {
        return values[0];
    }
    if t >= 1.0 {
        return values[values.len() - 1];
    }
    let pos = t * (values.len() - 1) as f64;
    let i = pos.floor() as usize;
    let f = pos - i as f64;
    values[i] + (values[i + 1] - values[i]) * f
}

/// WPF `CubicEase { EasingMode = EaseOut }`.
pub fn cubic_out(t: f64) -> f64 {
    1.0 - (1.0 - t).powi(3)
}

/// WPF `CubicEase { EasingMode = EaseIn }`.
pub fn cubic_in(t: f64) -> f64 {
    t * t * t
}

/// `Motion.Frames`: from, held through the delay, an optional overshoot at 58%, then to,
/// each segment eased out. `elapsed_ms` counts from the start of the delay.
pub fn sample_frames(from: f64, to: f64, duration_ms: f64, delay_ms: f64, overshoot: Option<f64>, elapsed_ms: f64) -> f64 {
    if elapsed_ms <= delay_ms {
        return from;
    }
    let t = elapsed_ms - delay_ms;
    if t >= duration_ms {
        return to;
    }
    match overshoot {
        Some(peak) => {
            let first = duration_ms * 0.58;
            if t < first {
                from + (peak - from) * cubic_out(t / first)
            } else {
                peak + (to - peak) * cubic_out((t - first) / (duration_ms - first))
            }
        }
        None => from + (to - from) * cubic_out(t / duration_ms),
    }
}

/// The one easing curve of the interface, `cubic-bezier(0.22, 1, 0.36, 1)`: fast out of the
/// start, a long soft landing. Every eased motion in squ4t uses it.
pub fn ease(t: f64) -> f64 {
    const X1: f64 = 0.22;
    const Y1: f64 = 1.0;
    const X2: f64 = 0.36;
    const Y2: f64 = 1.0;
    if t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return 1.0;
    }
    let bez = |s: f64, a: f64, b: f64| 3.0 * (1.0 - s) * (1.0 - s) * s * a + 3.0 * (1.0 - s) * s * s * b + s * s * s;
    // x(s) is monotonic: bisect for the s whose x is t.
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..50 {
        let mid = (lo + hi) / 2.0;
        if bez(mid, X1, X2) < t {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    bez((lo + hi) / 2.0, Y1, Y2)
}

/// Eased keys `(time ms, value)`: the value at `elapsed_ms`, each segment on [`ease`].
pub fn sample_keys(keys: &[(f64, f64)], elapsed_ms: f64) -> f64 {
    let Some(&(_, first)) = keys.first() else { return 0.0 };
    if elapsed_ms <= keys[0].0 {
        return first;
    }
    for w in keys.windows(2) {
        let ((t0, v0), (t1, v1)) = (w[0], w[1]);
        if elapsed_ms <= t1 {
            let span = (t1 - t0).max(1e-9);
            return v0 + (v1 - v0) * ease((elapsed_ms - t0) / span);
        }
    }
    keys[keys.len() - 1].1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_easing_curve_is_monotonic_and_lands() {
        let mut last = 0.0;
        for i in 0..=100 {
            let v = ease(i as f64 / 100.0);
            assert!(v >= last - 1e-12 && v <= 1.0 + 1e-12);
            last = v;
        }
        assert_eq!((ease(0.0), ease(1.0)), (0.0, 1.0));
        assert!(ease(0.5) > 0.85, "most of the way there by half time");
        assert_eq!(sample_keys(&[(0.0, 10.0), (300.0, 0.0)], 300.0), 0.0);
    }

    #[test]
    fn flights_land_and_stay_inside_the_track() {
        for feel in [Feel::BALANCED, Feel::FIRM, Feel::ELASTIC] {
            for (from, target) in [(Pose::new(0.0, 0.0, 1.0), 16.0), (Pose::new(16.0, 0.0, 1.0), 0.0), (Pose::new(9.0, 4.0, 0.95), 0.0)] {
                let poses = flight(from, target, feel);
                assert_eq!(poses[0], from);
                assert_eq!(poses[40], Pose::new(target, 0.0, 1.0));
                for p in poses {
                    assert!(p.x >= -3.00001 && p.x + p.stretch <= 19.00001, "{feel:?} {p:?}");
                }
            }
        }
    }
}
