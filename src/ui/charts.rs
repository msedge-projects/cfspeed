//! Chart primitives rendered directly into a ratatui `Buffer`.
//!
//! Braille glyphs pack 2 columns x 4 rows of dots into one terminal cell,
//! giving 8x the vertical resolution of a character sparkline. That is what
//! makes the graphs read as smooth curves rather than coarse staircases.
//!
//! Note: a glyph carries a single colour, so the "leading edge" highlight is
//! applied per cell — the cells containing the topmost lit dots get the bright
//! colour, producing a one-cell-tall ridge that traces the curve.

use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};

/// Bit for the braille dot at sub-column `sx` (0=left, 1=right) and sub-row
/// `sy` (0=top .. 3=bottom).
///
/// Braille dots are numbered:
/// ```text
/// dot1 dot4
/// dot2 dot5
/// dot3 dot6
/// dot7 dot8
/// ```
#[must_use]
pub fn dot_bit(sx: usize, sy: usize) -> u8 {
    // Columns 1-3 run down the left, 4-6 down the right, and 7-8 across the
    // bottom - they are not interleaved.
    let dot = match sy {
        0..=2 => {
            if sx == 0 {
                sy + 1
            } else {
                sy + 4
            }
        }
        _ => 7 + sx,
    };
    1u8 << (dot - 1)
}

/// Glyph for a raw dot bitmask.
#[must_use]
pub fn braille_from_bits(bits: u8) -> char {
    char::from_u32(0x2800 + u32::from(bits)).unwrap_or(' ')
}

/// Dot bitmask for a cell given per-sub-column lit heights.
#[must_use]
pub fn cell_bits(levels: &[u8; 2]) -> u8 {
    let mut bits: u8 = 0;
    for sx in 0..2usize {
        let level = usize::from(levels[sx].min(4));
        for sy in 0..4usize {
            if sy >= 4 - level {
                bits |= dot_bit(sx, sy);
            }
        }
    }
    bits
}

/// Build the glyph for a cell from the number of lit dots per sub-column,
/// counting upward from the bottom of the cell (0..=4).
#[must_use]
pub fn braille_cell(levels: [u8; 2]) -> char {
    braille_from_bits(cell_bits(&levels))
}

#[derive(Clone, Copy)]
pub struct ChartStyle {
    pub fill: Color,
    pub edge: Color,
    pub axis: Color,
}

/// Ease a chart's vertical scale toward the observed maximum, so the graph does
/// not rescale violently on every sample.
#[derive(Default)]
pub struct Scaler {
    current: f64,
}

impl Scaler {
    /// Leave headroom above the peak so the curve never pegs to the top edge.
    const HEADROOM: f64 = 1.12;

    pub fn new(initial: f64) -> Self {
        Self {
            current: (initial * Self::HEADROOM).max(1.0),
        }
    }

    pub fn reset(&mut self, value: f64) {
        self.current = (value * Self::HEADROOM).max(1.0);
    }

    /// Rise instantly so a peak is never clipped; fall gently so the graph does
    /// not jitter as samples decay.
    pub fn update(&mut self, observed_max: f64) -> f64 {
        let target = (observed_max * Self::HEADROOM).max(1.0);
        if target > self.current {
            self.current = target;
        } else {
            self.current += (target - self.current) * 0.08;
            if self.current < 1.0 {
                self.current = 1.0;
            }
        }
        self.current
    }

    pub fn value(&self) -> f64 {
        self.current
    }
}

/// Paint a filled area chart of `data` into `area`.
///
/// Newest samples sit at the right edge; when there is more data than fits the
/// most recent window is shown (the chart scrolls). While the test is still
/// early the curve grows left-to-right.
pub fn render_area(
    buf: &mut Buffer,
    area: Rect,
    data: &[f64],
    max: f64,
    style: ChartStyle,
    show_grid: bool,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let sub_width = usize::from(area.width) * 2;
    let sub_height = usize::from(area.height) * 4;
    let max = if max > 0.0 { max } else { 1.0 };

    // Grow from the left while the test is young; once there is more data than
    // fits, scroll so the newest samples stay visible.
    let first = data.len().saturating_sub(sub_width);
    let lit: Vec<u16> = (0..sub_width)
        .map(|c| {
            let i = first + c;
            if i >= data.len() {
                return 0;
            }
            let v = data[i];
            let norm = (v / max).clamp(0.0, 1.0);
            // Round, don't floor, so a full-scale peak reaches the top row.
            ((norm * sub_height as f64).round() as u16).min(sub_height as u16)
        })
        .collect();

    let fill_style = Style::default().fg(style.fill);
    let edge_style = Style::default().fg(style.edge).add_modifier(Modifier::BOLD);
    let grid_style = Style::default().fg(style.axis);

    // A single midline reads cleaner than a dense set of rules behind the curve.
    let midline = usize::from(area.height) / 2;
    for cy in 0..usize::from(area.height) {
        let grid_row = show_grid && cy == midline;
        for cx in 0..usize::from(area.width) {
            let mut edge_bits: u8 = 0;
            let mut col_levels = [0u8; 2];
            for sx in 0..2usize {
                let total = lit[cx * 2 + sx];
                // The fill is anchored to the baseline and grows upward, so
                // only the sub-rows at or below `start` are painted.
                let start = sub_height as u16 - total;
                let start_in_cell = start.saturating_sub((cy * 4) as u16);
                let level = usize::from((4u16).saturating_sub(start_in_cell).min(4));
                col_levels[sx] = level as u8;
                if level > 0 {
                    edge_bits |= dot_bit(sx, 4 - level);
                }
            }
            let fill_bits = cell_bits(&col_levels);

            let pos = Position::new(area.x + cx as u16, area.y + cy as u16);
            let Some(cell) = buf.cell_mut(pos) else {
                continue;
            };

            if fill_bits != 0 {
                let is_edge = edge_bits != 0;
                cell.set_symbol(braille_cell(col_levels).encode_utf8(&mut [0u8; 4]));
                cell.set_style(if is_edge { edge_style } else { fill_style });
            } else if grid_row {
                cell.set_symbol("·");
                cell.set_style(grid_style);
            } else {
                cell.set_symbol(" ");
            }
        }
    }
}

/// Horizontal meter with a bright leading cap.
#[must_use]
pub fn meter(fraction: f64, width: u16, fill: Color, track: Color) -> Text<'static> {
    let width = usize::from(width);
    if width == 0 {
        return Text::default();
    }
    let filled = ((fraction.clamp(0.0, 1.0)) * width as f64).round() as usize;
    let filled = filled.min(width);

    let fill_style = Style::default().fg(fill);
    let cap_style = Style::default()
        .fg(Color::White)
        .add_modifier(Modifier::BOLD);
    let track_style = Style::default().fg(track);

    let mut spans = Vec::new();
    let body = filled.saturating_sub(1);
    if body > 0 {
        spans.push(Span::styled("█".repeat(body), fill_style));
    }
    if filled > 0 {
        spans.push(Span::styled("█", cap_style));
    }
    let rest = width - filled;
    if rest > 0 {
        spans.push(Span::styled("─".repeat(rest), track_style));
    }
    Text::from(Line::from(spans))
}

/// Format a rate with a sensible unit.
#[must_use]
pub fn format_mbps(v: f64) -> String {
    if v >= 1000.0 {
        format!("{:.2} Gbps", v / 1000.0)
    } else if v >= 100.0 {
        format!("{v:.0} Mbps")
    } else if v >= 10.0 {
        format!("{v:.1} Mbps")
    } else {
        format!("{v:.2} Mbps")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style() -> ChartStyle {
        ChartStyle {
            fill: Color::Cyan,
            edge: Color::White,
            axis: Color::DarkGray,
        }
    }

    fn buffer(w: u16, h: u16) -> Buffer {
        Buffer::empty(Rect::new(0, 0, w, h))
    }

    fn symbol_at(buf: &Buffer, x: u16, y: u16) -> String {
        buf.cell(Position::new(x, y))
            .map(|c| c.symbol().to_string())
            .unwrap_or_else(|| "<oob>".to_string())
    }

    #[test]
    fn dot_bits_match_unicode_numbering() {
        assert_eq!(dot_bit(0, 0), 0b0000_0001, "top-left is dot1");
        assert_eq!(dot_bit(1, 0), 0b0000_1000, "top-right is dot4");
        assert_eq!(dot_bit(0, 3), 0b0100_0000, "bottom-left is dot7");
        assert_eq!(dot_bit(1, 3), 0b1000_0000, "bottom-right is dot8");
    }

    #[test]
    fn empty_cell_is_blank_braille() {
        assert_eq!(braille_cell([0, 0]), '\u{2800}');
    }

    #[test]
    fn full_cell_lights_every_dot() {
        assert_eq!(braille_cell([4, 4]), char::from_u32(0x28FF).unwrap());
    }

    #[test]
    fn single_bottom_dot_is_dot7() {
        assert_eq!(braille_cell([1, 0]), char::from_u32(0x2840).unwrap());
    }

    #[test]
    fn heights_are_monotonic() {
        let mut prev = 0u32;
        for h in 1..=4u8 {
            let code = u32::from(braille_cell([h, 0]));
            assert!(code > prev, "height {h} should exceed the previous");
            prev = code;
        }
    }

    #[test]
    fn chart_writes_into_the_requested_rect() {
        let mut buf = buffer(10, 5);
        render_area(
            &mut buf,
            Rect::new(0, 0, 10, 5),
            &[1.0, 2.0, 3.0],
            3.0,
            style(),
            false,
        );
        // Three samples occupy three sub-columns -> cells 0 and 1.
        assert_ne!(
            symbol_at(&buf, 0, 4),
            " ",
            "baseline under the first sample"
        );
        // The peak (3.0 of max 3.0) sits in the second cell and hits the top.
        assert_ne!(symbol_at(&buf, 1, 0), " ", "peak should reach the top row");
        // Trailing cells past the data stay blank.
        assert_eq!(symbol_at(&buf, 9, 4), " ", "right of the data stays blank");
    }

    #[test]
    fn chart_scrolls_when_data_overflows() {
        let mut buf = buffer(4, 4); // 8 sub-columns
                                    // 20 samples: only the most recent 8 are drawn, left-anchored.
        let data: Vec<f64> = (0..20).map(|i| i as f64).collect();
        render_area(&mut buf, Rect::new(0, 0, 4, 4), &data, 19.0, style(), false);
        assert_ne!(symbol_at(&buf, 0, 3), " ", "oldest visible sample is drawn");
    }

    #[test]
    fn fill_is_anchored_to_the_baseline() {
        // Two full-scale samples fill both sub-columns of cell 0 from the
        // bottom row up to the top - the curve must not hang from the top.
        let mut buf = buffer(2, 4);
        render_area(
            &mut buf,
            Rect::new(0, 0, 2, 4),
            &[100.0, 100.0],
            100.0,
            style(),
            false,
        );
        for y in 0..4 {
            assert_ne!(symbol_at(&buf, 0, y), " ", "row {y} should be filled");
        }
    }

    #[test]
    fn small_value_leaves_upper_rows_empty() {
        // 10% of max on an 8-row chart fills only the bottom sub-rows.
        let mut buf = buffer(2, 8);
        render_area(
            &mut buf,
            Rect::new(0, 0, 2, 8),
            &[10.0, 10.0],
            100.0,
            style(),
            false,
        );
        assert_eq!(symbol_at(&buf, 0, 0), " ", "top row must stay empty");
        assert_ne!(symbol_at(&buf, 0, 7), " ", "bottom row should be filled");
    }

    #[test]
    fn empty_data_leaves_blank_cells() {
        let mut buf = buffer(8, 4);
        render_area(&mut buf, Rect::new(0, 0, 8, 4), &[], 100.0, style(), false);
        for y in 0..4 {
            for x in 0..8 {
                let s = symbol_at(&buf, x, y);
                assert!(s == " " || s == "·", "expected blank, got {s:?}");
            }
        }
    }

    #[test]
    fn chart_respects_offset_rect() {
        let mut buf = buffer(20, 4);
        render_area(
            &mut buf,
            Rect::new(5, 2, 10, 4),
            &[100.0],
            100.0,
            style(),
            false,
        );
        // Outside the rect nothing should be touched.
        assert_eq!(symbol_at(&buf, 0, 0), " ");
        assert_eq!(symbol_at(&buf, 19, 0), " ");
    }

    #[test]
    fn scaler_rises_fast_and_falls_slowly() {
        let mut s = Scaler::new(1.0);
        s.update(100.0);
        // Headroom is applied so the peak sits just below the top edge.
        assert!(s.value() > 100.0, "peak must not be clipped");
        assert!(s.value() < 130.0, "headroom must stay modest");
        let before = s.value();
        s.update(10.0);
        assert!(
            s.value() > 12.0 && s.value() < before,
            "decay should be gradual"
        );
    }

    #[test]
    fn scaler_never_returns_a_nonpositive_scale() {
        let mut s = Scaler::new(1.0);
        s.update(0.0);
        assert!(s.value() >= 1.0);
    }

    #[test]
    fn meter_respects_width() {
        let t = meter(0.5, 10, Color::Green, Color::DarkGray);
        let s: String = t.lines[0]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(s.chars().count(), 10);
    }

    #[test]
    fn meter_clamps_out_of_range() {
        let t = meter(5.0, 8, Color::Green, Color::DarkGray);
        let s: String = t.lines[0]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(s.chars().count(), 8);
    }

    #[test]
    fn meter_at_zero_is_all_track() {
        let t = meter(0.0, 6, Color::Green, Color::DarkGray);
        let s: String = t.lines[0]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(s.chars().filter(|c| *c == '─').count(), 6);
    }

    #[test]
    fn units_switch_at_gigabits() {
        assert!(format_mbps(1500.0).contains("Gbps"));
        assert!(format_mbps(55.0).contains("Mbps"));
    }
}
