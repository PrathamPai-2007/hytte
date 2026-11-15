//! Colour skins for the pill: its body gradient, the tint of its rim and the colour of the
//! neutral glow. Pure data, applied by the renderer with the brushes it already caches, so a
//! skin costs nothing at idle. `accent` follows the Windows accent colour (which Windows
//! picks from the wallpaper when Colours is on Automatic), set with [`set_accent`].

use std::sync::atomic::{AtomicU32, Ordering};

/// The Windows accent as 0xRRGGBB, or [`NO_ACCENT`] before it is known (then the accent skin
/// looks like graphite). A global because it is one system-wide colour, read by `Skin` methods
/// that stay plain data.
static ACCENT: AtomicU32 = AtomicU32::new(NO_ACCENT);
const NO_ACCENT: u32 = u32::MAX;

/// The Windows accent colour changed (or was read at start). None: not known.
pub fn set_accent(c: Option<[f32; 3]>) {
    let v = c.map_or(NO_ACCENT, |c| {
        let b = c.map(|x| (x.clamp(0.0, 1.0) * 255.0).round() as u32);
        (b[0] << 16) | (b[1] << 8) | b[2]
    });
    ACCENT.store(v, Ordering::Relaxed);
}

fn accent() -> Option<[f32; 3]> {
    match ACCENT.load(Ordering::Relaxed) {
        NO_ACCENT => None,
        v => Some([v >> 16, (v >> 8) & 0xFF, v & 0xFF].map(|c| c as f32 / 255.0)),
    }
}

/// The custom skin's colour as 0xRRGGBB: what `skin = "#rrggbb"` says, or the picker is on.
/// A global for the same reason as [`ACCENT`].
static CUSTOM: AtomicU32 = AtomicU32::new(0x4C8DFF);

/// The custom skin's colour.
pub fn custom() -> u32 {
    CUSTOM.load(Ordering::Relaxed)
}

/// Sets the custom skin's colour (0xRRGGBB).
pub fn set_custom(v: u32) {
    CUSTOM.store(v & 0xFF_FFFF, Ordering::Relaxed);
}

fn rgb(v: u32) -> [f32; 3] {
    [v >> 16, (v >> 8) & 0xFF, v & 0xFF].map(|c| c as f32 / 255.0)
}

/// "#rrggbb" (or "rrggbb") as 0xRRGGBB.
pub fn parse_hex(s: &str) -> Option<u32> {
    let h = s.trim();
    let h = h.strip_prefix('#').unwrap_or(h);
    (h.len() == 6 && h.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| u32::from_str_radix(h, 16).ok())
        .flatten()
}

/// 0xRRGGBB as "#rrggbb", the way `config.toml` holds a custom skin.
pub fn hex(v: u32) -> String {
    format!("#{:06x}", v & 0xFF_FFFF)
}

/// The hue and vividness (HSV saturation), both 0..1, that the picker shows for a colour. The
/// brightness doesn't matter: the skin is kept dark (or light) whatever it is.
pub fn hue_vivid(v: u32) -> (f32, f32) {
    let [r, g, b] = rgb(v);
    let max = r.max(g).max(b);
    let d = max - r.min(g).min(b);
    if d < 1e-4 {
        return (0.0, 0.0);
    }
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h / 6.0, d / max)
}

/// The colour at hue `h` and vividness `s` (0..1), full brightness, as 0xRRGGBB.
pub fn from_hue_vivid(h: f32, s: f32) -> u32 {
    let h = h.rem_euclid(1.0) * 6.0;
    let s = s.clamp(0.0, 1.0);
    let x = 1.0 - (h % 2.0 - 1.0).abs();
    let (r, g, b) = match h as u32 {
        0 => (1.0, x, 0.0),
        1 => (x, 1.0, 0.0),
        2 => (0.0, 1.0, x),
        3 => (0.0, x, 1.0),
        4 => (x, 0.0, 1.0),
        _ => (1.0, 0.0, x),
    };
    let c = |v: f32| ((1.0 - s + s * v) * 255.0).round() as u32;
    (c(r) << 16) | (c(g) << 8) | c(b)
}

/// The accent's hue at full strength, a little washed out so a loud accent stays calm.
fn tint(a: [f32; 3], wash: f32) -> [f32; 3] {
    let max = a[0].max(a[1]).max(a[2]);
    let n = if max < 0.01 {
        [1.0; 3]
    } else {
        a.map(|c| c / max)
    };
    n.map(|c| c + (1.0 - c) * wash)
}

/// The accent skin's body, top and bottom, from the accent colour `a`. Kept as dark (or as
/// light) as the other skins whatever the accent, so the text on it always reads.
pub fn accent_body(a: [f32; 3], light: bool) -> ([f32; 3], [f32; 3]) {
    let n = tint(a, 0.25);
    if light {
        (
            n.map(|c| 1.0 - (1.0 - c) * 0.04),
            n.map(|c| 1.0 - (1.0 - c) * 0.2),
        )
    } else {
        (n.map(|c| 0.02 + c * 0.06), n.map(|c| 0.04 + c * 0.16))
    }
}

/// The accent skin's glow.
pub fn accent_glow(a: [f32; 3]) -> [f32; 3] {
    tint(a, 0.15)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Skin {
    #[default]
    Graphite,
    Aurora,
    Ember,
    Moss,
    Dusk,
    /// Follows the Windows accent colour.
    Accent,
    /// A colour of the user's own ([`set_custom`]).
    Custom,
}

impl Skin {
    pub const ALL: [Skin; 7] = [
        Skin::Graphite,
        Skin::Aurora,
        Skin::Ember,
        Skin::Moss,
        Skin::Dusk,
        Skin::Accent,
        Skin::Custom,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Skin::Graphite => "graphite",
            Skin::Aurora => "aurora",
            Skin::Ember => "ember",
            Skin::Moss => "moss",
            Skin::Dusk => "dusk",
            Skin::Accent => "accent",
            Skin::Custom => "custom",
        }
    }

    /// What, besides the skin itself, the body brush depends on: the accent colour for the
    /// accent skin (0xRRGGBB fits an f32 exactly), nothing for the others.
    pub fn cache_key(self) -> f32 {
        match self {
            Skin::Accent => ACCENT.load(Ordering::Relaxed) as f32,
            Skin::Custom => custom() as f32,
            _ => 0.0,
        }
    }

    /// From `[general] skin`; anything unknown is the default. A colour ("#rrggbb") is the
    /// custom skin, and becomes its colour.
    pub fn from_name(s: &str) -> Skin {
        if let Some(v) = parse_hex(s) {
            set_custom(v);
            return Skin::Custom;
        }
        Skin::ALL
            .into_iter()
            .find(|k| k.name() == s.trim().to_ascii_lowercase())
            .unwrap_or_default()
    }

    /// Body gradient, top and bottom.
    pub fn body(self, light: bool) -> ([f32; 3], [f32; 3]) {
        match (self, light) {
            (Skin::Graphite, false) => ([0.027, 0.027, 0.035], [0.075, 0.075, 0.095]),
            (Skin::Graphite, true) => ([0.97, 0.97, 0.985], [0.89, 0.89, 0.92]),
            (Skin::Aurora, false) => ([0.020, 0.070, 0.090], [0.055, 0.165, 0.215]),
            (Skin::Aurora, true) => ([0.93, 0.98, 0.98], [0.80, 0.92, 0.93]),
            (Skin::Ember, false) => ([0.090, 0.040, 0.030], [0.200, 0.090, 0.060]),
            (Skin::Ember, true) => ([0.99, 0.95, 0.93], [0.96, 0.85, 0.80]),
            (Skin::Moss, false) => ([0.030, 0.070, 0.040], [0.090, 0.160, 0.100]),
            (Skin::Moss, true) => ([0.95, 0.98, 0.94], [0.85, 0.92, 0.83]),
            (Skin::Dusk, false) => ([0.060, 0.040, 0.110], [0.150, 0.110, 0.260]),
            (Skin::Dusk, true) => ([0.96, 0.95, 0.99], [0.87, 0.84, 0.96]),
            (Skin::Accent, light) => match accent() {
                Some(a) => accent_body(a, light),
                None => Skin::Graphite.body(light),
            },
            (Skin::Custom, light) => accent_body(rgb(custom()), light),
        }
    }

    /// The colour of the glow where a scene has no colour of its own (Home, Media, panes).
    pub fn glow(self) -> [f32; 3] {
        match self {
            Skin::Graphite => [1.0; 3],
            Skin::Aurora => [0.45, 0.90, 0.78],
            Skin::Ember => [1.0, 0.60, 0.35],
            Skin::Moss => [0.65, 0.85, 0.60],
            Skin::Dusk => [0.70, 0.60, 1.0],
            Skin::Accent => accent().map_or([1.0; 3], accent_glow),
            Skin::Custom => accent_glow(rgb(custom())),
        }
    }

    /// The rim's colour: the text colour (`fg`) for graphite, a tint of the glow otherwise.
    pub fn rim(self, fg: [f32; 3], light: bool) -> [f32; 3] {
        if self == Skin::Graphite {
            return fg;
        }
        let g = self.glow();
        // Deeper on the light pill so the line still shows.
        let k = if light { 0.55 } else { 1.0 };
        g.map(|c| c * k)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_and_unknowns_fall_back() {
        for s in Skin::ALL {
            assert_eq!(Skin::from_name(s.name()), s);
        }
        assert_eq!(Skin::from_name(" Aurora "), Skin::Aurora);
        assert_eq!(Skin::from_name("neon"), Skin::Graphite);
        assert_eq!(Skin::from_name(""), Skin::Graphite);
    }

    #[test]
    fn dark_skins_stay_dark_and_light_ones_stay_light_so_text_reads() {
        let luma = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        for s in Skin::ALL {
            let (t, b) = s.body(false);
            assert!(
                luma(t) < 0.12 && luma(b) < 0.22,
                "{s:?} dark body too bright"
            );
            assert!(luma(b) >= luma(t), "{s:?}: the body brightens downward");
            let (t, b) = s.body(true);
            assert!(luma(t) > 0.8 && luma(b) > 0.75, "{s:?} light body too dark");
        }
    }

    #[test]
    fn graphite_is_exactly_what_shipped_before() {
        assert_eq!(
            Skin::Graphite.body(false),
            ([0.027, 0.027, 0.035], [0.075, 0.075, 0.095])
        );
        assert_eq!(
            Skin::Graphite.body(true),
            ([0.97, 0.97, 0.985], [0.89, 0.89, 0.92])
        );
        assert_eq!(Skin::Graphite.glow(), [1.0; 3]);
        assert_eq!(Skin::Graphite.rim([0.5; 3], false), [0.5; 3]);
    }

    #[test]
    fn only_graphite_has_a_neutral_glow() {
        // The accent skin glows in whatever the Windows accent is, so it is left out here.
        for s in Skin::ALL
            .into_iter()
            .filter(|s| !matches!(s, Skin::Accent | Skin::Custom))
        {
            assert_eq!(s.glow() == [1.0; 3], s == Skin::Graphite, "{s:?}");
        }
    }

    /// WCAG contrast between two sRGB colours.
    fn contrast(a: [f32; 3], b: [f32; 3]) -> f32 {
        let lin = |c: f32| {
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        let lum = |c: [f32; 3]| 0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2]);
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn text_reads_on_the_accent_skin_whatever_the_accent() {
        let accents = [
            [0.0, 0.47, 0.84],  // Windows blue
            [1.0, 0.85, 0.0],   // a loud yellow
            [0.9, 0.1, 0.1],    // red
            [0.0, 0.0, 0.0],    // black
            [1.0, 1.0, 1.0],    // white
            [0.45, 0.45, 0.45], // grey
            [0.0, 0.0, 1.0],    // pure blue
        ];
        let (white, ink) = ([1.0; 3], [0.07, 0.07, 0.09]);
        for a in accents {
            let (t, b) = accent_body(a, false);
            assert!(
                contrast(white, t) >= 4.5 && contrast(white, b) >= 4.5,
                "{a:?} dark"
            );
            let (t, b) = accent_body(a, true);
            assert!(
                contrast(ink, t) >= 4.5 && contrast(ink, b) >= 4.5,
                "{a:?} light"
            );
            let g = accent_glow(a);
            assert!(
                g.iter().all(|c| (0.0..=1.0).contains(c)),
                "{a:?} glow {g:?}"
            );
        }
    }

    #[test]
    fn custom_colours_parse_and_round_trip_through_the_picker() {
        assert_eq!(parse_hex("#3A6EA5"), Some(0x3A6EA5));
        assert_eq!(parse_hex(" 3a6ea5 "), Some(0x3A6EA5));
        assert_eq!(parse_hex("#3a6ea"), None);
        assert_eq!(parse_hex("#3a6eaz"), None);
        assert_eq!(parse_hex("+3a6ea"), None);
        assert_eq!(hex(0x3A6EA5), "#3a6ea5");
        assert_eq!(Skin::from_name("#ff0000"), Skin::Custom);
        assert_eq!(custom(), 0xFF0000);
        assert_eq!(from_hue_vivid(0.0, 1.0), 0xFF0000);
        assert_eq!(from_hue_vivid(1.0 / 3.0, 1.0), 0x00FF00);
        assert_eq!(from_hue_vivid(0.5, 0.0), 0xFFFFFF);
        for h in 0..36 {
            for s in [0.2f32, 0.6, 1.0] {
                let v = from_hue_vivid(h as f32 / 36.0, s);
                let (h2, s2) = hue_vivid(v);
                let dh = (h2 - h as f32 / 36.0).abs();
                assert!(dh.min(1.0 - dh) < 0.01 && (s2 - s).abs() < 0.01, "{h} {s}");
            }
        }
    }

    #[test]
    fn the_accent_skin_follows_the_accent_and_falls_back_to_graphite() {
        set_accent(None);
        assert_eq!(Skin::Accent.body(false), Skin::Graphite.body(false));
        assert_eq!(Skin::Accent.glow(), [1.0; 3]);
        let a = [0.0, 0.47, 0.84];
        set_accent(Some(a));
        let back = accent().unwrap();
        assert!(back.iter().zip(a).all(|(x, y)| (x - y).abs() < 0.003));
        assert_ne!(Skin::Accent.cache_key(), 0.0);
        assert_eq!(Skin::Accent.body(true), accent_body(back, true));
        // A blue accent makes a blue-tinted pill.
        let (_, b) = Skin::Accent.body(false);
        assert!(b[2] > b[0] && b[2] > b[1]);
        set_accent(None);
    }
}
