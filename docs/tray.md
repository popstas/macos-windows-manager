# Tray menu

The app has no windows of its own — only a status-bar icon. Everything it says,
it says through this menu.

```
3 windows tracked, 1 minimized
▦ Tile windows
❐ Cascade windows
✓ Ignore minimized windows
  claude-wt · WezTerm
  ccfzf-picker · Ghostty
  mwm · WezTerm
⚙ Settings
⚿ Grant Accessibility…
⏻ Quit
```

## Status line

The top line is the result of the last tick: `3 windows tracked`, with a
complaint appended when publishing or placement failed.

Minimized windows are counted on the same line — `3 windows tracked, 1
minimized` — and counted among the *bound* windows, not among everything seen:
the items below the line are the bound windows, and the number has to describe
that same list. A zero count is dropped together with its comma.

## The "Ignore minimized windows" checkbox

The checkbox has no glyph of its own — macOS draws the check mark itself, and a
font symbol next to it would read as a second state indicator.

It writes `features.showMinimized` into `config.yaml`, through the same path the
settings window uses, so it survives a restart. It is checked when
`showMinimized: false`: the label describes what will happen, while the key is
named the other way round, because every flag under `features:` defaults to on.

The check mark is set by the menu renderer from the config, not by the click:
the file is also edited by hand, and the menu is obliged to show the file. The
checkbox stays enabled without Accessibility — it is a setting, not an action on
windows.

## Icons

Every action item carries a glyph — `▦` tile, `❐` cascade, `⚙` settings, `⚿`
grant permission, `⏻` quit. These are font symbols, not images: `IconMenuItem`
would demand a raster per theme and per screen scale, while a glyph takes its
color and size from the menu.

The chosen glyphs have no colored rendering — a character with
`Emoji_Presentation` would land in the menu as a color image three times the
line height.

Hair spaces follow each glyph to pad it to the width of the widest one, so the
labels all start at the same place. The widths were measured on the machine
(`CTLineGetTypographicBounds`, menu font `.AppleSystemUIFont` 13 pt); the
remaining spread is under 0.4 pt.

The padding is needed because the glyphs come from different fonts: `▦` from
Apple SD Gothic Neo, `❐` from Zapf Dingbats, `⚙` from Menlo, `⚿` from STIX Two
Math, and only `⏻` from the menu font itself. The system font has no tile or
cascade glyph at all, so the mixture is unavoidable — at a larger size both the
weight and the drawing would visibly diverge.

## Window items

The window items are disabled — they are labels, not actions.

The terminal is appended because a session title alone does not say where the
session is open, and windows of one session are looked up by terminal
specifically.

The order follows the label, not the session id: that one is random and would
shuffle from tick to tick as soon as a window closed. Long titles are truncated
— the menu stretches to its longest item, and a single long name would push the
rest half a screen away from the cursor.

The list refreshes on the same tick as the status line (every two seconds), and
disappears together with the Accessibility permission: without it the tracker
never reaches the windows at all, and the remaining items would be naming
windows it no longer knows anything about.

## Grant Accessibility…

The item is visible only while the permission is missing. Granting it removes
the item, revoking it brings it back: the tracker asks the system every tick, no
restart needed.
