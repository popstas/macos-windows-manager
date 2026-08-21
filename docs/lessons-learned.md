# Lessons already paid for

Each of these cost a deploy, a silent failure, or a round of investigation.
They are written down so the next round is cheaper.

## Tracking and titles

**A terminal missing from `terminals` is invisible to the tracker — silently.**
`ax::list_windows` walks applications, not windows: `runningApplications()`, and
anything whose `bundleIdentifier` does not match the list is skipped before the
question of a title comes up. Such a terminal's windows land neither in the file
nor in the log — the session in the picker simply has no window mark, and
nothing distinguishes that from a dead tracker. That is how WezTerm went
missing: the picker moved to it before the list learned its bundle id
(`com.github.wez.wezterm`).

The default lives in one place — `default_terminals()` in
`crates/mwm-core/src/config.rs` — with a test next to it that spells the list
out verbatim; checking a default against itself would prove nothing. A list from
the config **replaces** the default rather than extending it: anyone with
`terminals` already written by hand has to add a new terminal there too.

**A window title is not always the pane title: WezTerm prepends its own.** The
default `format-window-title` builds `[Z] [i/n] <pane>`: `[i/n]` appears from
the second tab, `[Z]` on a zoomed pane. Binding is an exact comparison against
the session name from the dump, and the prefix breaks it entirely — the window
stops being found exactly when the human opens a second tab.

`strip_window_prefix` in `title.rs` removes it, and removes **exactly those two
forms**, not any leading bracket: `[wip] fix parser` is a legitimate session
name, and stripping it would pull the two sides of the comparison apart where
they used to meet. The space after the bracket is mandatory for the same reason.
The order is prefix first, then the Claude Code marker: the marker belongs to
the pane title, i.e. it stands after the prefix.

**Without the permission the tracker does not publish at all — verified.** Not
an empty file, but no file: an empty one would mean "no windows" and would clear
marks set by other machines' trackers.

## Signing and permissions

**An unsigned binary loses its Accessibility permission on every build.** The
linker signs it ad-hoc (`Signature=adhoc, linker-signed` — on Apple Silicon
there is no other way), the signing requirement comes out as a hash of the
contents, a new build has a new hash, and macOS remembers the old one. It looks
like a silent refusal: the process is alive, the icon is in place, and the
windows file goes stale.

The cure is a stable signature. Once: create a self-signed Code Signing
certificate in Keychain Access, sign the binary once **in a terminal on the Mac
itself**, and answer **Always Allow** — not **Allow** — when asked about key
access. The buttons sit next to each other and do different things: the first
lets one call through, the second records the permission in the keychain.
"Allow" pressed by mistake looks like success — the signing goes through — and
the dialog comes back on the next build, where a deploy over ssh runs into it
silently.

More reliable: grant access through the UI. Keychain Access → the **Keys**
category (not Certificates) → the private key under the certificate → Get Info →
Access Control → **Allow all applications to access this item**. After that
`deploy-mac.sh` signs by itself. The requirement becomes
`identifier "…" and certificate leaf = H"…"` — no content hash in it, so it
survives a rebuild.

**Signing over ssh is impossible, and it is not about privileges.**
`ssh mac codesign …` answers `errSecInternalComponent`, and `security` answers
"User interaction is not allowed". The reason: `launchctl managername` in an ssh
session is `Background`, and a background session has no access to the login
keychain at all. Neither a password nor `security set-key-partition-list`
changes that — tested, does not help. The `gui/<uid>` domain *is* reachable from
ssh, so signing is dispatched there as a one-off launchd job and runs in a
session where the keychain is open.

**Wait for the signature by result, not by the clock.** `launchctl bootstrap`
returns immediately while signing takes about six seconds: a fixed `sleep 3`
tore the job down halfway, the deploy ran to completion and reported success,
and the binary stayed linker-signed.

**What you wait for is a human at the screen, not a machine.** Until key access
is granted for all applications, the keychain raises a dialog on every signature
and `codesign` waits in it as long as it takes. With fifteen wait rounds this
became a race the human lost three times in a row: the password went in after
the loop had expired and the script had torn the job down with `bootout` — the
confirmation fell into nothing and the deploy reported that signing had failed.
From the outside that is indistinguishable from "there is no certificate at
all", and the two cases were told apart only by time: when `SecurityAgent`
started (`pgrep -fl SecurityAgent`) against the last write to
`/tmp/mwm-sign.log`, to the second. There are forty-five rounds now; the loop
exits on success, so a healthy deploy pays nothing for them. The permanent fix
is not more waiting but the key's Access Control.

**A certificate is a property of the machine, not of the project.** Nobody put
one on the second Mac, and `security find-identity -v -p codesigning` answers
"0 valid identities found" there. Nothing to sign with, the permission drops on
every rebuild, and the tracker afterwards looks alive and says nothing. Check
before deploying: on a new machine the first thing to create is a Code Signing
certificate with the name in `MWM_SIGN_ID`.

**Signing at deploy time does preserve granted permissions — where there is
something to sign with.** On the machine that has the certificate, after a
rebuild and restart the tracker brought the broker connection straight up
(`focus: true` in the published windows file within a minute of start) and kept
publishing; neither Local Network nor Accessibility had to be granted again.
`mqtt connection lost: No route to host` lines in `/tmp/mwm.err.log` at restart
are attempts made before the connection came up, not a permission refusal — tell
them apart by the last write time in the log and by `focus` in a fresh windows
file.

## Publishing and MQTT

**`focus` in the windows file is advertised from a live connection, not from a
constructed client.** `Client::new` in rumqttc returns a client immediately,
before any network; only a `ConnAck` from the broker makes the connection live,
and only after it does `is_live()` in `src-tauri/src/mqtt.rs` start answering
yes. Advertising the ability to raise a window earlier would hand the human a
silent Enter in the picker — worse than an open terminal.

### `focus` stays false on a live broker

This is the *second* permission, not the first. macOS 26 gave applications a
separate Local Network toggle (System Settings → Privacy & Security → Local
Network) and without it will not let a socket through to a machine on the same
network — brand new, sitting next to the long-familiar Accessibility.

The symptom in stderr is `mqtt connection lost: I/O: No route to host (os error
65)`, once per `RETRY`, on a live broker with correct credentials:
`mosquitto_sub` with the same login and address connects, `nc -z <broker> 1883`
says "succeeded", the name resolves to a single IPv4 address.

The usual checks lie here not because someone made a mistake but because they go
out through a different binary. Both the session dump (`dump.rs`) and the
windows file delivery (`deliver.rs`) have always reached the network through
`ssh` launched as a separate process — and that process has its own TCC record,
where `/usr/bin/ssh` already holds the permission. A probe using
`/usr/bin/python3` proves no more, even launched from the same `gui/<uid>`
launchd domain the tracker lives in: that is another system binary with someone
else's TCC record.

`rumqttc` in `mqtt.rs` is the first socket the tracker opens **itself**, and the
first thing that ever needed this permission. Like Accessibility, it is held by
the binary's signature: on a linker-signed binary it will drop on the very next
build.

**Without a log on that side, the cause is invisible.** A launchd job without
`StandardErrorPath` drops the tracker's whole stderr into nothing — not a line
in `Console.app` or `log show` — and a connection refusal looks like the absence
of a cause. Setting up both paths: [launchagent.md](launchagent.md).

## Raising windows

**`AXRaise` alone is not enough — the window rises but does not come forward.**
`AXRaise` moves a window inside its own application; among other apps' windows
it would still be behind. Bringing the application forward is AppKit's job, a
separate call in `raise()` (`src-tauri/src/ax.rs`). The foreground-permission
dance the whole Windows branch of `ccfzf-picker` is built around is not needed
here at all — on macOS that question is settled by the Accessibility permission,
granted once by a human.

**`NSRunningApplication` in `objc2-app-kit` 0.3.2 has no `activate()`.** Only
`activateWithOptions(NSApplicationActivationOptions) -> bool`, and it is
declared safe — `unsafe` around it and around
`runningApplicationWithProcessIdentifier` is redundant in that version, both are
safe by signature. This could not be learned on the development machine: code
under `#[cfg(target_os = "macos")]` does not compile there at all, `cargo` builds
the non-macOS branch, and until it was edited on the Mac everything looked
finished.

**`ActivateIgnoringOtherApps` has been a no-op since macOS 14.** The flag emits
a build warning ("will have no effect"); an empty option set gives the same
result and is what `raise()` uses. `ActivateAllWindows` is not a substitute in
principle: it would bring every window of the terminal forward and undo the
`AXRaise` a line above, which raised exactly the one that was wanted.

**`activateWithOptions` returns `bool`, and `false` there is a silent refusal.**
Without checking that return, `raise()` would report success while the human saw
nothing — the same family of bug as `SetForegroundWindow` on Windows, except the
price here is a return check rather than a permission dance.

**Unverifiable branches are checked against dependency sources, not against
memory.** In both previous stages the macOS branch of `ax.rs` failed to build on
the first try (`CFType::from` in the first, `NSRunningApplication::activate` in
the second) — each time a round trip of deploy → compiler error → fix, and there
is no other way: code under `#[cfg(target_os = "macos")]` does not compile on the
development machine, so the error is only visible after deploying to a Mac. This
time every platform call was grepped against the dependency sources in
`~/.cargo/registry/src/` before deploying, and the check found two errors before
the build — the branch compiled on the first try.

What the check found, verbatim: `accessibility` 0.2.0 has no
`AXAttribute::position()`, no `AXAttribute::size()`, and no `AXValue` type at
all. Only the `AXAttribute::<CFType>::new(&CFString)` form works, plus raw
`AXValueGetType` / `AXValueGetValue` / `AXValueCreate` from `accessibility-sys`.
The settability check is called `is_settable`, not `is_attribute_settable`.

**`CGDisplay::bounds()` lives in the same coordinate space as Accessibility** —
origin top-left ("global display coordinate space" in the crate's own doc
comment). No vertical flip is needed. `NSScreen` counts from the bottom left and
so does not fit; it would also require the main thread, while the tracker's tick
runs in a worker.

**`core-graphics` is declared a macOS-only dependency** and resolves without a
version conflict against the existing `core-foundation`.

## launchd and deploy

**`launchctl kickstart -k` leaves a ghost in the status bar.** It removes the
process with SIGKILL, and the app has things to clean up: the killed instance
leaves its icon hanging, and after a deploy you can see two — a live one with a
menu and a dead one without. Restart goes through `bootout` + `bootstrap`:
launchd sends SIGTERM and waits for the exit.

**`launchctl stop` does not stop a job with `KeepAlive`.** It is brought right
back up, and `--no-launch` would have reported success with the tracker running.
Only `bootout` stops it. For the same reason the deploy does not ask "is the job
loaded?": that is the state it changes itself, and the answer would depend on
the previous run of the script. It asks about the plist file instead.

**`launchctl bootstrap` loads a job but does not always start it.** `RunAtLoad`
is in the plist, yet `launchctl print` shows `state = not running` and
`active count = 0`. In the first stage this was blamed on a fresh install — it
turns out to happen on ordinary redeploys too, and then the machine is left with
no tracker at all: `bootout` has already removed the previous instance and there
is no new one. The cure is `launchctl kickstart` **without** `-k` — it starts a
loaded job and does not get in the way if it is already running. Without `-k`
specifically: that one kills with SIGKILL and leaves a dead instance's icon in
the status bar.

Both jobs need the `kickstart` chase, not just the tracker: the one-off signing
job is loaded by the same `bootstrap` and can fail to start in the same way. Its
symptom is its own and unambiguous: `/tmp/mwm-sign.log` does not appear at all —
and it is declared as both `StandardOutPath` and `StandardErrorPath`, so it
would be created even by a job that died instantly. No file means `codesign`
never ran, and the wait loop is honestly counting its rounds for nothing.

**The deploy used to abort silently — on the wait for the signature.** The wait
loop ends either with a signature or with its fifteenth round, and in the second
case its status is the failure of the last `grep`. Under `set -e` that killed
the script three lines before the check written for exactly this case ("a
refusal does not stop the deploy, but it must not be silent") — without a word,
with exit code 1, never reaching the restart. The machine stayed on the old
binary and looked deployed: `git pull` went through, the build went through, the
new binary is on disk, and the old one is running. It cost a full round of
investigation, after a human failed to see a layout apply.

Two rules came out of it. First: `|| true` on the wait loop — fixed. Second, for
reading the output: **look at the last line, not at the pipe's exit code.**
Reached `deployed to <host>` — it arrived; ended in `cargo` output — it did not,
whatever `echo $?` says after a `| tail`.

**`curl --proto =https` does not work under zsh.** zsh expands `=https` as a
path to a program by that name and answers `zsh:1: https not found`. The rustup
install command from the website is written for bash; it needs quotes:
`--proto "=https"`. An install broken at that point leaves `~/.rustup` without
`~/.cargo`, and is only fixed by
`rustup toolchain install stable --force`.

**`~/.cargo/bin` may not be in the login shell's PATH.** `ssh host '$SHELL -lc …'`
reads dotfiles, but rustup does not always add itself there. The deploy failed
honestly on `cargo: command not found` — before `pkill`, so the previous
instance stayed alive — but it failed. Hence the explicit
`export PATH=$HOME/.cargo/bin:$PATH` around the build in `deploy-mac.sh`.

**`data/scripts/deploy-mac.sh` exits 0 even when the build on the Mac failed.**
The compile error is visible in the script's output, but the deploy's status is
success — the same class of trap as waiting by the clock instead of by result.
Fixing it is out of scope; it is simply a reason not to trust a single "the
deploy went through" and to look at the output.

## Packaging and build system

**A bare binary with no bundle is treated by macOS as a regular application**
and keeps its icon in the Dock — next to the one in the status bar, two places
for one tray. `ActivationPolicy::Accessory` in `setup` does what `LSUIElement`
in `Info.plist` would, which a bare binary does not have.

**Nothing in `src-tauri/icons/` is edited by hand.** `icon.png` is the output of
`python3 scripts/make-icon.py`, and an edit survives exactly until the next run
of the script. Same arrangement as in the neighbouring `ccfzf-picker`.

**The tray build stamp lies when a build input lives outside the package.** Any
`cargo:rerun-if-*` directive cancels the default "re-run the script on any
change in the package", and `tauri_build::build()` always prints its own. So a
deploy that touched only `crates/mwm-core` left `build.rs` un-rerun, and the
menu item showed the previous build's time — exactly the thing it is looked at
for, to confirm that the new one came up. The code itself was fresh: dependency
paths land in the crate's dep-info, only the stamp lied. Fixed by
`cargo:rerun-if-changed=../crates` and `../frontend`; the test is
`the_build_script_watches_what_lives_outside_the_package`. This cannot be caught
by behaviour: cargo decides about `build.rs` before anything of ours runs. The
same price was paid in the picker the same day
(`cargo:rerun-if-changed=../frontend`, commit `1a636ae1`).

## Two aggregators

**There can be two aggregators, and the tracker talks to both symmetrically.**
It has exactly two routes, and both used to lead to one machine: the session
index arrived over `ssh <sshHost> cat ~/.ccfzf.sessions.json`, and the windows
file left over ssh to `<sshHost>:~/.ccfzf/windows/`. A session running on *this*
machine never got a window out of that arrangement — and not because there was
nothing to identify it by. The title matches character for character, as the
tracker's own log shows:

```
seen 3 / bound 2;
  unbound:    "✳ Debug Kitty terminal link and clipboard issues";
  unresolved: "Debug Kitty terminal link and clipboard issues"
```

There was nothing to compare against: the local session's name is not in the
remote dump, and the windows file went where that session is unknown — so the
record of its window was left an orphan. Hence `features.localSource`: the index
is also read from the local dump (`~/.ccfzf.sessions.json`), and the windows
file is also written to the local `~/.ccfzf/windows/`.

**Two halves, one switch**, and that is not a saving on settings: separately
they are meaningless. The index without local publishing would bind a session
whose window nobody will read; publishing without the index would write a file
with no local sessions in it.

**A remote session wins a name collision.** The "more alive wins" rule used to
separate namesakes inside one dump does not fit here: `activityAt` comes from
hook files, and on a machine without hooks it is zero for everyone — the outcome
would be decided by whether hooks are installed, not by freshness. The stronger
argument: this is how the tracker behaved before the second source existed, and
the window of an ssh session whose name matches a local one binds exactly as it
did. The price is named — a local namesake session gets no window; the error in
the other direction would cost the picker a wrong ▣ and an Enter that raises the
wrong machine.

**A file is read, `ccfzf` is not run.** The aggregator binary on the Mac lives
only inside the picker's directory, and calling it from there would tie the
tracker to someone else's installation. The price is freshness: the dump is
rewritten by the picker's poller, and with the window hidden its tick stretches
to eight minutes — a just-opened local session waits that long to be bound. A
human opening the picker brings the tick back to a second, and the window
appears.

The default is on, and it costs a machine without its own aggregator nothing:
there is no dump there, so the index is empty, and the extra windows file has no
reader.

Incidentally this closes another hole: the windows file gives a session
liveness regardless of its transcript, so a local session with a window stays
alive even when nothing has been said in it for a while — past the aggregator's
two-hour `fresh_ids` cutoff.

## Open questions

Not settled by any deploy so far:

- Do `AXPosition` / `AXSize` yield for every one of the terminals?
- How long does "settled" actually take when a window is dragged with the mouse?
- `CGDisplay::bounds()` returns the full screen rectangle rather than the work
  area — what happens to a window pressed against the top edge (does it go under
  the menu bar?) was never checked.
