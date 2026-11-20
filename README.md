<p align="center"><img src="site/kitten.svg" width="112" alt="kat's pixel kitten"></p>

<h1 align="center">kat</h1>

<p align="center"><b>Quit the app<br>Pet the kat</b><br>
A terminal app for macOS that quits your apps and frees your ports.<br>
<a href="https://kat.pboy.dev">kat.pboy.dev</a></p>

```
╭─ Apps │ Ports ─────────────────────────────────────────────╮
│ / filter                   2 selected · 2.5 GB · by memory │
│ ────────────────────────────────────────────────────────── │
│ ● Slack           ██████████   1.4 GB                      │
│ ● Google Chrome   ███████▊     1.1 GB                      │
│ ○ Spotify         ███▋         512 MB                      │
│ ○ zoom.us         ██▍          344 MB                      │
│ 🔒iTerm2          ██           290 MB  this terminal       │
╰─ tab ports  space select  a all  ⏎ quit  ⇧Q force  ? help ─╯
```

kat lists what's running and what's listening on a port. Pick what you're done with, press ⏎, and a small ginger kitten swats it away. The row breaks into dust when the process actually exits.

## Install

```sh
brew install pregnantboy/tap/kat
```

Update with `brew upgrade`. Builds are for Apple silicon and Intel Macs.

## Use

Run `kat`. It opens on your apps. Press `tab` for ports.

| Key | Apps | Ports |
|---|---|---|
| `↑` `↓` or `j` `k` | move | move |
| `space` | select | select |
| `a` | select all | select all |
| `⏎` | quit politely | stop (SIGTERM) |
| `⇧Q` | force quit, after a `y` | kill (SIGKILL), after a `y` |
| `/` | filter | filter |
| `h` | show background apps | show locked ports |
| `s` | sort by memory or name | |
| `r` | refresh | refresh |
| `?` | help | help |
| `esc` | clear filter, then exit | clear filter, then exit |

Apps get a polite quit first, so they can still ask to save. If one is still open after 10 seconds, kat tells you, which usually means a save dialog is waiting.

Memory is the same number Activity Monitor shows. The Ports tab shows each listening port, the folder the process was started from, and its pid, so you can stop `next dev` on `:3000` without hunting through `lsof`.

## What stays put

Some rows are locked 🔒 and nothing in kat will quit them:

- Finder
- the terminal you started kat from
- macOS background apps like `loginwindow` (quitting it logs you out)
- ports owned by Docker or OrbStack, by another user, or by the system

To lock more apps, put one bundle ID per line in `~/.config/kat/exclude`:

```sh
# ~/.config/kat/exclude
com.1password.1password
com.apple.Music
```

## Icons

In iTerm2 and WezTerm, rows show the real app icon. They're off inside tmux. Set `KAT_ICONS=0` to turn them off anywhere.

## Build from source

```sh
git clone https://github.com/pregnantboy/kat
cd kat
cargo install --path .
```

macOS only, since kat talks to AppKit. Built with Rust and [Ratatui](https://ratatui.rs).
