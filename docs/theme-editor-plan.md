# Theme editor (shipped in 1.5.0, revised in 1.6.0 and 1.7.0)

Status: implemented in 1.5.0. This document started as the 1.4.0 plan and was revised when the
design changed to **no built-in theme**. It was revised again in 1.6.0, when themes became
**user-level only**: system theme folders were dropped in favor of downloading the default
theme. In 1.7.0 it was brought in line with the theme system of acp-rs-scraper (see section 10).
It now describes what was built and why. Before 1.5.0,
`src/theme.rs` had two hard-coded palettes and a `Mode` (dark / light / system, global pref
`ui.theme_mode`).

## 1. Goals

- Themes are data, not code: one TOML file per theme, each with a `dark` and a `light` variant.
- **No theme is embedded in the binary.** `default` is a file like any other. The repo's
  `themes/default.toml` (exactly the 1.4.0 look) is its source, fetched from the release tag.
- **Themes are user-level only:** read from `<data dir>/themes` and nowhere else.
- An in-app editor with a live preview: edit colors and per-part styles, then save, revert,
  duplicate, create or delete themes.
- A theme switcher in the header next to the mode switcher, plus hotkeys.
- A bad, partial or missing theme file never breaks the UI. The app falls back to the terminal's
  own colors with a clear notice.

## 2. Storage and shipping

- User themes: `<data dir>/themes/<name>.toml` (normally `~/.grokbot-tui/themes/`, following
  `--data-dir`). Names match `[a-z0-9][a-z0-9_-]{0,31}`, and the file name is the theme id.
- **Only that folder is read.** There are no system theme folders (1.5.0 had
  `/usr/share/grokbot-tui/themes`, Homebrew `pkgshare`, next-to-binary and `$XDG_DATA_DIRS`
  lookups, and the packages installed `default.toml` there; 1.6.0 removed all of it).
- **First run:** when the folder has no `*.toml`, the app downloads
  `https://raw.githubusercontent.com/WeirdBrains/grokbot-tui/v<version>/themes/default.toml`
  (the tag of the running version, so file and binary match). It uses the existing reqwest/rustls
  stack, in the background (startup never waits), with a 5 s timeout (3 s to connect) and a
  64 KiB cap. It checks that the file parses as a theme, writes it verbatim (never over an existing
  file), loads it and shows a notice. Until then, and if it fails, plain terminal colors plus a
  notice. Mock mode never downloads.
- **Opt-out:** `GROKBOT_TUI_NO_THEME_DOWNLOAD=1` or the global pref `ui.theme_download = false`.
- Once the folder has any theme, upgrades don't touch it. A new default reaches existing users
  only via `/theme init <name>` or by deleting their copy. That's the accepted cost of "your
  files are yours".
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
  `serialize`, and the store (`list`, `load`, `save`, `write_new`, `delete`) and `download`.

## 5. Fallback rules

| Problem | Result |
|---|---|
| No themes at all (first run) | Download `default.toml` (above); header `‹default: downloading›` meanwhile |
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
- **F5** opens the theme pick list (themes, then *Edit themes…*; with no themes at all, *Download
  the default theme*, or *Write a starter theme* when downloading is off). **Shift-F5** (also F17, which some terminals send) opens the editor. F4
  still cycles the mode.
- `/theme` (alias `/mode`): bare cycles the mode, `dark|light|system`, `use [name]` (or
  `/theme <name>`), `list` (`ls`), `edit [name]`, `init [name]` (download the default theme as
  `<name>.toml`, else the starter), `starter [name]` (always the minimal starter), `delete <name>`
  (`rm`, asks first), `export <name> [path]` (1.7.0), `reload`.
- The pick list previews the highlighted theme live (1.7.0); Esc restores the previous one.
- Command line (1.7.0): `--theme <name>` for one run (an in-memory override, not saved, kept
  across config reloads until a theme is picked in the app; checked before the UI starts) and
  `--list-themes` (prints and exits; never downloads).

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
- `t` edits another theme; `[` / `]` step to the previous / next file (1.7.0; refused while there
  are unsaved changes). Esc / `q` / Shift-F5 / F17 close (Save / Discard / Cancel if there are
  unsaved changes). Ctrl-D / Ctrl-N / Ctrl-X alias `d` / `n` / `x`.
- `⚠` in the list marks items below 3:1 contrast (1.7.0).
- **Preview highlight (1.6.0):** the selected palette color or part is marked in reverse video in
  the preview (`h` toggles). It works by rendering the preview a second time with that item set to
  colors the variant doesn't use, and marking every cell that differs. So it follows the real
  drawing code for every part, with no per-part list to maintain. The preview also draws sample
  chat entries and shell frames, so those parts can be seen even in an empty chat. The title says
  when the selected part isn't on screen (the compact header's `brand`, `mouse_off`, `cursor_row`).

Removed from the 1.4.0 plan: "Reset to built-in" (there's no built-in). `/theme export` was dropped
too, then added in 1.7.0 to match acp-rs-scraper. Highlighting the selected part, deferred in 1.5.0, shipped in 1.6.0.

## 8. The starter trade-off

`/theme starter` and the fallback (and `/theme init` when it can't download) can't contain colors without re-embedding a theme. So the binary
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
  is byte-identical to the serializer output), partial files, refused files, load/save/delete,
  `write_new` (never overwrites, 0600), the download URL / opt-out, and `download` against a local mock
  server (exact text, 404, invalid, over 64 KiB, not UTF-8, connection refused).
- `theme_editor.rs` (temp data dir, never the real one, never the real network): background
  download at startup (success, failure, opt-out), `/theme init` download / fallback, `/theme
  starter`, only the user dir is read, preview highlight (colors and parts, `h`, off-screen
  parts), missing/broken
  fallback + notice + header marker, `/theme init|use|list|delete|reload`, F5 picker, header
  click, editor text/palette/form edits, revert, save-and-apply, close prompt, duplicate/new/
  delete, conflict and broken-file prompts, contrast warning, preview reflects unsaved edits,
  mouse.

## 10. Keeping in step with acp-rs-scraper (1.7.0)

grokbot-tui's theme system is kept in sync with the one in acp-rs-scraper. In 1.7.0 it took over:
`--theme` / `--list-themes`, `/theme export`, live preview in the pick list, atomic `0600` writes
(PID-named temp file created with the mode, removed on failure; folder `0700` only when created;
no mode on Windows), the size check before reading (64 KiB), a warning for a missing `schema`,
and the editor's `[` / `]`, Shift-F5 close, Ctrl-D/N/X and `⚠` contrast markers.

Deliberately different:
- Files live only in `<data dir>/themes`; the default is downloaded from the release tag (no
  system or next-to-binary folders, nothing copied from them).
- Schema: `schema = 1` with 14 fixed palette colors and 38 parts, `@name` palette references (one
  level), and a missing variant copying the other. acp-rs-scraper uses `version = 1`, free-form
  palette names with `@` chains, light inheriting dark per key, and optional style overrides.
- A non-integer `schema` is a warning here (acp-rs-scraper refuses the file).
- No legacy `[theme]` migration (grokbot-tui never had that format).
