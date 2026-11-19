mod apps;
mod config;
mod model;
mod ui;

use std::time::Duration;

use ratatui::crossterm::event::{self, Event, KeyEventKind};
use ratatui::DefaultTerminal;

use model::Model;

fn main() -> std::io::Result<()> {
    let (excluded, warning) = config::load();
    // Read before the UI starts, while the terminal that launched us is still frontmost.
    let terminal_pid = apps::frontmost_pid().unwrap_or(-1);
    let mut m = Model::new(excluded, terminal_pid);
    m.refresh();
    if let Some(w) = warning {
        m.set_toast(w);
    }
    ratatui::run(|terminal| run(terminal, &mut m))
}

fn run(terminal: &mut DefaultTerminal, m: &mut Model) -> std::io::Result<()> {
    loop {
        terminal.draw(|f| ui::draw(f, m))?;
        if event::poll(Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press && !m.on_key(key) {
                    return Ok(());
                }
            }
        }
        m.tick();
    }
}
