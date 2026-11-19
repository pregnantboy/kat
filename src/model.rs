use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::ListState;

use crate::apps::{self, App, Kind};

const REFRESH: Duration = Duration::from_secs(2);
const TOAST_TTL: Duration = Duration::from_secs(3);
const QUIT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sort {
    Memory,
    Name,
}

pub fn is_protected(app: &App, excluded: &[String], terminal_pid: i32) -> bool {
    app.pid == terminal_pid
        || excluded.iter().any(|b| *b == app.bundle_id)
        // Background Apple processes include loginwindow and Dock: quitting them logs you out or breaks the desktop.
        || (app.kind != Kind::Regular
            && (app.bundle_id.is_empty() || app.bundle_id.starts_with("com.apple.")))
}

pub fn sort_apps(apps: &mut [App], sort: Sort) {
    match sort {
        // None < Some, so comparing b to a puts unknown memory last.
        Sort::Memory => apps.sort_by(|a, b| {
            b.memory_bytes.cmp(&a.memory_bytes).then_with(|| a.name.cmp(&b.name))
        }),
        Sort::Name => apps.sort_by_key(|a| a.name.to_lowercase()),
    }
}

/// Keeps the order of `apps`; filtering only hides rows.
pub fn visible<'a>(apps: &'a [App], show_bg: bool, filter: &str) -> Vec<&'a App> {
    let needle = filter.to_lowercase();
    apps.iter()
        .filter(|a| show_bg || a.kind == Kind::Regular)
        .filter(|a| a.name.to_lowercase().contains(&needle))
        .collect()
}

/// The only source of pids passed to apps::quit / apps::force_quit.
pub fn quit_targets(
    apps: &[App],
    selected: &HashSet<i32>,
    cursor_pid: Option<i32>,
    excluded: &[String],
    terminal_pid: i32,
) -> Vec<App> {
    let wanted: HashSet<i32> = if selected.is_empty() {
        cursor_pid.into_iter().collect()
    } else {
        selected.clone()
    };
    apps.iter()
        .filter(|a| wanted.contains(&a.pid) && !is_protected(a, excluded, terminal_pid))
        .cloned()
        .collect()
}

fn names(apps: &[App]) -> String {
    apps.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", ")
}

pub struct Model {
    pub apps: Vec<App>,
    pub selected: HashSet<i32>,
    pub list: ListState,
    pub filter: String,
    pub filtering: bool,
    pub show_bg: bool,
    pub sort: Sort,
    pub pending: HashMap<i32, Instant>,
    pub toast: Option<(String, Instant)>,
    pub confirm: Option<Vec<App>>,
    pub help: bool,
    pub excluded: Vec<String>,
    pub terminal_pid: i32,
    refreshed: Instant,
}

impl Model {
    pub fn new(excluded: Vec<String>, terminal_pid: i32) -> Self {
        Self {
            apps: Vec::new(),
            selected: HashSet::new(),
            list: ListState::default(),
            filter: String::new(),
            filtering: false,
            show_bg: false,
            sort: Sort::Memory,
            pending: HashMap::new(),
            toast: None,
            confirm: None,
            help: false,
            excluded,
            terminal_pid,
            refreshed: Instant::now(),
        }
    }

    pub fn visible(&self) -> Vec<&App> {
        visible(&self.apps, self.show_bg, &self.filter)
    }

    pub fn protected(&self, app: &App) -> bool {
        is_protected(app, &self.excluded, self.terminal_pid)
    }

    fn cursor_pid(&self) -> Option<i32> {
        let i = self.list.selected()?;
        self.visible().get(i).map(|a| a.pid)
    }

    pub fn set_toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    fn clamp_cursor(&mut self) {
        let n = self.visible().len();
        let i = if n == 0 { None } else { Some(self.list.selected().unwrap_or(0).min(n - 1)) };
        self.list.select(i);
    }

    pub fn refresh(&mut self) {
        self.apply_refresh(apps::list());
    }

    /// Split from refresh() so tests can feed a fake app list.
    pub fn apply_refresh(&mut self, apps: Vec<App>) {
        // Rows keep their place between refreshes so the cursor doesn't jump; only the first load and `s` sort.
        if self.apps.is_empty() {
            self.apps = apps;
            sort_apps(&mut self.apps, self.sort);
        } else {
            let mut new = apps;
            let mut merged = Vec::with_capacity(new.len());
            for old in &self.apps {
                if let Some(i) = new.iter().position(|a| a.pid == old.pid) {
                    merged.push(new.remove(i));
                }
            }
            merged.extend(new); // what's left just launched
            self.apps = merged;
        }
        self.refreshed = Instant::now();
        let alive: HashSet<i32> = self.apps.iter().map(|a| a.pid).collect();
        self.selected.retain(|pid| alive.contains(pid));
        self.pending.retain(|pid, _| alive.contains(pid));

        let stuck: Vec<App> = self
            .apps
            .iter()
            .filter(|a| self.pending.get(&a.pid).is_some_and(|t| t.elapsed() >= QUIT_TIMEOUT))
            .cloned()
            .collect();
        if !stuck.is_empty() {
            for a in &stuck {
                self.pending.remove(&a.pid);
            }
            self.set_toast(format!("{} still open (save dialog?)", names(&stuck)));
        }
        self.clamp_cursor();
    }

    pub fn tick(&mut self) {
        if self.refreshed.elapsed() >= REFRESH {
            self.refresh();
        }
        if self.toast.as_ref().is_some_and(|(_, t)| t.elapsed() >= TOAST_TTL) {
            self.toast = None;
        }
    }

    pub fn targets(&self) -> Vec<App> {
        // Hidden rows stay selected but are never acted on.
        let shown: Vec<App> = self.visible().into_iter().cloned().collect();
        quit_targets(&shown, &self.selected, self.cursor_pid(), &self.excluded, self.terminal_pid)
    }

    fn send_quit(&mut self, targets: Vec<App>, force: bool) {
        let mut failed = Vec::new();
        let mut sent = Vec::new();
        for a in targets {
            let ok = if force { apps::force_quit(a.pid) } else { apps::quit(a.pid) };
            if ok {
                self.pending.insert(a.pid, Instant::now());
                sent.push(a);
            } else {
                failed.push(a);
            }
        }
        self.selected.clear();
        if failed.is_empty() {
            self.set_toast(format!("✓ Quitting {}", names(&sent)));
        } else {
            self.set_toast(format!("Couldn't quit {}", names(&failed)));
        }
        self.refresh();
    }

    pub fn toggle_cursor(&mut self) {
        let Some(pid) = self.cursor_pid() else { return };
        let Some(app) = self.apps.iter().find(|a| a.pid == pid).cloned() else { return };
        if self.protected(&app) {
            self.set_toast(format!("{} is protected", app.name));
        } else if !self.selected.remove(&pid) {
            self.selected.insert(pid);
        }
    }

    pub fn toggle_all(&mut self) {
        let pids: Vec<i32> = self
            .visible()
            .into_iter()
            .filter(|a| !self.protected(a))
            .map(|a| a.pid)
            .collect();
        if pids.iter().all(|p| self.selected.contains(p)) {
            for p in &pids {
                self.selected.remove(p);
            }
        } else {
            self.selected.extend(pids);
        }
    }

    /// Returns false when the app should exit.
    pub fn on_key(&mut self, key: KeyEvent) -> bool {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return false;
        }
        if let Some(targets) = self.confirm.take() {
            if key.code == KeyCode::Char('y') {
                self.send_quit(targets, true);
            }
            return true;
        }
        if self.help {
            self.help = false;
            return true;
        }
        if self.filtering {
            match key.code {
                KeyCode::Esc => {
                    self.filter.clear();
                    self.filtering = false;
                }
                KeyCode::Enter => self.filtering = false,
                KeyCode::Backspace => {
                    self.filter.pop();
                }
                KeyCode::Up => self.list.select_previous(),
                KeyCode::Down => self.list.select_next(),
                KeyCode::Char(c) => self.filter.push(c),
                _ => {}
            }
            self.clamp_cursor();
            return true;
        }
        match key.code {
            KeyCode::Esc if !self.filter.is_empty() => self.filter.clear(),
            KeyCode::Esc => return false,
            KeyCode::Up | KeyCode::Char('k') => self.list.select_previous(),
            KeyCode::Down | KeyCode::Char('j') => self.list.select_next(),
            KeyCode::Char(' ') => self.toggle_cursor(),
            KeyCode::Char('a') => self.toggle_all(),
            KeyCode::Enter => match self.targets() {
                t if t.is_empty() => self.set_toast("Nothing to quit"),
                t => self.send_quit(t, false),
            },
            KeyCode::Char('Q') => match self.targets() {
                t if t.is_empty() => self.set_toast("Nothing to quit"),
                t => self.confirm = Some(t),
            },
            KeyCode::Char('/') => self.filtering = true,
            KeyCode::Char('b') => self.show_bg = !self.show_bg,
            KeyCode::Char('s') => {
                self.sort = match self.sort {
                    Sort::Memory => Sort::Name,
                    Sort::Name => Sort::Memory,
                };
                sort_apps(&mut self.apps, self.sort);
            }
            KeyCode::Char('?') => self.help = true,
            _ => {}
        }
        self.clamp_cursor();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(pid: i32, name: &str, bundle_id: &str, kind: Kind, mem: Option<u64>) -> App {
        App { name: name.into(), bundle_id: bundle_id.into(), pid, kind, memory_bytes: mem }
    }

    fn sample() -> Vec<App> {
        vec![
            app(1, "Slack", "com.tinyspeck.slackmacgap", Kind::Regular, Some(1300)),
            app(2, "Safari", "com.apple.Safari", Kind::Regular, Some(1000)),
            app(3, "Finder", "com.apple.finder", Kind::Regular, Some(88)),
            app(4, "Ghostty", "com.mitchellh.ghostty", Kind::Regular, Some(200)),
            app(5, "Spotify", "com.spotify.client", Kind::MenuBar, Some(64)),
            app(6, "loginwindow", "com.apple.loginwindow", Kind::MenuBar, Some(100)),
            app(7, "Mystery", "com.example.mystery", Kind::Regular, None),
        ]
    }

    fn excluded() -> Vec<String> {
        vec!["com.apple.finder".into()]
    }

    const TERMINAL: i32 = 4;

    #[test]
    fn quit_targets_never_include_protected_apps() {
        let apps = sample();
        let everything: HashSet<i32> = apps.iter().map(|a| a.pid).collect();
        let pids: Vec<i32> =
            quit_targets(&apps, &everything, None, &excluded(), TERMINAL).iter().map(|a| a.pid).collect();
        assert_eq!(pids, vec![1, 2, 5, 7]); // not Finder (3), terminal (4), loginwindow (6)
    }

    #[test]
    fn quit_targets_fall_back_to_cursor() {
        let apps = sample();
        let pids: Vec<i32> = quit_targets(&apps, &HashSet::new(), Some(2), &excluded(), TERMINAL)
            .iter()
            .map(|a| a.pid)
            .collect();
        assert_eq!(pids, vec![2]);
        assert!(quit_targets(&apps, &HashSet::new(), Some(3), &excluded(), TERMINAL).is_empty());
    }

    #[test]
    fn filter_is_case_insensitive_substring() {
        let apps = sample();
        let names: Vec<&str> =
            visible(&apps, false, "AF").iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, vec!["Safari"]);
    }

    #[test]
    fn memory_sort_puts_unknown_last() {
        let mut apps = sample();
        sort_apps(&mut apps, Sort::Memory);
        let names: Vec<&str> =
            visible(&apps, false, "").iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, vec!["Slack", "Safari", "Ghostty", "Finder", "Mystery"]);
    }

    fn pids(m: &Model) -> Vec<i32> {
        m.apps.iter().map(|a| a.pid).collect()
    }

    #[test]
    fn first_refresh_sorts_by_memory() {
        let mut m = Model::new(excluded(), TERMINAL);
        m.apply_refresh(sample());
        assert_eq!(pids(&m), vec![1, 2, 4, 6, 3, 5, 7]);
    }

    #[test]
    fn refresh_keeps_row_order_when_memory_changes() {
        let mut m = Model::new(excluded(), TERMINAL);
        m.apply_refresh(sample());
        let mut next = sample();
        next[2].memory_bytes = Some(9999); // Finder now biggest
        m.apply_refresh(next);
        assert_eq!(pids(&m), vec![1, 2, 4, 6, 3, 5, 7]);
        assert_eq!(m.apps[4].memory_bytes, Some(9999)); // data still updated in place
    }

    #[test]
    fn refresh_appends_new_pid_and_removes_gone_pid() {
        let mut m = Model::new(excluded(), TERMINAL);
        m.apply_refresh(sample());
        let mut next: Vec<App> = sample().into_iter().filter(|a| a.pid != 2).collect();
        next.insert(0, app(8, "Zed", "dev.zed", Kind::Regular, Some(5000)));
        m.apply_refresh(next);
        assert_eq!(pids(&m), vec![1, 4, 6, 3, 5, 7, 8]);
    }

    #[test]
    fn s_key_resorts() {
        let mut m = Model::new(excluded(), TERMINAL);
        m.apply_refresh(sample());
        m.on_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
        assert_eq!(m.sort, Sort::Name);
        assert_eq!(m.apps[0].name, "Finder");
    }

    #[test]
    fn background_apps_hidden_unless_toggled() {
        let apps = sample();
        assert!(!visible(&apps, false, "").iter().any(|a| a.name == "Spotify"));
        assert!(visible(&apps, true, "").iter().any(|a| a.name == "Spotify"));
    }

    #[test]
    fn select_all_skips_protected_and_quits_only_safe_apps() {
        let mut m = Model::new(excluded(), TERMINAL);
        m.show_bg = true;
        m.apply_refresh(sample());
        m.toggle_all();
        let mut pids: Vec<i32> = m.targets().iter().map(|a| a.pid).collect();
        pids.sort();
        assert_eq!(pids, vec![1, 2, 5, 7]);
        m.toggle_all();
        assert!(m.selected.is_empty());
    }

    #[test]
    fn targets_ignore_selected_apps_hidden_by_filter() {
        let mut m = Model::new(excluded(), TERMINAL);
        m.show_bg = true;
        m.apply_refresh(sample());
        m.toggle_all();
        m.filter = "safari".into();
        assert_eq!(m.targets().iter().map(|a| a.pid).collect::<Vec<_>>(), vec![2]);
        m.filter.clear();
        assert_eq!(m.targets().len(), 4);
    }

    #[test]
    fn refresh_keeps_selection_and_drops_gone_pids() {
        let mut m = Model::new(excluded(), TERMINAL);
        m.apply_refresh(sample());
        m.selected.extend([1, 2]);
        let without_slack: Vec<App> = sample().into_iter().filter(|a| a.pid != 1).collect();
        m.apply_refresh(without_slack);
        assert_eq!(m.selected, HashSet::from([2]));
    }
}
