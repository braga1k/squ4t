//! The one sound squ4t makes: a set recorded lands like a weight plate set down on a rubber
//! floor, a soft thud under a short metallic ring. It is synthesised here, once, as a WAV in
//! memory, so there is no audio file to ship. It plays as a system sound: the System Sounds
//! slider in the Windows volume mixer sets its level and silences it.

use std::f64::consts::TAU;
use std::sync::OnceLock;

use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT, SND_SYSTEM};

const RATE: u32 = 44_100;

/// Ring partials of a struck plate: ratio to the fundamental, level, decay per second. The
/// ratios are inharmonic, as a disc's are, which is what makes it read as metal.
const PARTIALS: [(f64, f64, f64); 4] = [(1.0, 1.0, 9.0), (2.32, 0.42, 13.0), (4.25, 0.2, 20.0), (6.8, 0.09, 30.0)];
const FUNDAMENTAL: f64 = 660.0;

/// The samples, in -1..1.
pub fn plate() -> Vec<f64> {
    let n = (RATE as f64 * 0.55) as usize;
    let mut noise = 0x2545_F491_u32;
    let mut out: Vec<f64> = (0..n)
        .map(|i| {
            let t = i as f64 / RATE as f64;
            // The ring, with a 2 ms attack so it starts without a click.
            let attack = (t / 0.002).min(1.0);
            let ring: f64 = PARTIALS.iter().map(|&(ratio, level, decay)| level * (TAU * FUNDAMENTAL * ratio * t).sin() * (-decay * t).exp()).sum();
            // The plate meeting the floor: a low thump whose pitch drops as it settles.
            let thump_freq = 95.0 + 60.0 * (-40.0 * t).exp();
            let thump = 0.9 * (TAU * thump_freq * t).sin() * (-28.0 * t).exp();
            // The strike: a few milliseconds of noise.
            noise ^= noise << 13;
            noise ^= noise >> 17;
            noise ^= noise << 5;
            let grit = (noise as f64 / u32::MAX as f64 * 2.0 - 1.0) * 0.25 * (-900.0 * t).exp();
            attack * (0.55 * ring + thump) + grit
        })
        .collect();
    // A 30 ms fade at the end, and a peak of -6 dBFS.
    let fade = (RATE as f64 * 0.03) as usize;
    for (k, s) in out[n - fade..].iter_mut().enumerate() {
        *s *= 1.0 - k as f64 / fade as f64;
    }
    let peak = out.iter().fold(0.0f64, |m, s| m.max(s.abs()));
    out.iter_mut().for_each(|s| *s *= 0.5 / peak);
    out
}

/// A 16-bit mono PCM WAV file.
pub fn wav(samples: &[f64]) -> Vec<u8> {
    let data = samples.len() as u32 * 2;
    let mut b = Vec::with_capacity(44 + data as usize);
    b.extend(b"RIFF");
    b.extend((36 + data).to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(1u16.to_le_bytes()); // PCM
    b.extend(1u16.to_le_bytes()); // mono
    b.extend(RATE.to_le_bytes());
    b.extend((RATE * 2).to_le_bytes());
    b.extend(2u16.to_le_bytes());
    b.extend(16u16.to_le_bytes());
    b.extend(b"data");
    b.extend(data.to_le_bytes());
    for s in samples {
        b.extend(((s.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes());
    }
    b
}

/// Plays the plate, without waiting for it.
pub fn recorded() {
    static WAV: OnceLock<Vec<u8>> = OnceLock::new();
    let bytes = WAV.get_or_init(|| wav(&plate()));
    // SAFETY: the WAV lives for the rest of the process, so it outlasts the asynchronous play.
    unsafe { PlaySoundW(bytes.as_ptr().cast(), std::ptr::null_mut(), SND_MEMORY | SND_ASYNC | SND_NODEFAULT | SND_SYSTEM) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plate_is_short_and_never_clips() {
        let s = plate();
        assert_eq!(s.len(), (RATE as f64 * 0.55) as usize);
        let peak = s.iter().fold(0.0f64, |m, x| m.max(x.abs()));
        assert!((peak - 0.5).abs() < 1e-9);
        assert!(s[0].abs() < 0.3 && s.last().unwrap().abs() < 1e-3);
        let w = wav(&s);
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(w.len(), 44 + s.len() * 2);
    }
}
