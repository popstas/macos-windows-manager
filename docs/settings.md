# Settings window

`Settings` in the tray menu opens a config editor with four groups:

- **Features** — toggles for window placement, layout snapshots, MQTT requests
  and laying out minimized windows. All on by default, all limited to terminal
  windows.
- **Connection** — `sshHost`, `remoteDir`, `windowHost`.
- **Windows** — the `terminals` list (one bundle id per line), `tickMs`,
  `dumpCacheMs`.
- **MQTT** — host, port, user, password and the topic prefix.

Every key is documented in `config.example.yml`.

## Empty fields

An empty field means "not stated in the file", not "empty": below such a field
sits an `in effect:` hint with the value working in its place. A filled field
does not repeat the hint — it *is* the effective value.

## Saving

Save writes only what was touched. An empty password field is not written at all
— otherwise `Save`, pressed for the sake of a neighbouring field, would erase a
stored password.

A key cannot be deleted through the form: clearing a filled field is refused
with a request to remove the key from `config.yaml` by hand, because the writer
merges a patch by keys and cannot delete with them. The `state:` block is
deliberately absent from the form and survives a save untouched.

**Saving rewrites `config.yaml` in full, and comments in it are not
preserved.** The previous file is put next to it as `config.yaml.bak` — once,
before the first rewrite, so that a second save does not overwrite the backup
with an already-applied state. Both the file and the backup get `0600`
permissions: the config holds `mqtt.password`.

## When changes take effect

Everything except the MQTT group takes effect on the next tick: the tracker
re-reads its settings every round, no restart needed. Editing the same keys by
hand in `config.yaml` is picked up the same way.

The broker fields are marked "takes effect after restart" in the form — the
subscription thread is started at launch and cannot be stopped.

## What the toggles actually switch off

- **Placement off** does not erase remembered places: `state.json` keeps being
  written, and switching it back on places windows that appeared after that
  moment. (The placement list lives for a single tick, so a window opened while
  the toggle was off stays where the system put it.)
- **Requests off** removes `focus` from the published windows file — the picker
  stops offering Enter instead of offering it in vain.
- **Snapshots off** stops both writing and publishing them.

## The Log tab

A fifth tab shows what the tracker has been writing to stderr: which window it
sees but cannot hand to a session, why a layout did not apply, what happened to
MQTT. Started from the status bar under launchd, those lines used to go nowhere.

The last thousand lines, newest at the bottom, each stamped with a date and
time — so a line can be matched against what was happening at the same moment on
another machine: sleep, a dropped connection, a restart.

## Closing the window

The close button does not stop the tracker: closing the last window would by
default terminate a Tauri app, and `main()` carries a branch that prevents it.
Only `Quit` stops the tracker.
