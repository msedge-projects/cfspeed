//! `cfspeed` — a TUI speed test against Cloudflare's public endpoints.

mod app;
mod history;
mod speed;
mod ui;

use std::io::{self, Stdout};
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event as CEvent, EventStream, KeyEventKind,
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
use speed::{Config, Event, Runner};

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
    let (tx, mut rx) = mpsc::unbounded_channel::<Event>();
    start_run(&tx);

    app.begin_run();

    let mut events = EventStream::new();
    let mut ticker = tokio::time::interval(FRAME);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        terminal.draw(|frame| ui::views::draw(frame, &app))?;

        tokio::select! {
            // Measurement events take priority so charts stay smooth.
            Some(ev) = rx.recv() => {
                app.on_event(ev);
            }
            maybe = events.next() => {
                match maybe {
                    Some(Ok(CEvent::Key(key))) if key.kind == KeyEventKind::Press => {
                        app.on_key(key);
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
            start_run(&tx);
            app.begin_run();
        }
    }

    Ok(())
}

/// Spawn a measurement run. The engine streams [`Event`]s back over `tx`.
fn start_run(tx: &mpsc::UnboundedSender<Event>) {
    let tx = tx.clone();
    tokio::spawn(async move {
        match Runner::new(Config::default()) {
            Ok(runner) => runner.run(tx).await,
            Err(e) => {
                let _ = tx.send(Event::Failed(format!("could not start: {e}")));
            }
        }
    });
}
