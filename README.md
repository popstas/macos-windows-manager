# macos-windows-manager

A claude-wt window tracker for macOS. It watches terminal windows, binds their
titles to sessions, and puts the resulting list where the `ccfzf` aggregator will
read it — on the session machine and, if this machine runs an aggregator of its
own, next to itself.

The app has no windows of its own — only a status-bar icon.

*[Русская версия](README_ru.md)*

## Features

- **Binds windows to sessions** — matches terminal window titles against the
  `ccfzf` session dump, so the picker knows where each session is open.
- **Tile and cascade layouts** — from the tray menu or on request from the
  picker, with the order the human sees in the list.
- **Remembers window places** — a reopened window asks for the spot it had.
- **Layout snapshots** — the same list `^S` shows in the picker.
- **Raises windows on request** — Enter in the picker brings the right window
  forward, over MQTT.
- **Handles minimized windows** — published to the reader, optionally dropped
  from layouts.
- **Settings window** — every config key editable in a form, plus a Log tab
  showing what the tracker has been complaining about.
- **Works with two aggregators** — a remote one over ssh and a local one, at the
  same time.

## Requirements

- macOS with the **Accessibility** permission granted (see [Permissions](#permissions))
- Rust toolchain (`rustup`) for building
- Optional: an MQTT broker, for layout requests and window raising

## Install

There are no prebuilt binaries yet — the app is built and deployed from source.

```sh
cargo test -p mwm-core       # logic, runs on any machine
cargo build --release        # the app, macOS only
```

To deploy to a Mac over ssh:

```sh
MWM_HOST=<ssh host> ./data/scripts/deploy-mac.sh
MWM_HOST=<ssh host> ./data/scripts/deploy-mac.sh --no-build   # restart only
```

One-off setup on that Mac: `rustup`, the Accessibility permission, a LaunchAgent
([docs/launchagent.md](docs/launchagent.md)), and a Code Signing certificate —
without one, the Accessibility permission is lost on every rebuild.

→ Build details, the tray build stamp, and the full deploy procedure:
[docs/deploy.md](docs/deploy.md).

## Permissions

**Accessibility** is required. Without it, windows are not enumerated and the
windows file is not written at all — an empty file would mean "no windows" and
would clear the marks other machines' trackers had set.

The `Grant Accessibility…` tray item appears only while the permission is
missing, and disappears on its own once it is granted; the tracker asks the
system every tick, no restart needed.

On macOS 26, an MQTT broker on the LAN needs a second permission: **Local
Network**. Without it `focus` stays false on a perfectly healthy broker — a
failure mode worth reading about before debugging it:
[docs/lessons-learned.md](docs/lessons-learned.md#focus-stays-false-on-a-live-broker).

## Tray menu

```
3 windows tracked, 1 minimized
▦ Tile windows
❐ Cascade windows
✓ Ignore minimized windows
  claude-wt · WezTerm
  ccfzf-picker · Ghostty
  mwm · WezTerm
```

The top line is the result of the last tick, with a complaint appended when
publishing or placement failed. Below it are the layouts, the "ignore minimized"
checkbox, and one disabled item per bound window (`title · terminal`).

→ What each part means, and why the glyphs are what they are:
[docs/tray.md](docs/tray.md).

## Layouts

- **Tile** — a grid with no overlap. Window width is kept between 80 and 120
  columns, and the column count is decided by the screen rather than by the
  number of windows, so the work area is always filled.
- **Cascade** — a stack of half-width windows stepped 50 points right and down.

Both take the bound windows in the order shown in the tray menu, and raise them
afterwards so the layout is actually visible.

**There is no global hotkey here** — it belongs to
[ccfzf-picker](https://github.com/popstas/ccfzf-picker), which is the side that
knows the window order the human sees. The request arrives here over MQTT as a
`claude-place` message with `{"mode": "tile"|"cascade", "ids": [...]}`.

→ Grid maths, minimized-window handling, and the request format:
[docs/layouts.md](docs/layouts.md).

## Settings

`Settings` in the tray menu opens a config editor with four tabs — **Features**,
**Connection**, **Windows**, **MQTT** — plus a **Log** tab with the last thousand
lines the tracker wrote to stderr, timestamped.

Everything except the broker fields takes effect on the next tick; editing
`config.yaml` by hand is picked up the same way. Saving rewrites the file and
does not preserve comments, keeping the previous version as `config.yaml.bak`.

→ Empty-field semantics, what each toggle actually switches off, and the saving
rules: [docs/settings.md](docs/settings.md).

## Configuration

The config lives at `~/.config/macos-windows-manager/config.yaml`. Every key is
documented in [`config.example.yml`](config.example.yml).

The one that bites: `mqtt.base` is this machine's **full** topic prefix,
including the `/windows` suffix — unlike `mqtt.base` in the picker's config,
which is the bare machine prefix. Copying one into the other subscribes the Mac
to someone else's topics.

State files (`state.json`, `snapshots.json`) live under
`~/.local/state/macos-windows-manager/` and are configurable through the
`state:` block.

→ MQTT setup, state files, and how to verify a deploy arrived:
[docs/deploy.md](docs/deploy.md).

## Documentation

- [docs/tray.md](docs/tray.md) — the tray menu, item by item
- [docs/layouts.md](docs/layouts.md) — tile, cascade, minimized windows, MQTT requests
- [docs/settings.md](docs/settings.md) — the settings window and config writing
- [docs/deploy.md](docs/deploy.md) — build, deploy, permissions, MQTT, state files
- [docs/launchagent.md](docs/launchagent.md) — the LaunchAgent and its logs
- [docs/lessons-learned.md](docs/lessons-learned.md) — failures already paid for, and what they cost
- [docs/changelog.md](docs/changelog.md) — how the changelog is generated

## Changelog

See [CHANGELOG.md](CHANGELOG.md).

## Related

- [ccfzf-picker](https://github.com/popstas/ccfzf-picker) — the session picker
  this tracker feeds, and the side that owns the layout hotkey.
