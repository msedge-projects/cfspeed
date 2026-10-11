//! `cfspeed` — a TUI speed test against Cloudflare's public endpoints.

mod app;
mod history;
mod speed;
mod ui;

use std::io::{self, Stdout};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event as CEvent, EventStream, KeyEventKind,
    MouseButton, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use futures::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::sync::mpsc;

use app::App;
use speed::{Config, Event, Runner, TestMode};

type Tui = Terminal<CrosstermBackend<Stdout>>;

/// Target frame rate. The sampler emits every 200ms; redrawing faster keeps
/// the elapsed timer and transitions smooth without meaningful CPU cost.
const FRAME: Duration = Duration::from_millis(33);

fn main() -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(run())
}

async fn run() -> Result<()> {
    let mut terminal = setup_terminal()?;
    let result = event_loop(&mut terminal).await;
    restore_terminal(&mut terminal)?;
    result
}

fn setup_terminal() -> Result<Tui> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    Ok(Terminal::new(CrosstermBackend::new(stdout))?)
}

fn restore_terminal(terminal: &mut Tui) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

async fn event_loop(terminal: &mut Tui) -> Result<()> {
    let mut app = App::new();

    // One tagged channel for the UI. Each run gets its own inner channel and a
    // forwarder that stamps the generation, so a superseded run can never
    // deliver events into the current test.
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<(u64, Event)>();
    let mut cancel = Arc::new(AtomicBool::new(false));

    app.begin_run();
    start_run(&out_tx, app.mode, app.generation, Arc::clone(&cancel));

    let mut events = EventStream::new();
    let mut ticker = tokio::time::interval(FRAME);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        terminal.draw(|frame| ui::views::draw(frame, &mut app))?;

        tokio::select! {
            // Measurement events take priority so charts stay smooth.
            Some((gen, ev)) = out_rx.recv() => {
                app.on_event(gen, ev);
            }
            maybe = events.next() => {
                match maybe {
                    Some(Ok(CEvent::Key(key))) if key.kind == KeyEventKind::Press => {
                        app.on_key(key);
                    }
                    Some(Ok(CEvent::Mouse(mouse))) => {
                        // Only a left-button press counts as a navigation click.
                        if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                            app.on_click(mouse.column, mouse.row);
                        }
                    }
                    Some(Ok(CEvent::Resize(_, _))) => {
                        // Ratatui redraws at full size on the next frame.
                    }
                    Some(Err(_)) | None => {}
                    _ => {}
                }
            }
            _ = ticker.tick() => {
                app.tick();
            }
        }

        if app.quit {
            break;
        }
        if app.restart {
            // Stop the superseded run before starting the next one, so it
            // stops competing for bandwidth immediately.
            cancel.store(true, Ordering::Relaxed);
            app.begin_run();
            cancel = Arc::new(AtomicBool::new(false));
            start_run(&out_tx, app.mode, app.generation, Arc::clone(&cancel));
        }
    }

    cancel.store(true, Ordering::Relaxed);
    Ok(())
}

/// Spawn a measurement run, forwarding its events onto the shared tagged
/// channel.
fn start_run(
    out: &mpsc::UnboundedSender<(u64, Event)>,
    mode: TestMode,
    generation: u64,
    cancel: Arc<AtomicBool>,
) {
    let out = out.clone();
    tokio::spawn(async move {
        // A private channel per run: once this forwarder returns, the receiver
        // drops and any in-flight sends from the old runner fail harmlessly.
        let (tx, mut rx) = mpsc::unbounded_channel::<Event>();

        let runner = {
            let tx = tx.clone();
            tokio::spawn(async move {
                let config = Config {
                    mode,
                    ..Config::default()
                };
                match Runner::new(config) {
                    Ok(r) => r.run(tx, cancel).await,
                    Err(e) => {
                        let _ = tx.send(Event::Failed(format!("could not start: {e}")));
                    }
                }
            })
        };

        while let Some(ev) = rx.recv().await {
            if out.send((generation, ev)).is_err() {
                break;
            }
        }
        runner.abort();
    });
}
