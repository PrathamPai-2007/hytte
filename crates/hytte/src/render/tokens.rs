//! The look's numbers in one place: corner radii, text opacities and the pane rhythm.
//!
//! Radii nest: a shape inside a rounded one with `pad` px around it takes `inner(outer, pad)`,
//! so the two curves stay parallel.

/// The pill's bottom corners once it is tall enough (`render/pill.rs` caps them here).
pub const R_PILL: f32 = 24.0;
/// Large shapes inside the pill, 10 px in from its edge: tiles, cards.
pub const R_CARD: f32 = inner(R_PILL, 10.0);
/// A row's highlight: a shape 14 px in.
pub const R_ROW: f32 = inner(R_PILL, 14.0);

/// Credits and fine print: there if looked for, never competing with the content.
pub const INK_3: f32 = 0.40;

/// Room between a pane's last element and the tab strip under it.
pub const PANE_GAP: f32 = 12.0;

/// The radius for a shape `pad` px inside one of radius `outer`.
pub const fn inner(outer: f32, pad: f32) -> f32 {
    let r = outer - pad;
    if r < 2.0 {
        2.0
    } else {
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radii_nest_and_never_vanish() {
        assert_eq!((R_CARD, R_ROW), (14.0, 10.0));
        assert_eq!(inner(R_CARD, 20.0), 2.0);
    }
}
