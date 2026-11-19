mod apps;
mod config;
mod icons;
mod kitten;
mod model;
mod ports;
mod ui;

use std::time::Duration;

use ratatui::crossterm::event::{self, Event, KeyEventKind};
use ratatui::DefaultTerminal;

use model::Model;

fn main() -> std::io::Result<()> {
    let (excluded, warning) = config::load();
    // Read before the UI starts, while the terminal that launched us is still frontmost.
    let terminal_pid = apps::frontmost_pid();
    let mut m = Model::new(excluded, terminal_pid.unwrap_or(-1));
    m.icons_on = icons::supported();
    m.refresh();
    let missing = terminal_pid.is_none().then(|| "Couldn't detect this terminal, so it isn't protected".to_string());
    let toast: Vec<String> = warning.into_iter().chain(missing).collect();
    if !toast.is_empty() {
        m.set_toast(toast.join(" · "));
    }
    ratatui::run(|terminal| run(terminal, &mut m))
}

fn run(terminal: &mut DefaultTerminal, m: &mut Model) -> std::io::Result<()> {
    let mut last = 0u64;
    let mut drawn: Vec<(u16, u16)> = Vec::new();
    let mut timeout = Duration::from_millis(250);
    loop {
        let mut frame = terminal.draw(|f| ui::draw(f, m))?;
        if m.icons_on {
            // A cell that had an icon and now has none isn't rewritten by ratatui if it stays blank, so the
            // image would stay. Clearing erases every image; the redraw then repaints all cells.
            let now: Vec<(u16, u16)> = m
                .icon_slots
                .iter()
                .filter(|(_, _, b)| matches!(m.icons.get(b), Some(Some(_))))
                .map(|(x, y, _)| (*x, *y))
                .collect();
            if drawn.iter().any(|p| !now.contains(p)) {
                terminal.clear()?;
                frame = terminal.draw(|f| ui::draw(f, m))?;
                last = 0;
            }
            // ratatui only rewrites changed cells, and rewriting a cell erases the image under it,
            // so redraw every icon whenever anything on screen or any icon changed.
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            // The kitten animates on its own; its cells changing doesn't touch any icon.
            let mut screen = frame.buffer.clone();
            if let Some(area) = m.kitten_area {
                for p in area.positions() {
                    screen[p].reset();
                }
            }
            screen.content().hash(&mut h);
            m.icon_slots.hash(&mut h);
            m.icons.len().hash(&mut h);
            let sig = h.finish();
            if sig != last {
                last = sig;
                let slots: Vec<(u16, u16, &[u8])> = m
                    .icon_slots
                    .iter()
                    .filter_map(|(x, y, b)| m.icons.get(b)?.as_deref().map(|png| (*x, *y, png)))
                    .collect();
                icons::draw(&slots)?;
                drawn = slots.iter().map(|(x, y, _)| (*x, *y)).collect();
            }
            // Don't wait for a key while icons are still loading, so they fill in quickly.
            if m.load_icons() {
                timeout = Duration::ZERO;
            }
        }
        let mut wait = std::mem::replace(&mut timeout, Duration::from_millis(250));
        if m.animating() {
            wait = wait.min(Duration::from_millis(80));
        }
        if event::poll(wait)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press && !m.on_key(key) {
                    return Ok(());
                }
            }
        }
        m.tick();
    }
}
