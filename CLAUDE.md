# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

`kat` is a macOS-only terminal UI (Rust, Ratatui 0.30) for quitting running apps and stopping processes that hold listening TCP ports. Design specs live in `docs/specs/`, implementation plans in `docs/plans/`. The specs describe current behaviour and are kept in sync with the code.

## Commands

If `cargo` isn't found, run `. "$HOME/.cargo/env"` first.

- Test everything: `cargo test -q`
- One test: `cargo test -q <name>`, e.g. `cargo test -q pick_targets`
- Live port test (spawns `python3 -m http.server 8765`, finds it, stops it): `cargo test -q live -- --ignored`
- Warnings must stay at zero: `cargo build 2>&1 | grep -c warning`
- Install the binary to `~/.cargo/bin/kat`: `cargo install --path .`

Don't run `cargo run` or the binary from an agent shell. It needs a real TTY, and it quits apps and kills processes on this Mac. To look at it, open it in a new iTerm2 window: `osascript -e 'tell application "iTerm2" to create window with default profile command "<path to binary>"'`. Agent shells can't screenshot (no Screen Recording permission), so ask the user what they see.

## Architecture

**OS boundary.** Only `src/apps.rs` (AppKit via objc2, `proc_pid_rusage`, icon rasterising) and `src/ports.rs` (socket scan via the `libproc` crate, `KERN_PROCARGS2`, cwd, `kill`) call the OS or contain `unsafe`. Everything else is pure and testable. `apps::list()` pumps the Cocoa run loop for ~10ms first, because a CLI has no run loop and `NSWorkspace` would otherwise return a stale app list.

**One quit gate.** Each tab turns its visible rows into `model::Item { pid, label, protected }`. `model::pick_targets` is the only source of pids passed to `apps::quit`, `apps::force_quit`, or `ports::kill`, and the only callers of those are in `Model::send`. It drops protected rows, rows hidden by the filter or the `h` toggle, and duplicate pids. The force-kill modal re-runs the gate when `y` is pressed. Keep this invariant when adding actions.

**Protection.** Apps: the excluded bundle IDs (Finder plus `~/.config/kat/exclude`, case-insensitive), the launching terminal, and background apps with a `com.apple.*` or empty bundle ID (`loginwindow` is one, and quitting it logs the user out). Ports: `ports::protection` returns a badge (`this terminal`, `kat`, `other user`, `docker`, `system`). OrbStack and Docker own every forwarded container port, so they are protected. The launching terminal is whichever app is frontmost when `main` starts, read before the UI takes over.

**State.** `Model` holds both tabs. Selection, cursor, pending quits, and `gone` pids live in a `Picker` per tab (`app_pick`, `port_pick`). Lists refresh at startup, on `r`, and once a quit process's dust animation has finished. Every refresh keeps the cursor on the same pid (`follow_cursor`), so it can't land on a different process, and drops apps that fail `ports::alive` because NSWorkspace lists them briefly after exit. App refreshes merge into the existing order: only the first load and `s` sort. `tick` checks the pending pids with `ports::alive` (signal 0): dead ones become `gone` (dust, then removed by the refresh), and ones still alive after 10s produce a "still open / try ⇧Q" toast.

**Testing pattern.** `refresh()` calls the OS. Tests use `apply_refresh(Vec<App>)` and `apply_ports(Vec<Port>)` with hand-built data, and inject liveness into `Picker::check` with a closure. No test may send a real signal: use pids that aren't in the model's rows (e.g. 999999), so the gate drops them before `send`. UI tests render into ratatui's `TestBackend`.

**Icons.** In iTerm2 or WezTerm (and not inside tmux; `KAT_ICONS=0` turns them off), `ui::draw` reserves 3 columns per row and records `icon_slots`. After each frame, `main::run` writes iTerm2 inline-image escapes (`icons::draw`) at those cells. ratatui only rewrites changed cells, and a rewritten cell erases the image under it. So every icon is redrawn whenever the frame hash changes, and the screen is cleared and fully redrawn when a cell that had an icon no longer gets one. The kitten's cells (`m.kitten_area`) are left out of that hash, because it animates on its own. The slot x-offset assumes the always-reserved highlight column plus the 2-cell marker.
