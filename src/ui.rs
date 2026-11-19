use ratatui::layout::{Alignment, Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, List, ListItem, Paragraph};
use ratatui::Frame;

use crate::apps::{App, Kind};
use crate::model::{is_excluded, Model, Sort, Tab};
use crate::ports::{self, Port};

const NAME_WIDTH: usize = 22;
const BAR_WIDTH: usize = 12;
const PORT_NAME_WIDTH: usize = 24;
const PROJECT_WIDTH: usize = 24;
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

/// Cuts from the end, marking the cut with `…`.
pub fn fit(s: &str, width: usize) -> String {
    if s.chars().count() <= width {
        return s.to_string();
    }
    let mut out: String = s.chars().take(width.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Cuts from the start, so the end of a path stays readable.
pub fn fit_left(s: &str, width: usize) -> String {
    let n = s.chars().count();
    if n <= width {
        return s.to_string();
    }
    let tail: String = s.chars().skip(n - width.saturating_sub(1)).collect();
    format!("…{tail}")
}

fn marker(protected: bool, selected: bool) -> Span<'static> {
    if protected {
        Span::raw("🔒")
    } else if selected {
        Span::styled("● ", Style::new().fg(ACCENT))
    } else {
        Span::raw("○ ").dim()
    }
}

fn badge(m: &Model, a: &App) -> &'static str {
    if m.app_pick.pending.contains_key(&a.pid) {
        "quitting…"
    } else if a.pid == m.terminal_pid {
        "this terminal"
    } else if is_excluded(a, &m.excluded) {
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
    let name: String = a.name.chars().take(NAME_WIDTH).collect();
    let mem = a.memory_bytes.map(fmt_bytes).unwrap_or_else(|| "—".into());
    let frac = a.memory_bytes.unwrap_or(0) as f64 / max_mem as f64;
    let badge = badge(m, a);
    let line = Line::from(vec![
        marker(protected, m.app_pick.selected.contains(&a.pid)),
        Span::raw(format!(" {name:<NAME_WIDTH$} ")),
        Span::styled(format!("{:<BAR_WIDTH$}", bar(frac, BAR_WIDTH)), Style::new().fg(ACCENT)),
        Span::raw(format!(" {mem:>8}  ")),
        if badge == "quitting…" { Span::raw(badge).fg(Color::Yellow) } else { Span::raw(badge).dim() },
    ]);
    ListItem::new(if protected { line.dim() } else { line })
}

fn port_row(m: &Model, p: &Port) -> ListItem<'static> {
    let protection = m.port_protection(p);
    let detail = protection.map(String::from).or_else(|| ports::project(p, &m.home)).unwrap_or_default();
    let mut spans = vec![
        marker(protection.is_some(), m.port_pick.selected.contains(&p.pid)),
        Span::raw(format!(" :{:<6} ", p.port)),
        Span::raw(format!("{:<PORT_NAME_WIDTH$} ", fit(&ports::name(p), PORT_NAME_WIDTH))),
        Span::raw(format!("{:<PROJECT_WIDTH$} ", fit_left(&detail, PROJECT_WIDTH))),
        Span::raw(format!("{:<10} {:>7}", fit(ports::process(p), 10), p.pid)).dim(),
    ];
    if m.port_pick.pending.contains_key(&p.pid) {
        spans.push(Span::raw("  stopping…").fg(Color::Yellow));
    }
    let line = Line::from(spans);
    ListItem::new(if protection.is_some() { line.dim() } else { line })
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let (width, height) = (width.min(area.width), height.min(area.height));
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

fn tab_title(m: &Model) -> Line<'static> {
    let label = |tab: Tab, text: &'static str| {
        if m.tab == tab { Span::raw(text).bold().fg(ACCENT) } else { Span::raw(text).dim() }
    };
    Line::from(vec![
        " killer ".bold().fg(ACCENT),
        Span::raw("─ "),
        label(Tab::Apps, "Apps"),
        Span::raw(" │ ").dim(),
        label(Tab::Ports, "Ports"),
        Span::raw(" "),
    ])
}

pub fn draw(f: &mut Frame, m: &mut Model) {
    let keys = match m.tab {
        Tab::Apps => " tab ports  space select  a all  ⏎ quit  ⇧Q force  / filter  b background  s sort  ? help ",
        Tab::Ports => " tab apps  space select  a all  ⏎ stop  ⇧Q kill  / filter  ? help ",
    };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(tab_title(m))
        .title_bottom(Line::from(keys).dim());
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
    let summary = match m.tab {
        Tab::Apps => {
            let picked: Vec<&App> =
                m.visible().into_iter().filter(|a| m.app_pick.selected.contains(&a.pid)).collect();
            let total: u64 = picked.iter().filter_map(|a| a.memory_bytes).sum();
            let sort = if m.sort == Sort::Memory { "by memory" } else { "by name" };
            let bg = if m.show_bg { " · background on" } else { "" };
            format!("{} selected · {} · {sort}{bg} ", picked.len(), fmt_bytes(total))
        }
        Tab::Ports => {
            let shown = m.visible_ports();
            let picked = shown.iter().filter(|p| m.port_pick.selected.contains(&p.pid)).count();
            format!("{picked} selected · {} ports ", shown.len())
        }
    };
    f.render_widget(Paragraph::new(summary).alignment(Alignment::Right), header);

    let rule = Paragraph::new("─".repeat(inner.width as usize)).dim();
    f.render_widget(rule.clone(), rule1);
    f.render_widget(rule, rule2);

    // Build owned rows first so the borrow of m ends before rendering with &mut on the picker.
    let (items, empty): (Vec<ListItem>, &str) = match m.tab {
        Tab::Apps => {
            let visible = m.visible();
            let max_mem = visible.iter().filter_map(|a| a.memory_bytes).max().unwrap_or(1).max(1);
            (visible.iter().map(|a| row(m, a, max_mem)).collect(), "No apps match")
        }
        Tab::Ports => (m.visible_ports().iter().map(|p| port_row(m, p)).collect(), "No listening ports match"),
    };
    if items.is_empty() {
        f.render_widget(Paragraph::new(empty).dim().alignment(Alignment::Center), body);
    } else {
        let list = List::new(items).highlight_style(Style::new().reversed()).highlight_symbol("▌");
        let tab = m.tab;
        f.render_stateful_widget(list, body, &mut m.picker(tab).list);
    }

    if let Some((msg, _)) = &m.toast {
        f.render_widget(Paragraph::new(format!(" {msg}")).bold(), footer);
    }

    if let Some((tab, targets)) = &m.confirm {
        const SHOWN: usize = 8;
        let n = targets.len();
        let (title, question, action) = match tab {
            Tab::Apps => (
                "Force quit",
                format!("Force quit {n} {}? Unsaved work is lost.", if n == 1 { "app" } else { "apps" }),
                " force quit   any other key cancels",
            ),
            Tab::Ports => (
                "Kill",
                format!("Kill {n} {}? They stop immediately.", if n == 1 { "process" } else { "processes" }),
                " kill   any other key cancels",
            ),
        };
        let mut lines = vec![Line::from(question)];
        lines.extend(targets.iter().take(SHOWN).map(|t| Line::from(format!("  {}", t.label)).dim()));
        if n > SHOWN {
            lines.push(Line::from(format!("  +{} more", n - SHOWN)).dim());
        }
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::raw("y").bold().fg(Color::Red), Span::raw(action)]));
        modal(f, title, Color::Red, lines);
    } else if m.help {
        let keys = [
            ("tab", "switch apps / ports"),
            ("↑ ↓  k j", "move"),
            ("space", "select / unselect"),
            ("a", "select all (again to clear)"),
            ("⏎", "quit or stop selection, or cursor row"),
            ("Q", "force quit or kill (asks first)"),
            ("/", "filter, esc clears"),
            ("b", "show background apps"),
            ("s", "sort apps by memory / name"),
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
    use crate::model::Item;
    use ratatui::{backend::TestBackend, Terminal};

    fn render(m: &mut Model, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| draw(f, m)).unwrap();
        term.backend().buffer().content().iter().map(|c| c.symbol()).collect()
    }

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

    #[test]
    fn fit_cuts_and_marks() {
        assert_eq!(fit("next-server", 20), "next-server");
        assert_eq!(fit("next-server (v16.4.1)", 8), "next-se…");
        assert_eq!(fit_left("~/orca/workspaces/sso/apps/suite", 12), "…/apps/suite");
    }

    #[test]
    fn force_quit_modal_lists_eight_apps_then_a_count() {
        let names = ["Alpha", "Bravo", "Charlie", "Delta", "Echo", "Foxtrot", "Golf", "Hotel", "India", "Juliet"];
        let items =
            names.iter().enumerate().map(|(i, n)| Item { pid: i as i32 + 100, label: n.to_string(), protected: false });
        let mut m = Model::new(vec![], -1);
        m.confirm = Some((Tab::Apps, items.collect()));
        let text = render(&mut m, 60, 20);
        assert!(text.contains("Alpha") && text.contains("Hotel") && text.contains("+2 more"));
        assert!(!text.contains("India"));
    }

    #[test]
    fn ports_tab_shows_name_project_and_protection() {
        let mut m = Model::new(vec![], -1);
        m.my_uid = 502;
        m.home = "/Users/ian".into();
        m.tab = Tab::Ports;
        let node = "/Users/ian/.nvm/versions/node/v24.22.1/bin/node";
        let cc = "/System/Library/CoreServices/ControlCenter.app/Contents/MacOS/ControlCenter";
        m.apply_ports(vec![
            Port {
                port: 3000,
                pid: 58388,
                uid: 502,
                exe: node.into(),
                args: vec!["next-server (v16.4.1) ".into()],
                cwd: Some("/Users/ian/code/shop".into()),
            },
            Port { port: 5000, pid: 747, uid: 502, exe: cc.into(), args: vec![cc.into()], cwd: Some("/".into()) },
        ]);
        let text = render(&mut m, 100, 10);
        for want in [":3000", "next-server (v16.4.1)", "~/code/shop", "node", "58388", ":5000", "ControlCenter", "system", "2 ports", "⏎ stop"] {
            assert!(text.contains(want), "missing {want}");
        }
    }
}
