//! Application state, event ingestion, and key bindings.

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::history;
use crate::speed::{
    bufferbloat_grade, cv_percent, stability_grade, Event, LatencyStats, Meta, Phase, RunResult,
};
use crate::ui::charts::Scaler;

/// Bound on retained samples so a long session cannot grow without limit.
const MAX_POINTS: usize = 1200;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    Live,
    Charts,
    History,
    Help,
}

impl View {
    pub const ALL: [View; 4] = [View::Live, View::Charts, View::History, View::Help];

    pub fn label(self) -> &'static str {
        match self {
            View::Live => "LIVE",
            View::Charts => "CHARTS",
            View::History => "HISTORY",
            View::Help => "HELP",
        }
    }
}

pub struct App {
    pub view: View,
    pub phase: Phase,
    pub meta: Meta,

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
                self.meta = meta;
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

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.quit = true,

            KeyCode::Char('r') => {
                if !self.running {
                    self.restart = true;
                }
            }

            KeyCode::Char('1') => self.view = View::Live,
            KeyCode::Char('2') => self.view = View::Charts,
            KeyCode::Char('3') => self.view = View::History,
            KeyCode::Char('?') | KeyCode::Char('h') => self.view = View::Help,

            KeyCode::Tab | KeyCode::Right => {
                let i = View::ALL.iter().position(|v| *v == self.view).unwrap_or(0);
                self.view = View::ALL[(i + 1) % View::ALL.len()];
            }
            KeyCode::BackTab | KeyCode::Left => {
                let i = View::ALL.iter().position(|v| *v == self.view).unwrap_or(0);
                self.view = View::ALL[(i + len_minus_one()) % View::ALL.len()];
            }

            KeyCode::Char('H') => self.hide_info = !self.hide_info,

            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),

            _ => {}
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
}
