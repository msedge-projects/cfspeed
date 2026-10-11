//! Central palette and shared text styles.
//!
//! The surface colours below assume a dark terminal background, which is what
//! nearly every modern terminal ships with by default. Cards carry their own
//! slightly-lighter background so panels read like web surfaces rather than
//! outlines drawn on the page.

use ratatui::style::{Color, Modifier, Style};

use crate::speed::Phase;

// Every colour below is validated by `tools/contrast_check.py` against the two
// surfaces used in the UI. Body text clears WCAG AA (4.5:1) on both; `BORDER` is
// decorative and deliberately subtle, while *active* borders use an accent,
// which clears 7:1.
//
// Deliberately desaturated rather than the saturated cyan/magenta that every
// generic dashboard reaches for - the accents have to sit against a dark
// surface for a full test run without becoming fatiguing.

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

// --- Surfaces ---------------------------------------------------------------

/// Card background: a hair above the terminal background so panels read as
/// raised surfaces.
pub const SURFACE: Color = rgb(23, 24, 28);
/// Slightly brighter surface for nested elements (tiles inside cards).
pub const SURFACE_ALT: Color = rgb(31, 33, 38);
/// Hairline borders. Decorative only - never used for text.
pub const BORDER: Color = rgb(52, 56, 66);

// --- Accents ---------------------------------------------------------------

/// Download accent (muted teal).
pub const ACCENT_DOWN: Color = rgb(92, 192, 168);
/// Upload accent (warm amber).
pub const ACCENT_UP: Color = rgb(224, 164, 88);
/// Latency accent (muted lavender).
pub const ACCENT_LAT: Color = rgb(155, 143, 212);
/// Highlight along the crest of a chart.
pub const CREST: Color = rgb(237, 239, 242);

// --- Text -----------------------------------------------------------------

pub const TEXT: Color = rgb(196, 199, 206);
pub const TEXT_BRIGHT: Color = rgb(237, 239, 242);
pub const TEXT_DIM: Color = rgb(142, 147, 156);

pub fn style_title() -> Style {
    Style::default()
        .fg(TEXT_BRIGHT)
        .add_modifier(Modifier::BOLD)
}

pub fn style_label() -> Style {
    Style::default().fg(TEXT_DIM)
}

/// The single most important number on screen.
pub fn style_headline() -> Style {
    Style::default()
        .fg(TEXT_BRIGHT)
        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
}

/// Blend two RGB colours. Falls back to `to` if either is not an RGB colour.
#[must_use]
pub fn blend(from: Color, to: Color, t: f64) -> Color {
    let (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) = (from, to) else {
        return to;
    };
    let t = t.clamp(0.0, 1.0);
    let mix = |a: u8, b: u8| (f64::from(a) * (1.0 - t) + f64::from(b) * t).round() as u8;
    Color::Rgb(mix(r1, r2), mix(g1, g2), mix(b1, b2))
}

/// Colour a letter grade from best to worst.
#[must_use]
pub fn grade_color(grade: &str) -> Color {
    match grade {
        "A+" | "A" => rgb(111, 191, 115),
        "B" => rgb(163, 197, 101),
        "C" => rgb(224, 192, 96),
        "D" => rgb(224, 146, 94),
        "F" => rgb(220, 107, 98),
        _ => TEXT_DIM,
    }
}

#[must_use]
pub fn phase_color(phase: Phase) -> Color {
    match phase {
        Phase::Connecting | Phase::Finalising => rgb(224, 192, 96),
        Phase::Idle => ACCENT_LAT,
        Phase::Download => ACCENT_DOWN,
        Phase::Upload => ACCENT_UP,
        Phase::Done => rgb(111, 191, 115),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grades_get_warmer_as_they_worsen() {
        assert_eq!(grade_color("A+"), rgb(111, 191, 115));
        assert_eq!(grade_color("F"), rgb(220, 107, 98));
    }

    #[test]
    fn palette_matches_the_validated_contrast_report() {
        // tools/contrast_check.py asserts these clear WCAG AA (4.5:1) against
        // both SURFACE and SURFACE_ALT. Keep them in sync with that script.
        let to_hex = |c: Color| match c {
            Color::Rgb(r, g, b) => (r, g, b),
            _ => panic!("palette must use explicit RGB so it is theme-independent"),
        };
        assert_eq!(to_hex(SURFACE), (23, 24, 28));
        assert_eq!(to_hex(SURFACE_ALT), (31, 33, 38));
        assert_eq!(to_hex(BORDER), (52, 56, 66));
        assert_eq!(to_hex(TEXT), (196, 199, 206));
        assert_eq!(to_hex(TEXT_BRIGHT), (237, 239, 242));
        assert_eq!(to_hex(TEXT_DIM), (142, 147, 156));
        assert_eq!(to_hex(ACCENT_DOWN), (92, 192, 168));
        assert_eq!(to_hex(ACCENT_UP), (224, 164, 88));
        assert_eq!(to_hex(ACCENT_LAT), (155, 143, 212));
    }

    #[test]
    fn unknown_grade_is_muted() {
        assert_eq!(grade_color("-"), TEXT_DIM);
    }

    #[test]
    fn phases_have_distinct_colours() {
        assert_ne!(phase_color(Phase::Download), phase_color(Phase::Upload));
    }

    #[test]
    fn blend_interpolates_between_endpoints() {
        assert_eq!(
            blend(Color::Rgb(0, 0, 0), Color::Rgb(10, 20, 30), 0.0),
            Color::Rgb(0, 0, 0)
        );
        assert_eq!(
            blend(Color::Rgb(0, 0, 0), Color::Rgb(10, 20, 30), 1.0),
            Color::Rgb(10, 20, 30)
        );
        assert_eq!(
            blend(Color::Rgb(0, 0, 0), Color::Rgb(10, 20, 30), 0.5),
            Color::Rgb(5, 10, 15)
        );
    }

    #[test]
    fn blend_falls_back_for_non_rgb() {
        // Indexed colours have no channels to interpolate, so the target wins.
        assert_eq!(
            blend(Color::Red, Color::Rgb(1, 2, 3), 0.5),
            Color::Rgb(1, 2, 3)
        );
    }

    #[test]
    fn blend_clamps_out_of_range_t() {
        assert_eq!(
            blend(Color::Rgb(5, 5, 5), Color::Rgb(1, 2, 3), -1.0),
            Color::Rgb(5, 5, 5)
        );
        assert_eq!(
            blend(Color::Rgb(5, 5, 5), Color::Rgb(1, 2, 3), 9.0),
            Color::Rgb(1, 2, 3)
        );
    }
}
