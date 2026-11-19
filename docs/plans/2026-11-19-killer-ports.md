# killer Ports tab Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a Ports tab to `killer` that lists listening TCP ports with a readable name for each, and stops (`SIGTERM`) or kills (`SIGKILL`) the owning process under the same protection model as apps.

**Architecture:** New `src/ports.rs` scans sockets with the `libproc` crate and holds the pure naming, protection, and filter rules. `src/model.rs` moves selection, cursor, and pending-quit state into a `Picker` per tab, and routes both tabs through one gate, `pick_targets`. `src/ui.rs` adds the tab title and port rows.

**Tech Stack:** Rust (stable 1.100), Ratatui 0.30, objc2-app-kit 0.3, libc 0.2, libproc 0.15 (new).

**Spec:** `docs/specs/2026-11-19-killer-ports-design.md` (builds on `docs/specs/2026-11-19-killer-design.md`)

## Global Constraints

- macOS only. Every command runs from `/Users/ian/Documents/killer`, on branch `feat/v1`.
- If `cargo` isn't found, run `. "$HOME/.cargo/env"` first.
- NEVER run `cargo run` or `target/*/killer`: it is an interactive TUI that quits apps and kills processes. Verify with `cargo test` and `cargo build` only.
- New dependency allowed: `libproc` 0.15 only.
- Only `apps.rs` and `ports.rs` may contain `unsafe` or call the OS.
- Every pid passed to `apps::quit`, `apps::force_quit`, or `ports::kill` must come from `model::pick_targets`.
- Port protection badges, exactly: `this terminal`, `killer`, `other user`, `docker`, `system`. `/usr/local/` is not system.
- Colors: only the 16 ANSI colors plus `dim`, `bold`, and `reversed`. No RGB.
- Timings unchanged: key poll 250ms, refresh 2s, toast 3s, "still running" after 10s.

---

### Task 1: Port scanning, naming, and protection

**Files:**
- Modify: `Cargo.toml`, `Cargo.lock` (via `cargo add`)
- Create: `src/ports.rs`
- Modify: `src/main.rs` (add `mod ports;` below `mod model;`)

**Interfaces:**
- Produces:
  - `pub struct Port { pub port: u16, pub pid: i32, pub uid: u32, pub exe: String, pub args: Vec<String>, pub cwd: Option<String> }` (derives `Clone, Debug, PartialEq`)
  - `pub fn list() -> Vec<Port>`: sorted by port then pid, no duplicates
  - `pub fn kill(pid: i32, force: bool) -> bool`
  - `pub fn my_uid() -> u32`
  - `pub fn process(p: &Port) -> &str`
  - `pub fn name(p: &Port) -> String`
  - `pub fn project(p: &Port, home: &str) -> Option<String>`
  - `pub fn protection(p: &Port, my_uid: u32, terminal_pid: i32, self_pid: i32) -> Option<&'static str>`
  - `pub fn matches(p: &Port, needle: &str, home: &str) -> bool`

- [ ] **Step 1: Add the dependency**

```bash
cargo add libproc@0.15
```

- [ ] **Step 2: Write the tests against stubs (RED)**

Create `src/ports.rs` containing only the `Port` struct, these stubs, and the `#[cfg(test)] mod tests` block from the full file in Step 3 (copy it exactly; leave out `mod live` for now):

```rust
#[derive(Clone, Debug, PartialEq)]
pub struct Port {
    pub port: u16,
    pub pid: i32,
    pub uid: u32,
    pub exe: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
}

pub fn name(_p: &Port) -> String {
    todo!()
}

pub fn project(_p: &Port, _home: &str) -> Option<String> {
    todo!()
}

pub fn protection(_p: &Port, _my_uid: u32, _terminal_pid: i32, _self_pid: i32) -> Option<&'static str> {
    todo!()
}

pub fn matches(_p: &Port, _needle: &str, _home: &str) -> bool {
    todo!()
}
```

Add `mod ports;` to `src/main.rs` below `mod model;`.

Run: `cargo test -q ports::tests`
Expected: 4 tests FAIL with `not yet implemented`.

- [ ] **Step 3: Write the full file (GREEN)**

Replace `src/ports.rs` with:

```rust
use std::ffi::CStr;

use libproc::libproc::bsd_info::BSDInfo;
use libproc::libproc::file_info::{pidfdinfo, ListFDs, ProcFDType};
use libproc::libproc::net_info::{SocketFDInfo, SocketInfoKind};
use libproc::libproc::proc_pid::{listpidinfo, pidinfo, pidpath};
use libproc::processes::{pids_by_type, ProcFilter};

const TSI_S_LISTEN: i32 = 1;
const INTERPRETERS: &[&str] = &["node", "python", "ruby", "bun", "deno", "java", "php", "perl"];

#[derive(Clone, Debug, PartialEq)]
pub struct Port {
    pub port: u16,
    pub pid: i32,
    pub uid: u32,
    pub exe: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
}

/// Listening TCP ports owned by processes we can inspect (our own user's), sorted by port then pid.
pub fn list() -> Vec<Port> {
    let mut found: Vec<(u16, i32, u32)> = Vec::new();
    for pid in pids_by_type(ProcFilter::All).unwrap_or_default() {
        let pid = pid as i32;
        let Ok(info) = pidinfo::<BSDInfo>(pid, 0) else { continue };
        let Ok(fds) = listpidinfo::<ListFDs>(pid, info.pbi_nfiles as usize) else { continue };
        for fd in fds {
            if !matches!(fd.proc_fdtype.into(), ProcFDType::Socket) {
                continue;
            }
            let Ok(s) = pidfdinfo::<SocketFDInfo>(pid, fd.proc_fd) else { continue };
            if !matches!(s.psi.soi_kind.into(), SocketInfoKind::Tcp) {
                continue;
            }
            let tcp = unsafe { s.psi.soi_proto.pri_tcp };
            if tcp.tcpsi_state == TSI_S_LISTEN {
                found.push((u16::from_be(tcp.tcpsi_ini.insi_lport as u16), pid, info.pbi_uid));
            }
        }
    }
    found.sort();
    found.dedup(); // IPv4 + IPv6 listeners on the same port
    found
        .into_iter()
        .map(|(port, pid, uid)| Port {
            port,
            pid,
            uid,
            exe: pidpath(pid).unwrap_or_default(),
            args: args(pid).unwrap_or_default(),
            cwd: cwd(pid),
        })
        .collect()
}

pub fn kill(pid: i32, force: bool) -> bool {
    unsafe { libc::kill(pid, if force { libc::SIGKILL } else { libc::SIGTERM }) == 0 }
}

pub fn my_uid() -> u32 {
    unsafe { libc::getuid() }
}

fn cwd(pid: i32) -> Option<String> {
    let mut info: libc::proc_vnodepathinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_vnodepathinfo>() as i32;
    let rc = unsafe {
        libc::proc_pidinfo(pid, libc::PROC_PIDVNODEPATHINFO, 0, &mut info as *mut _ as *mut _, size)
    };
    if rc != size {
        return None;
    }
    let path = unsafe { CStr::from_ptr(info.pvi_cdir.vip_path.as_ptr() as *const libc::c_char) };
    Some(path.to_string_lossy().into_owned())
}

// KERN_PROCARGS2 buffer: argc (i32), the exec path, NUL padding, then argc NUL-separated args.
fn args(pid: i32) -> Option<Vec<String>> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    let mut size: libc::size_t = 0;
    let null = std::ptr::null_mut();
    if unsafe { libc::sysctl(mib.as_mut_ptr(), 3, null, &mut size, null, 0) } != 0 {
        return None;
    }
    let mut buf = vec![0u8; size];
    if unsafe { libc::sysctl(mib.as_mut_ptr(), 3, buf.as_mut_ptr() as *mut _, &mut size, null, 0) } != 0 {
        return None;
    }
    buf.truncate(size);
    let argc = i32::from_ne_bytes(buf.get(..4)?.try_into().ok()?) as usize;
    let rest = &buf[4..];
    let rest = &rest[rest.iter().position(|&b| b == 0)?..];
    let rest = &rest[rest.iter().position(|&b| b != 0)?..];
    Some(rest.split(|&b| b == 0).take(argc).map(|s| String::from_utf8_lossy(s).into_owned()).collect())
}

fn file_name(path: &str) -> &str {
    path.trim().rsplit('/').next().unwrap_or(path)
}

/// The executable's file name, e.g. `node`.
pub fn process(p: &Port) -> &str {
    file_name(&p.exe)
}

/// Case-insensitive: python3 re-executes itself as `.../Python.app/Contents/MacOS/Python`.
fn is_interpreter(cmd: &str) -> bool {
    let cmd = cmd.to_lowercase();
    INTERPRETERS.iter().any(|i| {
        cmd.strip_prefix(i).is_some_and(|v| v.chars().all(|c| c.is_ascii_digit() || c == '.'))
    })
}

/// What a person would call this process: the script an interpreter runs, the Mac app it belongs to, or its command.
pub fn name(p: &Port) -> String {
    let cmd = p.args.first().map(|a| file_name(a)).unwrap_or_default();
    // Interpreters come first: Xcode's and Homebrew's python3 live inside a Python.app bundle.
    if !is_interpreter(cmd) {
        if let Some(app) = p.exe.split('/').find_map(|c| c.strip_suffix(".app")) {
            return app.to_string();
        }
        return if cmd.is_empty() { file_name(&p.exe) } else { cmd }.to_string();
    }
    let mut rest = p.args[1..].iter().filter(|a| !a.starts_with('-'));
    let Some(script) = rest.next() else { return cmd.to_string() };
    let mut name = file_name(script).to_string();
    if let Some(sub) = rest.next().filter(|a| !a.contains('/')) {
        name.push(' ');
        name.push_str(sub);
    }
    name
}

pub fn project(p: &Port, home: &str) -> Option<String> {
    let cwd = p.cwd.as_deref().filter(|c| *c != "/")?;
    Some(match cwd.strip_prefix(home) {
        Some(rest) if !home.is_empty() => format!("~{rest}"),
        _ => cwd.to_string(),
    })
}

/// The badge for a port whose process must not be stopped, or None if it can be.
pub fn protection(p: &Port, my_uid: u32, terminal_pid: i32, self_pid: i32) -> Option<&'static str> {
    if p.pid == terminal_pid {
        Some("this terminal")
    } else if p.pid == self_pid {
        Some("killer")
    } else if p.uid != my_uid {
        Some("other user")
    } else if p.exe.contains("OrbStack") || p.exe.contains("Docker") {
        // These own every forwarded container port; killing one stops every container.
        Some("docker")
    } else if p.exe.starts_with("/System/") || (p.exe.starts_with("/usr/") && !p.exe.starts_with("/usr/local/")) {
        Some("system")
    } else {
        None
    }
}

pub fn matches(p: &Port, needle: &str, home: &str) -> bool {
    let needle = needle.trim_start_matches(':').to_lowercase();
    p.port.to_string().contains(&needle)
        || name(p).to_lowercase().contains(&needle)
        || project(p, home).is_some_and(|d| d.to_lowercase().contains(&needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn port(port: u16, pid: i32, exe: &str, args: &[&str], cwd: Option<&str>) -> Port {
        Port {
            port,
            pid,
            uid: 502,
            exe: exe.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            cwd: cwd.map(String::from),
        }
    }

    const NODE: &str = "/Users/ian/.nvm/versions/node/v24.22.1/bin/node";

    #[test]
    fn names_from_real_command_lines() {
        let helper = "/Applications/OrbStack.app/Contents/Frameworks/OrbStack Helper.app/Contents/MacOS/OrbStack Helper";
        assert_eq!(name(&port(5432, 1, helper, &[helper, "vm"], None)), "OrbStack");
        let cc = "/System/Library/CoreServices/ControlCenter.app/Contents/MacOS/ControlCenter";
        assert_eq!(name(&port(5000, 1, cc, &[cc], None)), "ControlCenter");
        assert_eq!(name(&port(3000, 1, NODE, &["next-server (v16.4.1) "], None)), "next-server (v16.4.1)");
        assert_eq!(name(&port(5158, 1, NODE, &["node", "test/mock-singpass/server.mjs"], None)), "server.mjs");
        assert_eq!(name(&port(3001, 1, NODE, &[NODE, "/x/node_modules/.bin/next", "dev"], None)), "next dev");
        let xcode_py = "/Applications/Xcode.app/Contents/Developer/Library/Frameworks/Python3.framework/Versions/3.9/Resources/Python.app/Contents/MacOS/Python";
        assert_eq!(name(&port(8764, 1, xcode_py, &[xcode_py, "-m", "http.server", "8764"], None)), "http.server 8764");
        let py = "/opt/homebrew/bin/python3.12";
        assert_eq!(name(&port(8765, 1, py, &["python3.12", "-m", "http.server", "8765"], None)), "http.server 8765");
        let codex = "/Users/ian/.codex/bin/codex";
        assert_eq!(name(&port(50892, 1, codex, &["codex", "--no-daemon", "-c", "x=1"], None)), "codex");
        assert_eq!(name(&port(1, 1, "/usr/libexec/rapportd", &[], None)), "rapportd");
    }

    #[test]
    fn project_shortens_home_and_skips_root() {
        let p = port(3000, 1, NODE, &[], Some("/Users/ian/code/shop"));
        assert_eq!(project(&p, "/Users/ian").as_deref(), Some("~/code/shop"));
        assert_eq!(project(&port(1, 1, NODE, &[], Some("/")), "/Users/ian"), None);
        assert_eq!(project(&port(1, 1, NODE, &[], None), "/Users/ian"), None);
    }

    #[test]
    fn protection_badges() {
        let me = 502;
        let ok = port(3000, 10, NODE, &[], None);
        assert_eq!(protection(&ok, me, 4, 99), None);
        assert_eq!(protection(&port(1, 4, NODE, &[], None), me, 4, 99), Some("this terminal"));
        assert_eq!(protection(&port(1, 99, NODE, &[], None), me, 4, 99), Some("killer"));
        assert_eq!(protection(&Port { uid: 0, ..ok.clone() }, me, 4, 99), Some("other user"));
        let orb = "/Applications/OrbStack.app/Contents/MacOS/OrbStack";
        assert_eq!(protection(&port(5432, 10, orb, &[], None), me, 4, 99), Some("docker"));
        assert_eq!(protection(&port(1, 10, "/usr/libexec/rapportd", &[], None), me, 4, 99), Some("system"));
        let cc = "/System/Library/CoreServices/ControlCenter.app/Contents/MacOS/ControlCenter";
        assert_eq!(protection(&port(5000, 10, cc, &[], None), me, 4, 99), Some("system"));
        assert_eq!(protection(&port(5432, 10, "/usr/local/bin/postgres", &[], None), me, 4, 99), None);
    }

    #[test]
    fn filter_matches_port_name_and_project() {
        let p = port(3000, 1, NODE, &["next-server (v16.4.1)"], Some("/Users/ian/code/shop"));
        for needle in ["3000", ":3000", "NEXT", "shop"] {
            assert!(matches(&p, needle, "/Users/ian"), "{needle}");
        }
        assert!(!matches(&p, "5432", "/Users/ian"));
    }
}

/// Spawns a real server and stops it. Run with `cargo test -- --ignored`.
#[cfg(test)]
mod live {
    use super::*;
    use std::process::{Command, Stdio};
    use std::thread::sleep;
    use std::time::Duration;

    #[test]
    #[ignore]
    fn finds_names_and_stops_a_real_server() {
        let mut child = Command::new("python3")
            .args(["-m", "http.server", "8765"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        sleep(Duration::from_secs(1));
        let all = list();
        let p = all.iter().find(|p| p.port == 8765).expect(":8765 should be listed");
        assert_eq!(name(p), "http.server 8765");
        assert_eq!(protection(p, my_uid(), -1, -1), None);
        assert!(kill(p.pid, false));
        sleep(Duration::from_millis(500));
        assert!(child.try_wait().unwrap().is_some(), "server should have exited");
        assert!(!list().iter().any(|p| p.port == 8765));
    }
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -q`
Expected: `22 passed; 0 failed; 1 ignored` (18 existing tests, 4 new ports tests, and the ignored live test). Dead-code warnings for `list`, `kill`, `my_uid`, `process`, and `matches` are expected until Task 2 uses them.

Then run the live check, which starts and stops a real `python3 -m http.server 8765` (safe: it only touches the process it spawned):

Run: `cargo test -q live -- --ignored`
Expected: `1 passed`.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src/ports.rs src/main.rs
git commit -m "feat: scan listening TCP ports and name their processes"
```

---

### Task 2: Ports tab (model and UI)

**Files:**
- Replace: `src/model.rs`
- Replace: `src/ui.rs`

**Interfaces:**
- Consumes: everything `ports.rs` produces in Task 1; `apps::{App, Kind, list, quit, force_quit}`.
- Produces:
  - `pub enum Tab { Apps, Ports }`
  - `pub struct Item { pub pid: i32, pub label: String, pub protected: bool }`
  - `pub fn pick_targets(items: &[Item], selected: &HashSet<i32>, cursor_pid: Option<i32>) -> Vec<Item>` (replaces `quit_targets`)
  - `pub struct Picker { pub selected, pub list, pub pending }` with `cursor_pid`, `clamp`, `toggle_cursor`, `toggle_all`, `prune`
  - `Model` fields `tab`, `app_pick`, `ports`, `port_pick`, `my_uid`, `self_pid`, `home`; `confirm: Option<(Tab, Vec<Item>)>`; methods `visible_ports`, `port_protection`, `items`, `picker`, `pick`, `apply_ports`
  - `ui::fit`, `ui::fit_left`

This task is a refactor plus a feature. `model.rs` and `ui.rs` change together because `ui.rs` reads the model fields that move into `Picker`, so they are replaced in one step. The existing app tests are kept and adapted to the new fields (for example `m.selected` becomes `m.app_pick.selected`). They are the regression net for the refactor. New tests cover `pick_targets`, `Picker::prune`, and the Ports tab.

- [ ] **Step 1: Replace `src/model.rs`**

```rust
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::ListState;

use crate::apps::{self, App, Kind};
use crate::ports::{self, Port};

const REFRESH: Duration = Duration::from_secs(2);
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
            return Some(item.label.clone());
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

    /// Forgets pids that are gone, and returns (and forgets) pending quits older than the timeout.
    pub fn prune(&mut self, alive: &HashSet<i32>) -> Vec<i32> {
        self.selected.retain(|pid| alive.contains(pid));
        self.pending.retain(|pid, _| alive.contains(pid));
        let stuck: Vec<i32> =
            self.pending.iter().filter(|(_, t)| t.elapsed() >= QUIT_TIMEOUT).map(|(p, _)| *p).collect();
        for pid in &stuck {
            self.pending.remove(pid);
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
    pub sort: Sort,
    pub toast: Option<(String, Instant)>,
    pub confirm: Option<(Tab, Vec<Item>)>,
    pub help: bool,
    pub excluded: Vec<String>,
    pub terminal_pid: i32,
    pub my_uid: u32,
    pub self_pid: i32,
    pub home: String,
    refreshed: Instant,
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
            sort: Sort::Memory,
            toast: None,
            confirm: None,
            help: false,
            excluded,
            terminal_pid,
            my_uid: ports::my_uid(),
            self_pid: std::process::id() as i32,
            home: std::env::var("HOME").unwrap_or_default(),
            refreshed: Instant::now(),
        }
    }

    pub fn visible(&self) -> Vec<&App> {
        visible(&self.apps, self.show_bg, &self.filter)
    }

    pub fn visible_ports(&self) -> Vec<&Port> {
        self.ports.iter().filter(|p| ports::matches(p, &self.filter, &self.home)).collect()
    }

    pub fn protected(&self, app: &App) -> bool {
        is_protected(app, &self.excluded, self.terminal_pid)
    }

    pub fn port_protection(&self, p: &Port) -> Option<&'static str> {
        ports::protection(p, self.my_uid, self.terminal_pid, self.self_pid)
    }

    /// The current tab's visible rows.
    pub fn items(&self) -> Vec<Item> {
        match self.tab {
            Tab::Apps => self
                .visible()
                .into_iter()
                .map(|a| Item { pid: a.pid, label: a.name.clone(), protected: self.protected(a) })
                .collect(),
            Tab::Ports => self
                .visible_ports()
                .into_iter()
                .map(|p| Item { pid: p.pid, label: port_label(p), protected: self.port_protection(p).is_some() })
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
        self.refreshed = Instant::now();
        let alive: HashSet<i32> = self.apps.iter().map(|a| a.pid).collect();
        let stuck = self.app_pick.prune(&alive);
        if !stuck.is_empty() {
            let names: Vec<&str> =
                self.apps.iter().filter(|a| stuck.contains(&a.pid)).map(|a| a.name.as_str()).collect();
            self.set_toast(format!("{} still open (save dialog?)", names.join(", ")));
        }
        self.clamp_cursor();
    }

    /// `ports` arrives sorted by port from ports::list().
    pub fn apply_ports(&mut self, ports: Vec<Port>) {
        self.ports = ports;
        let alive: HashSet<i32> = self.ports.iter().map(|p| p.pid).collect();
        let stuck = self.port_pick.prune(&alive);
        if !stuck.is_empty() {
            // One label per process, from its first port.
            let labels: Vec<String> = stuck
                .iter()
                .filter_map(|pid| self.ports.iter().find(|p| p.pid == *pid))
                .map(port_label)
                .collect();
            self.set_toast(format!("{} still running, try ⇧Q", labels.join(", ")));
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
        self.refresh();
    }

    pub fn toggle_cursor(&mut self) {
        let items = self.items();
        if let Some(label) = self.picker(self.tab).toggle_cursor(&items) {
            self.set_toast(format!("{label} is protected"));
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
                self.send(tab, targets, true);
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

    #[test]
    fn prune_reports_quits_past_the_timeout() {
        let mut p = Picker::default();
        p.pending.insert(1, Instant::now() - QUIT_TIMEOUT - Duration::from_secs(1));
        p.pending.insert(2, Instant::now());
        p.pending.insert(3, Instant::now());
        assert_eq!(p.prune(&HashSet::from([1, 2])), vec![1]);
        assert_eq!(p.pending.keys().copied().collect::<Vec<_>>(), vec![2]); // 1 timed out, 3 is gone
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
}
```

- [ ] **Step 2: Replace `src/ui.rs`**

```rust
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
```

- [ ] **Step 3: Run tests and check warnings**

Run: `cargo test -q && cargo build 2>&1 | grep -c warning`
Expected: `29 passed; 0 failed; 1 ignored`, then `0`.

Check that no other code calls the OS kill functions directly:

Run: `grep -n "ports::kill\|apps::quit\|apps::force_quit" src/*.rs`
Expected: matches only inside `fn send` in `src/model.rs`.

- [ ] **Step 4: Commit**

```bash
git add src/model.rs src/ui.rs
git commit -m "feat: Ports tab with shared selection and one quit gate"
```

---

### Task 3: Manual verification

**Files:** none. These steps need a real terminal and are for the user, not an agent.

- [ ] **Step 1: Install**

Run: `cargo install --path .`, then start `killer` in a new terminal tab.

- [ ] **Step 2: Stop a server**

In another tab run `python3 -m http.server 8765`. In `killer` press `tab`, then `/`, type `8765`, press `⏎` to leave the filter, then `⏎` again.
Expected: the row reads `:8765  http.server 8765` with the folder you started it in, the toast says `✓ Stopping :8765 http.server 8765`, the server exits, and the row disappears within 2 seconds.

- [ ] **Step 3: Kill a server that ignores SIGTERM**

Run `python3 -c "import signal,socket; signal.signal(signal.SIGTERM, signal.SIG_IGN); s=socket.socket(); s.bind(('127.0.0.1',8766)); s.listen(); input()"`. In `killer`, move to `:8766` and press `⏎`.
Expected: after 10–12 seconds the toast says `:8766 … still running, try ⇧Q`. Press `⇧Q`: the modal says `Kill 1 process? They stop immediately.` Press `y`: the process exits.

- [ ] **Step 4: Protection**

Expected: `:5000` (ControlCenter) shows `system`, OrbStack or Docker rows show `docker`, and `space` on them toasts `… is protected`. `a` then `⏎` stops only the unlocked rows.
