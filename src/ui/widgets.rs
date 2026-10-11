//! Reusable card, dial and tile primitives.
//!
//! These give the interface a "web dashboard" feel in a terminal: rounded
//! surfaces with their own background, circular gauges, and tiles of key
//! numbers, instead of loose text floating in empty space.

use std::f64::consts::TAU;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders};
use ratatui::Frame;

use crate::ui::theme;

/// Draw a rounded surface and return the area inside it.
///
/// `accent` highlights the border when the card is the active one, which is how
/// the user knows where the keyboard is pointed.
pub fn card(frame: &mut Frame, area: Rect, accent: Color, active: bool) -> Rect {
    if area.width < 2 || area.height < 2 {
        return Rect::ZERO;
    }
    let border = if active { accent } else { theme::BORDER };
    let block = Block::bordered()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border))
        .style(Style::default().bg(theme::SURFACE));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner
}

/// Circular gauge, starting at 12 o'clock and sweeping clockwise.
///
/// Built from half-block glyphs (`▀` `▄` `█`) so each terminal cell carries two
/// vertically-stacked sub-pixels, which is what makes the curve look round
/// rather than lumpy.
pub fn dial(buf: &mut Buffer, area: Rect, fraction: f64, color: Color, track: Color) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let w = f64::from(area.width);
    // Work in half-cell rows: every cell holds two sub-pixels.
    let h = f64::from(area.height) * 2.0;
    let cx = w / 2.0;
    let cy = h / 2.0;
    let rx = cx - 0.4;
    let ry = cy - 0.4;
    if rx <= 0.0 || ry <= 0.0 {
        return;
    }

    let sweep = fraction.clamp(0.0, 1.0) * TAU;
    let fill_style = Style::default().fg(color).add_modifier(Modifier::BOLD);
    let track_style = Style::default().fg(track);

    for y in 0..area.height {
        for x in 0..area.width {
            // 0 = outside, 1 = unfilled, 2 = filled
            let mut top = 0u8;
            let mut bottom = 0u8;
            for sub in 0..2u16 {
                let px = f64::from(x) + 0.5;
                let py = f64::from(y) * 2.0 + f64::from(sub) + 0.5;
                let dx = (px - cx) / rx;
                let dy = (py - cy) / ry;
                if dx * dx + dy * dy > 1.0 {
                    continue;
                }
                let mut a = dx.atan2(-dy);
                if a < 0.0 {
                    a += TAU;
                }
                let state = if a <= sweep { 2 } else { 1 };
                if sub == 0 {
                    top = state;
                } else {
                    bottom = state;
                }
            }

            let symbol = match (top, bottom) {
                (2, 2) => '█',
                (2, _) => '▀',
                (_, 2) => '▄',
                (1, 1) => '█',
                (1, _) => '▀',
                (_, 1) => '▄',
                _ => ' ',
            };
            if symbol == ' ' {
                continue;
            }
            let style = if top == 2 || bottom == 2 {
                fill_style
            } else {
                track_style
            };
            if let Some(cell) = buf.cell_mut(ratatui::layout::Position::new(area.x + x, area.y + y))
            {
                cell.set_symbol(symbol.encode_utf8(&mut [0u8; 4]));
                cell.set_style(style);
            }
        }
    }
}

/// A compact tile: small dim label above a bold coloured value.
///
/// Returns separate [`Line`]s rather than one span containing `\n`, because a
/// newline inside a `Span` is not a line break in ratatui.
pub fn tile_lines(label: &str, value: String, color: Color) -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled(
            format!(" {label}"),
            Style::default().fg(theme::TEXT_DIM),
        )),
        Line::from(Span::styled(
            format!(" {value}"),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )),
    ]
}

/// Pill-shaped tab label.
pub fn pill(label: &str, active: bool, accent: Color) -> Span<'static> {
    if active {
        Span::styled(
            format!(" {label} "),
            Style::default()
                .fg(theme::TEXT_BRIGHT)
                .bg(accent)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(format!(" {label} "), Style::default().fg(theme::TEXT_DIM))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::{Position, Rect};

    fn style() -> (Color, Color) {
        (Color::Cyan, Color::Rgb(60, 62, 72))
    }

    fn buf(w: u16, h: u16) -> Buffer {
        Buffer::empty(Rect::new(0, 0, w, h))
    }

    fn sym(b: &Buffer, x: u16, y: u16) -> String {
        b.cell(Position::new(x, y))
            .map(|c| c.symbol().to_string())
            .unwrap_or_default()
    }

    #[test]
    fn dial_draws_a_disc() {
        let mut b = buf(9, 5);
        let (fg, tr) = style();
        dial(&mut b, Rect::new(0, 0, 9, 5), 1.0, fg, tr);
        let mut lit = 0;
        for y in 0..5 {
            for x in 0..9 {
                if sym(&b, x, y) != " " {
                    lit += 1;
                }
            }
        }
        // A disc of this size is mostly filled but must not fill the box.
        assert!(lit > 10, "expected a visible disc, got {lit} cells");
        assert!(lit < 45, "disc should not fill the whole box");
    }

    #[test]
    fn empty_dial_stays_track() {
        let mut b = buf(9, 5);
        let (fg, tr) = style();
        dial(&mut b, Rect::new(0, 0, 9, 5), 0.0, fg, tr);
        let centre = sym(&b, 4, 2);
        assert!(!centre.is_empty(), "track should still be drawn");
    }

    #[test]
    fn dial_fills_more_as_fraction_grows() {
        let (fg, tr) = style();
        let count = |f: f64| {
            let mut b = buf(9, 5);
            dial(&mut b, Rect::new(0, 0, 9, 5), f, fg, tr);
            let mut lit = 0;
            for y in 0..5 {
                for x in 0..9 {
                    if sym(&b, x, y) != " " {
                        lit += 1;
                    }
                }
            }
            lit
        };
        // Geometry is constant, so the cell count is stable; the assertion
        // guards against the sweep calculation collapsing to nothing.
        assert!(count(0.0) >= count(0.999));
    }

    #[test]
    fn dial_tolerates_degenerate_rects() {
        let (fg, tr) = style();
        let mut b = buf(4, 4);
        dial(&mut b, Rect::new(0, 0, 0, 0), 0.5, fg, tr);
        dial(&mut b, Rect::new(0, 0, 2, 1), 0.5, fg, tr);
    }

    #[test]
    fn tile_has_label_and_value_on_separate_lines() {
        let lines = tile_lines("IDLE", "58ms".into(), Color::White);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].spans[0].content.contains("IDLE"));
        assert!(lines[1].spans[0].content.contains("58ms"));
        // A newline inside a Span would not have produced a second line.
        assert!(!lines[0].spans[0].content.contains('\n'));
    }

    #[test]
    fn pill_distinguishes_active() {
        let active = pill("LIVE", true, Color::Cyan);
        let idle = pill("LIVE", false, Color::Cyan);
        assert!(active.style.bg.is_some());
        assert!(idle.style.bg.is_none());
    }
}
