//! Reusable primitives for the bezelless layout.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

use crate::ui::theme;

/// Pill-shaped nav label. Used for the clickable view tabs.
pub fn pill(label: &str, active: bool, accent: Color) -> Span<'static> {
    if active {
        Span::styled(
            format!(" {label} "),
            Style::default()
                .fg(theme::SURFACE)
                .bg(accent)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(format!(" {label} "), Style::default().fg(theme::TEXT_DIM))
    }
}

/// Inverse-video label, for the active phase.
pub fn badge(text: &str, color: Color) -> Span<'static> {
    Span::styled(
        format!(" {text} "),
        Style::default()
            .fg(theme::SURFACE)
            .bg(color)
            .add_modifier(Modifier::BOLD),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_pill_gets_a_background() {
        let active = pill("LIVE", true, Color::Cyan);
        let idle = pill("LIVE", false, Color::Cyan);
        assert!(active.style.bg.is_some());
        assert!(idle.style.bg.is_none());
    }

    #[test]
    fn badge_always_inverts() {
        assert!(badge("DOWNLOAD", Color::Cyan).style.bg.is_some());
    }
}
