# Build and deploy

## Build and tests

```sh
cargo test -p mwm-core       # logic, runs on any machine
cargo build --release        # the app, macOS only
```

The crates are split on purpose. `mwm-core` knows nothing about macOS or Tauri:
its tests run anywhere, and all of the tracker's logic lives there. `src-tauri`
is the application and only builds on a Mac — keep logic inside it and there is
nowhere to verify it except the very machine it runs on.

## The build stamp

The last tray menu item is a build stamp: `v0.7.0 · 17:42` for today's build,
`v0.7.0 · 2026-08-13 17:42` for an older one. It exists because deploying
replaces the binary in place while every build between releases carries the same
version — without the stamp there is no way to tell after a restart that the new
one came up.

Built with `MWM_RELEASE=1` it shows the version alone, without a time: a release
is named by its version.

## Deploy to a Mac

```sh
MWM_HOST=<ssh host> ./data/scripts/deploy-mac.sh
MWM_HOST=<ssh host> ./data/scripts/deploy-mac.sh --no-build   # restart only
```

One-off setup on the Mac: `rustup`, the Accessibility permission, a LaunchAgent
(see [launchagent.md](launchagent.md)), and a Code Signing certificate named in
`MWM_SIGN_ID` — see
[lessons-learned.md](lessons-learned.md#signing-and-permissions) for why an
unsigned binary loses its permissions on every build.

> `deploy-mac.sh` exits 0 even when the build on the Mac failed. Read the last
> line of its output — `deployed to <host>` means it arrived; output ending in
> `cargo` errors means it did not, whatever the exit code says.

## Permissions

Only one is required — Accessibility. Without it windows are not enumerated and
the file is not written at all: an empty file would mean "no windows" and would
clear the marks other machines' trackers had set.

On macOS 26 an MQTT broker on the LAN needs a second one: **Local Network**
(System Settings → Privacy & Security → Local Network). Without it the socket
never connects and `focus` stays false on a healthy broker — see
[lessons-learned.md](lessons-learned.md#focus-stays-false-on-a-live-broker).

## MQTT configuration

The tracker's config lives at `~/.config/macos-windows-manager/config.yaml`.
Its `mqtt:` block takes `host`, `port`, `user`, `password`, `base`. Without the
block the tracker still lists windows and writes the file, but does not
subscribe and does not advertise that it can raise a window.

```yaml
mqtt:
  host: broker.lan
  port: 8883
  user: picker
  password: secret
  base: home/room/mac/windows
```

`base` is this machine's topic prefix (the subscription is `<base>/#`), and it
**must differ between machines**: an identical `base` on the Mac and the Windows
box would collide their subscriptions, and a request addressed to one would be
executed by the other.

**The value does not match `mqtt.base` in the picker's config, and copying one
into the other is wrong.** In the picker, `base` is the bare machine prefix
(e.g. `home/room/mac`), to which the picker itself appends `/windows` when
talking to the tracker. Here, `base` is the full address including that suffix
and is used as-is. Copied over unchanged, it would subscribe the Mac to someone
else's prefix — the Mac would start hearing requests meant for the Windows
machine.

## State files

The tracker writes two files to disk, both under
`~/.local/state/macos-windows-manager/`:

- `state.json` — remembered window places, one record per session (the place a
  reopened window asks for).
- `snapshots.json` — layout snapshots: bound sessions with their coordinates at
  the moment of the snapshot, the same list `^S` shows in the picker.

Both are written with `fsync` and only when the contents actually changed — the
price is paid for durability, not per tick.

The directory is deliberately not next to `config.yaml`: the config is edited by
hand, these two are written by the machine, and putting them together invites
confusing one's backup for the other's working file — carrying off
`config.yaml.bak` as if it were a layout snapshot. Both paths are configurable
through the `state:` block in `config.example.yml`.

A machine with no terminals open publishes `windows: 0`, `snapshots: 0`, and
the state directory is not created at all. A missing state directory on a fresh
machine is not a fault.

## Verifying that it arrived

On the aggregator machine:

```sh
python3 -c "import json,time;o=json.load(open('$HOME/.ccfzf/windows/<host>.json'));print(time.time()-o['generated'],len(o['windows']))"
```

The first number under thirty means the tracker is publishing. The second is how
many windows it sees.

A silent tracker is almost always Accessibility, and it is told apart from a
dead one by three questions: `pgrep -x macos-windows-manager` (the process is
there), `stat -f %Sm /tmp/mwm.err.log` (the log has stopped), and the age of
`generated` in the windows file on the aggregator (not growing). All three
agreeing means go and grant Accessibility rather than read code.
