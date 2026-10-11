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
use ratatui::text::{Line, Span, Text};

use crate::ui::theme;

/// Bit for the braille dot at sub-column `sx` (0=left, 1=right) and sub-row
/// `sy` (0=top .. 3=bottom).
///
/// Dots are numbered down each column first:
/// ```text
/// dot1 dot4
/// dot2 dot5
/// dot3 dot6
/// dot7 dot8
/// ```
#[must_use]
pub fn dot_bit(sx: usize, sy: usize) -> u8 {
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

/// Centred moving average used to smooth the drawn trace.
///
/// The window is five samples, a second of trace at the engine's 200ms
/// interval: enough for the eye to follow it as a curve, small enough that a
/// real drop or spike still shows. Edges use a partial window so the first and
/// last samples are not pulled toward zero.
#[must_use]
pub fn smooth_series(data: &[f64]) -> Vec<f64> {
    const WINDOW: usize = 5;
    if data.len() <= WINDOW {
        return data.to_vec();
    }
    let half = WINDOW / 2;
    data.iter()
        .enumerate()
        .map(|(i, _)| {
            let lo = i.saturating_sub(half);
            let hi = (i + half + 1).min(data.len());
            data[lo..hi].iter().sum::<f64>() / (hi - lo) as f64
        })
        .collect()
}

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
    /// Draw the wave with braille sub-pixels instead of half blocks.
    ///
    /// Half blocks give the edge two steps per cell; braille gives four
    /// vertically and two horizontally, so the wave is noticeably finer. The
    /// body stays solid either way - only the top band that the curve passes
    /// through is drawn with dots.
    pub smooth: bool,
}

impl Default for ChartOpts {
    fn default() -> Self {
        Self {
            y_axis: true,
            x_axis: true,
            unit: "Mbps",
            window_seconds: 0,
            smooth: false,
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

/// Paint a smooth area chart with labelled axes.
///
/// Newest samples sit at the right edge; while the test is young the curve grows
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
    // Braille splits a cell 2x4, giving four vertical and two horizontal steps.
    // Half blocks only split it horizontally, which is why the raw trace reads
    // as a row of blocks rather than a curve.
    let sub_rows = 4;
    let sub_h = h * sub_rows;
    let sub_w = w * 2;

    // Smooth the trace before drawing it. Samples land 200ms apart and a real
    // link is noisy, so the raw series is mostly high-frequency spikes that no
    // amount of pixel resolution can turn into a wave. A web chart fits a curve
    // through the points; this is the terminal equivalent. It affects the drawn
    // line only - every statistic is still computed from the raw samples.
    let series: Vec<f64> = if opts.smooth {
        smooth_series(data)
    } else {
        data.to_vec()
    };

    // Curve height per sub-column, in sub-rows from the top. `None` marks a
    // column before the first sample arrived.
    let first = series.len().saturating_sub(w);
    let y_sub: Vec<Option<f64>> = (0..sub_w)
        .map(|c| {
            let pos = first as f64 + c as f64 / 2.0;
            let i0 = pos.floor() as usize;
            if i0 >= series.len() {
                return None;
            }
            let frac = pos - i0 as f64;
            let a = series[i0];
            let b = series.get(i0 + 1).copied().unwrap_or(a);
            let norm = ((a + (b - a) * frac) / max).clamp(0.0, 1.0);
            Some((1.0 - norm) * (sub_h - 1) as f64)
        })
        .collect();

    let grid_style = Style::default().fg(style.axis);
    let grid_row = plot.height / 2;

    // Grid first; the fill paints over it wherever the chart reaches.
    for cx in 0..w {
        let pos = Position::new(plot.x + cx as u16, plot.y + grid_row);
        if let Some(cell) = buf.cell_mut(pos) {
            cell.set_symbol("\u{b7}");
            cell.set_style(grid_style);
        }
    }

    // Trace the curve. Each sub-column gets a dot, and consecutive sub-columns
    // are joined vertically: without that a curve that moves more than one
    // sub-row between columns breaks into detached specks instead of a line.
    let mut line_bits = vec![0u8; w * h];
    // Topmost row the curve reaches in each column, so the fill starts there
    // instead of leaving the stroke hanging over empty space.
    let mut col_top = vec![h; w];
    //
    // The stroke is two sub-rows deep, which is an even line rather than a
    // dotted one. Bridging between sub-columns was tried here and made it worse:
    // a steep segment joins as a vertical run of dots, and those runs are what
    // made the curve look like scattered specks. With the trace smoothed, a
    // half-cell line leaves only occasional small gaps and reads cleanly.
    for (c, slot) in y_sub.iter().enumerate() {
        let Some(v) = *slot else { continue };
        let y = (v.round().max(0.0) as usize).min(sub_h - 1);
        let ccx = c / 2;
        for yy in y..=(y + 1).min(sub_h - 1) {
            let ccy = yy / sub_rows;
            if ccx < w && ccy < h {
                line_bits[ccy * w + ccx] |= dot_bit(c % 2, yy % sub_rows);
                col_top[ccx] = col_top[ccx].min(ccy);
            }
        }
    }

    for cx in 0..w {
        let top = col_top[cx];
        if top >= h {
            continue;
        }
        let cell_x = plot.x + cx as u16;
        for cy in top..h {
            let pos = Position::new(cell_x, plot.y + cy as u16);
            let Some(cell) = buf.cell_mut(pos) else {
                continue;
            };
            // Area gradient: faint where it meets the line, saturating toward
            // the baseline. Running the ramp the other way - heaviest at the
            // crest - reads as a slab with a pale foot rather than an area.
            let depth = (cy - top) as f64;
            let span = h.saturating_sub(top).max(1) as f64;
            // Start bright enough that the fill visibly meets the line: a top
            // step of near-zero reads as a gap under the curve.
            let k = 0.16 + 0.42 * (depth / span);
            // Quantise: a per-row ramp spans the full range in as many steps as
            // there are rows, and each step lands as a hard edge, which reads
            // as stripes rather than depth.
            const FILL_STEPS: f64 = 6.0;
            let k = (k * FILL_STEPS).round() / FILL_STEPS;
            let body = theme::blend(theme::SURFACE, style.fill, k);

            let bits = line_bits[cy * w + cx];
            if bits == 0 {
                cell.set_symbol(" ");
                cell.set_style(Style::default().bg(body));
            } else {
                cell.set_symbol(braille_from_bits(bits).encode_utf8(&mut [0u8; 4]));
                cell.set_style(Style::default().fg(style.line).bg(body));
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

/// Horizontal meter that fades along its length, matching the graded body of
/// the charts above it.
///
/// Without the fade a progress bar reads as a flat slab of colour; the same
/// surface -> accent ramp that fills a chart makes the bar feel lit from the
/// left and keeps it consistent with the rest of the panel.
#[must_use]
pub fn meter(fraction: f64, width: u16, fill: Color, track: Color) -> Text<'static> {
    let width = usize::from(width);
    if width == 0 {
        return Text::default();
    }
    let filled = ((fraction.clamp(0.0, 1.0)) * width as f64).round() as usize;
    let filled = filled.min(width);

    // Quantise the ramp so adjacent cells share a span instead of allocating
    // one per character.
    const STEPS: usize = 12;
    let step_style = |step: usize| {
        let t = step as f64 / (STEPS - 1) as f64;
        Style::default().fg(theme::blend(track, fill, 0.25 + t * 0.75))
    };

    let mut spans: Vec<Span<'static>> = Vec::new();
    if filled > 0 {
        let mut run_start = 0usize;
        let mut run_step = 0usize;
        for i in 0..filled {
            let t = if filled > 1 {
                i as f64 / (filled - 1) as f64
            } else {
                1.0
            };
            let step = (t * (STEPS - 1) as f64).round() as usize;
            if step != run_step {
                spans.push(Span::styled(
                    "█".repeat(i - run_start),
                    step_style(run_step),
                ));
                run_start = i;
                run_step = step;
            }
        }
        spans.push(Span::styled(
            "█".repeat(filled - run_start),
            step_style(run_step),
        ));
        // Bright cap marks the leading edge, the same idea as the chart crest.
        let last = spans
            .last_mut()
            .expect("filled > 0 guarantees at least one span");
        *last = Span::styled(
            last.content.to_string(),
            Style::default().fg(fill).add_modifier(Modifier::BOLD),
        );
    }
    if filled < width {
        spans.push(Span::styled(
            "─".repeat(width - filled),
            Style::default().fg(track),
        ));
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

    /// True when the cell carries part of the chart: a painted background
    /// (the solid body) or a half-block crest glyph.
    fn is_solid(b: &Buffer, x: u16, y: u16) -> bool {
        let Some(c) = b.cell(Position::new(x, y)) else {
            return false;
        };
        let crest = matches!(c.symbol().chars().next(), Some('▀' | '▄'));
        crest || !matches!(c.bg, Color::Reset)
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
    fn full_scale_value_fills_every_row() {
        let b = plot(12, 6, &[100.0], 100.0);
        for y in 0..6 {
            assert!(is_solid(&b, 0, y), "row {y} should carry a dot");
        }
    }

    #[test]
    fn empty_data_leaves_the_plot_blank() {
        let b = plot(10, 4, &[], 100.0);
        for y in 0..4 {
            for x in 0..10 {
                assert!(!is_solid(&b, x, y), "expected blank at {x},{y}");
            }
        }
    }

    /// The whole point of braille: a ramp must produce a continuous stroke with
    /// no gaps, and every column must carry ink.
    #[test]
    fn a_ramp_draws_a_continuous_stroke() {
        let data: Vec<f64> = (0..20).map(|i| i as f64 * 5.0).collect();
        let b = plot(20, 10, &data, 100.0);
        for x in 0..20 {
            let mut ink = false;
            for y in 0..10 {
                if is_solid(&b, x, y) {
                    ink = true;
                }
            }
            assert!(ink, "column {x} has a gap - stroke is not continuous");
        }
    }

    /// Vertical resolution is the reason braille is used: a ramp must reach the
    /// top row gradually, not in two or three big jumps.
    #[test]
    fn a_ramp_resolves_more_than_one_step_per_cell() {
        let data: Vec<f64> = (0..40).map(|i| i as f64 * 2.5).collect();
        let b = plot(40, 8, &data, 100.0);
        // Record which row the topmost ink sits in, per column.
        let top: Vec<i32> = (0..40u16)
            .map(|x| {
                (0..8u16)
                    .find(|y| is_solid(&b, x, *y))
                    .map(i32::from)
                    .unwrap_or(-1)
            })
            .collect();
        // Count how many distinct rows the curve occupies across the sweep.
        let distinct: std::collections::BTreeSet<i32> = top.iter().copied().collect();
        assert!(
            distinct.len() >= 6,
            "curve only occupied {} distinct rows, expected a smooth sweep",
            distinct.len()
        );
    }

    #[test]
    fn overflowing_data_scrolls_keeping_the_newest_on_the_right() {
        let data: Vec<f64> = (0..30).map(|i| i as f64).collect();
        let b = plot(10, 4, &data, 100.0);
        // Values rise 20..29 across the window, so the right column is the peak.
        let mut right_ink = 0;
        let mut left_ink = 0;
        for y in 0..4 {
            if is_solid(&b, 9, y) {
                right_ink += 1;
            }
            if is_solid(&b, 0, y) {
                left_ink += 1;
            }
        }
        assert!(
            right_ink >= left_ink,
            "newest should not be lower than oldest"
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
            Rect::new(0, 0, 3, 1),
            &[1.0],
            1.0,
            style(),
            ChartOpts::default(),
        );
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
                ..Default::default()
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

    /// Odd sub-rows once made the fill depth underflow and panicked the app
    /// mid-run. Both edge styles are swept here across many heights and widths,
    /// because the crash only showed up on values that happened to land on an
    /// odd sub-row at a particular chart height.
    #[test]
    fn every_edge_style_survives_every_sub_row_alignment() {
        for smooth in [false, true] {
            for h in 1u16..=14 {
                for w in 1u16..=20 {
                    // A sawtooth crosses every sub-row alignment in turn.
                    let data: Vec<f64> = (0..40).map(|i| (i % 7) as f64 * 13.0).collect();
                    let mut b = buffer(w, h);
                    render_chart(
                        &mut b,
                        Rect::new(0, 0, w, h),
                        &data,
                        100.0,
                        style(),
                        ChartOpts {
                            y_axis: false,
                            x_axis: false,
                            smooth,
                            ..Default::default()
                        },
                    );
                }
            }
        }
    }

    /// The drawn trace is smoothed; the reported numbers are not. Averaging must
    /// preserve the series length and its overall level, or the curve would sit
    /// at a different height from the headline figure beside it.
    #[test]
    fn smoothing_preserves_length_and_level() {
        let data: Vec<f64> = (0..20).map(|i| 100.0 + (i % 5) as f64 * 10.0).collect();
        let s = smooth_series(&data);
        assert_eq!(s.len(), data.len(), "length must be preserved");
        let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
        assert!(
            (mean(&s) - mean(&data)).abs() < 1.0,
            "smoothing shifted the level: {} vs {}",
            mean(&s),
            mean(&data)
        );
    }

    /// Smoothing must actually flatten noise, which is the whole point of it.
    #[test]
    fn smoothing_reduces_sample_to_sample_jitter() {
        let spikes: Vec<f64> = (0..30)
            .map(|i| if i % 2 == 0 { 100.0 } else { 180.0 })
            .collect();
        let jitter = |v: &[f64]| v.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f64>();
        assert!(
            jitter(&smooth_series(&spikes)) < jitter(&spikes) * 0.5,
            "smoothing did not reduce jitter"
        );
        // Too short to smooth: passed through untouched.
        assert_eq!(smooth_series(&[1.0, 9.0]), vec![1.0, 9.0]);
    }
}
