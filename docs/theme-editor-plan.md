# Theme editor (shipped in 1.5.0)

Status: implemented in 1.5.0. This document started as the 1.4.0 plan and was revised when the
design changed to **no built-in theme**. It now describes what was built and why. Before 1.5.0,
`src/theme.rs` had two hard-coded palettes and a `Mode` (dark / light / system, global pref
`ui.theme_mode`).

## 1. Goals

- Themes are data, not code: one TOML file per theme, each with a `dark` and a `light` variant.
- **No theme is embedded in the binary.** `default` is a file like any other. The repo ships
  `themes/default.toml` (exactly the 1.4.0 look), and the packages install it.
- An in-app editor with a live preview: edit colors and per-part styles, then save, revert,
  duplicate, create or delete themes.
- A theme switcher in the header next to the mode switcher, plus hotkeys.
- A bad, partial or missing theme file never breaks the UI. The app falls back to the terminal's
  own colors with a clear notice.

## 2. Storage and shipping

- User themes: `<data dir>/themes/<name>.toml` (normally `~/.grokbot-tui/themes/`, following
  `--data-dir`). Names match `[a-z0-9][a-z0-9_-]{0,31}`, and the file name is the theme id.
- Shipped themes (`themes/*.toml` in the repo) are installed to a shared directory: deb/rpm/AUR
  `/usr/share/grokbot-tui/themes`, archives `themes/` (Homebrew: `pkgshare` =
  `share/grokbot-tui/themes`), MSI `share\grokbot-tui\themes` next to `bin`. See PACKAGING.md.
- `themes::shared_dirs()` searches, in order: `<canonical exe dir>/../share/grokbot-tui/themes`,
  `<exe dir>/themes`, `$XDG_DATA_DIRS/*/grokbot-tui/themes` (default `/usr/local/share:/usr/share`),
  then `/usr/share`, `/usr/local/share`, `/opt/homebrew/share`. Duplicates are removed.
- **Seeding:** on startup, if the user's themes folder has no `*.toml`, every valid-named
  `*.toml` in the shared dirs is copied in (the first dir wins per name; nothing is ever
  overwritten), and a notice names the files. Once the folder has any theme, upgrades don't
  touch it. A new shipped default reaches existing users only if they delete or replace their
  copy. That's the accepted cost of "your files are yours".
- Writes are atomic (temp file, fsync, rename) with 0600 files in a 0700 directory.

## 3. TOML schema (v1)

```toml
schema = 1                 # files with a higher version are refused, not misread
name = "Default"           # display name; the file name is the id
description = "…"

[dark.colors]              # the palette: 14 named colors
text = 252                 # 0-255 (integer or string), "#rrggbb", "#rgb", a name, or "reset"
background = 234
accent = "cyan"
subtle = "gray"
muted = "darkgray"
header = 237
warn = "yellow"
danger = "red"
ok = "green"
on_accent = "black"
modal_text = "white"
modal_background = "black"
logo_dark = 232            # the header logo (1.3.2 colors)
logo_light = 255

[dark.styles]              # one entry per part; fields: fg, bg, border, bold, italic,
                           # underline, reverse, dim. Colors: a literal or "@palette-name".
brand = { fg = "@on_accent", bg = "@accent", bold = true }
title = { fg = "@accent" }                 # not bold since 1.3.2
panel_focused = { fg = "@text", border = "@accent" }
# …

[light.colors]
# …
[light.styles]
# …
```

Parts (38, `theme::STYLE_SLOTS`): `base`, `brand`, `top_bar`, `title`, `header_label`,
`account_switch`, `theme_switch`, `mode_active`, `mode_inactive`, `key_hint`, `mouse_on`,
`mouse_off`, `mock_tag`, `selected`, `cursor_row`, `panel_focused`, `panel_unfocused`,
`chat_panel`, `muted`, `entry_{system,error,user,bot}`, `entry_label_{system,error,user,bot}`,
`modal_text`, `help_frame`, `help_title`, `shell_frame_{running,failed,ok}`,
`shell_title_{running,failed,ok}`, `link_selected`, `link_unselected`.

`border` only applies to boxed parts (`panel_*`, `chat_panel`, `help_frame`, `shell_frame_*`).
Layout (borders, padding such as `chat_panel`'s `(1, 2, 0, 2)`) stays in code, so a theme can't
break the layout. Only the header title's glyph choice (`ui.title_glyphs`, 1.4.0) and the logo
pixel art are code. The logo's two colors are the palette's `logo_dark` / `logo_light`.

## 4. Runtime model

- `theme.rs` holds only the schema: `COLOR_SLOTS`, `STYLE_SLOTS`, `ColorRef { Lit, Slot }`,
  `StyleSpec`, `VariantSpec { colors: [Color; 14], styles: Vec<StyleSpec> }`, `Theme`.
  It also holds `starter_style(slot)`, the minimal part→role mapping (title → `@accent`,
  errors → `@danger`, highlights → reverse video, …) that carries no colors.
- `Theme::unstyled(id)` is the starter rules with every palette color `Reset`. It's used both as
  the fallback and as the `/theme init` content.
- A thread-local active theme (`Arc<Theme>`) next to the `Variant`. The style functions keep their
  names (`theme::brand()` …), so call sites in ui.rs, picker.rs etc. didn't change.
- `themes.rs`: color parsing/formatting, WCAG contrast, `parse(id, text) -> (Theme, warnings)`,
  `serialize`, and the store (`list`, `load`, `save`, `delete`, `seed`, `shared_dirs`).

## 5. Fallback rules

| Problem | Result |
|---|---|
| Selected theme file missing | Terminal colors (`Theme::unstyled`), notice with the download URL and `/theme init`, header `‹name: missing›` |
| Invalid TOML (with line number), > 64 KiB, no `[dark]`/`[light]`, `schema` > 1 | Same, header `‹name: broken›`. The file is never overwritten without asking |
| Missing color | `reset` (terminal default) + warning |
| Missing part | Its starter rule, resolved against the file's palette (silent) |
| Bad value / unknown key | That key falls back + warning naming it |
| One variant missing | Copies the other + warning |

Warnings appear as one notice (first five) when the theme loads. The `ui.theme` pref is kept when
the file is missing, so restoring the file brings it back.

## 6. Prefs, header, keys, commands

- `ui.theme` (global, default `default`) next to `ui.theme_mode`.
- Header: `theme  ‹default› ▾  dark  light  system  F4`. The `‹name› ▾` segment is clickable in
  both tall and compact headers and opens the theme pick list.
- **F5** opens the theme pick list (themes, then *Edit themes…*; with no themes at all, *Write a
  starter theme*). **Shift-F5** (also F17, which some terminals send) opens the editor. F4
  still cycles the mode.
- `/theme` (alias `/mode`): bare cycles the mode, `dark|light|system`, `use [name]` (or
  `/theme <name>`), `list` (`ls`), `edit [name]`, `init [name]`, `delete <name>` (`rm`, asks
  first), `reload`.

## 7. Editor

Full screen. A theme list on the left (palette colors, then parts grouped by area; `●` marks
changes since the last save). A live preview on the right, drawn by the real `ui::draw_main` with
the edited theme into an off-screen `TestBackend` and copied in. A detail pane at the bottom.

- Tab flips the dark/light variant (independent of the app's mode).
- Enter / `e`: a color takes a literal or `@name` (invalid input shows the reason and is refused).
  A part opens a form: fg, bg, border (boxes only), then bold/italic/underline/reverse/dim
  toggles. Backspace / Delete clears a field.
- `p`: 256-color palette grid (keyboard or mouse). Typing a `#rrggbb` warns when `COLORTERM`
  isn't truecolor/24bit.
- Contrast: WCAG ratio for text/background pairs, with a warning below 3:1.
- `s` / Ctrl-S: save (asks before replacing a broken file or one changed on disk). It applies right
  away when this is the theme in use.
- `r` / `R` (Ctrl-R): revert the item / everything since the last save.
- `d` duplicate, `n` new starter theme (both ask for a name), `x` / Delete: delete (asks; any
  theme, including default; deleting the active theme switches to default or the first theme
  left, else falls back).
- `t` edits another theme. Esc / `q` close (Save / Discard / Cancel if there are unsaved changes).

Removed from the 1.4.0 plan: "Reset to built-in" (there's no built-in) and `/theme export`
(themes are already files). Not done: highlighting the selected part inside the preview.

## 8. The starter trade-off

`/theme init` and the fallback can't contain colors without re-embedding a theme. So the binary
keeps only the *structure*: which palette role each part uses, plus reverse video for highlights so
selection stays visible. Every color is the terminal default.
- **Pros:** one source of truth for the look (the shipped file); no stale built-in defaults; the
  fallback always works and matches the terminal's colors.
- **Cons:** a starter or fallback looks plain until colors are set; the role mapping is still code,
  so new parts in future releases get their starter rule in files that don't mention them.

## 9. Tests

- `theme.rs`: golden test, where the repo's `themes/default.toml` reproduces the 1.4.0 palettes
  and recipes exactly (both variants, logo 232/255, title not bold). Also slot tables, the
  unstyled fallback, active theme and variant, boxed styles, and mode parsing.
- `themes.rs`: color literals round-trip, contrast, names, serialize ↔ parse (the shipped file
  is byte-identical to the serializer output), partial files, refused files, seeding (only into an
  empty dir, never overwrites, 0600), load/save/delete.
- `theme_editor.rs` (temp data dir, never the real one): seeding at startup, missing/broken
  fallback + notice + header marker, `/theme init|use|list|delete|reload`, F5 picker, header
  click, editor text/palette/form edits, revert, save-and-apply, close prompt, duplicate/new/
  delete, conflict and broken-file prompts, contrast warning, preview reflects unsaved edits,
  mouse.
