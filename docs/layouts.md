# Window layouts

Two layouts — `Tile windows` and `Cascade windows`. Both take the bound windows
and arrange them on the main screen; the order matches the order of the tray
menu items, so the layout and the list line up by eye.

Without the Accessibility permission the items are disabled — there is nothing
to move windows with.

## No global hotkey here — it lives in the picker

The old `Cmd+Alt+Ctrl+C` (key `tileHotkey`) was removed from this project. The
keypress went into a layout request without `ids`, so the windows were arranged
in *this machine's* order rather than the order the human sees in the list. Only
the side that shows the list knows that order, so the key belongs to
[ccfzf-picker](https://github.com/popstas/ccfzf-picker) — key `tileHotkey` in
**its** `config.yaml`, same default. The request arrives here as an ordinary
`claude-place` message, order included; the menu items work as before.

## Minimized windows

Minimized windows are laid out unless told otherwise. The key
`features.showMinimized` (the "Ignore minimized windows" checkbox in the menu, a
toggle in the settings) excludes them from tile and cascade: there is nothing to
lay out — the window is not on screen — while it would still claim a grid cell
and squeeze its neighbours for the sake of empty space.

The grid is computed from what is left: three windows, one minimized → a
two-window tile. The exclusion applies both to menu items and to a request with
`ids` from the picker — otherwise the key would arrange something different from
what the menu item arranges. Nothing else is lost: a minimized window keeps its
menu item, its remembered place, and its line in the windows file.

**The minimized flag is published.** Each window record in the windows file
carries `minimized: true|false`, which
[ccfzf-picker](https://github.com/popstas/ccfzf-picker) uses to dim the row and
keep it out of its own tiling: it cannot know this by itself, only the tracker
sees the windows. The flag is part of the layout fingerprint, so minimizing
arrives on the same tick instead of up to half a minute later with the next
heartbeat. The `showMinimized` flag does not affect publishing at all — it is
about this machine's layout, while the reader needs the fact.

## Raising

Placement does not change stacking order: a window that sat behind another would
move to its new place still behind it, and the layout would be invisible. So
after placement each window is raised inside its own application, and the
applications themselves are brought forward with all their windows. A failure to
raise does not cancel the layout — the windows are already in place — but it
shows up in the status line.

## The work area

The work area excludes the menu bar and the Dock, whichever side the Dock is on:
it comes from `NSScreen::visibleFrame`. Only the main thread may ask for it,
while windows are moved by the tracker thread, so the area travels there in a
cell updated by the menu renderer, which visits the main thread every two
seconds.

For the first two seconds after launch the cell is empty and the layout falls
back to a spare area — the screen without the menu bar, but with the Dock.
Triggered in that window, it will put the outermost window under the Dock.

## Tile

A grid with no overlap.

Window width is kept between **80 and 120 columns**. The ideal is three
terminals side by side; on a wide screen the column count grows (otherwise lines
run past 120 characters and the eye loses them), on a narrow one it shrinks
(otherwise the terminal is under 80 and any command drawing a table breaks).

**The screen decides the column count, not the number of windows**, and the work
area is filled completely. Windows are distributed across columns and split
their column's height evenly: on a two-column screen, three windows become one
full-height window on the left and two half-height ones on the right — no empty
cells. Extra windows go to the right-hand columns: the first window in the list
is the topmost one in the picker, and it gets a whole column.

A single window occupies one column, not the whole screen: stretched across the
screen it would be twice as wide as the 120 characters the columns are computed
for.

Columns are converted to points by a fixed character width — 8 points, the
middle of what common monospace fonts give on a Mac. A terminal does not report
its width in characters, and Accessibility knows nothing about fonts, so the
number is knowingly approximate: half a point of error shifts the boundary by a
few columns but does not break the layout. It is tuned in
`crates/mwm-core/src/layout.rs`.

## Cascade

A stack: windows half the work area wide, one below another, stepped 50 points
right and down. A title bar's height would be enough to recognize a window but
not to grab it — fifty points give both the whole title and room for the cursor.

Height takes whatever is left after the steps, computed from the number of
windows in the stack: two windows need one step, and giving them as much room as
ten would waste height for nothing.

The stack fits as many steps as keeps a window at least half the work area tall
and inside the right edge (eight on a laptop). After that it restarts from the
top-left corner: windows that ran out of steps have nothing left to be offered,
however it is counted.

## Layout requests over MQTT

The same thing is requested with a `claude-place` message carrying
`{"mode": "tile"|"cascade", "ids": [...]}`, where `ids` set the order. An empty
or absent list means "all bound windows, in this machine's order"; a bare
`tile` or `cascade` string in the body is understood the same way — that is what
the panel sends.

The `requests` toggle in the settings silences MQTT requests but not the menu
items: there a human is pressing by hand. The `placement` toggle does not touch
layouts at all — it is about returning a window to its remembered place when it
opens, which is a separate thing.

A laid-out window remembers itself: the slot relearns the window's current
position, and the layout is never rolled back.
