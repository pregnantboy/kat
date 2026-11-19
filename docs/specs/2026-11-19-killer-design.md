# killer: design

A macOS terminal UI for quitting several apps at once, modelled on the features of QuitAll (amicoapps.com/app/quitall).

## Scope

In v1:

- List running apps, with a toggle that also shows background apps (menu bar apps and helpers)
- Select one, several, or all apps
- Quit politely, so apps can still ask to save
- Force quit, after a confirmation prompt
- Exclusion list that "quit all" and every other quit path respects
- Memory use per app, sorted largest first
- Type-to-filter

Out of v1:

- Auto Quit (quitting apps after an idle period). It needs a background agent to be useful.
- Configurable key bindings. Keys are hardcoded.
- App icons. Most terminals can't show images.
- Linux and Windows support.

## Stack

- Rust, single binary
- Ratatui 0.30 for the UI, started with `ratatui::run` so the terminal is restored on exit and on panic
- `objc2-app-kit` for `NSWorkspace` and `NSRunningApplication`
- `libc` for `proc_pid_rusage` (memory)

## Files

Five files: `apps.rs` (AppKit), `config.rs` (exclusions), `model.rs` (state, keys, and the pure logic that tests cover), `ui.rs` (rendering), `main.rs` (the event loop).

### `src/apps.rs`

The only file that calls AppKit or `libc`.

- `list() -> Vec<App>`: reads `NSWorkspace.sharedWorkspace.runningApplications`. Before reading, it runs the current run loop for about 10ms, because a command-line process has no Cocoa run loop and the list goes stale otherwise.
- `App` fields: `name`, `bundle_id`, `pid`, `kind`, `memory_bytes`.
- `kind` comes from `activationPolicy`: `Regular` (Dock app), `Accessory` (menu bar app), or `Prohibited` (helper).
- `memory_bytes` is `ri_phys_footprint` from `proc_pid_rusage`, the same number Activity Monitor shows as "Memory". It is `None` when the call fails, for example for another user's process.
- `quit(pid) -> bool` calls `terminate()`. `force_quit(pid) -> bool` calls `forceTerminate()`. Both return as soon as the request is sent, not when the app exits.
- `frontmost_pid()`: used once at startup to find the launching terminal.

### `src/config.rs`

Reads `~/.config/killer/exclude`, a plain text file with one bundle ID per line. Blank lines and lines starting with `#` are ignored. Entries in the file are added to the defaults. If the file is missing, only the defaults apply. If it can't be read, the defaults apply and the UI shows a toast. Default exclusions: `com.apple.finder`.

### `src/model.rs` and `src/ui.rs`

`model.rs` holds the state, key handling, and the pure functions listed under Testing. `ui.rs` draws a frame from the state. `main.rs` runs the loop.

State: the app rows, selected pids, cursor, filter text, the background toggle, sort mode (memory or name), pending quits with their start time, the current toast, and whether the force-quit modal is open.

## Screen

```
╭─ killer ──────────────────────────────────────────────────────╮
│  / sla▏                                    3 selected · 2.4 GB │
├────────────────────────────────────────────────────────────────┤
│ ● Slack              ████████▌         1.3 GB                  │
│▌○ Safari             ██████            1.0 GB                  │
│ ● Preview            █                 112 MB                  │
│ ● Spotify            ▌                  64 MB   menu bar       │
│ ○ 1Password          ▌                  41 MB   menu bar       │
│ 🔒 Finder                               88 MB   excluded       │
├────────────────────────────────────────────────────────────────┤
│ ✓ Quit Preview · Slack is asking to save                       │
╰─ space select  a all  ⏎ quit  ⇧Q force  b background  ? help ─╯
```

- `●` selected, `○` not selected. The cursor row is drawn in reverse video, so it stays readable in any terminal theme.
- Excluded rows are dimmed, show a lock, and can't be selected.
- Background apps show a grey `menu bar` or `helper` badge and appear only when the background toggle is on.
- Memory bars scale to the largest visible app.
- The header shows the count and total memory of the selected apps.
- The toast line shows the result of the last action and clears after 3 seconds.
- Force quit opens a centered modal: `Force quit 3 apps? Unsaved work is lost.`, then the app names, then `y force quit   any other key cancels`. The count uses `1 app` / `N apps`.
- Colors come from the terminal's 16 ANSI colors, so the UI follows the terminal's light or dark theme.

## Keys

| Key | Action |
|---|---|
| `↑` `↓` / `k` `j` | Move cursor |
| `space` | Toggle selection on the cursor row |
| `a` | Select all visible, quittable apps. Press again to clear |
| `⏎` | Quit the selection, or the cursor row if nothing is selected |
| `Q` | Force quit the same set, after confirmation |
| `/` | Start filtering by name. `esc` clears the filter |
| `b` | Toggle background apps |
| `s` | Toggle sort between memory and name |
| `?` | Show the key list |
| `esc` / `ctrl-c` | Exit (when no filter or modal is active) |

## Behaviour

1. The loop polls for key events every 250ms.
2. The app list refreshes every 2 seconds and right after any quit. Selections are kept by pid across refreshes. A selected pid that no longer exists is dropped.
3. After a quit, the targeted rows show `quitting…`. Apps that exit disappear on the next refresh.
4. If an app is still running 10 seconds after a polite quit, the toast says `<name> still open (save dialog?)`. The row goes back to normal so it can be force quit.
5. If `quit` or `force_quit` returns false, the toast says `Couldn't quit <name>` and the list refreshes.

## Safety

- The app that is frontmost when `killer` starts is treated as the launching terminal and is always excluded.
- Background apps (menu bar or helper) whose bundle ID starts with `com.apple.` or is empty are always protected. macOS reports `loginwindow`, Dock, WindowManager, and SystemUIServer as menu bar apps, and quitting `loginwindow` logs the user out. These rows show a `system` badge.
- Every quit path gets its targets from one function, `quit_targets`, which removes excluded apps and the launching terminal. No key handler calls `apps::quit` or `apps::force_quit` with a pid that didn't come from `quit_targets`.
- Force quit always asks for confirmation.

Known limit: if `killer` is started while another app is frontmost (for example from a script), that app is protected instead of the terminal. Walking the process tree from `getppid()` would fix this.

## Testing

Unit tests in `#[cfg(test)]` modules in `src/config.rs`, `src/model.rs`, and `src/ui.rs`, run with `cargo test`, using hand-built `Vec<App>` values:

- `quit_targets` never returns an excluded app, the launching terminal, or a protected system process, including after "select all"
- Filtering is case-insensitive and matches anywhere in the name
- Sorting by memory puts apps with unknown memory last
- Selections survive a refresh and drop pids that are gone
- Background apps are hidden unless the toggle is on

Manual checks for the AppKit layer:

1. Open TextEdit, type something, don't save. Select it and press `⏎`. TextEdit asks to save.
2. Cancel the save dialog. After 10 seconds the "still open" toast appears.
3. Press `Q`, confirm with `y`. TextEdit closes without asking.
4. Press `a` then `⏎`. The terminal running `killer` and Finder stay open.
