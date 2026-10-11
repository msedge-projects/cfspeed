//! All rendering. Layout is built from whitespace, rules and colour weight
//! rather than bordered boxes.

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::{App, View};
use crate::speed::{bufferbloat_grade, cv_percent, median, percentile, stability_grade, Phase};
use crate::ui::charts::{format_mbps, meter, render_area, ChartStyle};
use crate::ui::theme::{self, ACCENT_DOWN, ACCENT_UP};

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    if area.width < 24 || area.height < 10 {
        frame.render_widget(
            Paragraph::new("Terminal too small\nneed at least 24x10"),
            area,
        );
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // brand + phase
            Constraint::Length(1), // tabs + status
            Constraint::Length(1), // rule
            Constraint::Min(1),    // body
            Constraint::Length(1), // rule
            Constraint::Length(1), // keybinds
        ])
        .split(area);

    header(frame, rows[0], app);
    tabs(frame, rows[1], app);
    frame.render_widget(theme::rule(rows[2].width), rows[2]);

    match app.view {
        View::Live => live(frame, rows[3], app),
        View::Charts => charts(frame, rows[3], app),
        View::History => history(frame, rows[3], app),
        View::Help => help(frame, rows[3], app),
    }

    frame.render_widget(theme::rule(rows[4].width), rows[4]);
    footer(frame, rows[5], app);
}

// ---------------------------------------------------------------------------
// Chrome
// ---------------------------------------------------------------------------

fn header(frame: &mut Frame, area: Rect, app: &App) {
    let left = Line::from(vec![
        Span::styled("cfspeed", theme::style_title()),
        Span::raw("  "),
        Span::styled(target_line(app), theme::style_dim()),
    ]);
    frame.render_widget(Paragraph::new(left), area);

    let pill = Style::default()
        .fg(theme::TEXT_BRIGHT)
        .bg(theme::phase_color(app.phase))
        .add_modifier(Modifier::BOLD);
    let right = Line::from(vec![
        Span::styled(
            format!("  {}  ", fmt_dur(app.elapsed)),
            Style::default().fg(theme::TEXT),
        ),
        Span::styled(format!(" {} ", app.phase.label()), pill),
    ]);
    frame.render_widget(Paragraph::new(right).alignment(Alignment::Right), area);
}

fn target_line(app: &App) -> String {
    if app.hide_info {
        return "network details hidden   (Shift+H to reveal)".to_string();
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
    if !app.meta.warp.is_empty() && app.meta.warp != "off" {
        parts.push(format!("warp {}", app.meta.warp));
    }
    if parts.is_empty() {
        "connecting…".to_string()
    } else {
        parts.join("  ·  ")
    }
}

fn tabs(frame: &mut Frame, area: Rect, app: &App) {
    let mut spans = Vec::new();
    for (i, v) in View::ALL.iter().enumerate() {
        let style = if *v == app.view {
            Style::default()
                .fg(theme::TEXT_BRIGHT)
                .add_modifier(Modifier::BOLD | Modifier::REVERSED)
        } else {
            theme::style_dim()
        };
        spans.push(Span::styled(format!(" {} ", v.label()), style));
        if i + 1 < View::ALL.len() {
            spans.push(Span::raw("   "));
        }
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);

    let msg: Line = if let Some(e) = &app.error {
        Line::from(Span::styled(
            format!("error: {e}"),
            Style::default().fg(theme::grade_color("F")),
        ))
    } else if !app.status.is_empty() {
        Line::from(Span::styled(app.status.clone(), theme::style_dim()))
    } else if app.running {
        Line::from(Span::styled("running…", theme::style_dim()))
    } else {
        Line::default()
    };
    frame.render_widget(Paragraph::new(msg).alignment(Alignment::Right), area);
}

fn footer(frame: &mut Frame, area: Rect, app: &App) {
    let mut spans = vec![
        kbd("1-4"),
        Span::raw(" view   "),
        kbd("Tab"),
        Span::raw(" cycle   "),
    ];
    if !app.running {
        spans.push(kbd("r"));
        spans.push(Span::raw(" run again   "));
    }
    spans.push(kbd("Shift+H"));
    spans.push(Span::raw(" privacy   "));
    spans.push(kbd("q"));
    spans.push(Span::raw(" quit"));
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
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
// Live view
// ---------------------------------------------------------------------------

fn live(frame: &mut Frame, area: Rect, app: &App) {
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(8),
            Constraint::Min(8),
            Constraint::Length(1),
            Constraint::Length(2),
        ])
        .split(area);

    phase_panel(
        frame,
        parts[0],
        "DOWNLOAD",
        ACCENT_DOWN,
        &app.down_samples,
        app.down_scaler.value(),
        app.phase == Phase::Download,
    );
    phase_panel(
        frame,
        parts[1],
        "UPLOAD",
        ACCENT_UP,
        &app.up_samples,
        app.up_scaler.value(),
        app.phase == Phase::Upload,
    );
    frame.render_widget(stats_strip(app), parts[3]);
}

/// One phase block: label, headline + meter, chart, summary.
fn phase_panel(
    frame: &mut Frame,
    area: Rect,
    label: &str,
    color: Color,
    samples: &[f64],
    scale_max: f64,
    active: bool,
) {
    if area.width == 0 || area.height < 4 {
        return;
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);

    let instant = samples.last().copied().unwrap_or(0.0);

    // 1. Label, with the live instantaneous rate while this phase is active.
    let label_style = if active {
        Style::default().fg(color).add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(theme::TEXT_MUTED)
            .add_modifier(Modifier::BOLD)
    };
    let mut head = vec![Span::styled(format!(" {label}"), label_style)];
    if active {
        head.push(Span::styled(
            format!("   {}", format_mbps(instant)),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(head)), rows[0]);

    // 2. Headline number, then a meter across the remaining width.
    let big_text = fmt_value(instant);
    let unit = unit_for(instant);
    let mut line = vec![
        Span::styled(format!(" {big_text}"), theme::style_headline()),
        Span::styled(format!(" {unit}"), Style::default().fg(theme::TEXT_MUTED)),
    ];
    let used = 1 + big_text.len() + 1 + unit.len() + 3;
    let meter_width = usize::from(rows[1].width).saturating_sub(used);
    if meter_width > 8 {
        if let Some(first) = meter(
            instant / scale_max.max(1.0),
            meter_width as u16,
            color,
            theme::TRACK,
        )
        .lines
        .into_iter()
        .next()
        {
            line.push(Span::raw("  "));
            line.extend(first.spans);
        }
    }
    frame.render_widget(Paragraph::new(Line::from(line)), rows[1]);

    // 3. Chart.
    render_area(
        frame.buffer_mut(),
        rows[2],
        samples,
        scale_max,
        ChartStyle {
            fill: color,
            edge: theme::TEXT_BRIGHT,
            axis: theme::RULE,
        },
        true,
    );

    // 4. Summary.
    let (avg, p95, max) = sample_stats(samples);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" avg ", theme::style_label()),
            Span::styled(format_mbps(avg), Style::default().fg(theme::TEXT)),
            Span::styled("   p95 ", theme::style_label()),
            Span::styled(format_mbps(p95), Style::default().fg(theme::TEXT)),
            Span::styled("   max ", theme::style_label()),
            Span::styled(format_mbps(max), Style::default().fg(theme::TEXT)),
            Span::styled(format!("   {} samples", samples.len()), theme::style_dim()),
        ])),
        rows[3],
    );
}

fn stat(label: &str, value: String, style: Style) -> Vec<Span<'static>> {
    vec![
        Span::styled(format!(" {label} "), theme::style_label()),
        Span::styled(format!(" {value}  "), style),
    ]
}

fn stats_strip(app: &App) -> Text<'static> {
    let mut line: Vec<Span<'static>> = Vec::new();
    let value = Style::default().fg(theme::TEXT);

    let measuring = app.phase == Phase::Idle || app.running;
    let (idle_med, loaded_med, jitter, loss) = match &app.result {
        Some(r) => (r.idle.med, r.loaded_down.med, r.idle.jitter, r.idle.loss),
        None if measuring => (med(&app.probe_samples), 0.0, jit(&app.probe_samples), 0.0),
        None => (0.0, 0.0, 0.0, 0.0),
    };

    if idle_med > 0.0 {
        line.extend(stat("idle", fmt_ms(idle_med), value));
    }
    if loaded_med > 0.0 {
        line.extend(stat("loaded", fmt_ms(loaded_med), value));
    }
    if jitter > 0.0 {
        line.extend(stat("jitter", fmt_ms(jitter), value));
    }
    if loss > 0.0 {
        line.extend(stat(
            "loss",
            format!("{loss:.1}%"),
            Style::default().fg(theme::grade_color("D")),
        ));
    }
    if let Some(q) = app.quality() {
        line.extend(stat(
            "bloat",
            format!("{} ({:.0}ms)", q.bufferbloat, q.bufferbloat_ms),
            Style::default().fg(theme::grade_color(q.bufferbloat)),
        ));
        line.extend(stat(
            "stability",
            format!("{} ({:.0}%)", q.stability, q.stability_cv),
            Style::default().fg(theme::grade_color(q.stability)),
        ));
    }
    if app.meta.dns_ms > 0.0 {
        line.extend(stat("dns", fmt_ms(app.meta.dns_ms), value));
    }

    Text::from(Line::from(line))
}

// ---------------------------------------------------------------------------
// Charts view
// ---------------------------------------------------------------------------

fn charts(frame: &mut Frame, area: Rect, app: &App) {
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(7), Constraint::Min(7), Constraint::Min(6)])
        .split(area);

    big_chart(
        frame,
        parts[0],
        "DOWNLOAD  Mbps",
        &app.down_samples,
        app.down_scaler.value(),
        ACCENT_DOWN,
    );
    big_chart(
        frame,
        parts[1],
        "UPLOAD  Mbps",
        &app.up_samples,
        app.up_scaler.value(),
        ACCENT_UP,
    );

    let lat = scaled(&app.probe_samples, 0.0, 200.0);
    big_chart(frame, parts[2], "LATENCY  ms", &lat, 200.0, Color::Blue);
}

fn big_chart(frame: &mut Frame, area: Rect, title: &str, samples: &[f64], max: f64, color: Color) {
    if area.width == 0 || area.height < 2 {
        return;
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(area);

    let (_, p95, peak) = sample_stats(samples);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!(" {title}"),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("    peak {}", format_mbps(peak)),
                theme::style_dim(),
            ),
            Span::styled(format!("    p95 {}", format_mbps(p95)), theme::style_dim()),
            Span::styled(format!("    {} samples", samples.len()), theme::style_dim()),
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
            edge: theme::TEXT_BRIGHT,
            axis: theme::RULE,
        },
        true,
    );
}

// ---------------------------------------------------------------------------
// History view
// ---------------------------------------------------------------------------

fn history(frame: &mut Frame, area: Rect, app: &App) {
    if app.history.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("  No saved runs yet.", theme::style_dim()),
                Span::raw("\n\n  Press "),
                Span::styled(
                    " r ",
                    Style::default()
                        .fg(theme::TEXT_BRIGHT)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" to run a test.", theme::style_dim()),
            ])),
            area,
        );
        return;
    }

    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(8)])
        .split(area);

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
            Style::default()
                .fg(theme::TEXT_BRIGHT)
                .add_modifier(Modifier::REVERSED)
        } else {
            Style::default().fg(theme::TEXT)
        };
        let q_bloat = bufferbloat_grade(run.bufferbloat_ms());
        let cv = cv_percent(&run.upload.samples).max(cv_percent(&run.download.samples));
        let q_stab = stability_grade(cv);

        let style_for = |grade: &str| {
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
            Span::styled(col_padded(q_bloat, 7), style_for(q_bloat)),
            Span::styled(col_padded(q_stab, 6), style_for(q_stab)),
        ]));
    }
    frame.render_widget(Paragraph::new(Text::from(lines)), parts[0]);

    // Trend across saved runs.
    let downs: Vec<f64> = app.history.iter().map(|r| r.download.avg).collect();
    let peak = (downs.iter().copied().fold(0.0_f64, f64::max) * 1.12).max(1.0);
    let trend_rows = Layout::default()
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
        trend_rows[0],
    );
    render_area(
        frame.buffer_mut(),
        trend_rows[1],
        &downs,
        peak,
        ChartStyle {
            fill: ACCENT_DOWN,
            edge: theme::TEXT_BRIGHT,
            axis: theme::RULE,
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
            Span::styled(d.to_string(), theme::style_dim()),
        ]));
    }
    lines.extend([
        Line::default(),
        Line::from(Span::styled("  About", theme::style_title())),
        Line::default(),
        Line::from(Span::styled(
            "  Measures against Cloudflare's public speed-test endpoints.",
            theme::style_dim(),
        )),
        Line::from(Span::styled(
            "  Throughput is sampled every 200ms and drawn as a braille area",
            theme::style_dim(),
        )),
        Line::from(Span::styled(
            "  chart - 8x the vertical resolution of a character sparkline.",
            theme::style_dim(),
        )),
        Line::default(),
        Line::from(Span::styled(
            format!("  {} saved run(s) in history", app.history.len()),
            theme::style_dim(),
        )),
    ]);

    frame.render_widget(Paragraph::new(Text::from(lines)), area);
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

fn med(samples: &[f64]) -> f64 {
    median(samples)
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
        // 1970-01-01T09:14:56Z
        assert_eq!(fmt_time(33_296_000), "1970-01-01  09:14");
        // 2026-10-11T00:00:00Z - 20737 days after the epoch.
        assert_eq!(fmt_time(1_791_676_800_000), "2026-10-11  00:00");
        assert_eq!(fmt_time(0), "1970-01-01  00:00");
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
