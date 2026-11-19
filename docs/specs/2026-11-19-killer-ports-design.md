# kat: Ports tab design

Adds a second tab to `kat` that lists listening TCP ports, names what's running on each one in a form a person recognises, and stops or kills the owning process. Builds on `docs/specs/2026-11-19-killer-design.md`. Everything there still applies unless this file says otherwise.

## Scope

In:

- A Ports tab next to the Apps tab, switched with `tab`
- Listening TCP ports only. IPv4 and IPv6 listeners for the same pid and port are one row
- A readable name, project folder, process name, and pid for each port
- Stop (`SIGTERM`) and kill (`SIGKILL`, after confirmation), with the same protection model as apps

Out:

- UDP and established connections
- Ports owned by root or another user. macOS doesn't reveal their owner without `sudo`, so they aren't listed
- Stopping individual Docker containers

## Data

New file `src/ports.rs`. Like `apps.rs`, it may call the OS and contain `unsafe`.

- `Port { port: u16, pid: i32, uid: u32, exe: String, args: Vec<String>, cwd: Option<String> }`
- `list() -> Vec<Port>`: walks every pid with the `libproc` crate, keeps TCP sockets in the LISTEN state, and reads the executable path (`proc_pidpath`), arguments (`sysctl KERN_PROCARGS2`), and working directory (`proc_pidinfo PROC_PIDVNODEPATHINFO`). Sorted by port, then pid, with duplicates removed. A full scan takes about 10ms. It runs at startup and when you press `r`.
- `kill(pid, force) -> bool`: `SIGTERM`, or `SIGKILL` when `force` is true. Returns whether the signal was delivered.

New dependency: `libproc` 0.15. `libc` has no socket-info structs; `lsof` takes about 320ms per call, which would stall key handling.

## Names

`name(&Port)` is pure and tested. In order:

1. If the first argument's file name is an interpreter (`node`, `python`, `ruby`, `bun`, `deno`, `java`, `php`, `perl`, matched case-insensitively and including versioned names like `python3.12`), the name is the file name of the first non-flag argument, plus the next non-flag argument if it isn't a path. `node .../node_modules/.bin/next dev` is `next dev`. `python3 manage.py runserver` is `manage.py runserver`. `python3 -m http.server 8765` is `http.server 8765`. Interpreters are checked first because Xcode's and Homebrew's `python3` re-execute as `.../Python.app/Contents/MacOS/Python`.
2. If the executable path contains an `X.app` component, the name is `X` (the outermost one). `.../OrbStack.app/.../OrbStack Helper.app/...` is `OrbStack`.
3. Otherwise the name is the file name of the first argument. Processes that rename themselves show through: Next's server is `next-server (v16.4.1)`.
4. With no arguments, the executable's file name.

`project(&Port, home)` is the working directory with the home folder shown as `~`, or nothing when it is `/` or unknown. The UI shortens it from the left to fit.

## Protection

`protection(&Port, my_uid, terminal_pid, self_pid) -> Option<&'static str>` returns the badge text for a protected row, or `None` if it can be stopped:

| Badge | Rule |
|---|---|
| `this terminal` | pid is the launching terminal |
| `kat` | pid is `kat` itself |
| `other user` | uid is not the current user |
| `docker` | executable path contains `OrbStack` or `Docker`. These own every forwarded container port, and killing them stops every container |
| `system` | executable is under `/System/` or `/usr/`, except `/usr/local/` |

Protected rows are dimmed, show a lock, and can't be selected. They are hidden by default; `h` shows or hides them, and the header shows `N locked hidden` or `locked shown`.

## Model changes

- A `Picker` struct holds one list's selection (pids), cursor, and pending stops. The Apps and Ports tabs each own one. The selection, toggle, select-all, cursor clamp, and pending-timeout logic now lives in `Picker`, used by both tabs.
- Both tabs turn their visible rows into `Item { pid, label, protected }`. One pure function, `pick_targets(items, selected, cursor_pid)`, picks what to act on: the visible selected items, or the cursor row if nothing is selected, minus protected items, one per pid. It replaces `quit_targets` and is the only source of pids for `apps::quit`, `apps::force_quit`, and `ports::kill`.
- Selection is by pid. Selecting one row of a process that holds several ports marks all its rows, because a stop ends the whole process.
- Each tab has its own cursor and selection. Switching tabs clears the filter.
- On the Ports tab, the filter matches the port number (with or without a leading `:`), the name, or the project. `s` does nothing there. `h` shows or hides locked ports, the same key that shows background apps on the Apps tab.
- Port rows are sorted by port number each time you press `r`. Nothing refreshes automatically, so rows only move when you ask.

## Screen

```
╭─ kat ─ Apps │ Ports ───────────────────────────────────────────╮
│  / 3000                                  1 selected · 4 ports     │
├───────────────────────────────────────────────────────────────────┤
│ ● :3000   next-server (v16.4.1)  ~/orca/…/apps/suite  node  58388 │
│▌○ :5158   server.mjs             ~/orca/…/apps/suite  node  55833 │
│ 🔒 :5432  OrbStack               docker                     73073 │
│ 🔒 :5000  ControlCenter          system                       747 │
├───────────────────────────────────────────────────────────────────┤
│ ✓ Stopping :3000 next-server (v16.4.1)                            │
╰─ tab apps/ports  space select  a all  ⏎ stop  ⇧Q kill  / filter ─╯
```

- The active tab is bold and cyan in the title. The other is dim.
- Columns: port, name, project (or the protection badge for protected rows), process name (dim), pid (dim).
- The header shows the number of selected visible rows and the total number of rows on the Ports tab.
- The footer and help list `tab`, `r`, and `h`. On the Ports tab the footer says `⏎ stop  ⇧Q kill` and leaves out `s`.

## Behaviour

1. `⏎` sends `SIGTERM` to each target. The toast says `✓ Stopping :3000 next-server (v16.4.1)`. Rows show `stopping…`, then `gone` (dimmed, can't be selected) once the process exits, until the next `r`.
2. If a process is still running 10s later, the toast says `<label> still running, try ⇧Q`.
3. `⇧Q` opens the confirm modal: `Kill 2 processes? They stop immediately.` (`1 process` for one), one label per line (at most 8, then `+N more`), then `y kill   any other key cancels`. `y` re-checks the targets through `pick_targets` and sends `SIGKILL` to what's left.
4. If a signal can't be delivered, the toast says `Couldn't stop <label>`.

## Testing

Unit tests:

- `name` on real command lines: an app helper, ControlCenter, `next-server (v16.4.1)`, `node .../server.mjs`, `node .../node_modules/.bin/next dev`, `python3 -m http.server 8765`, `codex --no-daemon ...`
- `project`: home shortening, `/` and unknown give nothing
- `protection`: each badge, and `/usr/local/bin/postgres` is not protected
- Port filter: `3000`, `:3000`, name, and project
- `pick_targets`: never returns protected or hidden items, falls back to the cursor, and returns one item per pid
- `Picker`: toggle, select-all, prune, stuck detection
- A TestBackend render of the Ports tab

Manual:

1. Run `python3 -m http.server 8765` in another tab. In `kat`, press `tab`, find `:8765`, and press `⏎`. The server exits, the row turns to dust and disappears.
2. Run `python3 -c "import signal,socket; signal.signal(signal.SIGTERM, signal.SIG_IGN); s=socket.socket(); s.bind(('127.0.0.1',8766)); s.listen(); input()"`. Press `⏎` on `:8766`. After 10–12s the toast says `still running, try ⇧Q`. Press `⇧Q`, then `y`. The process exits.
3. `:5000` (ControlCenter) and any OrbStack or Docker rows are dimmed with a lock and can't be selected.
