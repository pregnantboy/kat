use ratatui::layout::{Alignment, Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, List, ListItem, Paragraph};
use ratatui::Frame;

use crate::apps::{App, Kind};
use crate::model::{Model, Sort};

const NAME_WIDTH: usize = 22;
const BAR_WIDTH: usize = 12;
const ACCENT: Color = Color::Cyan;

pub fn fmt_bytes(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b >= GB { format!("{:.1} GB", b / GB) } else { format!("{:.0} MB", b / MB) }
}

pub fn bar(frac: f64, width: usize) -> String {
    const PARTS: [&str; 8] = ["", "▏", "▎", "▍", "▌", "▋", "▊", "▉"];
    let eighths = (frac.clamp(0.0, 1.0) * (width * 8) as f64).round() as usize;
    let mut s = "█".repeat(eighths / 8);
    s.push_str(PARTS[eighths % 8]);
    s
}

fn badge(m: &Model, a: &App) -> &'static str {
    if m.pending.contains_key(&a.pid) {
        "quitting…"
    } else if a.pid == m.terminal_pid {
        "this terminal"
    } else if m.excluded.contains(&a.bundle_id) {
        "excluded"
    } else if m.protected(a) {
        "system"
    } else {
        match a.kind {
            Kind::Regular => "",
            Kind::MenuBar => "menu bar",
            Kind::Helper => "helper",
        }
    }
}

fn row(m: &Model, a: &App, max_mem: u64) -> ListItem<'static> {
    let protected = m.protected(a);
    let selected = m.selected.contains(&a.pid);
    let marker = if protected {
        Span::raw("🔒")
    } else if selected {
        Span::styled("● ", Style::new().fg(ACCENT))
    } else {
        Span::raw("○ ").dim()
    };
    let name: String = a.name.chars().take(NAME_WIDTH).collect();
    let mem = a.memory_bytes.map(fmt_bytes).unwrap_or_else(|| "—".into());
    let frac = a.memory_bytes.unwrap_or(0) as f64 / max_mem as f64;
    let badge = badge(m, a);
    let line = Line::from(vec![
        marker,
        Span::raw(format!(" {name:<NAME_WIDTH$} ")),
        Span::styled(format!("{:<BAR_WIDTH$}", bar(frac, BAR_WIDTH)), Style::new().fg(ACCENT)),
        Span::raw(format!(" {mem:>8}  ")),
        if badge == "quitting…" { Span::raw(badge).fg(Color::Yellow) } else { Span::raw(badge).dim() },
    ]);
    ListItem::new(if protected { line.dim() } else { line })
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let [area] = Layout::horizontal([Constraint::Length(width)]).flex(Flex::Center).areas(area);
    let [area] = Layout::vertical([Constraint::Length(height)]).flex(Flex::Center).areas(area);
    area
}

fn modal(f: &mut Frame, title: &str, color: Color, lines: Vec<Line>) {
    let width = lines.iter().map(|l| l.width()).max().unwrap_or(0) as u16 + 4;
    let area = centered(f.area(), width, lines.len() as u16 + 2);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(color))
        .title(format!(" {title} "));
    f.render_widget(Clear, area);
    f.render_widget(Paragraph::new(lines).block(block), area);
}

pub fn draw(f: &mut Frame, m: &mut Model) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(" killer ".bold().fg(ACCENT))
        .title_bottom(
            Line::from(" space select  a all  ⏎ quit  ⇧Q force  / filter  b background  s sort  ? help ")
                .dim(),
        );
    let inner = block.inner(f.area());
    f.render_widget(block, f.area());
    let [header, rule1, body, rule2, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(inner);

    // Header: filter on the left, selection summary on the right.
    let filter = if m.filtering {
        Line::from(vec![Span::raw(" / ").fg(ACCENT), Span::raw(format!("{}▏", m.filter))])
    } else if !m.filter.is_empty() {
        Line::from(vec![Span::raw(" / ").fg(ACCENT), Span::raw(m.filter.clone())])
    } else {
        Line::from(" / filter").dim()
    };
    f.render_widget(Paragraph::new(filter), header);
    let total: u64 = m
        .apps
        .iter()
        .filter(|a| m.selected.contains(&a.pid))
        .filter_map(|a| a.memory_bytes)
        .sum();
    let sort = if m.sort == Sort::Memory { "by memory" } else { "by name" };
    let bg = if m.show_bg { " · background on" } else { "" };
    let summary = format!("{} selected · {} · {sort}{bg} ", m.selected.len(), fmt_bytes(total));
    f.render_widget(Paragraph::new(summary).alignment(Alignment::Right), header);

    let rule = Paragraph::new("─".repeat(inner.width as usize)).dim();
    f.render_widget(rule.clone(), rule1);
    f.render_widget(rule, rule2);

    // App list. Build owned rows first so the borrow of m ends before rendering with &mut m.list.
    let visible = m.visible();
    let max_mem = visible.iter().filter_map(|a| a.memory_bytes).max().unwrap_or(1).max(1);
    let items: Vec<ListItem> = visible.iter().map(|a| row(m, a, max_mem)).collect();
    if items.is_empty() {
        f.render_widget(Paragraph::new("No apps match").dim().alignment(Alignment::Center), body);
    } else {
        let list = List::new(items).highlight_style(Style::new().reversed()).highlight_symbol("▌");
        f.render_stateful_widget(list, body, &mut m.list);
    }

    if let Some((msg, _)) = &m.toast {
        f.render_widget(Paragraph::new(format!(" {msg}")).bold(), footer);
    }

    if let Some(targets) = &m.confirm {
        let names: Vec<&str> = targets.iter().map(|a| a.name.as_str()).collect();
        modal(f, "Force quit", Color::Red, vec![
            Line::from(format!("Force quit {} app(s)? Unsaved work is lost.", targets.len())),
            Line::from(names.join(", ")).dim(),
            Line::from(""),
            Line::from(vec![Span::raw("y").bold().fg(Color::Red), Span::raw(" force quit   any other key cancels")]),
        ]);
    } else if m.help {
        let keys = [
            ("↑ ↓  k j", "move"),
            ("space", "select / unselect"),
            ("a", "select all (again to clear)"),
            ("⏎", "quit selection, or cursor row"),
            ("Q", "force quit (asks first)"),
            ("/", "filter by name, esc clears"),
            ("b", "show background apps"),
            ("s", "sort by memory / name"),
            ("esc  ctrl-c", "exit"),
        ];
        let lines = keys
            .iter()
            .map(|(k, v)| Line::from(vec![Span::raw(format!("{k:<12}")).fg(ACCENT), Span::raw(*v)]))
            .collect();
        modal(f, "Keys", ACCENT, lines);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_bytes() {
        assert_eq!(fmt_bytes(112 * 1024 * 1024), "112 MB");
        assert_eq!(fmt_bytes(1_395_864_371), "1.3 GB");
    }

    #[test]
    fn bar_uses_eighth_blocks() {
        assert_eq!(bar(1.0, 4), "████");
        assert_eq!(bar(0.5, 4), "██");
        assert_eq!(bar(0.0, 4), "");
        assert_eq!(bar(0.125, 4), "▌"); // 4 of 32 eighths: half a cell
    }
}
