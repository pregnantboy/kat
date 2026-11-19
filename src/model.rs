use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::ListState;

use crate::apps::{self, App, Kind};
use crate::ports::{self, Port};

const TOAST_TTL: Duration = Duration::from_secs(3);
const QUIT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sort {
    Memory,
    Name,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tab {
    Apps,
    Ports,
}

/// One visible row, reduced to what selection and quitting need.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub pid: i32,
    pub label: String,
    pub protected: bool,
}

/// macOS treats bundle IDs case-insensitively.
pub fn is_excluded(app: &App, excluded: &[String]) -> bool {
    excluded.iter().any(|b| b.eq_ignore_ascii_case(&app.bundle_id))
}

pub fn is_protected(app: &App, excluded: &[String], terminal_pid: i32) -> bool {
    app.pid == terminal_pid
        || is_excluded(app, excluded)
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

/// The only source of pids passed to apps::quit, apps::force_quit and ports::kill.
/// `items` are the visible rows, so hidden selections are never acted on.
pub fn pick_targets(items: &[Item], selected: &HashSet<i32>, cursor_pid: Option<i32>) -> Vec<Item> {
    let wanted: HashSet<i32> = if selected.is_empty() {
        cursor_pid.into_iter().collect()
    } else {
        selected.clone()
    };
    let mut seen = HashSet::new();
    items
        .iter()
        .filter(|i| wanted.contains(&i.pid) && !i.protected && seen.insert(i.pid))
        .cloned()
        .collect()
}

/// Selection, cursor and in-flight quits for one list.
#[derive(Default)]
pub struct Picker {
    pub selected: HashSet<i32>,
    pub list: ListState,
    pub pending: HashMap<i32, Instant>,
    /// Quit pids confirmed dead; their rows stay until the next refresh.
    pub gone: HashSet<i32>,
}

impl Picker {
    pub fn cursor_pid(&self, items: &[Item]) -> Option<i32> {
        items.get(self.list.selected()?).map(|i| i.pid)
    }

    pub fn clamp(&mut self, n: usize) {
        let i = if n == 0 { None } else { Some(self.list.selected().unwrap_or(0).min(n - 1)) };
        self.list.select(i);
    }

    /// Returns the label of the cursor row if it is protected and so wasn't toggled.
    pub fn toggle_cursor(&mut self, items: &[Item]) -> Option<String> {
        let item = items.get(self.list.selected()?)?;
        if item.protected {
            return Some(item.label.clone()); // protected or gone
        }
        if !self.selected.remove(&item.pid) {
            self.selected.insert(item.pid);
        }
        None
    }

    pub fn toggle_all(&mut self, items: &[Item]) {
        let pids: Vec<i32> = items.iter().filter(|i| !i.protected).map(|i| i.pid).collect();
        if pids.iter().all(|p| self.selected.contains(p)) {
            for p in &pids {
                self.selected.remove(p);
            }
        } else {
            self.selected.extend(pids);
        }
    }

    /// Forgets pids that are no longer listed. Their rows are gone, so `gone` is cleared too.
    pub fn prune(&mut self, alive: &HashSet<i32>) {
        self.selected.retain(|pid| alive.contains(pid));
        self.pending.retain(|pid, _| alive.contains(pid));
        self.gone.clear();
    }

    /// Settles pending quits: dead pids become `gone`; returns (and forgets) live ones past the timeout.
    pub fn check(&mut self, alive: impl Fn(i32) -> bool) -> Vec<i32> {
        let mut stuck = Vec::new();
        for (pid, since) in self.pending.clone() {
            if !alive(pid) {
                self.pending.remove(&pid);
                self.selected.remove(&pid);
                self.gone.insert(pid);
            } else if since.elapsed() >= QUIT_TIMEOUT {
                self.pending.remove(&pid);
                stuck.push(pid);
            }
        }
        stuck
    }
}

fn port_label(p: &Port) -> String {
    format!(":{} {}", p.port, ports::name(p))
}

pub struct Model {
    pub tab: Tab,
    pub apps: Vec<App>,
    pub app_pick: Picker,
    pub ports: Vec<Port>,
    pub port_pick: Picker,
    pub filter: String,
    pub filtering: bool,
    pub show_bg: bool,
    pub show_locked: bool,
    pub sort: Sort,
    pub toast: Option<(String, Instant)>,
    pub confirm: Option<(Tab, Vec<Item>)>,
    pub help: bool,
    pub excluded: Vec<String>,
    pub terminal_pid: i32,
    pub my_uid: u32,
    pub self_pid: i32,
    pub home: String,
}

impl Model {
    pub fn new(excluded: Vec<String>, terminal_pid: i32) -> Self {
        Self {
            tab: Tab::Apps,
            apps: Vec::new(),
            app_pick: Picker::default(),
            ports: Vec::new(),
            port_pick: Picker::default(),
            filter: String::new(),
            filtering: false,
            show_bg: false,
            show_locked: false,
            sort: Sort::Memory,
            toast: None,
            confirm: None,
            help: false,
            excluded,
            terminal_pid,
            my_uid: ports::my_uid(),
            self_pid: std::process::id() as i32,
            home: std::env::var("HOME").unwrap_or_default(),
        }
    }

    pub fn visible(&self) -> Vec<&App> {
        visible(&self.apps, self.show_bg, &self.filter)
    }

    pub fn visible_ports(&self) -> Vec<&Port> {
        self.ports
            .iter()
            .filter(|p| ports::matches(p, &self.filter, &self.home))
            .filter(|p| self.show_locked || self.port_protection(p).is_none())
            .collect()
    }

    /// Locked ports the text filter lets through but `show_locked` hides.
    pub fn locked_hidden(&self) -> usize {
        if self.show_locked {
            return 0;
        }
        self.ports
            .iter()
            .filter(|p| ports::matches(p, &self.filter, &self.home) && self.port_protection(p).is_some())
            .count()
    }

    pub fn protected(&self, app: &App) -> bool {
        is_protected(app, &self.excluded, self.terminal_pid)
    }

    pub fn port_protection(&self, p: &Port) -> Option<&'static str> {
        ports::protection(p, self.my_uid, self.terminal_pid, self.self_pid)
    }

    /// The current tab's visible rows.
    pub fn items(&self) -> Vec<Item> {
        // A gone row is protected too, so the gate and select-all skip it.
        match self.tab {
            Tab::Apps => self
                .visible()
                .into_iter()
                .map(|a| Item {
                    pid: a.pid,
                    label: a.name.clone(),
                    protected: self.protected(a) || self.app_pick.gone.contains(&a.pid),
                })
                .collect(),
            Tab::Ports => self
                .visible_ports()
                .into_iter()
                .map(|p| Item {
                    pid: p.pid,
                    label: port_label(p),
                    protected: self.port_protection(p).is_some() || self.port_pick.gone.contains(&p.pid),
                })
                .collect(),
        }
    }

    pub fn picker(&mut self, tab: Tab) -> &mut Picker {
        match tab {
            Tab::Apps => &mut self.app_pick,
            Tab::Ports => &mut self.port_pick,
        }
    }

    pub fn pick(&self) -> &Picker {
        match self.tab {
            Tab::Apps => &self.app_pick,
            Tab::Ports => &self.port_pick,
        }
    }

    pub fn set_toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    fn clamp_cursor(&mut self) {
        let n = self.items().len();
        self.picker(self.tab).clamp(n);
    }

    pub fn refresh(&mut self) {
        self.apply_refresh(apps::list());
        self.apply_ports(ports::list());
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
        let alive: HashSet<i32> = self.apps.iter().map(|a| a.pid).collect();
        self.app_pick.prune(&alive);
        self.clamp_cursor();
    }

    /// `ports` arrives sorted by port from ports::list().
    pub fn apply_ports(&mut self, ports: Vec<Port>) {
        self.ports = ports;
        let alive: HashSet<i32> = self.ports.iter().map(|p| p.pid).collect();
        self.port_pick.prune(&alive);
        self.clamp_cursor();
    }

    pub fn tick(&mut self) {
        let stuck = self.app_pick.check(ports::alive);
        if !stuck.is_empty() {
            let names: Vec<&str> =
                self.apps.iter().filter(|a| stuck.contains(&a.pid)).map(|a| a.name.as_str()).collect();
            self.set_toast(format!("{} still open (save dialog?)", names.join(", ")));
        }
        let stuck = self.port_pick.check(ports::alive);
        if !stuck.is_empty() {
            // One label per process, from its first port.
            let labels: Vec<String> = stuck
                .iter()
                .filter_map(|pid| self.ports.iter().find(|p| p.pid == *pid))
                .map(port_label)
                .collect();
            self.set_toast(format!("{} still running, try ⇧Q", labels.join(", ")));
        }
        if self.toast.as_ref().is_some_and(|(_, t)| t.elapsed() >= TOAST_TTL) {
            self.toast = None;
        }
    }

    pub fn targets(&self) -> Vec<Item> {
        let items = self.items();
        let pick = self.pick();
        pick_targets(&items, &pick.selected, pick.cursor_pid(&items))
    }

    fn send(&mut self, tab: Tab, targets: Vec<Item>, force: bool) {
        let mut sent = Vec::new();
        let mut failed = Vec::new();
        for t in targets {
            let ok = match tab {
                Tab::Apps if force => apps::force_quit(t.pid),
                Tab::Apps => apps::quit(t.pid),
                Tab::Ports => ports::kill(t.pid, force),
            };
            if ok {
                self.picker(tab).pending.insert(t.pid, Instant::now());
                sent.push(t.label);
            } else {
                failed.push(t.label);
            }
        }
        self.picker(tab).selected.clear();
        let (doing, verb) = match (tab, force) {
            (Tab::Apps, false) => ("Quitting", "quit"),
            (Tab::Apps, true) => ("Force quitting", "quit"),
            (Tab::Ports, false) => ("Stopping", "stop"),
            (Tab::Ports, true) => ("Killing", "stop"),
        };
        if failed.is_empty() {
            self.set_toast(format!("✓ {doing} {}", sent.join(", ")));
        } else {
            self.set_toast(format!("Couldn't {verb} {}", failed.join(", ")));
        }
    }

    pub fn toggle_cursor(&mut self) {
        let items = self.items();
        if let Some(label) = self.picker(self.tab).toggle_cursor(&items) {
            self.set_toast(format!("{label} can't be selected"));
        }
    }

    pub fn toggle_all(&mut self) {
        let items = self.items();
        self.picker(self.tab).toggle_all(&items);
    }

    /// Returns false when the app should exit.
    pub fn on_key(&mut self, key: KeyEvent) -> bool {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return false;
        }
        if let Some((tab, targets)) = self.confirm.take() {
            if key.code == KeyCode::Char('y') {
                // The modal may have sat open a while: re-check against the current rows.
                let pids: HashSet<i32> = targets.iter().map(|t| t.pid).collect();
                match pick_targets(&self.items(), &pids, None) {
                    t if t.is_empty() => {
                        self.set_toast(if tab == Tab::Apps { "Nothing to quit" } else { "Nothing to stop" })
                    }
                    t => self.send(tab, t, true),
                }
            }
            return true;
        }
        if self.help {
            self.help = false;
            return true;
        }
        let tab = self.tab;
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
                KeyCode::Up => self.picker(tab).list.select_previous(),
                KeyCode::Down => self.picker(tab).list.select_next(),
                KeyCode::Char(c) => self.filter.push(c),
                _ => {}
            }
            self.clamp_cursor();
            return true;
        }
        let nothing = if tab == Tab::Apps { "Nothing to quit" } else { "Nothing to stop" };
        match key.code {
            KeyCode::Esc if !self.filter.is_empty() => self.filter.clear(),
            KeyCode::Esc => return false,
            KeyCode::Tab => {
                self.tab = if tab == Tab::Apps { Tab::Ports } else { Tab::Apps };
                self.filter.clear();
            }
            KeyCode::Up | KeyCode::Char('k') => self.picker(tab).list.select_previous(),
            KeyCode::Down | KeyCode::Char('j') => self.picker(tab).list.select_next(),
            KeyCode::Char(' ') => self.toggle_cursor(),
            KeyCode::Char('a') => self.toggle_all(),
            KeyCode::Enter => match self.targets() {
                t if t.is_empty() => self.set_toast(nothing),
                t => self.send(tab, t, false),
            },
            KeyCode::Char('Q') => match self.targets() {
                t if t.is_empty() => self.set_toast(nothing),
                t => self.confirm = Some((tab, t)),
            },
            KeyCode::Char('/') => self.filtering = true,
            KeyCode::Char('b') if tab == Tab::Apps => self.show_bg = !self.show_bg,
            KeyCode::Char('s') if tab == Tab::Apps => {
                self.sort = match self.sort {
                    Sort::Memory => Sort::Name,
                    Sort::Name => Sort::Memory,
                };
                sort_apps(&mut self.apps, self.sort);
            }
            KeyCode::Char('h') if tab == Tab::Ports => self.show_locked = !self.show_locked,
            KeyCode::Char('r') => {
                self.refresh();
                self.set_toast(format!("↻ {} apps · {} ports", self.apps.len(), self.ports.len()));
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
    const ME: u32 = 502;

    fn item(pid: i32, protected: bool) -> Item {
        Item { pid, label: format!("pid {pid}"), protected }
    }

    fn sorted(mut v: Vec<i32>) -> Vec<i32> {
        v.sort();
        v
    }

    fn target_pids(m: &Model) -> Vec<i32> {
        sorted(m.targets().iter().map(|t| t.pid).collect())
    }

    #[test]
    fn targets_never_include_protected_apps() {
        let mut m = Model::new(excluded(), TERMINAL);
        m.show_bg = true;
        m.apply_refresh(sample());
        m.app_pick.selected = (1..=7).collect();
        assert_eq!(target_pids(&m), vec![1, 2, 5, 7]); // not Finder (3), terminal (4), loginwindow (6)
    }

    #[test]
    fn pick_targets_fall_back_to_cursor() {
        let items = vec![item(2, false), item(3, true)];
        let pids = |c| pick_targets(&items, &HashSet::new(), Some(c)).iter().map(|i| i.pid).collect::<Vec<_>>();
        assert_eq!(pids(2), vec![2]);
        assert!(pids(3).is_empty());
    }

    #[test]
    fn pick_targets_returns_one_item_per_pid() {
        let items = vec![item(9, false), item(9, false), item(8, false)];
        let picked = pick_targets(&items, &HashSet::from([8, 9]), None);
        assert_eq!(picked.iter().map(|i| i.pid).collect::<Vec<_>>(), vec![9, 8]);
    }

    #[test]
    fn filter_is_case_insensitive_substring() {
        let apps = sample();
        let names: Vec<&str> = visible(&apps, false, "AF").iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, vec!["Safari"]);
    }

    #[test]
    fn memory_sort_puts_unknown_last() {
        let mut apps = sample();
        sort_apps(&mut apps, Sort::Memory);
        let names: Vec<&str> = visible(&apps, false, "").iter().map(|a| a.name.as_str()).collect();
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
    fn tick_does_not_refresh_rows() {
        let mut m = port_model();
        m.apply_refresh(sample());
        let (apps, ports) = (m.apps.clone(), m.ports.clone());
        m.tick();
        assert_eq!((m.apps, m.ports), (apps, ports));
    }

    #[test]
    fn confirm_y_drops_targets_no_longer_in_rows() {
        let mut m = port_model();
        m.confirm = Some((Tab::Ports, vec![item(999999, false)]));
        m.on_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert!(m.confirm.is_none());
        assert_eq!(m.toast.as_ref().map(|t| t.0.as_str()), Some("Nothing to stop"));
        assert!(m.port_pick.pending.is_empty());
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
        assert_eq!(target_pids(&m), vec![1, 2, 5, 7]);
        m.toggle_all();
        assert!(m.app_pick.selected.is_empty());
    }

    #[test]
    fn targets_ignore_selected_apps_hidden_by_filter() {
        let mut m = Model::new(excluded(), TERMINAL);
        m.show_bg = true;
        m.apply_refresh(sample());
        m.toggle_all();
        m.filter = "safari".into();
        assert_eq!(target_pids(&m), vec![2]);
        m.filter.clear();
        assert_eq!(m.targets().len(), 4);
    }

    #[test]
    fn excluded_bundle_ids_match_case_insensitively() {
        let finder = app(9, "Finder", "com.apple.Finder", Kind::Regular, None);
        assert!(is_protected(&finder, &excluded(), TERMINAL));
    }

    #[test]
    fn refresh_keeps_selection_and_drops_gone_pids() {
        let mut m = Model::new(excluded(), TERMINAL);
        m.apply_refresh(sample());
        m.app_pick.selected.extend([1, 2]);
        let without_slack: Vec<App> = sample().into_iter().filter(|a| a.pid != 1).collect();
        m.apply_refresh(without_slack);
        assert_eq!(m.app_pick.selected, HashSet::from([2]));
    }

    fn overdue() -> Instant {
        Instant::now() - QUIT_TIMEOUT - Duration::from_secs(1)
    }

    #[test]
    fn check_marks_dead_pending_pid_gone_and_unselects_it() {
        let mut p = Picker::default();
        p.pending.insert(1, Instant::now());
        p.selected.insert(1);
        assert!(p.check(|_| false).is_empty());
        assert!(p.pending.is_empty() && p.selected.is_empty());
        assert_eq!(p.gone, HashSet::from([1]));
    }

    #[test]
    fn check_returns_live_overdue_pid_as_stuck_and_keeps_recent_pending() {
        let mut p = Picker::default();
        p.pending.insert(1, overdue());
        p.pending.insert(2, Instant::now());
        assert_eq!(p.check(|_| true), vec![1]);
        assert_eq!(p.pending.keys().copied().collect::<Vec<_>>(), vec![2]);
        assert!(p.gone.is_empty());
    }

    #[test]
    fn gone_rows_are_not_targets_or_selected_by_select_all() {
        let mut m = Model::new(excluded(), TERMINAL);
        m.apply_refresh(sample());
        m.app_pick.gone.insert(1);
        m.toggle_all();
        assert_eq!(target_pids(&m), vec![2, 7]);
        m.app_pick.selected.insert(1);
        assert_eq!(target_pids(&m), vec![2, 7]);
    }

    #[test]
    fn refresh_clears_gone() {
        let mut m = port_model();
        m.apply_refresh(sample());
        m.app_pick.gone.insert(1);
        m.port_pick.gone.insert(10);
        m.apply_refresh(sample());
        m.apply_ports(vec![port(3000, 10, "/x/node")]);
        assert!(m.app_pick.gone.is_empty() && m.port_pick.gone.is_empty());
    }

    fn port(port: u16, pid: i32, exe: &str) -> Port {
        Port { port, pid, uid: ME, exe: exe.into(), args: vec![], cwd: None }
    }

    fn port_model() -> Model {
        let mut m = Model::new(excluded(), TERMINAL);
        m.my_uid = ME;
        m.self_pid = 99;
        m.tab = Tab::Ports;
        m.apply_ports(vec![
            port(3000, 10, "/x/node"),
            port(5000, 11, "/System/Library/CoreServices/ControlCenter.app/Contents/MacOS/ControlCenter"),
            port(5432, 12, "/Applications/OrbStack.app/Contents/MacOS/OrbStack"),
            port(8000, 13, "/x/python3"),
            port(8001, 13, "/x/python3"),
        ]);
        m
    }

    #[test]
    fn port_select_all_skips_protected_and_targets_each_pid_once() {
        let mut m = port_model();
        m.toggle_all();
        assert_eq!(target_pids(&m), vec![10, 13]);
    }

    #[test]
    fn port_filter_by_number_hides_other_targets() {
        let mut m = port_model();
        m.toggle_all();
        m.filter = ":3000".into();
        assert_eq!(target_pids(&m), vec![10]);
    }

    #[test]
    fn tab_switches_list_and_clears_filter() {
        let mut m = port_model();
        m.apply_refresh(sample());
        m.filter = "x".into();
        m.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(m.tab, Tab::Apps);
        assert!(m.filter.is_empty());
        assert_eq!(m.items().len(), 5); // regular apps
    }

    #[test]
    fn locked_ports_hidden_until_h() {
        let mut m = port_model();
        assert_eq!(m.items().iter().map(|i| i.pid).collect::<Vec<_>>(), vec![10, 13, 13]);
        assert_eq!(m.locked_hidden(), 2);
        m.on_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE));
        assert!(m.show_locked);
        assert_eq!(m.visible_ports().len(), 5);
    }

    #[test]
    fn h_does_nothing_on_apps_tab() {
        let mut m = port_model();
        m.tab = Tab::Apps;
        m.on_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE));
        assert!(!m.show_locked);
    }
}
