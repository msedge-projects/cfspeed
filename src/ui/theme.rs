//! Central palette and shared text styles.
//!
//! Hierarchy is expressed through colour weight rather than borders or boxes,
//! which is what separates a modern TUI from the default boxed look.

use ratatui::style::{Color, Modifier, Style};

pub const ACCENT_DOWN: Color = Color::Cyan;
pub const ACCENT_UP: Color = Color::Magenta;

pub const TEXT: Color = Color::Gray;
pub const TEXT_BRIGHT: Color = Color::White;
pub const TEXT_DIM: Color = Color::DarkGray;
pub const TEXT_MUTED: Color = Color::Gray;
pub const RULE: Color = Color::DarkGray;
pub const TRACK: Color = Color::Rgb(48, 48, 48);

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
        .add_modifier(Modifier::BOLD)
}

pub fn style_dim() -> Style {
    Style::default().fg(TEXT_DIM)
}

/// Colour a letter grade from best to worst.
#[must_use]
pub fn grade_color(grade: &str) -> Color {
    match grade {
        "A+" | "A" => Color::Green,
        "B" => Color::LightGreen,
        "C" => Color::Yellow,
        "D" => Color::LightRed,
        "F" => Color::Red,
        _ => TEXT_DIM,
    }
}

#[must_use]
pub fn phase_color(phase: crate::speed::Phase) -> Color {
    use crate::speed::Phase;
    match phase {
        Phase::Connecting => Color::Yellow,
        Phase::Idle => Color::Blue,
        Phase::Download => ACCENT_DOWN,
        Phase::Upload => ACCENT_UP,
        Phase::Finalising => Color::Yellow,
        Phase::Done => Color::Green,
    }
}

/// A hairline rule, sized to the given width.
#[must_use]
pub fn rule(width: u16) -> ratatui::text::Line<'static> {
    ratatui::text::Line::from(ratatui::text::Span::styled(
        "─".repeat(usize::from(width)),
        Style::default().fg(RULE),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::speed::Phase;

    #[test]
    fn grades_get_warmer_as_they_worsen() {
        assert_eq!(grade_color("A+"), Color::Green);
        assert_eq!(grade_color("C"), Color::Yellow);
        assert_eq!(grade_color("F"), Color::Red);
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
    fn rule_matches_width() {
        let r = rule(10);
        assert_eq!(r.spans[0].content.chars().count(), 10);
    }
}
