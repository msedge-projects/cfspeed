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
// The palette is Tokyo Night, taken from btop's own shipped theme
// (themes/tokyo-night.theme) rather than invented: bg #1a1b26, cyan #7dcfff for
// keyboard accents, and the green -> orange -> red graph ramp that btop uses for
// its meters. Using a palette that already ships inside a tool people trust on
// a dashboard beats picking colours by eye.

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

// --- Surfaces ---------------------------------------------------------------

/// Card background: a hair above the terminal background so panels read as
/// raised surfaces.
pub const SURFACE: Color = rgb(26, 27, 38);
/// Slightly brighter neutral grey for nested elements and meter tracks.
///
/// Deliberately desaturated. These two surfaces were Tokyo Night's own, which
/// carry a blue cast tuned to sit on bg `#1a1b26`; on a terminal whose
/// background is pure black that cast became visible as blue rules and blue-
/// tinged bars floating on the background.
pub const SURFACE_ALT: Color = rgb(48, 48, 50);
/// Hairline borders. Decorative only - never used for text.
pub const BORDER: Color = rgb(64, 64, 64);

// --- Accents ---------------------------------------------------------------

/// Download accent - btop's graph green.
pub const ACCENT_DOWN: Color = rgb(158, 206, 106);
/// Upload accent - btop's graph orange.
pub const ACCENT_UP: Color = rgb(224, 175, 104);
/// Latency / keyboard accent - Tokyo Night cyan.
pub const ACCENT_LAT: Color = rgb(125, 207, 255);

// --- Text -----------------------------------------------------------------

pub const TEXT: Color = rgb(169, 177, 214);
pub const TEXT_BRIGHT: Color = rgb(192, 202, 245);
pub const TEXT_DIM: Color = rgb(147, 153, 181);

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

/// Darken a colour toward black along its own hue.
///
/// Blending an accent toward `SURFACE_ALT` used to do this, but `SURFACE_ALT`
/// is blue-grey, so on a black terminal every shaded bar picked up a blue tinge
/// that fought the colour it was meant to be a darker version of.
#[must_use]
pub fn shade(color: Color, factor: f64) -> Color {
    let f = factor.clamp(0.0, 1.0);
    match color {
        Color::Rgb(r, g, b) => Color::Rgb(
            (f64::from(r) * f).round() as u8,
            (f64::from(g) * f).round() as u8,
            (f64::from(b) * f).round() as u8,
        ),
        other => other,
    }
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
    // Tokyo Night ramp: green -> yellow -> orange -> red, the same ordering
    // btop uses for its meters.
    match grade {
        "A+" | "A" => rgb(158, 206, 106),
        "B" => rgb(187, 224, 140),
        "C" => rgb(224, 175, 104),
        "D" => rgb(255, 158, 100),
        "F" => rgb(247, 118, 142),
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
        assert_eq!(grade_color("A+"), rgb(158, 206, 106));
        assert_eq!(grade_color("F"), rgb(247, 118, 142));
    }

    #[test]
    fn palette_matches_the_validated_contrast_report() {
        // tools/contrast_check.py asserts these clear WCAG AA (4.5:1) against
        // both SURFACE and SURFACE_ALT. Keep them in sync with that script.
        let to_hex = |c: Color| match c {
            Color::Rgb(r, g, b) => (r, g, b),
            _ => panic!("palette must use explicit RGB so it is theme-independent"),
        };
        assert_eq!(to_hex(SURFACE), (26, 27, 38));
        assert_eq!(to_hex(SURFACE_ALT), (48, 48, 50));
        assert_eq!(to_hex(BORDER), (64, 64, 64));
        assert_eq!(to_hex(TEXT), (169, 177, 214));
        assert_eq!(to_hex(TEXT_BRIGHT), (192, 202, 245));
        assert_eq!(to_hex(TEXT_DIM), (147, 153, 181));
        assert_eq!(to_hex(ACCENT_DOWN), (158, 206, 106));
        assert_eq!(to_hex(ACCENT_UP), (224, 175, 104));
        assert_eq!(to_hex(ACCENT_LAT), (125, 207, 255));
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

    /// Shading must darken along the colour's own hue. Blending toward a
    /// blue-grey surface did the job visually but tinted every bar blue on a
    /// black terminal, so this is the property worth locking in.
    #[test]
    fn shade_scales_a_colour_without_hue_shift() {
        use super::shade as theme_shade;
        let green = Color::Rgb(158, 206, 106);
        let dark = match theme_shade(green, 0.5) {
            Color::Rgb(r, g, b) => (r, g, b),
            other => panic!("expected rgb, got {other:?}"),
        };
        // Ratios between channels are preserved: that is what "no hue shift"
        // means.
        let (r0, g0, _b0) = (158.0, 206.0, 106.0);
        let ratio_before = g0 / r0;
        let ratio_after = f64::from(dark.1) / f64::from(dark.0).max(1.0);
        assert!(
            (ratio_before - ratio_after).abs() < 0.05,
            "hue shifted: {ratio_before} vs {ratio_after}"
        );
        assert_eq!(theme_shade(green, 1.0), green, "factor 1 must be identity");
        assert_eq!(
            theme_shade(green, 0.0),
            Color::Rgb(0, 0, 0),
            "factor 0 must be black"
        );
        // Non-rgb colours pass through rather than being mangled.
        assert_eq!(theme_shade(Color::Red, 0.5), Color::Red);
    }
}
