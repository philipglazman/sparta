mod api;
mod app;
mod ui;

use std::io;
use std::time::Duration;

use app::{App, FetchStatus};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::prelude::*;
use tokio::sync::mpsc;

const LINKS: &[&str] = &[
    "https://velo.xyz/chart",
    "https://tradingeconomics.com/calendar",
    "https://mangoworks.grafana.net/d/pht2wll/hl",
];

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Terminal setup
    enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;

    let result = run(&mut terminal).await;

    // Terminal teardown
    disable_raw_mode()?;
    io::stdout().execute(LeaveAlternateScreen)?;

    result
}

async fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<(), Box<dyn std::error::Error>> {
    let mut app = App::new();
    let client = reqwest::Client::builder()
        .user_agent("trade-tui/0.1.0")
        .build()
        .map_err(|e| e.to_string())?;

    let (tx, mut rx) = mpsc::channel(4);

    // Spawn background fetch loop
    let fetch_client = client.clone();
    let fetch_tx = tx.clone();
    tokio::spawn(async move {
        loop {
            let result = api::fetch_data(&fetch_client).await;
            if fetch_tx.send(result).await.is_err() {
                break; // receiver dropped, app is shutting down
            }
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    });

    // Countdown ticker — fires every second
    let tick_tx = tx.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let _ = tick_tx.send(Err("__tick__".to_string())).await;
        }
    });

    loop {
        terminal.draw(|frame| ui::draw(frame, &app))?;

        // Poll for events with a short timeout so we can check the channel
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    if app.pnl.active {
                        // PNL calculator has focus
                        match key.code {
                            KeyCode::Esc => {
                                app.pnl.active = false;
                            }
                            KeyCode::Tab => {
                                app.pnl.focused_field = app.pnl.focused_field.next();
                            }
                            KeyCode::BackTab => {
                                app.pnl.focused_field = app.pnl.focused_field.prev();
                            }
                            KeyCode::Char('d') => {
                                app.pnl.direction = match app.pnl.direction {
                                    app::Direction::Long => app::Direction::Short,
                                    app::Direction::Short => app::Direction::Long,
                                };
                            }
                            KeyCode::Char(c) if (c.is_ascii_digit() || c == '.') => {
                                app.pnl.active_buf_mut().push(c);
                            }
                            KeyCode::Backspace => {
                                app.pnl.active_buf_mut().pop();
                            }
                            _ => {}
                        }
                    } else {
                        // Normal mode
                        match key.code {
                            KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                            KeyCode::Char('1') => { let _ = open::that(LINKS[0]); }
                            KeyCode::Char('2') => { let _ = open::that(LINKS[1]); }
                            KeyCode::Char('3') => { let _ = open::that(LINKS[2]); }
                            KeyCode::Char('p') => {
                                app.pnl.active = true;
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        // Check for data updates
        while let Ok(result) = rx.try_recv() {
            match result {
                Ok(data) => {
                    app.log(format!("Fetched BTC: ${:.2}", data.price));
                    app.data = Some(data);
                    app.status = FetchStatus::Ok;
                    app.seconds_until_refresh = 60;
                }
                Err(e) if e == "__tick__" => {
                    if app.seconds_until_refresh > 0 {
                        app.seconds_until_refresh -= 1;
                    }
                }
                Err(e) => {
                    app.log(format!("Fetch error: {}", e));
                    app.status = FetchStatus::Error(e);
                }
            }
        }
    }
}
