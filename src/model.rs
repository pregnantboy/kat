use std::collections::HashSet;

use crate::apps::{App, Kind};

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

pub fn visible<'a>(apps: &'a [App], show_bg: bool, filter: &str, sort: Sort) -> Vec<&'a App> {
    let needle = filter.to_lowercase();
    let mut out: Vec<&App> = apps
        .iter()
        .filter(|a| show_bg || a.kind == Kind::Regular)
        .filter(|a| a.name.to_lowercase().contains(&needle))
        .collect();
    match sort {
        // None < Some, so comparing b to a puts unknown memory last.
        Sort::Memory => out.sort_by(|a, b| {
            b.memory_bytes.cmp(&a.memory_bytes).then_with(|| a.name.cmp(&b.name))
        }),
        Sort::Name => out.sort_by_key(|a| a.name.to_lowercase()),
    }
    out
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
            visible(&apps, false, "AF", Sort::Name).iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, vec!["Safari"]);
    }

    #[test]
    fn memory_sort_puts_unknown_last() {
        let apps = sample();
        let names: Vec<&str> =
            visible(&apps, false, "", Sort::Memory).iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, vec!["Slack", "Safari", "Ghostty", "Finder", "Mystery"]);
    }

    #[test]
    fn background_apps_hidden_unless_toggled() {
        let apps = sample();
        assert!(!visible(&apps, false, "", Sort::Name).iter().any(|a| a.name == "Spotify"));
        assert!(visible(&apps, true, "", Sort::Name).iter().any(|a| a.name == "Spotify"));
    }
}
