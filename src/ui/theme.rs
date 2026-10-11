//! Central palette and shared text styles.
//!
//! The surface colours below assume a dark terminal background, which is what
//! nearly every modern terminal ships with by default. Cards carry their own
//! slightly-lighter background so panels read like web surfaces rather than
//! outlines drawn on the page.

use ratatui::style::{Color, Modifier, Style};

use crate::speed::Phase;

// --- Surfaces ---------------------------------------------------------------

/// Card background: a hair above the terminal background so panels read as
/// raised surfaces.
pub const SURFACE: Color = Color::Rgb(24, 24, 27);
/// Slightly brighter surface for nested elements (tiles inside cards).
pub const SURFACE_ALT: Color = Color::Rgb(39, 39, 42);
/// Hairline borders.
pub const BORDER: Color = Color::Rgb(63, 63, 70);

// --- Accents ---------------------------------------------------------------

/// Download accent (sky).
pub const ACCENT_DOWN: Color = Color::Rgb(56, 189, 248);
/// Upload accent (fuchsia).
pub const ACCENT_UP: Color = Color::Rgb(232, 121, 249);
/// Latency accent.
pub const ACCENT_LAT: Color = Color::Rgb(167, 139, 250);
/// Highlight along the crest of a chart.
pub const CREST: Color = Color::Rgb(226, 232, 240);

// --- Text -----------------------------------------------------------------

pub const TEXT: Color = Color::Rgb(212, 212, 216);
pub const TEXT_BRIGHT: Color = Color::Rgb(250, 250, 250);
pub const TEXT_DIM: Color = Color::Rgb(113, 113, 122);

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
        "A+" | "A" => Color::Rgb(74, 222, 128),
        "B" => Color::Rgb(163, 230, 53),
        "C" => Color::Rgb(250, 204, 21),
        "D" => Color::Rgb(251, 146, 60),
        "F" => Color::Rgb(248, 113, 113),
        _ => TEXT_DIM,
    }
}

#[must_use]
pub fn phase_color(phase: Phase) -> Color {
    match phase {
        Phase::Connecting => Color::Rgb(250, 204, 21),
        Phase::Idle => ACCENT_LAT,
        Phase::Download => ACCENT_DOWN,
        Phase::Upload => ACCENT_UP,
        Phase::Finalising => Color::Rgb(250, 204, 21),
        Phase::Done => Color::Rgb(74, 222, 128),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grades_get_warmer_as_they_worsen() {
        assert_eq!(grade_color("A+"), Color::Rgb(74, 222, 128));
        assert_eq!(grade_color("F"), Color::Rgb(248, 113, 113));
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
