//! Slate — dashboard TUI (proyectos, to-dos, notas, calendario y pomodoro).

mod app;
mod config;
mod model;
mod ui;

use std::io;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyEventKind};

use app::App;

fn main() -> io::Result<()> {
    let mut terminal = ratatui::init();
    let result = run(&mut terminal);
    ratatui::restore();
    result
}

fn run(terminal: &mut ratatui::DefaultTerminal) -> io::Result<()> {
    let mut app = App::new();
    // Cadencia del refresco: suficiente para que el timer se vea fluido.
    let tick_rate = Duration::from_millis(250);
    let mut last_tick = Instant::now();

    while !app.should_quit {
        terminal.draw(|frame| ui::draw(frame, &app))?;

        // Espera por input hasta el siguiente tick.
        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or(Duration::ZERO);

        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    app.on_key(key);
                }
            }
        }

        let elapsed = last_tick.elapsed();
        if elapsed >= tick_rate {
            app.tick(elapsed);
            last_tick = Instant::now();
        }
    }

    app.save();
    Ok(())
}
