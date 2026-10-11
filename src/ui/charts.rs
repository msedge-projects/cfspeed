//! Chart primitives rendered directly into a ratatui `Buffer`.
//!
//! Braille dots carry 2x4 sub-pixels per cell, which gives fine resolution but
//! produces output nobody can read at a glance - it looks like texture rather
//! than a curve. These charts use the `▁▂▃▄▅▆▇█` block ramp instead: one sample per
//! column, a solid filled area, and explicit axis labels, so the value is
//! legible without decoding anything.

use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};

use crate::ui::theme;

/// Partial-block ramp, index 0..=7 where 7 is a full block.
const BLOCKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

#[derive(Clone, Copy)]
pub struct ChartStyle {
    pub fill: Color,
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

#[derive(Clone, Copy)]
pub struct ChartOpts {
    /// Reserve a left gutter and print the vertical scale.
    pub y_axis: bool,
    /// Reserve a bottom row and print the time axis.
    pub x_axis: bool,
    /// Human label for the vertical axis unit, e.g. "Mbps".
    pub unit: &'static str,
    /// Seconds shown on the time axis (0 omits it).
    pub window_seconds: u64,
}

impl Default for ChartOpts {
    fn default() -> Self {
        Self {
            y_axis: true,
            x_axis: true,
            unit: "Mbps",
            window_seconds: 0,
        }
    }
}

/// Columns taken by the value gutter.
fn gutter(opts: ChartOpts) -> u16 {
    if opts.y_axis {
        8
    } else {
        0
    }
}

/// Compact axis label for a value, e.g. `120`, `1.2G`.
fn axis_label(v: f64, unit: &str) -> String {
    let n = if v <= 0.0 {
        "0".to_string()
    } else if v >= 1000.0 {
        format!("{:.1}G", v / 1000.0)
    } else {
        format!("{v:.0}")
    };
    if unit.is_empty() {
        n
    } else {
        format!("{n}{unit}")
    }
}

/// Paint a filled area chart with labelled axes.
///
/// Newest samples sit at the right edge; when there is more data than fits the
/// most recent window is shown (the chart scrolls). While the test is young the
/// curve grows left-to-right.
pub fn render_chart(
    buf: &mut Buffer,
    area: Rect,
    data: &[f64],
    max: f64,
    style: ChartStyle,
    opts: ChartOpts,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let g = gutter(opts);
    let plot = Rect {
        x: area.x + g,
        y: area.y,
        width: area.width.saturating_sub(g),
        height: area.height.saturating_sub(u16::from(opts.x_axis)),
    };
    if plot.width == 0 || plot.height == 0 {
        return;
    }

    let max = if max > 0.0 { max } else { 1.0 };
    let w = usize::from(plot.width);
    let h = usize::from(plot.height);

    // One sample per column, anchored right so the newest data is always shown.
    let first = data.len().saturating_sub(w);
    let levels: Vec<u16> = (0..w)
        .map(|c| {
            let i = first + c;
            if i >= data.len() {
                return 0;
            }
            let norm = (data[i] / max).clamp(0.0, 1.0);
            // Eight sub-levels per row, via the block ramp.
            ((norm * (h * 8) as f64).round() as u16).min((h * 8) as u16)
        })
        .collect();

    let max_levels = (h * 8) as f64;
    let fill_of = |lvl: u16, row_from_bottom: usize| -> Option<(char, Style)> {
        let row_start = (row_from_bottom * 8) as u16;
        let row_end = row_start + 8;
        if lvl >= row_end {
            // Fully filled row.
            let t = f64::from(lvl) / max_levels;
            Some((
                BLOCKS[7],
                Style::default().fg(theme::blend(theme::SURFACE, style.fill, t)),
            ))
        } else if lvl > row_start {
            let partial = (lvl - row_start) as usize;
            Some((BLOCKS[partial - 1], Style::default().fg(style.fill)))
        } else {
            None
        }
    };

    let grid_style = Style::default().fg(style.axis);
    let grid_row = plot.height / 2;

    for (idx, y) in (0..plot.height).enumerate() {
        let from_bottom = plot.height as usize - 1 - idx;
        for (c, x) in (0..plot.width).enumerate() {
            let pos = Position::new(plot.x + x, plot.y + y);
            let Some(cell) = buf.cell_mut(pos) else {
                continue;
            };
            if let Some((sym, st)) = fill_of(levels[c], from_bottom) {
                cell.set_symbol(sym.encode_utf8(&mut [0u8; 4]));
                cell.set_style(st);
            } else if idx == usize::from(grid_row) && opts.y_axis {
                // A faint dotted mid-line gives the eye something to measure
                // the curve against.
                cell.set_symbol("·");
                cell.set_style(grid_style);
            } else {
                cell.set_symbol(" ");
            }
        }
    }

    if opts.y_axis {
        render_y_axis(buf, area, plot, max, opts.unit);
    }
    if opts.x_axis {
        render_x_axis(buf, area, plot, opts.window_seconds);
    }
}

fn render_y_axis(buf: &mut Buffer, area: Rect, plot: Rect, max: f64, unit: &str) {
    let label_style = Style::default().fg(theme::TEXT_DIM);
    // Top = scale maximum, middle = half, bottom = zero.
    for (row, frac) in [
        (0u16, 1.0f64),
        (plot.height / 2, 0.5),
        (plot.height - 1, 0.0),
    ] {
        let text = axis_label(max * frac, unit);
        let y = area.y + row;
        let pad = usize::from(plot.width).saturating_sub(0);
        let _ = pad;
        write_at(buf, area.x + 1, y, &text, label_style);
    }
}

fn render_x_axis(buf: &mut Buffer, area: Rect, plot: Rect, window_seconds: u64) {
    let y = plot.y + plot.height;
    if y >= area.y + area.height {
        return;
    }
    let style = Style::default().fg(theme::TEXT_DIM);
    if window_seconds == 0 {
        write_at(buf, plot.x, y, "now", style);
    } else {
        write_at(buf, plot.x, y, &format!("-{window_seconds}s"), style);
        let now = "now";
        let nx = plot.x + plot.width.saturating_sub(now.len() as u16);
        write_at(buf, nx, y, now, style);
    }
}

fn write_at(buf: &mut Buffer, x: u16, y: u16, text: &str, style: Style) {
    for (i, ch) in text.chars().enumerate() {
        let x = x.saturating_add(i as u16);
        if x >= buf.area.right() {
            break;
        }
        if let Some(cell) = buf.cell_mut(Position::new(x, y)) {
            cell.set_symbol(ch.encode_utf8(&mut [0u8; 4]));
            cell.set_style(style);
        }
    }
}

/// Single-row inline sparkline, for compact contexts.
#[must_use]
#[cfg_attr(not(test), allow(dead_code))]
pub fn sparkline(data: &[f64], max: f64, width: u16) -> Text<'static> {
    if width == 0 || data.is_empty() {
        return Text::default();
    }
    let w = usize::from(width);
    let max = if max > 0.0 { max } else { 1.0 };
    let first = data.len().saturating_sub(w);
    let chars: String = (0..w)
        .map(|c| {
            let i = first + c;
            if i >= data.len() {
                return ' ';
            }
            let norm = (data[i] / max).clamp(0.0, 1.0);
            BLOCKS[((norm * 7.0).round() as usize).min(7)]
        })
        .collect();
    Text::from(Line::from(Span::raw(chars)))
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
            fill: Color::Rgb(158, 206, 106),
            axis: Color::DarkGray,
        }
    }

    fn buffer(w: u16, h: u16) -> Buffer {
        Buffer::empty(Rect::new(0, 0, w, h))
    }

    fn sym(b: &Buffer, x: u16, y: u16) -> String {
        b.cell(Position::new(x, y))
            .map(|c| c.symbol().to_string())
            .unwrap_or_else(|| "<oob>".to_string())
    }

    fn is_plot_char(s: &str) -> bool {
        BLOCKS.contains(&s.chars().next().unwrap_or(' '))
    }

    #[test]
    fn full_scale_value_fills_every_row() {
        let mut b = buffer(12, 6);
        render_chart(
            &mut b,
            Rect::new(0, 0, 12, 6),
            &[100.0],
            100.0,
            style(),
            ChartOpts {
                y_axis: false,
                x_axis: false,
                ..Default::default()
            },
        );
        // A single early sample sits in the first column (the chart grows from
        // the left) and must be solid top to bottom.
        for y in 0..6 {
            assert_eq!(sym(&b, 0, y), "█", "row {y} should be a full block");
        }
    }

    #[test]
    fn empty_data_leaves_the_plot_blank() {
        let mut b = buffer(10, 4);
        render_chart(
            &mut b,
            Rect::new(0, 0, 10, 4),
            &[],
            100.0,
            style(),
            ChartOpts {
                y_axis: false,
                x_axis: false,
                ..Default::default()
            },
        );
        for y in 0..4 {
            for x in 0..10 {
                assert!(!is_plot_char(&sym(&b, x, y)), "expected blank at {x},{y}");
            }
        }
    }

    #[test]
    fn axis_labels_are_rendered() {
        let mut b = buffer(40, 8);
        render_chart(
            &mut b,
            Rect::new(0, 0, 40, 8),
            &[50.0],
            100.0,
            style(),
            ChartOpts {
                y_axis: true,
                x_axis: true,
                unit: "Mbps",
                window_seconds: 30,
            },
        );
        let text: String = (0..8)
            .map(|y| (0..40).map(|x| sym(&b, x, y)).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("100Mbps"), "missing scale maximum:\n{text}");
        assert!(text.contains("-30s"), "missing time origin:\n{text}");
        assert!(text.contains("now"), "missing 'now' marker:\n{text}");
    }

    #[test]
    fn curve_grows_from_the_left_as_data_arrives() {
        let mut b = buffer(20, 5);
        render_chart(
            &mut b,
            Rect::new(0, 0, 20, 5),
            &[80.0, 80.0, 80.0],
            100.0,
            style(),
            ChartOpts {
                y_axis: false,
                x_axis: false,
                ..Default::default()
            },
        );
        // Three samples occupy the first three columns; the rest is still empty.
        assert!(
            is_plot_char(&sym(&b, 2, 4)),
            "newest column should have data"
        );
        assert!(
            !is_plot_char(&sym(&b, 10, 4)),
            "unfilled region should be blank"
        );
    }

    #[test]
    fn overflowing_data_scrolls_with_newest_at_the_right() {
        let mut b = buffer(10, 4);
        // 30 samples into 10 columns: only the last 10 fit, newest on the right.
        let data: Vec<f64> = (0..30).map(|i| i as f64).collect();
        render_chart(
            &mut b,
            Rect::new(0, 0, 10, 4),
            &data,
            100.0,
            style(),
            ChartOpts {
                y_axis: false,
                x_axis: false,
                ..Default::default()
            },
        );
        // Values rise 20..29 across the visible window, so the newest column
        // must be drawn taller than the oldest.
        let oldest = sym(&b, 0, 3);
        let newest = sym(&b, 9, 3);
        assert!(is_plot_char(&oldest) && is_plot_char(&newest));
        assert_eq!(
            newest, "█",
            "newest sample fills its row, got {newest:?} vs oldest {oldest:?}"
        );
    }

    #[test]
    fn chart_scrolls_when_data_overflows() {
        let mut b = buffer(10, 4);
        let data: Vec<f64> = (0..50).map(|i| (i % 10) as f64 * 10.0).collect();
        render_chart(
            &mut b,
            Rect::new(0, 0, 10, 4),
            &data,
            100.0,
            style(),
            ChartOpts {
                y_axis: false,
                x_axis: false,
                ..Default::default()
            },
        );
        // Overflowing data must not panic or write outside the buffer.
        assert_eq!(b.area.width, 10);
    }

    #[test]
    fn degenerate_rects_are_tolerated() {
        let mut b = buffer(4, 4);
        render_chart(
            &mut b,
            Rect::new(0, 0, 0, 0),
            &[1.0],
            1.0,
            style(),
            ChartOpts::default(),
        );
        render_chart(
            &mut b,
            Rect::new(0, 0, 3, 1),
            &[1.0],
            1.0,
            style(),
            ChartOpts::default(),
        );
    }

    #[test]
    fn axis_labels_scale_units() {
        assert_eq!(axis_label(120.0, "Mbps"), "120Mbps");
        assert_eq!(axis_label(1500.0, "Mbps"), "1.5GMbps");
        assert_eq!(axis_label(0.0, "ms"), "0ms");
    }

    #[test]
    fn scaler_never_clips_the_peak() {
        let mut s = Scaler::new(1.0);
        s.update(100.0);
        assert!(s.value() > 100.0);
    }

    #[test]
    fn meter_respects_and_clamps_width() {
        let t = meter(0.5, 10, Color::Green, Color::DarkGray);
        let s: String = t.lines[0]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(s.chars().count(), 10);
        let t = meter(9.0, 8, Color::Green, Color::DarkGray);
        let s: String = t.lines[0]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(s.chars().count(), 8);
    }

    #[test]
    fn sparkline_is_one_row_wide() {
        let t = sparkline(&[1.0, 5.0, 9.0], 10.0, 10);
        let s: String = t.lines[0]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(s.chars().count(), 10);
        assert!(is_plot_char(&s));
    }
}
