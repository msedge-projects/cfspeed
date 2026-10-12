//! All rendering.
//!
//! Full-bleed and bezelless: no cards, no borders, no boxes. Structure comes
//! from whitespace, thin rules and colour weight, so the interface reads as one
//! continuous surface rather than a set of framed panels.

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::{App, View, MENU, QUICK_ACTIONS};
use crate::speed::{
    bufferbloat_grade, cv_percent, median, percentile, stability_grade, LatencyStats, Phase,
};
use crate::ui::charts::{format_mbps, meter, render_chart, ChartOpts, ChartStyle};
use crate::ui::theme::{self, ACCENT_DOWN, ACCENT_LAT, ACCENT_UP};
use crate::ui::widgets::{badge, pill};

/// Colour used for the "edge" side of the route strip.
const ROUTE_EDGE: Color = Color::Rgb(125, 207, 255);
/// Colour used for the "you" side of the route strip.
const ROUTE_SELF: Color = Color::Rgb(158, 206, 106);

const MIN_WIDTH: u16 = 64;
const MIN_HEIGHT: u16 = 18;

pub fn draw(frame: &mut Frame, app: &mut App) {
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
            Constraint::Length(1), // brand + phase + timer
            Constraint::Length(1), // rule
            Constraint::Length(2), // route: you -> edge
            Constraint::Length(1), // rule
            Constraint::Min(1),    // content
            Constraint::Length(1), // rule
            Constraint::Length(2), // stat tiles
            Constraint::Length(1), // rule
            Constraint::Length(1), // nav + keys
        ])
        .split(area);

    top_bar(frame, rows[0], app);
    rule(frame, rows[1]);
    route(frame, rows[2], app);
    rule(frame, rows[3]);

    match app.view {
        View::Live => live(frame, rows[4], app),
        View::Charts => charts(frame, rows[4], app),
        View::Details => details(frame, rows[4], app),
        View::History => history(frame, rows[4], app),
        View::Help => help(frame, rows[4], app),
    }

    rule(frame, rows[5]);
    stat_tiles(frame, rows[6], app);
    rule(frame, rows[7]);
    nav_bar(frame, rows[8], app);

    if app.menu_open {
        command_palette(frame, area, app);
    }
}

/// Command palette: every action in the app, reachable from one key, without
/// adding permanent chrome to the layout.
fn command_palette(frame: &mut Frame, area: Rect, app: &App) {
    let w = 56.min(area.width.saturating_sub(4));
    let h = (MENU.len() as u16 + 4).min(area.height.saturating_sub(2));
    if w < 24 || h < 6 {
        return;
    }
    let box_area = Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + 2,
        width: w,
        height: h,
    };

    let border_style = Style::default().fg(ACCENT_LAT);
    let right = box_area.right();

    // Top and bottom rails.
    for (y, left_glyph, right_glyph) in [
        (box_area.y, '╭', '╮'),
        (box_area.y + box_area.height - 1, '╰', '╯'),
    ] {
        for x in box_area.x..right {
            let glyph = if x == box_area.x {
                left_glyph
            } else if x + 1 == right {
                right_glyph
            } else {
                '─'
            };
            if let Some(c) = frame.buffer_mut().cell_mut(Position::new(x, y)) {
                c.set_symbol(glyph.encode_utf8(&mut [0u8; 4]));
                c.set_style(border_style);
            }
        }
    }

    // Side rails.
    for y in box_area.y + 1..box_area.y + box_area.height - 1 {
        for x in [box_area.x, right - 1] {
            if let Some(c) = frame.buffer_mut().cell_mut(Position::new(x, y)) {
                c.set_symbol("│");
                c.set_style(border_style);
            }
        }
    }

    let inner = Rect {
        x: box_area.x + 2,
        y: box_area.y + 1,
        width: box_area.width.saturating_sub(4),
        height: box_area.height.saturating_sub(2),
    };

    let mut lines = vec![Line::from(vec![
        Span::styled(" Actions", theme::style_title()),
        Span::styled("   ↑↓ select   ⏎ run   esc close", theme::style_label()),
    ])];
    for (i, action) in MENU.iter().enumerate() {
        let selected = i == app.menu_sel;
        let style = if selected {
            Style::default()
                .fg(theme::TEXT_BRIGHT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::TEXT)
        };
        let mut row = vec![Span::styled(format!(" {}  ", i + 1), theme::style_label())];
        if selected {
            row.push(Span::styled("▸ ", Style::default().fg(ACCENT_LAT)));
        } else {
            row.push(Span::raw("  "));
        }
        row.push(Span::styled(format!("{:<26}", action.label()), style));
        row.push(Span::styled(action.hint(), theme::style_label()));
        lines.push(Line::from(row));
    }
    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

// ---------------------------------------------------------------------------
// Chrome
// ---------------------------------------------------------------------------

fn rule(frame: &mut Frame, area: Rect) {
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "─".repeat(usize::from(area.width)),
            Style::default().fg(theme::BORDER),
        ))),
        area,
    );
}

fn top_bar(frame: &mut Frame, area: Rect, app: &App) {
    let left = Line::from(vec![
        Span::styled(" cfspeed", theme::style_title()),
        Span::styled("  ·  ", theme::style_label()),
        Span::styled(target_line(app), theme::style_label()),
    ]);
    frame.render_widget(Paragraph::new(left), area);

    let right = Line::from(vec![
        Span::styled(
            format!(" {}  ", fmt_dur(app.elapsed)),
            Style::default().fg(theme::TEXT),
        ),
        badge(app.phase.label(), theme::phase_color(app.phase)),
        Span::raw(" "),
    ]);
    frame.render_widget(Paragraph::new(right).alignment(Alignment::Right), area);
}

/// The two-hop route line: this machine's public identity, then the Cloudflare
/// edge we actually measured against. Labelled explicitly because a raw IP on
/// screen is otherwise ambiguous.
fn route(frame: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    let m = &app.meta;

    // Two fixed columns with a rule between them, so the two sides line up on
    // the same grid every run instead of trailing off at whatever the addresses
    // happen to be.
    const YOU_COLS: usize = 52;
    const EDGE_COLS: usize = 6; // "EDGE  "

    let rule = Span::styled("\u{2502}", Style::default().fg(theme::BORDER));

    let node = if !m.colo.is_empty() {
        m.colo.clone()
    } else if !m.loc.is_empty() {
        m.loc.clone()
    } else {
        "\u{2026}".to_string()
    };

    // Ports are the same for every address; printing them four times is noise,
    // so they are stated once on the node.
    let (v4, v6): (Vec<String>, Vec<String>) = if app.hide_info {
        (Vec::new(), Vec::new())
    } else {
        m.edge_ips
            .iter()
            .map(|a| {
                a.split(':')
                    .filter(|p| !p.is_empty() && *p != "443")
                    .collect::<Vec<_>>()
                    .join(":")
            })
            .partition(|a| !a.contains(':'))
    };
    let port = m
        .edge_ips
        .first()
        .and_then(|a| a.rsplit(':').next())
        .filter(|p| p.len() <= 5 && p.chars().all(|c| c.is_ascii_digit()))
        .map(|p| format!(":{p}"))
        .unwrap_or_default();

    // --- Row 1: you, rule, edge -----------------------------------------
    let mut line: Vec<Span<'static>> = Vec::new();
    if app.hide_info {
        line.push(Span::styled(
            "  YOU   ".to_string(),
            Style::default().fg(ROUTE_SELF).add_modifier(Modifier::BOLD),
        ));
        line.push(Span::styled(
            pad("hidden \u{00b7} Shift+H reveals", YOU_COLS - 8),
            Style::default().fg(theme::TEXT_DIM),
        ));
    } else {
        let ip = if m.ip.is_empty() {
            "\u{2026}"
        } else {
            m.ip.as_str()
        };
        let mut who: Vec<String> = Vec::new();
        if !m.asn_org.is_empty() {
            who.push(m.asn_org.clone());
        }
        if !m.asn.is_empty() {
            who.push(format!("AS{}", m.asn));
        }
        if !m.loc.is_empty() {
            who.push(m.loc.clone());
        }
        line.push(Span::styled(
            "  YOU   ".to_string(),
            Style::default().fg(ROUTE_SELF).add_modifier(Modifier::BOLD),
        ));
        line.push(Span::styled(
            format!("{ip:20}"),
            Style::default().fg(theme::TEXT_BRIGHT),
        ));
        line.push(Span::styled(
            pad(&who.join(" \u{00b7} "), YOU_COLS - 28),
            Style::default().fg(theme::TEXT),
        ));
    }
    line.push(rule.clone());
    line.push(Span::styled(
        " EDGE  ".to_string(),
        Style::default().fg(ROUTE_EDGE).add_modifier(Modifier::BOLD),
    ));
    line.push(Span::styled(
        pad(&format!("{node}{port}"), EDGE_COLS + 2),
        Style::default().fg(theme::TEXT_BRIGHT),
    ));
    if !v4.is_empty() {
        line.push(Span::styled(
            v4.join("  "),
            Style::default().fg(theme::TEXT),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(line)), rows[0]);

    // --- Row 2: the edge's other addresses, under the first ---------------
    let mut line2: Vec<Span<'static>> = vec![Span::raw(pad("", YOU_COLS))];
    line2.push(rule);
    line2.push(Span::raw(pad("", EDGE_COLS + 2)));
    if !v6.is_empty() {
        line2.push(Span::styled(
            v6.join("  "),
            Style::default().fg(theme::TEXT),
        ));
    }
    let mut bits: Vec<String> = Vec::new();
    if !app.hide_info {
        if !m.tls.is_empty() {
            let mut t = m.tls.clone();
            if !m.tls_cipher.is_empty() {
                t.push(' ');
                t.push_str(&m.tls_cipher);
            }
            bits.push(t);
        }
        if !m.http.is_empty() {
            bits.push(m.http.clone());
        }
        if !m.warp.is_empty() && m.warp != "off" {
            bits.push(format!("WARP {}", m.warp));
        }
    }
    if !bits.is_empty() {
        line2.push(Span::styled(
            format!("   {}", bits.join(" \u{00b7} ")),
            Style::default().fg(theme::TEXT_DIM),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(line2)), rows[1]);
}

/// Pad or clip to an exact width so columns stay on the grid.
fn pad(s: &str, w: usize) -> String {
    let n = s.chars().count();
    if n >= w {
        s.chars().take(w).collect()
    } else {
        format!("{s}{}", " ".repeat(w - n))
    }
}

fn target_line(app: &App) -> String {
    if app.hide_info {
        return "network hidden".to_string();
    }
    let mut parts = Vec::new();
    if !app.meta.colo.is_empty() {
        parts.push(app.meta.colo.clone());
    }
    if !app.meta.loc.is_empty() {
        parts.push(app.meta.loc.clone());
    }
    if parts.is_empty() {
        "connecting…".to_string()
    } else {
        parts.join("  ·  ")
    }
}

/// Nav tabs plus key hints. The tab geometry is recorded into `app.nav` so a
/// mouse click can be resolved to a view.
fn nav_bar(frame: &mut Frame, area: Rect, app: &mut App) {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut x = area.x + 1;
    app.nav.clear();

    for v in View::ALL {
        let label = v.label();
        let w = label.len() + 2;
        spans.push(pill(label, v == app.view, ACCENT_LAT));
        app.nav.push((
            v,
            Rect {
                x,
                y: area.y,
                width: w as u16,
                height: area.height.max(1),
            },
        ));
        x = x.saturating_add(w as u16 + 2);
        spans.push(Span::raw(" "));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);

    let pause_word = if app.paused { "resume" } else { "pause" };
    let zoom_label = app.zoom.label();
    let mode_label = app.mode.label();
    let mut keys = vec![
        Span::raw("space "),
        Span::styled(pause_word, theme::style_label()),
        Span::raw("  +/- zoom ("),
        Span::styled(zoom_label, theme::style_label()),
        Span::raw(")  m actions  "),
    ];
    // Quick test modes stay visible so they never need to be memorised.
    for (key, action, _) in QUICK_ACTIONS {
        keys.push(Span::styled(
            format!("{key} "),
            Style::default().fg(theme::TEXT_BRIGHT),
        ));
        keys.push(Span::styled(
            format!("{}  ", action.label().rsplit(' ').next().unwrap_or("")),
            theme::style_label(),
        ));
    }
    keys.push(Span::raw("r rerun  q quit"));
    if app.paused {
        keys.insert(0, badge("PAUSED", theme::TEXT_BRIGHT));
        keys.insert(1, Span::raw(" "));
    }
    let _ = mode_label;
    if let Some(e) = &app.error {
        keys = vec![Span::styled(
            format!("error: {e}"),
            Style::default().fg(theme::grade_color("F")),
        )];
    }
    if !app.status.is_empty() {
        keys.push(Span::styled(
            format!("  {}", app.status),
            theme::style_label(),
        ));
    }
    frame.render_widget(
        Paragraph::new(Line::from(keys)).alignment(Alignment::Right),
        area,
    );
}

// ---------------------------------------------------------------------------
// Stat tiles
// ---------------------------------------------------------------------------

/// Grade for a packet-loss percentage. Anything above a tenth of a percent is
/// already visible on a call.
fn loss_grade(loss: f64) -> &'static str {
    match loss {
        ..=0.0 => "A+",
        ..=0.5 => "A",
        ..=1.0 => "B",
        ..=2.0 => "C",
        ..=5.0 => "D",
        _ => "F",
    }
}

/// Smallest positive sample, ignoring probes that recorded nothing. Returns 0
/// when there is no usable sample at all.
fn min_of(samples: &[f64]) -> f64 {
    let min = samples
        .iter()
        .copied()
        .filter(|v| *v > 0.0)
        .fold(f64::INFINITY, f64::min);
    if min.is_finite() {
        min
    } else {
        0.0
    }
}

fn stat_tiles(frame: &mut Frame, area: Rect, app: &App) {
    let measuring = app.phase == Phase::Idle || app.running;
    let (idle, loaded, jitter, loss, best) = match &app.result {
        Some(r) => (
            r.idle.med,
            r.loaded_down.med,
            r.idle.jitter,
            r.idle.loss,
            r.idle.min,
        ),
        None if measuring => (
            median(&app.probe_samples),
            0.0,
            jit(&app.probe_samples),
            0.0,
            min_of(&app.probe_samples),
        ),
        None => (0.0, 0.0, 0.0, 0.0, 0.0),
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
    // Loss is shown even at zero. Hiding it when there is none makes a clean
    // link indistinguishable from a test that never probed anything.
    if measuring || app.result.is_some() {
        tiles.push((
            "LOSS",
            format!("{loss:.1}%"),
            theme::grade_color(loss_grade(loss)),
        ));
    }
    if best > 0.0 {
        tiles.push(("MIN", fmt_ms(best), ACCENT_LAT));
    }
    if let Some(q) = app.quality() {
        tiles.push((
            "BLOAT",
            format!("{} +{:.0}ms", q.bufferbloat, q.bufferbloat_ms),
            theme::grade_color(q.bufferbloat),
        ));
        tiles.push((
            "STAB",
            format!("{} {:.0}%", q.stability, q.stability_cv),
            theme::grade_color(q.stability),
        ));
    }
    if app.meta.dns_ms > 0.0 {
        tiles.push(("DNS", fmt_ms(app.meta.dns_ms), theme::TEXT));
    }
    if app.meta.tls_ms > 0.0 {
        tiles.push(("TLS", fmt_ms(app.meta.tls_ms), theme::TEXT));
    }
    if tiles.is_empty() {
        tiles.push(("STATUS", "measuring…".into(), theme::TEXT_DIM));
    }

    // Flow the tiles across however many rows are available.
    let per_row = ((usize::from(area.width) / 26).max(1)).min(tiles.len());
    for (row, chunk) in tiles.chunks(per_row).enumerate() {
        if row >= usize::from(area.height) {
            break;
        }
        let y = area.y + row as u16;
        let mut spans: Vec<Span<'static>> = Vec::new();
        for (label, value, color) in chunk {
            spans.push(Span::styled(
                format!("{label} "),
                Style::default().fg(theme::TEXT_DIM),
            ));
            spans.push(Span::styled(
                format!("{value}   "),
                Style::default().fg(*color).add_modifier(Modifier::BOLD),
            ));
        }
        frame.render_widget(
            Paragraph::new(Line::from(spans)),
            Rect {
                x: area.x,
                y,
                width: area.width,
                height: 1,
            },
        );
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
            Constraint::Length(2),
            Constraint::Ratio(1, 2),
        ])
        .split(area);

    // The samples actually drawn, and the span they really cover. The zoom
    // setting is a maximum, not the truth: a 10s phase inside a 240s window
    // still only holds 10s of data, and labelling that "240s" makes the time
    // axis lie about how long the curve is.
    let down = app.window(&app.down_samples);
    let up = app.window(&app.up_samples);
    let span = |n: usize| (n as u64 * 200) / 1000;

    phase_column(
        frame,
        cols[0],
        "↓",
        "DOWNLOAD",
        ACCENT_DOWN,
        down,
        app.down_scaler.value(),
        app.phase == Phase::Download,
        app.paused,
        span(down.len()),
    );
    phase_column(
        frame,
        cols[2],
        "↑",
        "UPLOAD",
        ACCENT_UP,
        up,
        app.up_scaler.value(),
        app.phase == Phase::Upload,
        app.paused,
        span(up.len()),
    );
}

#[allow(clippy::too_many_arguments)]
fn phase_column(
    frame: &mut Frame,
    area: Rect,
    arrow: &str,
    label: &str,
    color: Color,
    samples: &[f64],
    scale_max: f64,
    active: bool,
    paused: bool,
    window_seconds: u64,
) {
    if area.width < 24 || area.height < 8 {
        return;
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // label
            Constraint::Length(1), // headline
            Constraint::Length(1), // meter
            Constraint::Min(1),    // chart
            Constraint::Length(1), // summary
        ])
        .split(area);

    // 1. Label.
    let mut head = vec![Span::styled(
        format!(" {arrow} "),
        Style::default().fg(color),
    )];
    head.push(Span::styled(
        label.to_string(),
        Style::default()
            .fg(if active { color } else { theme::TEXT_DIM })
            .add_modifier(Modifier::BOLD),
    ));
    if active {
        head.push(Span::styled(
            "  measuring".to_string(),
            Style::default().fg(theme::TEXT_DIM),
        ));
    }
    if paused {
        head.push(Span::raw("  "));
        head.push(badge("PAUSED", theme::TEXT_BRIGHT));
    }
    frame.render_widget(Paragraph::new(Line::from(head)), rows[0]);

    // 2. Headline figure.
    let instant = samples.last().copied().unwrap_or(0.0);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw(" "),
            Span::styled(fmt_value(instant), theme::style_headline()),
            Span::styled(
                format!(" {}", unit_for(instant)),
                Style::default().fg(theme::TEXT_DIM),
            ),
        ])),
        rows[1],
    );

    // 3. Meter showing the phase average against the scale.
    let (avg, _, _) = sample_stats(samples);
    if let Some(line) = meter(
        avg / scale_max.max(1.0),
        area.width.saturating_sub(2),
        color,
        theme::SURFACE_ALT,
    )
    .lines
    .into_iter()
    .next()
    {
        frame.render_widget(Paragraph::new(line), rows[2]);
    }

    // 4. Chart with labelled axes.
    render_chart(
        frame.buffer_mut(),
        rows[3],
        samples,
        scale_max,
        ChartStyle {
            line: line_color(color),
            fill: color,
            axis: theme::BORDER,
        },
        ChartOpts {
            y_axis: true,
            x_axis: true,
            unit: "Mbps",
            window_seconds,
        },
    );

    // 5. Summary.
    let (_, p95, peak) = sample_stats(samples);
    let mut summary = vec![
        Span::styled(" avg ", theme::style_label()),
        Span::styled(format_mbps(avg), Style::default().fg(theme::TEXT)),
        Span::styled("  p95 ", theme::style_label()),
        Span::styled(format_mbps(p95), Style::default().fg(theme::TEXT)),
        Span::styled("  max ", theme::style_label()),
        Span::styled(format_mbps(peak), Style::default().fg(theme::TEXT)),
    ];
    if samples.len() >= 5 && avg == 0.0 {
        summary.push(Span::styled(
            "  rate limited (429)",
            Style::default().fg(theme::grade_color("D")),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(summary)), rows[4]);
}

// ---------------------------------------------------------------------------
// Charts view
// ---------------------------------------------------------------------------

fn charts(frame: &mut Frame, area: Rect, app: &App) {
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(7), Constraint::Min(7), Constraint::Min(6)])
        .split(area);

    let rows = [
        (
            parts[0],
            "DOWNLOAD",
            &app.down_samples,
            app.down_scaler.value(),
            ACCENT_DOWN,
            "Mbps",
        ),
        (
            parts[1],
            "UPLOAD",
            &app.up_samples,
            app.up_scaler.value(),
            ACCENT_UP,
            "Mbps",
        ),
        (
            parts[2],
            "LATENCY",
            &scaled(&app.probe_samples, 0.0, 200.0),
            200.0,
            ACCENT_LAT,
            "ms",
        ),
    ];

    for (slot, title, samples, max, color, unit) in rows {
        let (_, p95, peak) = sample_stats(samples);
        let head = Line::from(vec![
            Span::styled(
                format!(" {title}"),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(
                    "    peak {}    p95 {}    {} samples",
                    format_mbps(peak),
                    format_mbps(p95),
                    samples.len()
                ),
                theme::style_label(),
            ),
        ]);
        let split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1)])
            .split(slot);
        frame.render_widget(Paragraph::new(head), split[0]);
        render_chart(
            frame.buffer_mut(),
            split[1],
            samples,
            max,
            ChartStyle {
                line: line_color(color),
                fill: color,
                axis: theme::BORDER,
            },
            ChartOpts {
                y_axis: true,
                x_axis: true,
                unit,
                window_seconds: 0,
            },
        );
    }
}

// ---------------------------------------------------------------------------
// Details view
// ---------------------------------------------------------------------------

/// One labelled measurement in the spec sheet.
struct Metric {
    label: String,
    value: String,
    color: Color,
}

impl Metric {
    fn new(label: impl Into<String>, value: impl Into<String>, color: Color) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            color,
        }
    }
}

fn details(frame: &mut Frame, area: Rect, app: &App) {
    if area.width < 30 || area.height < 6 {
        return;
    }
    let mut groups: Vec<(&str, Vec<Metric>)> = Vec::new();
    let m = &app.meta;

    // --- Connection -------------------------------------------------------
    let mut conn: Vec<Metric> = Vec::new();
    if app.hide_info {
        conn.push(Metric::new("privacy", "hidden (Shift+H)", theme::TEXT_DIM));
    } else {
        if !m.asn_org.is_empty() {
            conn.push(Metric::new("ISP", &m.asn_org, theme::TEXT));
        }
        if !m.asn.is_empty() {
            conn.push(Metric::new("ASN", &m.asn, theme::TEXT));
        }
        if !m.ip.is_empty() {
            conn.push(Metric::new("Your IP", &m.ip, ROUTE_SELF));
        }
        if !m.loc.is_empty() {
            conn.push(Metric::new("Your location", &m.loc, theme::TEXT));
        }
        if !m.colo.is_empty() {
            conn.push(Metric::new("Edge node", &m.colo, ROUTE_EDGE));
        }
        if !m.edge_ips.is_empty() {
            conn.push(Metric::new("Edge IPs", m.edge_ips.join(", "), ROUTE_EDGE));
        }
    }
    if !m.tls.is_empty() {
        let extra = if m.tls_ms > 0.0 {
            format!("  ({:.0}ms)", m.tls_ms)
        } else {
            String::new()
        };
        conn.push(Metric::new("TLS", format!("{}{extra}", m.tls), theme::TEXT));
    }
    if !m.tls_cipher.is_empty() {
        conn.push(Metric::new("Cipher", &m.tls_cipher, theme::TEXT_DIM));
    }
    if !m.http.is_empty() {
        conn.push(Metric::new("HTTP", &m.http, theme::TEXT));
    }
    if !m.warp.is_empty() {
        let c = if m.warp == "off" {
            theme::TEXT_DIM
        } else {
            theme::grade_color("A")
        };
        conn.push(Metric::new("WARP", &m.warp, c));
    }
    if m.dns_ms > 0.0 {
        conn.push(Metric::new("DNS", fmt_ms(m.dns_ms), theme::TEXT));
    }
    groups.push(("CONNECTION", conn));

    // --- Throughput -------------------------------------------------------
    let mut tp: Vec<Metric> = Vec::new();
    match &app.result {
        Some(r) => {
            for (name, t, color) in [
                ("Download", &r.download, ACCENT_DOWN),
                ("Upload", &r.upload, ACCENT_UP),
            ] {
                tp.push(Metric::new(
                    format!("{name} avg"),
                    format_mbps(t.avg),
                    color,
                ));
                tp.push(Metric::new(
                    format!("{name} median"),
                    format_mbps(t.med),
                    color,
                ));
                tp.push(Metric::new(
                    format!("{name} p25/p75"),
                    format!("{}/{}", format_mbps(t.p25), format_mbps(t.p75)),
                    theme::TEXT_DIM,
                ));
                tp.push(Metric::new(
                    format!("{name} p95"),
                    format_mbps(t.p95),
                    color,
                ));
                tp.push(Metric::new(
                    format!("{name} p99"),
                    format_mbps(t.p99),
                    color,
                ));
                tp.push(Metric::new(
                    format!("{name} min/max"),
                    format!("{}/{}", format_mbps(t.min), format_mbps(t.max)),
                    theme::TEXT_DIM,
                ));
                tp.push(Metric::new(
                    format!("{name} data"),
                    format_bytes(t.bytes),
                    theme::TEXT,
                ));
                tp.push(Metric::new(
                    format!("{name} window"),
                    format!("{}s ({} samples)", t.duration_ms / 1000, t.samples.len()),
                    theme::TEXT_DIM,
                ));
            }
        }
        None => {
            for (name, samples, color) in [
                ("Download", &app.down_samples, ACCENT_DOWN),
                ("Upload", &app.up_samples, ACCENT_UP),
            ] {
                let (avg, _, peak) = sample_stats(samples);
                tp.push(Metric::new(format!("{name} avg"), format_mbps(avg), color));
                tp.push(Metric::new(
                    format!("{name} peak"),
                    format_mbps(peak),
                    color,
                ));
                tp.push(Metric::new(
                    format!("{name} samples"),
                    samples.len().to_string(),
                    theme::TEXT_DIM,
                ));
            }
        }
    }
    groups.push(("THROUGHPUT", tp));

    // --- Latency ----------------------------------------------------------
    let mut lat: Vec<Metric> = Vec::new();
    for (name, s) in [
        ("Idle", &app.idle),
        ("Loaded down", &loaded_down(app)),
        ("Loaded up", &loaded_up(app)),
    ] {
        if s.avg <= 0.0 {
            continue;
        }
        lat.push(Metric::new(
            format!("{name} avg"),
            fmt_ms(s.avg),
            theme::TEXT,
        ));
        lat.push(Metric::new(
            format!("{name} min/med"),
            format!("{}/{}", fmt_ms(s.min), fmt_ms(s.med)),
            theme::TEXT,
        ));
        lat.push(Metric::new(
            format!("{name} p25/p75"),
            format!("{}/{}", fmt_ms(s.p25), fmt_ms(s.p75)),
            theme::TEXT_DIM,
        ));
        lat.push(Metric::new(
            format!("{name} p95/p99"),
            format!("{}/{}", fmt_ms(s.p95), fmt_ms(s.p99)),
            theme::TEXT_DIM,
        ));
        lat.push(Metric::new(
            format!("{name} max"),
            fmt_ms(s.max),
            theme::TEXT,
        ));
        lat.push(Metric::new(
            format!("{name} jitter"),
            fmt_ms(s.jitter),
            theme::TEXT,
        ));
        lat.push(Metric::new(
            format!("{name} loss"),
            format!("{:.1}%", s.loss),
            theme::TEXT,
        ));
    }
    groups.push(("LATENCY", lat));

    // --- Quality ----------------------------------------------------------
    let mut q: Vec<Metric> = Vec::new();
    if let Some(qual) = app.quality() {
        q.push(Metric::new(
            "Bufferbloat",
            format!("{}  +{:.0}ms", qual.bufferbloat, qual.bufferbloat_ms),
            theme::grade_color(qual.bufferbloat),
        ));
        q.push(Metric::new(
            "Stability",
            format!("{}  CV {:.0}%", qual.stability, qual.stability_cv),
            theme::grade_color(qual.stability),
        ));
    }
    q.push(Metric::new("Run time", fmt_dur(app.elapsed), theme::TEXT));
    groups.push(("QUALITY", q));

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Ratio(1, 2),
            Constraint::Length(2),
            Constraint::Ratio(1, 2),
        ])
        .split(area);
    render_spec_sheet(frame, cols[0], &groups[..2.min(groups.len())]);
    render_spec_sheet(frame, cols[2], &groups[2.min(groups.len())..]);
}

fn render_spec_sheet(frame: &mut Frame, area: Rect, groups: &[(&str, Vec<Metric>)]) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let label_w = groups
        .iter()
        .flat_map(|(_, rows)| rows.iter())
        .map(|m| m.label.chars().count())
        .max()
        .unwrap_or(0)
        .clamp(12, 20);

    let mut lines = Vec::new();
    for (title, rows) in groups {
        if !lines.is_empty() {
            lines.push(Line::default());
        }
        lines.push(Line::from(Span::styled(
            format!(" {title}"),
            Style::default().fg(ACCENT_LAT).add_modifier(Modifier::BOLD),
        )));
        if rows.is_empty() {
            lines.push(Line::from(Span::styled(
                "   not measured yet",
                Style::default().fg(theme::TEXT_DIM),
            )));
            continue;
        }
        for m in rows {
            lines.push(Line::from(vec![
                Span::styled(format!("   {:<label_w$}  ", m.label), theme::style_label()),
                Span::styled(m.value.clone(), Style::default().fg(m.color)),
            ]));
        }
    }
    frame.render_widget(Paragraph::new(Text::from(lines)), area);
}

fn loaded_down(app: &App) -> LatencyStats {
    app.result
        .as_ref()
        .map(|r| r.loaded_down.clone())
        .unwrap_or_default()
}

fn loaded_up(app: &App) -> LatencyStats {
    app.result
        .as_ref()
        .map(|r| r.loaded_up.clone())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// History view
// ---------------------------------------------------------------------------

fn history(frame: &mut Frame, area: Rect, app: &App) {
    if area.width < 30 || area.height < 6 {
        return;
    }
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
            area,
        );
        return;
    }

    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(4), Constraint::Length(9)])
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
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " DOWNLOAD TREND",
                Style::default()
                    .fg(ACCENT_DOWN)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("   oldest → newest", theme::style_label()),
        ])),
        parts[1],
    );
    render_chart(
        frame.buffer_mut(),
        Rect {
            x: parts[1].x,
            y: parts[1].y + 1,
            width: parts[1].width,
            height: parts[1].height.saturating_sub(1),
        },
        &downs,
        peak,
        ChartStyle {
            line: line_color(ACCENT_DOWN),
            fill: ACCENT_DOWN,
            axis: theme::BORDER,
        },
        ChartOpts {
            y_axis: true,
            x_axis: false,
            unit: "Mbps",
            window_seconds: 0,
        },
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
    let bindings: Vec<(&str, &str)> = vec![
        ("q / Esc", "quit"),
        ("Ctrl+C", "force quit"),
        ("r", "run a new test"),
        ("space", "pause / resume the charts"),
        ("+ / -", "zoom the chart time window"),
        ("1 2 3 4", "live · charts · details · history"),
        ("?", "help"),
        ("Tab / ← →", "cycle views"),
        ("↑ ↓ / j k", "move history selection"),
        ("Shift+H", "toggle network privacy"),
        ("mouse", "click a tab to switch view"),
    ];

    let mut lines = vec![
        Line::from(Span::styled(" Keyboard", theme::style_title())),
        Line::default(),
    ];
    for (k, d) in bindings {
        lines.push(Line::from(vec![
            Span::styled(format!(" {k:<14}"), Style::default().fg(theme::TEXT_BRIGHT)),
            Span::styled(d.to_string(), theme::style_label()),
        ]));
    }
    lines.extend([
        Line::default(),
        Line::from(Span::styled(" About", theme::style_title())),
        Line::default(),
        Line::from(Span::styled(
            " Measures against Cloudflare's public speed-test endpoints.",
            theme::style_label(),
        )),
        Line::from(Span::styled(
            " YOU is your public address; EDGE is the node we measured against.",
            theme::style_label(),
        )),
        Line::from(Span::styled(
            " Press 3 for every metric we collect.",
            theme::style_label(),
        )),
        Line::default(),
        Line::from(Span::styled(
            format!(" {} saved run(s) in history", app.history.len()),
            theme::style_label(),
        )),
    ]);
    frame.render_widget(Paragraph::new(Text::from(lines)), area);
}

/// A lightened accent for the curve stroke, so the edge stays crisp against
/// the darker filled body beneath it.
fn line_color(accent: Color) -> Color {
    theme::blend(theme::TEXT_BRIGHT, accent, 0.45)
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

fn format_bytes(b: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = MB * 1024.0;
    let v = b as f64;
    if v >= GB {
        format!("{:.2} GB", v / GB)
    } else if v >= MB {
        format!("{:.1} MB", v / MB)
    } else if v >= 1024.0 {
        format!("{:.1} KB", v / 1024.0)
    } else {
        format!("{b} B")
    }
}

fn fmt_dur(d: std::time::Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}", s / 60, s % 60)
}

/// Convert days since the Unix epoch to a civil date (Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
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

    fn render(view: View, w: u16, h: u16) -> String {
        let backend = ratatui::backend::TestBackend::new(w, h);
        let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
        let mut app = App::new();
        app.history.clear();
        app.view = view;
        terminal
            .draw(|f| draw(f, &mut app))
            .expect("draw must not panic");
        let buf = terminal.backend().buffer().clone();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn every_view_renders_at_minimum_size() {
        for view in View::ALL {
            render(view, MIN_WIDTH, MIN_HEIGHT);
        }
    }

    #[test]
    fn live_view_shows_the_route_without_privacy() {
        let mut app = App::new();
        app.meta.ip = "203.0.113.9".into();
        app.meta.colo = "MAA".into();
        app.meta.edge_ips = vec!["162.159.140.220".into()];
        let backend = ratatui::backend::TestBackend::new(120, 30);
        let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
        terminal.draw(|f| draw(f, &mut app)).expect("draw");
        let buf = terminal.backend().buffer().clone();
        let text: String = (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("YOU"), "main view must label the user side");
        assert!(text.contains("EDGE"), "main view must label the edge side");
        assert!(text.contains("203.0.113.9"), "user IP missing:\n{text}");
        assert!(text.contains("162.159.140.220"), "edge IP missing:\n{text}");
    }

    #[test]
    fn privacy_mode_hides_both_ips_on_the_main_view() {
        let mut app = App::new();
        app.meta.ip = "203.0.113.9".into();
        app.meta.edge_ips = vec!["162.159.140.220".into()];
        app.hide_info = true;
        let backend = ratatui::backend::TestBackend::new(120, 30);
        let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
        terminal.draw(|f| draw(f, &mut app)).expect("draw");
        let buf = terminal.backend().buffer().clone();
        let text: String = (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!text.contains("203.0.113.9"), "user IP leaked");
        assert!(!text.contains("162.159.140.220"), "edge IP leaked");
    }

    #[test]
    fn no_view_draws_box_drawing_borders() {
        // "No bezels" means no rounded or square card frames anywhere.
        for view in View::ALL {
            let text = render(view, 120, 34);
            for glyph in ['╭', '╮', '╰', '╯', '┌', '┐', '└', '┘'] {
                assert!(
                    !text.contains(glyph),
                    "{view:?} still draws a border glyph {glyph:?}"
                );
            }
        }
    }

    #[test]
    fn charts_draw_solid_bars_with_no_braille() {
        let mut app = App::new();
        app.view = View::Charts;
        app.down_samples = (0..40).map(|i| (i % 20) as f64 * 6.0).collect();
        let backend = ratatui::backend::TestBackend::new(120, 34);
        let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
        terminal.draw(|f| draw(f, &mut app)).expect("draw");
        let buf = terminal.backend().buffer().clone();

        let mut bars = 0usize;
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                let ch = buf[(x, y)].symbol().chars().next().unwrap_or(' ');
                // No dotted rendering anywhere: braille turns an area chart
                // into a screen of dots, which is the look being avoided.
                assert!(
                    !('\u{2800}'..='\u{28FF}').contains(&ch),
                    "braille dot at {x},{y}: the chart must draw solid blocks"
                );
                if ch == '\u{2588}' {
                    bars += 1;
                }
            }
        }
        assert!(bars > 50, "expected bars, drew only {bars} blocks");
    }
    #[test]
    fn details_view_renders_all_group_headings() {
        let text = render(View::Details, 120, 40);
        for heading in ["CONNECTION", "THROUGHPUT", "LATENCY", "QUALITY"] {
            assert!(text.contains(heading), "details view missing {heading}");
        }
    }

    #[test]
    fn nav_rects_are_recorded_for_mouse_hit_testing() {
        let mut app = App::new();
        app.view = View::Live;
        let backend = ratatui::backend::TestBackend::new(120, 30);
        let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
        terminal.draw(|f| draw(f, &mut app)).expect("draw");
        assert_eq!(app.nav.len(), View::ALL.len(), "every tab needs a hit rect");
        // Clicking the recorded rect for CHARTS must switch to it.
        if let Some((_, rect)) = app.nav.iter().find(|(v, _)| *v == View::Charts) {
            app.on_click(rect.x + 1, rect.y);
            assert_eq!(app.view, View::Charts);
        }
    }

    #[test]
    fn tiny_terminal_shows_a_hint_instead_of_corrupting() {
        let text = render(View::Live, 20, 5);
        assert!(text.contains("too small"), "expected a size hint");
    }

    #[test]
    fn bytes_scale_to_the_right_unit() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(format_bytes(3 * 1024 * 1024 * 1024), "3.00 GB");
    }

    #[test]
    fn timestamps_render_as_dates() {
        assert_eq!(fmt_time(0), "1970-01-01  00:00");
        assert_eq!(fmt_time(1_791_676_800_000), "2026-10-11  00:00");
    }

    /// The time axis has to describe the data actually drawn. A 10s phase inside
    /// the default 240s window still only holds 10s of samples, so the axis must
    /// read -10s; labelling it -240s claims four minutes of history that were
    /// never measured.
    #[test]
    fn the_time_axis_reports_the_span_actually_measured() {
        let mut app = App::new();
        app.view = View::Live;
        app.zoom = crate::app::Zoom(1200);
        // 50 samples at 200ms each is a 10s phase.
        app.down_samples = vec![100.0; 50];
        app.up_samples = vec![80.0; 50];
        let backend = ratatui::backend::TestBackend::new(120, 34);
        let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
        terminal.draw(|f| draw(f, &mut app)).expect("draw");
        let buf = terminal.backend().buffer().clone();
        let text: String = (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            text.contains("-10s"),
            "axis should read -10s for a 10s phase"
        );
        assert!(
            !text.contains("-240s"),
            "axis must not claim 240s of history that was never measured"
        );
    }

    /// Loss used to be gated on `loss > 0`, so a clean link showed nothing at
    /// all - the one result where the figure matters most was the one that
    /// never appeared.
    #[test]
    fn packet_loss_is_shown_even_when_zero() {
        let mut app = App::new();
        app.view = View::Live;
        app.running = false;
        let mut run = crate::speed::RunResult::default();
        run.idle.med = 52.0;
        run.idle.min = 47.0;
        run.idle.jitter = 11.0;
        run.idle.loss = 0.0;
        run.loaded_down.med = 114.0;
        app.result = Some(run);

        let backend = ratatui::backend::TestBackend::new(120, 34);
        let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
        terminal.draw(|f| draw(f, &mut app)).expect("draw");
        let buf = terminal.backend().buffer().clone();
        let text: String = (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("LOSS"), "loss tile missing:\n{text}");
        assert!(text.contains("0.0%"), "zero loss must be shown as a figure");
        assert!(text.contains("MIN"), "best-case latency should be shown");
    }

    /// The two sides of the route strip share one grid, so the rule that
    /// separates them lands on the same column on both rows.
    #[test]
    fn route_strip_puts_both_sides_on_one_grid() {
        let mut app = App::new();
        app.view = View::Live;
        app.meta.ip = "59.182.148.163".into();
        app.meta.colo = "MAA".into();
        app.meta.edge_ips = vec!["162.159.140.220:443".into(), "[2606:4700:7::da]:443".into()];
        app.meta.tls = "TLSv1.3".into();

        let backend = ratatui::backend::TestBackend::new(120, 34);
        let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
        terminal.draw(|f| draw(f, &mut app)).expect("draw");
        let buf = terminal.backend().buffer().clone();

        let column_of = |y: u16| -> Option<usize> {
            (0..buf.area.width)
                .find(|x| buf[(*x, y)].symbol() == "\u{2502}")
                .map(usize::from)
        };
        let row_text =
            |y: u16| -> String { (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect() };
        let row_you = (0..buf.area.height)
            .find(|y| row_text(*y).contains("YOU"))
            .expect("route row");
        // The second route row is the one below it.
        let a = column_of(row_you).expect("rule on the you row");
        let b = column_of(row_you + 1).expect("rule on the edge row");
        assert_eq!(a, b, "both sides must start at the same column");
    }
}
