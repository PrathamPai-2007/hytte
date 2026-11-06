//! Colour skins for the pill: its body gradient, the tint of its rim and the colour of the
//! neutral glow. Pure data, applied by the renderer with the brushes it already caches, so a
//! skin costs nothing at idle.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Skin {
    #[default]
    Graphite,
    Aurora,
    Ember,
    Moss,
    Dusk,
}

impl Skin {
    pub const ALL: [Skin; 5] = [
        Skin::Graphite,
        Skin::Aurora,
        Skin::Ember,
        Skin::Moss,
        Skin::Dusk,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Skin::Graphite => "graphite",
            Skin::Aurora => "aurora",
            Skin::Ember => "ember",
            Skin::Moss => "moss",
            Skin::Dusk => "dusk",
        }
    }

    /// From `[general] skin`; anything unknown is the default.
    pub fn from_name(s: &str) -> Skin {
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
        for s in Skin::ALL {
            assert_eq!(s.glow() == [1.0; 3], s == Skin::Graphite, "{s:?}");
        }
    }
}
