//! Application state, event ingestion, and key bindings.

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;

use crate::history;
use crate::speed::{
    bufferbloat_grade, cv_percent, stability_grade, Event, LatencyStats, Meta, Phase, RunResult,
    TestMode,
};
use crate::ui::charts::Scaler;

/// A command-palette entry.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Run(TestMode),
    ExportJson,
    ExportCsv,
    ClearHistory,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Action::Run(TestMode::Full) => "Run full test",
            Action::Run(TestMode::Latency) => "Latency only  (ping)",
            Action::Run(TestMode::Download) => "Download only",
            Action::Run(TestMode::Upload) => "Upload only",
            Action::ExportJson => "Export history as JSON",
            Action::ExportCsv => "Export history as CSV",
            Action::ClearHistory => "Clear history",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Action::Run(m) => m.label(),
            Action::ExportJson => "writes cfspeed-history.json",
            Action::ExportCsv => "writes cfspeed-history.csv",
            Action::ClearHistory => "removes every saved run",
        }
    }
}

/// Quick-access keys, shown in the footer so they need no menu.
pub const QUICK_ACTIONS: [(char, Action, &str); 4] = [
    ('f', Action::Run(TestMode::Full), "full"),
    ('p', Action::Run(TestMode::Latency), "ping"),
    ('d', Action::Run(TestMode::Download), "down"),
    ('u', Action::Run(TestMode::Upload), "up"),
];

/// Bound on retained samples so a long session cannot grow without limit.
const MAX_POINTS: usize = 1200;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    Live,
    Charts,
    Details,
    History,
    Help,
}

impl View {
    pub const ALL: [View; 5] = [
        View::Live,
        View::Charts,
        View::Details,
        View::History,
        View::Help,
    ];

    pub fn label(self) -> &'static str {
        match self {
            View::Live => "LIVE",
            View::Charts => "CHARTS",
            View::Details => "DETAILS",
            View::History => "HISTORY",
            View::Help => "HELP",
        }
    }
}

/// Visible time span of the live charts, in samples. Toggled with `+` / `-`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Zoom(pub usize);

impl Zoom {
    /// Window choices in samples; each sample is ~200ms, so these run from
    /// ~10s up to the full retained history.
    pub const STEPS: [usize; 5] = [50, 100, 250, 500, 1200];

    pub fn index(&self) -> usize {
        Self::STEPS
            .iter()
            .position(|s| *s == self.0)
            .unwrap_or(Self::STEPS.len() - 1)
    }

    pub fn widen(&mut self) {
        let i = (self.index() + 1).min(Self::STEPS.len() - 1);
        self.0 = Self::STEPS[i];
    }

    pub fn narrow(&mut self) {
        let i = self.index().saturating_sub(1);
        self.0 = Self::STEPS[i];
    }

    pub fn label(&self) -> String {
        format!("{:.0}s window", self.0 as f64 * 0.2)
    }
}

impl Default for Zoom {
    fn default() -> Self {
        Self(Self::STEPS[Self::STEPS.len() - 1])
    }
}

/// Static list of palette entries, so the menu never has to be rebuilt.
pub const MENU: [Action; 7] = [
    Action::Run(TestMode::Full),
    Action::Run(TestMode::Latency),
    Action::Run(TestMode::Download),
    Action::Run(TestMode::Upload),
    Action::ExportJson,
    Action::ExportCsv,
    Action::ClearHistory,
];

pub struct App {
    pub view: View,
    pub phase: Phase,
    pub meta: Meta,
    /// Which phases the current run will exercise.
    pub mode: TestMode,

    pub idle: LatencyStats,
    pub down_samples: Vec<f64>,
    pub up_samples: Vec<f64>,
    pub probe_samples: Vec<f64>,

    pub instant: f64,
    pub down_scaler: Scaler,
    pub up_scaler: Scaler,

    pub result: Option<RunResult>,
    pub history: Vec<RunResult>,
    pub history_sel: usize,

    pub running: bool,
    pub started: Option<Instant>,
    pub elapsed: Duration,
    pub hide_info: bool,

    /// Clickable nav tabs, recorded during draw so a mouse press can be
    /// resolved to a view. Rendering and hit-testing must agree, so the
    /// geometry is computed once per frame in `views::draw`.
    pub nav: Vec<(View, Rect)>,

    /// Freezes chart updates without stopping the measurement, so a number can
    /// be read off the screen without it racing away.
    pub paused: bool,
    pub zoom: Zoom,

    /// Command palette state. The menu is an overlay, so extra actions cost
    /// nothing until they are summoned.
    pub menu_open: bool,
    pub menu_sel: usize,

    pub status: String,
    pub error: Option<String>,
    pub restart: bool,
    pub quit: bool,
}

impl App {
    pub fn new() -> Self {
        Self {
            view: View::Live,
            phase: Phase::Connecting,
            meta: Meta::default(),
            mode: TestMode::Full,
            idle: LatencyStats::default(),
            down_samples: Vec::new(),
            up_samples: Vec::new(),
            probe_samples: Vec::new(),
            instant: 0.0,
            down_scaler: Scaler::new(1.0),
            up_scaler: Scaler::new(1.0),
            result: None,
            history: history::load(),
            history_sel: 0,
            running: false,
            started: None,
            elapsed: Duration::ZERO,
            hide_info: false,
            paused: false,
            zoom: Zoom::default(),
            menu_open: false,
            menu_sel: 0,
            nav: Vec::new(),
            status: String::new(),
            error: None,
            restart: false,
            quit: false,
        }
    }

    /// Ingest one event from the measurement engine.
    pub fn on_event(&mut self, ev: Event) {
        match ev {
            Event::Meta(meta) => {
                self.meta = *meta;
            }
            Event::Phase(phase) => {
                self.phase = phase;
                if phase == Phase::Download {
                    self.down_samples.clear();
                    self.down_scaler.reset(1.0);
                } else if phase == Phase::Upload {
                    self.up_samples.clear();
                    self.up_scaler.reset(1.0);
                }
                if phase == Phase::Done {
                    self.running = false;
                }
            }
            Event::Tick { mbps } => {
                self.instant = mbps;
                match self.phase {
                    Phase::Download => {
                        push_capped(&mut self.down_samples, mbps);
                        let m = self.down_samples.iter().copied().fold(0.0_f64, f64::max);
                        self.down_scaler.update(m);
                    }
                    Phase::Upload => {
                        push_capped(&mut self.up_samples, mbps);
                        let m = self.up_samples.iter().copied().fold(0.0_f64, f64::max);
                        self.up_scaler.update(m);
                    }
                    _ => {}
                }
            }
            Event::Probe { ms } => {
                push_capped(&mut self.probe_samples, ms);
            }
            Event::Finished(run) => {
                let run = *run;
                self.idle = run.idle.clone();
                self.result = Some(run.clone());
                self.history.push(run);
                if let Err(e) = history::save(&self.history) {
                    self.status = format!("could not save history: {e}");
                }
                self.phase = Phase::Done;
                self.running = false;
            }
            Event::Failed(msg) => {
                self.error = Some(msg);
                self.running = false;
                self.phase = Phase::Done;
            }
        }
    }

    /// Called each frame so the elapsed timer advances even without samples.
    pub fn tick(&mut self) {
        if let Some(started) = self.started {
            if self.running {
                self.elapsed = started.elapsed();
            }
        }
    }

    pub fn begin_run(&mut self) {
        self.running = true;
        self.restart = false;
        self.error = None;
        self.status.clear();
        self.result = None;
        self.idle = LatencyStats::default();
        self.down_samples.clear();
        self.up_samples.clear();
        self.probe_samples.clear();
        self.instant = 0.0;
        self.phase = Phase::Connecting;
        self.started = Some(Instant::now());
        self.elapsed = Duration::ZERO;
        self.down_scaler.reset(1.0);
        self.up_scaler.reset(1.0);
    }

    /// Handle a key press. Returns nothing; inspect `quit` / `restart`.
    pub fn on_key(&mut self, key: KeyEvent) {
        // On Windows, key events arrive as both Press and Release.
        if key.kind != crossterm::event::KeyEventKind::Press {
            return;
        }

        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }

        // While the palette is open it owns the keyboard.
        if self.menu_open {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => self.menu_open = false,
                KeyCode::Up | KeyCode::Char('k') => {
                    self.menu_sel = self.menu_sel.saturating_sub(1).min(MENU.len() - 1)
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.menu_sel = (self.menu_sel + 1).min(MENU.len() - 1)
                }
                KeyCode::Home => self.menu_sel = 0,
                KeyCode::End => self.menu_sel = MENU.len() - 1,
                KeyCode::Enter => {
                    let action = MENU[self.menu_sel];
                    self.menu_open = false;
                    self.run_action(action);
                }
                // 1-9 jump straight to an entry, as a command palette should.
                KeyCode::Char(c) => {
                    if let Some(i) = c.to_digit(10).map(|d| d as usize - 1) {
                        if i < MENU.len() {
                            self.menu_sel = i;
                            let action = MENU[i];
                            self.menu_open = false;
                            self.run_action(action);
                        }
                    }
                }
                _ => {}
            }
            return;
        }

        // Quick-access test modes.
        if let Some((_, action, _)) = QUICK_ACTIONS
            .iter()
            .find(|(k, _, _)| key.code == KeyCode::Char(*k))
        {
            self.run_action(*action);
            return;
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.quit = true,

            KeyCode::Char('m') | KeyCode::Char('/') => {
                self.menu_open = true;
                self.menu_sel = 0;
            }

            KeyCode::Char('r') => {
                if !self.running {
                    self.restart = true;
                }
            }

            KeyCode::Char('1') => self.view = View::Live,
            KeyCode::Char('2') => self.view = View::Charts,
            KeyCode::Char('3') => self.view = View::Details,
            KeyCode::Char('4') => self.view = View::History,
            KeyCode::Char('?') => self.view = View::Help,

            KeyCode::Tab | KeyCode::Right => {
                let i = View::ALL.iter().position(|v| *v == self.view).unwrap_or(0);
                self.view = View::ALL[(i + 1) % View::ALL.len()];
            }
            KeyCode::BackTab | KeyCode::Left => {
                let i = View::ALL.iter().position(|v| *v == self.view).unwrap_or(0);
                self.view = View::ALL[(i + len_minus_one()) % View::ALL.len()];
            }

            KeyCode::Char('H') => self.hide_info = !self.hide_info,

            // Pause freezes the charts; the measurement keeps running so the
            // number can be read at leisure.
            KeyCode::Char(' ') => self.paused = !self.paused,
            KeyCode::Char('+') | KeyCode::Char('=') => self.zoom.widen(),
            KeyCode::Char('-') | KeyCode::Char('_') => self.zoom.narrow(),

            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),

            _ => {}
        }
    }

    /// Samples limited to the current zoom window.
    pub fn window<'a>(&self, samples: &'a [f64]) -> &'a [f64] {
        let n = self.zoom.0.min(samples.len());
        &samples[samples.len() - n..]
    }

    /// Perform a palette or quick-key action.
    pub fn run_action(&mut self, action: Action) {
        match action {
            Action::Run(mode) => {
                self.mode = mode;
                self.status = format!("running {} test", mode.label());
                // Re-run immediately; the engine reads `mode` when it starts.
                self.restart = true;
            }
            Action::ExportJson => match history::export_json(&self.history) {
                Ok(path) => {
                    self.status =
                        format!("exported {} runs to {}", self.history.len(), path.display())
                }
                Err(e) => self.status = format!("export failed: {e}"),
            },
            Action::ExportCsv => match history::export_csv(&self.history) {
                Ok(path) => {
                    self.status =
                        format!("exported {} runs to {}", self.history.len(), path.display())
                }
                Err(e) => self.status = format!("export failed: {e}"),
            },
            Action::ClearHistory => match history::clear() {
                Ok(_) => {
                    self.history.clear();
                    self.history_sel = 0;
                    self.status = "history cleared".to_string();
                }
                Err(e) => self.status = format!("could not clear history: {e}"),
            },
        }
    }

    /// Resolve a mouse click to a nav tab.
    ///
    /// Mouse capture is enabled, so presses arrive here rather than being
    /// swallowed by the terminal. A click outside every recorded tab is
    /// ignored, which keeps stray clicks from changing view.
    pub fn on_click(&mut self, x: u16, y: u16) {
        for (view, rect) in &self.nav {
            if x >= rect.x
                && x < rect.x.saturating_add(rect.width)
                && y >= rect.y
                && y < rect.y.saturating_add(rect.height)
            {
                self.view = *view;
                return;
            }
        }
    }

    fn move_selection(&mut self, delta: isize) {
        if self.history.is_empty() {
            self.history_sel = 0;
            return;
        }
        let max = self.history.len() as isize - 1;
        let next = self.history_sel as isize + delta;
        self.history_sel = next.clamp(0, max) as usize;
    }

    /// Derived quality signals for the results strip.
    pub fn quality(&self) -> Option<Quality> {
        let r = self.result.as_ref()?;
        let bb = r.bufferbloat_ms();
        let dl_cv = cv_percent(&r.download.samples);
        let ul_cv = cv_percent(&r.upload.samples);
        let cv = if dl_cv >= ul_cv { dl_cv } else { ul_cv };
        Some(Quality {
            bufferbloat: bufferbloat_grade(bb),
            bufferbloat_ms: bb,
            stability: stability_grade(cv),
            stability_cv: cv,
        })
    }
}

fn len_minus_one() -> usize {
    View::ALL.len() - 1
}

pub struct Quality {
    pub bufferbloat: &'static str,
    pub bufferbloat_ms: f64,
    pub stability: &'static str,
    pub stability_cv: f64,
}

fn push_capped(buf: &mut Vec<f64>, v: f64) {
    buf.push(v);
    if buf.len() > MAX_POINTS {
        let excess = buf.len() - MAX_POINTS;
        buf.drain(..excess);
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        let mut a = App::new();
        a.history.clear();
        a
    }

    #[test]
    fn ticks_accumulate_per_phase() {
        let mut a = app();
        a.on_event(Event::Phase(Phase::Download));
        a.on_event(Event::Tick { mbps: 10.0 });
        a.on_event(Event::Tick { mbps: 20.0 });
        assert_eq!(a.down_samples, vec![10.0, 20.0]);
        assert!(a.up_samples.is_empty(), "upload must stay separate");

        a.on_event(Event::Phase(Phase::Upload));
        a.on_event(Event::Tick { mbps: 5.0 });
        assert_eq!(a.up_samples, vec![5.0]);
        assert_eq!(
            a.down_samples,
            vec![10.0, 20.0],
            "download history preserved"
        );
    }

    #[test]
    fn starting_a_phase_clears_its_samples() {
        let mut a = app();
        a.on_event(Event::Phase(Phase::Download));
        a.on_event(Event::Tick { mbps: 10.0 });
        a.on_event(Event::Phase(Phase::Download));
        assert!(a.down_samples.is_empty());
    }

    #[test]
    fn samples_are_capped() {
        let mut a = app();
        a.on_event(Event::Phase(Phase::Download));
        for i in 0..(MAX_POINTS + 250) {
            a.on_event(Event::Tick { mbps: i as f64 });
        }
        assert_eq!(a.down_samples.len(), MAX_POINTS);
    }

    #[test]
    fn scaler_tracks_peak_instantly() {
        let mut a = app();
        a.on_event(Event::Phase(Phase::Download));
        a.on_event(Event::Tick { mbps: 500.0 });
        assert!(a.down_scaler.value() >= 500.0);
    }

    #[test]
    fn selection_clamps_at_bounds() {
        let mut a = app();
        a.move_selection(-5);
        assert_eq!(a.history_sel, 0);
    }

    #[test]
    fn failure_records_message() {
        let mut a = app();
        a.on_event(Event::Failed("boom".into()));
        assert_eq!(a.error.as_deref(), Some("boom"));
        assert!(!a.running);
    }

    #[test]
    fn quality_absent_before_completion() {
        let a = app();
        assert!(a.quality().is_none());
    }

    #[test]
    fn space_toggles_pause() {
        let mut a = app();
        assert!(!a.paused);
        a.on_key(press(' '));
        assert!(a.paused);
        a.on_key(press(' '));
        assert!(!a.paused);
    }

    #[test]
    fn zoom_starts_wide_and_narrows_bounded() {
        let mut a = app();
        assert_eq!(a.zoom.0, Zoom::STEPS[Zoom::STEPS.len() - 1]);
        a.on_key(press('-'));
        assert!(a.zoom.0 < Zoom::STEPS[Zoom::STEPS.len() - 1]);
        // Cannot narrow past the smallest window.
        for _ in 0..10 {
            a.on_key(press('-'));
        }
        assert_eq!(a.zoom.0, Zoom::STEPS[0]);
        // ...nor widen past the largest.
        for _ in 0..10 {
            a.on_key(press('+'));
        }
        assert_eq!(a.zoom.0, Zoom::STEPS[Zoom::STEPS.len() - 1]);
    }

    #[test]
    fn window_returns_the_most_recent_samples() {
        let mut a = app();
        a.zoom = Zoom(50);
        a.down_samples = (0..200).map(|i| i as f64).collect();
        let w = a.window(&a.down_samples);
        assert_eq!(w.len(), 50);
        assert_eq!(w[49], 199.0, "window must end at the newest sample");
    }

    #[test]
    fn window_tolerates_short_history() {
        let a = app();
        let short = [1.0, 2.0];
        assert_eq!(a.window(&short).len(), 2);
        assert_eq!(a.window(&[]).len(), 0);
    }

    #[test]
    fn details_view_is_reachable_by_number_key() {
        let mut a = app();
        a.on_key(press('3'));
        assert_eq!(a.view, View::Details);
        a.on_key(press('1'));
        assert_eq!(a.view, View::Live);
    }

    /// Build a Press key event; Windows emits Release too, which must be ignored.
    /// A click inside the given rect activates the tab.
    #[test]
    fn clicking_a_nav_tab_switches_view() {
        let mut a = app();
        // Simulate what `views::draw` records for the first tab.
        a.nav = vec![
            (View::Live, Rect::new(2, 30, 6, 1)),
            (View::Charts, Rect::new(10, 30, 8, 1)),
        ];
        a.on_click(11, 30);
        assert_eq!(a.view, View::Charts);
        a.on_click(3, 30);
        assert_eq!(a.view, View::Live);
    }

    #[test]
    fn clicking_dead_space_changes_nothing() {
        let mut a = app();
        a.nav = vec![(View::Live, Rect::new(2, 30, 6, 1))];
        a.view = View::History;
        a.on_click(40, 5);
        assert_eq!(a.view, View::History, "a stray click must be ignored");
        // Edge case: the row just past the last column of a tab.
        a.on_click(8, 30);
        assert_eq!(a.view, View::History);
    }

    #[test]
    fn m_opens_the_palette_and_escape_closes_it() {
        let mut a = app();
        a.on_key(press('m'));
        assert!(a.menu_open);
        a.on_key(press_key(KeyCode::Esc));
        assert!(!a.menu_open);
    }

    #[test]
    fn palette_swallows_keys_it_does_not_handle() {
        let mut a = app();
        a.view = View::Live;
        a.on_key(press('m'));
        // An unbound character must not reach the normal bindings...
        a.on_key(press('x'));
        assert!(a.menu_open, "palette stays open");
        // ...and neither must the privacy toggle, which would otherwise
        // redact the screen out from under the open palette.
        a.on_key(press('H'));
        assert!(!a.hide_info, "palette must capture Shift+H");
        assert!(a.menu_open);
    }

    #[test]
    fn palette_navigation_wraps_within_bounds() {
        let mut a = app();
        a.on_key(press('m'));
        a.on_key(press_key(KeyCode::Home));
        a.on_key(press_key(KeyCode::Up));
        assert_eq!(a.menu_sel, 0, "must not underflow");
        a.on_key(press_key(KeyCode::End));
        a.on_key(press_key(KeyCode::Down));
        assert_eq!(a.menu_sel, MENU.len() - 1, "must not overflow");
    }

    #[test]
    fn enter_runs_the_selected_action() {
        let mut a = app();
        a.on_key(press('m'));
        // Row 3 is "Download only".
        a.on_key(press('3'));
        assert_eq!(a.mode, TestMode::Download);
        assert!(a.restart, "choosing a test must start it");
        assert!(!a.menu_open, "palette closes after running");
    }

    #[test]
    fn quick_keys_select_test_modes() {
        let mut a = app();
        a.on_key(press('d'));
        assert_eq!(a.mode, TestMode::Download);
        a.on_key(press('u'));
        assert_eq!(a.mode, TestMode::Upload);
        a.on_key(press('p'));
        assert_eq!(a.mode, TestMode::Latency);
        a.on_key(press('f'));
        assert_eq!(a.mode, TestMode::Full);
    }

    #[test]
    fn every_menu_entry_has_a_label_and_hint() {
        for a in MENU {
            assert!(!a.label().is_empty());
            assert!(!a.hint().is_empty());
        }
    }

    #[test]
    fn test_modes_gate_the_phases() {
        assert!(TestMode::Full.runs_download() && TestMode::Full.runs_upload());
        assert!(!TestMode::Latency.runs_download());
        assert!(!TestMode::Latency.runs_upload());
        assert!(TestMode::Download.runs_download());
        assert!(!TestMode::Download.runs_upload());
        assert!(!TestMode::Upload.runs_download());
        assert!(TestMode::Upload.runs_upload());
    }

    fn press(c: char) -> crossterm::event::KeyEvent {
        crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char(c),
            crossterm::event::KeyModifiers::NONE,
        )
    }

    /// Build a non-character key press (arrows, Home/End, Enter).
    fn press_key(code: crossterm::event::KeyCode) -> crossterm::event::KeyEvent {
        crossterm::event::KeyEvent::new(code, crossterm::event::KeyModifiers::NONE)
    }

    #[test]
    fn key_releases_are_ignored() {
        let mut a = app();
        a.view = View::Charts;
        let mut release = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('1'),
            crossterm::event::KeyModifiers::NONE,
        );
        release.kind = crossterm::event::KeyEventKind::Release;
        a.on_key(release);
        assert_eq!(a.view, View::Charts, "a Release must not switch views");
    }
}
