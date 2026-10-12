//! Chart primitives rendered directly into a ratatui `Buffer`.
//!
//! The body under the curve is painted with *background colour* rather than
//! glyphs. A cell carrying a background fills its whole area with no gaps, so
//! the area reads as one solid shape.
//!
//! A background covers the entire cell, so on its own it quantises the top edge
//! to a cell boundary and the curve steps once per cell. Two renderings deal
//! with that, and the palette switches between them:
//!
//! * `smooth` (default) draws the single cell the wave crosses with braille, so
//!   the edge lands on any of four sub-rows and two horizontal steps per cell.
//!   Those dots are the only textured part of the chart.
//! * `solid` uses a half-block for the crest and no dots anywhere, trading the
//!   finer edge for a completely dot-free chart.
//!
//! The fill is deliberately a light tint. A heavy fill makes the quantised edge
//! the dominant thing on screen and the chart stops reading as a curve; keeping
//! it faint lets the eye follow the edge instead.
//!
//! Two earlier revisions failed this. The `▁▂▃▄▅▆▇█` block ramp is readable but steps
//! a whole character per column. Filling with braille has finer resolution but
//! can only ever cover a cell in dots, so a filled body drawn as `⠹` reads
//! as a dot screen rather than a graph. Half blocks plus a background fill give
//! up two of braille's eight sub-rows and buy an area with no texture at all.

use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};

use crate::ui::theme;

#[derive(Clone, Copy)]
pub struct ChartStyle {
    /// Colour of the curve itself.
    pub line: Color,
    /// Colour the filled body is graded towards.
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

/// Paint a bar graph with labelled axes, one column per sample.
///
/// This is the btop/htop idiom: solid blocks on a character grid, the leading
/// edge of each bar brightened. It is deliberately not an area chart. Smoothing
/// a trace and drawing it as a curve looks closer to a web chart, but a cell
/// can only be filled whole or in half-block steps, so the curve always either
/// steps once per cell or renders as a field of dots. Bars state exactly what
/// was measured, one column per sample, with nothing to infer.
///
/// Newest samples sit at the right edge; while the test is young the trace grows
/// left-to-right.
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

    // One column per sample. Where the trace is denser than the plot, a column
    // takes the peak of its bucket rather than the mean: averaging spikes away
    // is how a speed trace loses the very thing it exists to show.
    let column_value = |cx: usize| -> Option<f64> {
        if data.len() >= w && w > 0 {
            let lo = cx * data.len() / w;
            let hi = (((cx + 1) * data.len()) / w).max(lo + 1).min(data.len());
            Some(data[lo..hi].iter().copied().fold(0.0, f64::max))
        } else {
            data.get(cx).copied()
        }
    };

    let grid_style = Style::default().fg(style.axis);
    let grid_row = plot.height / 2;

    for cx in 0..w {
        let cell_x = plot.x + cx as u16;

        // Grid rule first, so the bars paint over it.
        if opts.y_axis {
            if let Some(cell) = buf.cell_mut(Position::new(cell_x, plot.y + grid_row)) {
                cell.set_symbol("\u{b7}");
                cell.set_style(grid_style);
            }
        }

        let Some(value) = column_value(cx) else {
            continue;
        };
        let filled = ((value / max).clamp(0.0, 1.0) * h as f64).round() as usize;
        if filled == 0 {
            continue;
        }
        let top = h - filled.min(h);

        for cy in top..h {
            let Some(cell) = buf.cell_mut(Position::new(cell_x, plot.y + cy as u16)) else {
                continue;
            };
            cell.set_symbol("\u{2588}");
            // btop's look: the bar carries the phase accent, and the block at
            // its leading edge is brighter, so the eye finds the current value
            // without having to read the axis.
            let st = if cy == top {
                Style::default().fg(style.line).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme::shade(style.fill, 0.72))
            };
            cell.set_style(st);
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
        write_at(
            buf,
            area.x + 1,
            area.y + row,
            &axis_label(max * frac, unit),
            label_style,
        );
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
            line: Color::Rgb(158, 206, 106),
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

    fn is_block(s: &str) -> bool {
        s == "\u{2588}"
    }

    fn fg_of(b: &Buffer, x: u16, y: u16) -> Color {
        b.cell(Position::new(x, y))
            .map(|c| c.fg)
            .unwrap_or(Color::Reset)
    }

    fn plot(w: u16, h: u16, data: &[f64], max: f64) -> Buffer {
        let mut b = buffer(w, h);
        render_chart(
            &mut b,
            Rect::new(0, 0, w, h),
            data,
            max,
            style(),
            ChartOpts {
                y_axis: false,
                x_axis: false,
                ..Default::default()
            },
        );
        b
    }

    #[test]
    fn empty_data_leaves_the_plot_blank() {
        let b = plot(10, 4, &[], 100.0);
        for y in 0..4 {
            for x in 0..10 {
                assert!(!is_block(&sym(&b, x, y)), "expected blank at {x},{y}");
            }
        }
    }

    #[test]
    fn full_scale_value_fills_every_row() {
        let b = plot(12, 6, &[100.0], 100.0);
        for y in 0..6 {
            assert!(is_block(&sym(&b, 0, y)), "row {y} should carry a bar");
        }
    }

    #[test]
    fn a_zero_value_draws_no_bar() {
        let b = plot(4, 5, &[0.0], 100.0);
        for y in 0..5 {
            assert!(!is_block(&sym(&b, 0, y)), "a zero sample must not draw");
        }
    }

    /// Half a sample's worth of value must round to a whole row, never a
    /// partial-block glyph: a partial block is the boxy look this chart is
    /// meant to avoid.
    #[test]
    fn bars_are_whole_rows_only() {
        let b = plot(6, 8, &[1.0, 5.0, 12.5, 33.0], 100.0);
        for y in 0..8u16 {
            for x in 0..6u16 {
                let s = sym(&b, x, y);
                assert!(
                    s == " " || is_block(&s),
                    "unexpected glyph {s:?} at {x},{y}"
                );
            }
        }
    }

    #[test]
    fn no_braille_is_emitted() {
        let data: Vec<f64> = (0..40).map(|i| (i % 20) as f64 * 5.0).collect();
        let b = plot(40, 10, &data, 100.0);
        for y in 0..10u16 {
            for x in 0..40u16 {
                let ch = sym(&b, x, y).chars().next().unwrap_or(' ');
                assert!(
                    !('\u{2800}'..='\u{28FF}').contains(&ch),
                    "braille dot at {x},{y}: a dotted body is not a bar graph"
                );
            }
        }
    }

    /// Denser data than columns is bucketed by peak. Averaging would erase the
    /// spikes, which are the part of a speed trace worth seeing.
    #[test]
    fn dense_data_keeps_its_peaks() {
        let mut data = vec![10.0; 100];
        data[50] = 100.0;
        let b = plot(100, 10, &data, 100.0);
        let tallest = (0..10u16).filter(|y| is_block(&sym(&b, 50, *y))).count();
        assert_eq!(tallest, 10, "the spike must reach full height");
    }

    #[test]
    fn the_leading_edge_is_brightened() {
        let b = plot(3, 6, &[50.0], 100.0);
        let top = (0..6u16).find(|y| is_block(&sym(&b, 0, *y))).expect("bar");
        let below = top + 1;
        assert_ne!(
            fg_of(&b, 0, top),
            fg_of(&b, 0, below),
            "the top block should stand out from the body"
        );
    }

    #[test]
    fn chart_respects_offset_rect() {
        let mut b = buffer(20, 4);
        render_chart(
            &mut b,
            Rect::new(5, 2, 10, 4),
            &[100.0, 100.0],
            100.0,
            style(),
            ChartOpts {
                y_axis: false,
                x_axis: false,
                ..Default::default()
            },
        );
        assert_eq!(sym(&b, 0, 0), " ", "outside the rect must be untouched");
        assert_eq!(sym(&b, 19, 0), " ");
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
            Rect::new(0, 0, 1, 1),
            &[1.0],
            1.0,
            style(),
            ChartOpts::default(),
        );
    }

    #[test]
    fn axes_are_labelled() {
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
        assert!(text.contains("100Mbps"), "missing top of scale:\n{text}");
        assert!(text.contains("0Mbps"), "missing baseline label:\n{text}");
        assert!(text.contains("-30s"), "missing time label:\n{text}");
        assert!(text.contains("now"), "missing 'now' label:\n{text}");
    }

    #[test]
    fn scaler_never_clips_the_peak() {
        let mut s = Scaler::new(100.0);
        let v = s.update(150.0);
        assert!(v >= 150.0, "peak must fit: {v}");
        assert!(v > 150.0, "but leave headroom: {v}");
    }
}
