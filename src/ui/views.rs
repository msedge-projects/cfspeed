//! All rendering.
//!
//! Layout follows web-dashboard conventions rather than the usual TUI habit of
//! text floating between box-drawn rules: a top bar surface, two metric cards
//! side by side, and a row of stat tiles. Panels are rounded surfaces with
//! their own background, which is the `posting`/Textual idiom.

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::{App, View};
use crate::speed::{bufferbloat_grade, cv_percent, median, percentile, stability_grade, Phase};
use crate::ui::charts::{format_mbps, meter, render_area, ChartStyle};
use crate::ui::theme::{self, ACCENT_DOWN, ACCENT_LAT, ACCENT_UP};
use crate::ui::widgets::{card, dial, pill, tile_lines};

/// Below this the dashboard cannot breathe, so we degrade to the live view.
const MIN_WIDTH: u16 = 46;
const MIN_HEIGHT: u16 = 20;

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        frame.render_widget(
            Paragraph::new(format!(
                "Terminal too small\nneed at least {MIN_WIDTH}x{MIN_HEIGHT}"
            )),
            area,
        );
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // top bar surface
            Constraint::Length(1), // gap
            Constraint::Min(1),    // content
            Constraint::Length(1), // gap
            Constraint::Length(4), // stat tiles
            Constraint::Length(1), // gap
            Constraint::Length(3), // key bar surface
        ])
        .split(area);

    top_bar(frame, rows[0], app);

    match app.view {
        View::Live => live(frame, rows[2], app),
        View::Charts => charts(frame, rows[2], app),
        View::History => history(frame, rows[2], app),
        View::Help => help(frame, rows[2], app),
    }

    stat_tiles(frame, rows[4], app);
    key_bar(frame, rows[6], app);
}

// ---------------------------------------------------------------------------
// Chrome
// ---------------------------------------------------------------------------

fn top_bar(frame: &mut Frame, area: Rect, app: &App) {
    let inner = card(frame, area, theme::ACCENT_DOWN, true);
    if inner.height == 0 {
        return;
    }
    let line = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(1), Constraint::Length(28)])
        .split(inner);

    let brand = Line::from(vec![
        Span::styled(" cfspeed", theme::style_title()),
        Span::styled("   ", Style::default()),
        Span::styled(target_line(app), theme::style_label()),
    ]);
    frame.render_widget(Paragraph::new(brand), line[0]);

    let phase = theme::phase_color(app.phase);
    let right = Line::from(vec![
        Span::styled(
            format!(" {} ", app.phase.label()),
            Style::default()
                .fg(theme::SURFACE)
                .bg(phase)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {}  ", fmt_dur(app.elapsed)),
            Style::default().fg(theme::TEXT),
        ),
    ]);
    frame.render_widget(Paragraph::new(right).alignment(Alignment::Right), line[1]);
}

fn target_line(app: &App) -> String {
    if app.hide_info {
        return "network hidden  (Shift+H)".to_string();
    }
    let mut parts = Vec::new();
    if !app.meta.colo.is_empty() {
        parts.push(app.meta.colo.clone());
    }
    if !app.meta.loc.is_empty() {
        parts.push(app.meta.loc.clone());
    }
    if !app.meta.ip.is_empty() {
        parts.push(app.meta.ip.clone());
    }
    if parts.is_empty() {
        "connecting…".to_string()
    } else {
        parts.join("  ·  ")
    }
}

fn key_bar(frame: &mut Frame, area: Rect, app: &App) {
    let inner = card(frame, area, theme::ACCENT_DOWN, true);
    if inner.height == 0 {
        return;
    }
    let line = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(1), Constraint::Length(30)])
        .split(inner);

    // Nav pills.
    let mut spans = Vec::new();
    for (i, v) in View::ALL.iter().enumerate() {
        spans.push(pill(v.label(), *v == app.view, theme::ACCENT_DOWN));
        if i + 1 < View::ALL.len() {
            spans.push(Span::raw(" "));
        }
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), line[0]);

    let mut keys = vec![
        kbd("r"),
        Span::raw(" run  "),
        kbd("Tab"),
        Span::raw(" view  "),
    ];
    if let Some(e) = &app.error {
        keys.push(Span::styled(
            format!(" error: {e} "),
            Style::default().fg(theme::grade_color("F")),
        ));
    } else {
        keys.push(kbd("Shift+H"));
        keys.push(Span::raw(" hide  "));
        keys.push(kbd("q"));
        keys.push(Span::raw(" quit"));
    }
    frame.render_widget(
        Paragraph::new(Line::from(keys)).alignment(Alignment::Right),
        line[1],
    );
}

fn kbd(k: &str) -> Span<'static> {
    Span::styled(
        format!(" {k} "),
        Style::default()
            .fg(theme::TEXT_BRIGHT)
            .add_modifier(Modifier::BOLD),
    )
}

// ---------------------------------------------------------------------------
// Stat tiles
// ---------------------------------------------------------------------------

fn stat_tiles(frame: &mut Frame, area: Rect, app: &App) {
    if area.width < 10 || area.height < 3 {
        return;
    }

    let measuring = app.phase == Phase::Idle || app.running;
    let (idle, loaded, jitter, loss) = match &app.result {
        Some(r) => (r.idle.med, r.loaded_down.med, r.idle.jitter, r.idle.loss),
        None if measuring => (
            median(&app.probe_samples),
            0.0,
            jit(&app.probe_samples),
            0.0,
        ),
        None => (0.0, 0.0, 0.0, 0.0),
    };

    let mut tiles: Vec<(&str, String, Color)> = Vec::new();
    if idle > 0.0 {
        tiles.push(("IDLE", fmt_ms(idle), ACCENT_LAT));
    }
    if loaded > 0.0 {
        tiles.push(("LOADED", fmt_ms(loaded), ACCENT_LAT));
    }
    if jitter > 0.0 {
        tiles.push(("JITTER", fmt_ms(jitter), ACCENT_LAT));
    }
    if loss > 0.0 {
        tiles.push(("LOSS", format!("{loss:.1}%"), theme::grade_color("D")));
    }
    if let Some(q) = app.quality() {
        tiles.push((
            "BUFFERBLOAT",
            format!("{}  ({:.0}ms)", q.bufferbloat, q.bufferbloat_ms),
            theme::grade_color(q.bufferbloat),
        ));
        tiles.push((
            "STABILITY",
            format!("{}  ({:.0}%)", q.stability, q.stability_cv),
            theme::grade_color(q.stability),
        ));
    }
    if app.meta.dns_ms > 0.0 {
        tiles.push(("DNS", fmt_ms(app.meta.dns_ms), theme::TEXT));
    }
    if tiles.is_empty() {
        tiles.push(("STATUS", "measuring…".into(), theme::TEXT_DIM));
    }

    // Two rows of tiles when they would otherwise be too narrow to read.
    let count = tiles.len().max(1);
    let per_row = if area.width / count as u16 >= 18 {
        count
    } else {
        count.div_ceil(2)
    };
    let rows: usize = if per_row >= tiles.len() { 1 } else { 2 };
    let row_height = (area.height / rows as u16).max(1);

    for row in 0..rows {
        let slice = &tiles[row * per_row..((row + 1) * per_row).min(tiles.len())];
        if slice.is_empty() {
            continue;
        }
        let row_area = Rect {
            x: area.x,
            y: area.y + (row * usize::from(row_height)) as u16,
            width: area.width,
            height: row_height,
        };
        let widths: Vec<Constraint> = slice
            .iter()
            .map(|_| Constraint::Ratio(1, slice.len() as u32))
            .collect();
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(widths)
            .split(row_area);

        for (i, (label, value, color)) in slice.iter().enumerate() {
            let inner = card(frame, cols[i], *color, false);
            if inner.height == 0 {
                continue;
            }
            frame.render_widget(
                Paragraph::new(Text::from(tile_lines(label, value.clone(), *color))),
                inner,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Live view
// ---------------------------------------------------------------------------

fn live(frame: &mut Frame, area: Rect, app: &App) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Ratio(1, 2),
            Constraint::Length(1),
            Constraint::Ratio(1, 2),
        ])
        .split(area);

    phase_card(
        frame,
        cols[0],
        "↓",
        "DOWNLOAD",
        ACCENT_DOWN,
        &app.down_samples,
        app.down_scaler.value(),
        app.phase == Phase::Download,
    );
    phase_card(
        frame,
        cols[2],
        "↑",
        "UPLOAD",
        ACCENT_UP,
        &app.up_samples,
        app.up_scaler.value(),
        app.phase == Phase::Upload,
    );
}

#[allow(clippy::too_many_arguments)]
fn phase_card(
    frame: &mut Frame,
    area: Rect,
    arrow: &str,
    label: &str,
    color: Color,
    samples: &[f64],
    scale_max: f64,
    active: bool,
) {
    if area.width < 20 || area.height < 10 {
        return;
    }
    let inner = card(frame, area, color, active);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // label
            Constraint::Length(6), // dial + headline
            Constraint::Min(1),    // chart
            Constraint::Length(1), // summary
        ])
        .split(inner);

    // 1. Label.
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!(" {arrow} "), Style::default().fg(color)),
            Span::styled(
                label.to_string(),
                Style::default()
                    .fg(theme::TEXT_BRIGHT)
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
        rows[0],
    );

    // 2. Dial on the left, headline number and bar on the right.
    let instant = samples.last().copied().unwrap_or(0.0);
    let split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(10),
            Constraint::Length(1), // breathing room beside the dial
            Constraint::Min(1),
        ])
        .split(rows[1]);

    dial(
        frame.buffer_mut(),
        split[0],
        instant / scale_max.max(1.0),
        color,
        theme::SURFACE_ALT,
    );

    let info = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Min(1),
        ])
        .split(split[2]);

    let big_text = fmt_value(instant);
    let unit = unit_for(instant);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!(" {big_text}"), theme::style_headline()),
            Span::styled(format!(" {unit}"), Style::default().fg(theme::TEXT_DIM)),
        ])),
        info[0],
    );
    let (avg, _, _) = sample_stats(samples);
    let bar_w = usize::from(info[1].width).saturating_sub(1);
    if bar_w > 4 {
        if let Some(line) = meter(
            avg / scale_max.max(1.0),
            bar_w as u16,
            color,
            theme::SURFACE_ALT,
        )
        .lines
        .into_iter()
        .next()
        {
            frame.render_widget(Paragraph::new(line), info[1]);
        }
    }

    // 3. Chart.
    render_area(
        frame.buffer_mut(),
        rows[2],
        samples,
        scale_max,
        ChartStyle {
            fill: color,
            edge: theme::CREST,
            axis: theme::BORDER,
        },
        true,
    );

    // 4. Summary.
    let (_, p95, peak) = sample_stats(samples);
    let mut summary = vec![
        Span::styled(" avg ", theme::style_label()),
        Span::styled(format_mbps(avg), Style::default().fg(theme::TEXT)),
        Span::styled("  p95 ", theme::style_label()),
        Span::styled(format_mbps(p95), Style::default().fg(theme::TEXT)),
        Span::styled("  max ", theme::style_label()),
        Span::styled(format_mbps(peak), Style::default().fg(theme::TEXT)),
    ];
    // A full phase that never moved a byte is almost always the endpoint
    // refusing us (HTTP 429), not a genuinely motionless link.
    if samples.len() >= 5 && avg == 0.0 {
        summary.push(Span::styled(
            "  rate limited (429)",
            Style::default().fg(theme::grade_color("D")),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(summary)), rows[3]);
}

// ---------------------------------------------------------------------------
// Charts view
// ---------------------------------------------------------------------------

fn charts(frame: &mut Frame, area: Rect, app: &App) {
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(7), Constraint::Min(7), Constraint::Min(5)])
        .split(area);

    for (slot, title, samples, max, color) in [
        (
            parts[0],
            "DOWNLOAD  Mbps",
            &app.down_samples,
            app.down_scaler.value(),
            ACCENT_DOWN,
        ),
        (
            parts[1],
            "UPLOAD  Mbps",
            &app.up_samples,
            app.up_scaler.value(),
            ACCENT_UP,
        ),
        (
            parts[2],
            "LATENCY  ms",
            &scaled(&app.probe_samples, 0.0, 200.0),
            200.0,
            ACCENT_LAT,
        ),
    ] {
        let inner = card(frame, slot, color, false);
        if inner.height < 3 {
            continue;
        }
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1)])
            .split(inner);

        let (_, p95, peak) = sample_stats(samples);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    format!(" {title}"),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("   peak {}", format_mbps(peak)),
                    theme::style_label(),
                ),
                Span::styled(format!("   p95 {}", format_mbps(p95)), theme::style_label()),
                Span::styled(
                    format!("   {} samples", samples.len()),
                    theme::style_label(),
                ),
            ])),
            rows[0],
        );
        render_area(
            frame.buffer_mut(),
            rows[1],
            samples,
            max,
            ChartStyle {
                fill: color,
                edge: theme::CREST,
                axis: theme::BORDER,
            },
            true,
        );
    }
}

// ---------------------------------------------------------------------------
// History view
// ---------------------------------------------------------------------------

fn history(frame: &mut Frame, area: Rect, app: &App) {
    if area.width < 20 || area.height < 6 {
        return;
    }
    let inner = card(frame, area, theme::ACCENT_DOWN, false);
    if app.history.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("  No saved runs yet.", theme::style_label()),
                Span::raw("\n\n  Press "),
                Span::styled(
                    " r ",
                    Style::default()
                        .fg(theme::TEXT_BRIGHT)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" to run a test.", theme::style_label()),
            ])),
            inner,
        );
        return;
    }

    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(4), Constraint::Length(8)])
        .split(inner);

    let mut lines = vec![Line::from(vec![
        header_col("TIME", 17),
        header_col("DOWN", 10),
        header_col("UP", 10),
        header_col("IDLE", 8),
        header_col("JITTER", 8),
        header_col("LOSS", 7),
        header_col("BLOAT", 7),
        header_col("STAB", 6),
    ])];

    let count = app.history.len();
    let rows_avail = usize::from(parts[0].height).saturating_sub(1).max(1);
    let start = app
        .history_sel
        .saturating_sub(rows_avail - 1)
        .min(count - 1);

    for (idx, run) in app.history.iter().enumerate().skip(start).take(rows_avail) {
        let selected = idx == app.history_sel;
        let base = if selected {
            Style::default().fg(theme::SURFACE).bg(theme::TEXT_BRIGHT)
        } else {
            Style::default().fg(theme::TEXT)
        };
        let q_bloat = bufferbloat_grade(run.bufferbloat_ms());
        let cv = cv_percent(&run.upload.samples).max(cv_percent(&run.download.samples));
        let q_stab = stability_grade(cv);
        let styled = |grade: &str| {
            if selected {
                base
            } else {
                Style::default().fg(theme::grade_color(grade))
            }
        };

        lines.push(Line::from(vec![
            Span::styled(format!(" {}", fmt_time(run.timestamp_ms)), base),
            Span::styled(col_padded(&format_mbps(run.download.avg), 10), base),
            Span::styled(col_padded(&format_mbps(run.upload.avg), 10), base),
            Span::styled(col_padded(&fmt_ms(run.idle.med), 8), base),
            Span::styled(col_padded(&fmt_ms(run.idle.jitter), 8), base),
            Span::styled(col_padded(&format!("{:.1}%", run.idle.loss), 7), base),
            Span::styled(col_padded(q_bloat, 7), styled(q_bloat)),
            Span::styled(col_padded(q_stab, 6), styled(q_stab)),
        ]));
    }
    frame.render_widget(Paragraph::new(Text::from(lines)), parts[0]);

    let downs: Vec<f64> = app.history.iter().map(|r| r.download.avg).collect();
    let peak = (downs.iter().copied().fold(0.0_f64, f64::max) * 1.12).max(1.0);
    let trend = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(parts[1]);
    frame.render_widget(
        Paragraph::new(Span::styled(
            " DOWNLOAD TREND   oldest → newest",
            Style::default()
                .fg(ACCENT_DOWN)
                .add_modifier(Modifier::BOLD),
        )),
        trend[0],
    );
    render_area(
        frame.buffer_mut(),
        trend[1],
        &downs,
        peak,
        ChartStyle {
            fill: ACCENT_DOWN,
            edge: theme::CREST,
            axis: theme::BORDER,
        },
        false,
    );
}

fn header_col(text: &str, width: usize) -> Span<'static> {
    Span::styled(col_padded(text, width), theme::style_label())
}

fn col_padded(text: &str, width: usize) -> String {
    let mut s = format!("{text:>width$} ");
    if s.len() > width + 1 {
        s.truncate(width + 1);
    }
    s
}

// ---------------------------------------------------------------------------
// Help view
// ---------------------------------------------------------------------------

fn help(frame: &mut Frame, area: Rect, app: &App) {
    if area.width < 20 || area.height < 6 {
        return;
    }
    let inner = card(frame, area, ACCENT_DOWN, false);
    let bindings: Vec<(&str, &str)> = vec![
        ("q / Esc", "quit"),
        ("r", "run a new test"),
        ("1 2 3 4", "live · charts · history · help"),
        ("Tab / Shift+Tab", "cycle views"),
        ("↑ ↓ / j k", "move history selection"),
        ("Shift+H", "toggle network privacy"),
        ("Ctrl+C", "force quit"),
    ];

    let mut lines = vec![
        Line::from(Span::styled("  Keyboard", theme::style_title())),
        Line::default(),
    ];
    for (k, d) in bindings {
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {k:<16}"),
                Style::default().fg(theme::TEXT_BRIGHT),
            ),
            Span::styled(d.to_string(), theme::style_label()),
        ]));
    }
    lines.extend([
        Line::default(),
        Line::from(Span::styled("  About", theme::style_title())),
        Line::default(),
        Line::from(Span::styled(
            "  Measures against Cloudflare's public speed-test endpoints.",
            theme::style_label(),
        )),
        Line::from(Span::styled(
            "  Throughput is sampled every 200ms and drawn as a braille area",
            theme::style_label(),
        )),
        Line::from(Span::styled(
            "  chart - 8x the vertical resolution of a character sparkline.",
            theme::style_label(),
        )),
        Line::default(),
        Line::from(Span::styled(
            format!("  {} saved run(s) in history", app.history.len()),
            theme::style_label(),
        )),
    ]);
    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

fn fmt_dur(d: std::time::Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}", s / 60, s % 60)
}

/// Convert days since the Unix epoch to a civil date (Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn fmt_time(ms: u64) -> String {
    let secs = (ms / 1000) as i64;
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    let (y, mo, d) = civil_from_days(days);
    format!(
        "{y:04}-{mo:02}-{d:02}  {:02}:{:02}",
        sod / 3600,
        (sod % 3600) / 60
    )
}

fn fmt_ms(v: f64) -> String {
    if v >= 1000.0 {
        format!("{:.2}s", v / 1000.0)
    } else if v >= 10.0 {
        format!("{v:.0}ms")
    } else {
        format!("{v:.1}ms")
    }
}

fn fmt_value(v: f64) -> String {
    if v >= 100.0 {
        format!("{v:.0}")
    } else if v >= 10.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.2}")
    }
}

fn unit_for(v: f64) -> &'static str {
    if v >= 1000.0 {
        "Gbps"
    } else {
        "Mbps"
    }
}

fn sample_stats(samples: &[f64]) -> (f64, f64, f64) {
    if samples.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    (
        samples.iter().sum::<f64>() / samples.len() as f64,
        percentile(&sorted, 0.95),
        sorted.last().copied().unwrap_or(0.0),
    )
}

fn jit(samples: &[f64]) -> f64 {
    if samples.len() < 2 {
        return 0.0;
    }
    samples.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f64>() / (samples.len() - 1) as f64
}

/// Rescale a series into a fixed 0..max window (used for the latency chart).
fn scaled(samples: &[f64], lo: f64, hi: f64) -> Vec<f64> {
    let span = (hi - lo).max(1e-9);
    samples
        .iter()
        .map(|v| ((v - lo) / span).clamp(0.0, 1.0) * hi)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_formats_as_mmss() {
        assert_eq!(fmt_dur(std::time::Duration::from_secs(75)), "1:15");
        assert_eq!(fmt_dur(std::time::Duration::from_secs(5)), "0:05");
    }

    #[test]
    fn timestamp_formats_as_date_and_clock() {
        assert_eq!(fmt_time(0), "1970-01-01  00:00");
        // 2026-10-11T00:00:00Z - 20737 days after the epoch.
        assert_eq!(fmt_time(1_791_676_800_000), "2026-10-11  00:00");
    }

    #[test]
    fn values_scale_precision_with_magnitude() {
        assert_eq!(fmt_value(5.0), "5.00");
        assert_eq!(fmt_value(55.0), "55.0");
        assert_eq!(fmt_value(550.0), "550");
    }

    #[test]
    fn units_promote_to_gigabits() {
        assert_eq!(unit_for(1500.0), "Gbps");
        assert_eq!(unit_for(150.0), "Mbps");
    }

    #[test]
    fn sample_stats_handle_empty_and_populated() {
        assert_eq!(sample_stats(&[]), (0.0, 0.0, 0.0));
        let (avg, _, max) = sample_stats(&[10.0, 20.0, 30.0]);
        assert!((avg - 20.0).abs() < 1e-9);
        assert!((max - 30.0).abs() < 1e-9);
    }

    #[test]
    fn column_padding_is_bounded() {
        assert_eq!(col_padded("abc", 5).len(), 6);
        assert_eq!(col_padded("a-very-long-value", 5).len(), 6);
    }

    #[test]
    fn latency_scaling_is_clamped() {
        let out = scaled(&[50.0, 100.0, 400.0], 0.0, 200.0);
        assert_eq!(out[2], 200.0, "above-range values clamp to the ceiling");
    }

    #[test]
    fn hidden_target_is_redacted() {
        let mut app = App::new();
        app.meta.ip = "1.2.3.4".into();
        assert!(target_line(&app).contains("1.2.3.4"));
        app.hide_info = true;
        assert!(!target_line(&app).contains("1.2.3.4"));
    }
}
