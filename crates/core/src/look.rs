//! The squ4t look, after an unslop pass (heydamir/unslop, CC BY 4.0).
//!
//! Direction:
//! - Product: a tray timer that tells you when to stand up for squats and keeps an honest
//!   count of the reps you actually did.
//! - Character: brisk, honest, physical.
//! - World: a gym wall: a stopwatch, chalk tallies, a rubber floor.
//! - Moments: the two counts, set large in steady figures.
//! - Signature: the countdown ring of the tray icon, repeated beside the status line.
//!
//! One graphite ramp for every surface, three text roles, one ember accent used only by the
//! ring and the switch. No gradients, glows or shadows: depth comes from surface steps and
//! hairlines. High contrast hands every colour to Windows.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Rgba {
        Rgba { r, g, b, a: 255 }
    }
    /// "#RRGGBB".
    pub const fn hex(v: u32) -> Rgba {
        Rgba::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
    }
    pub const fn with_alpha(self, a: u8) -> Rgba {
        Rgba { a, ..self }
    }
}

fn linear(v: u8) -> f64 {
    let c = v as f64 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

pub fn luminance(c: Rgba) -> f64 {
    0.2126 * linear(c.r) + 0.7152 * linear(c.g) + 0.0722 * linear(c.b)
}

/// WCAG contrast ratio of two opaque colours.
pub fn contrast(a: Rgba, b: Rgba) -> f64 {
    let (x, y) = (luminance(a) + 0.05, luminance(b) + 0.05);
    x.max(y) / x.min(y)
}

/// Windows' own colours, used as they are in high contrast.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SystemColors {
    pub window: Rgba,
    pub window_text: Rgba,
    pub highlight: Rgba,
    pub highlight_text: Rgba,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub light: bool,
    pub high_contrast: bool,
    /// The window.
    pub surface: Rgba,
    /// Inputs and secondary buttons, one step up; and that step again on hover.
    pub raised: Rgba,
    pub raised_hover: Rgba,
    /// Separators and control edges: primary text at 8%.
    pub hairline: Rgba,
    pub text: Rgba,
    pub text_secondary: Rgba,
    pub text_muted: Rgba,
    /// The primary button: an off-white on dark, ink on light.
    pub primary: Rgba,
    pub primary_hover: Rgba,
    pub on_primary: Rgba,
    /// Ember: the countdown ring and the switch, nothing else.
    pub accent: Rgba,
    /// The knob of a switch that is on.
    pub on_accent: Rgba,
    pub focus: Rgba,
    pub negative: Rgba,
}

pub const EMBER_DARK: Rgba = Rgba::hex(0xF07A3A);
pub const EMBER_LIGHT: Rgba = Rgba::hex(0xC2410C);

pub fn look(light: bool, high_contrast: bool, system: SystemColors) -> Look {
    if high_contrast {
        let s = system;
        return Look {
            light,
            high_contrast,
            surface: s.window,
            raised: s.window,
            raised_hover: s.window,
            hairline: s.window_text,
            text: s.window_text,
            text_secondary: s.window_text,
            text_muted: s.window_text,
            primary: s.highlight,
            primary_hover: s.highlight,
            on_primary: s.highlight_text,
            accent: s.highlight,
            on_accent: s.highlight_text,
            focus: s.window_text,
            negative: s.window_text,
        };
    }
    if light {
        let text = Rgba::hex(0x171717);
        Look {
            light,
            high_contrast,
            surface: Rgba::hex(0xFAFAFA),
            raised: Rgba::hex(0xF0F0F1),
            raised_hover: Rgba::hex(0xE7E7E9),
            hairline: text.with_alpha(20),
            text,
            text_secondary: Rgba::hex(0x525252),
            text_muted: Rgba::hex(0x737373),
            primary: text,
            primary_hover: Rgba::hex(0x2E2E2E),
            on_primary: Rgba::hex(0xFFFFFF),
            accent: EMBER_LIGHT,
            on_accent: Rgba::hex(0xFFFFFF),
            focus: text,
            negative: Rgba::hex(0xDC2626),
        }
    } else {
        let text = Rgba::hex(0xEDEDED);
        Look {
            light,
            high_contrast,
            surface: Rgba::hex(0x141415),
            raised: Rgba::hex(0x232325),
            raised_hover: Rgba::hex(0x2C2C2F),
            hairline: text.with_alpha(20),
            text,
            text_secondary: Rgba::hex(0xA1A1A6),
            text_muted: Rgba::hex(0x85858B),
            primary: text,
            primary_hover: Rgba::hex(0xFFFFFF),
            on_primary: Rgba::hex(0x141415),
            accent: EMBER_DARK,
            on_accent: Rgba::hex(0xFFFFFF),
            focus: text,
            negative: Rgba::hex(0xE5484D),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sys() -> SystemColors {
        SystemColors { window: Rgba::hex(0), window_text: Rgba::hex(0xFFFFFF), highlight: Rgba::hex(0x00FFFF), highlight_text: Rgba::hex(0) }
    }

    /// Text roles and the accent stay readable on their surfaces in both themes.
    #[test]
    fn contrast_holds() {
        for light in [false, true] {
            let l = look(light, false, sys());
            assert!(contrast(l.text, l.surface) >= 12.0, "primary text, light={light}");
            assert!(contrast(l.text_secondary, l.surface) >= 4.5, "secondary text, light={light}");
            assert!(contrast(l.text_muted, l.surface) >= 4.5, "muted text, light={light}");
            assert!(contrast(l.text, l.raised) >= 4.5, "text in inputs, light={light}");
            assert!(contrast(l.on_primary, l.primary) >= 4.5, "primary button label, light={light}");
            assert!(contrast(l.accent, l.surface) >= 3.0, "accent as a graphic, light={light}");
        }
    }

    #[test]
    fn high_contrast_uses_windows_colours() {
        let l = look(false, true, sys());
        assert_eq!((l.surface, l.text, l.accent), (Rgba::hex(0), Rgba::hex(0xFFFFFF), Rgba::hex(0x00FFFF)));
    }
}
