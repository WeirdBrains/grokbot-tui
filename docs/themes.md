# Themes and the theme editor

The built-in themes, dark/light/system modes, theme files and the in-app editor. See the [theme gallery](https://weirdbrains.github.io/grokbot-tui/themes/) for a screenshot of each.

## Themes

Every theme, including `default`, is a TOML file in `~/.grokbot-tui/themes/<name>.toml` (it follows
`--data-dir`), and themes are read only from that folder: nothing is built into the binary and no
system folder is searched. Each file has a `[dark]` and a `[light]` variant:
a palette of 14 named colors and a style for each part of the screen.

```toml
schema = 1
name = "Default"
description = "The original grokbot-tui look"

[dark.colors]
text = 252                 # 0-255, "#rrggbb", "#rgb", a color name, or "reset" (terminal default)
background = 234
accent = "cyan"
# … subtle, muted, header, warn, danger, ok, on_accent, modal_text, modal_background,
#   logo_dark, logo_light

[dark.styles]              # fg, bg, border (boxes only), bold, italic, underline, reverse, dim
brand = { fg = "@on_accent", bg = "@accent", bold = true }   # "@name" = a palette color
panel_focused = { fg = "@text", border = "@accent" }
# …

[light.colors]
# …
```

- **Where `default.toml` comes from:** on first run, when your themes folder has no themes,
  grokbot-tui downloads the repo's [`themes/default.toml`](../themes/default.toml) from the git tag
  matching its version
  (`https://raw.githubusercontent.com/WeirdBrains/grokbot-tui/v<version>/themes/default.toml`),
  checks that it's a valid theme, saves it there and shows a notice. This works the same for every
  install method. Startup never waits for it: plain terminal colors until it arrives, and it gives
  up after 5 seconds. Offline or failed: plain colors plus a notice; `/theme init` tries again. Your
  files are never overwritten. **Opt out** with `GROKBOT_TUI_NO_THEME_DOWNLOAD=1` or
  `/prefs set ui.theme_download false`.
- **Missing or broken theme:** grokbot-tui uses your terminal's own colors (highlights stay visible
  via reverse video), shows a notice saying why, and marks the header `‹name: missing›` /
  `‹name: broken›`. A broken file is never overwritten without asking. Partial files are fine:
  missing colors use the terminal default, missing parts use the starter rule, a missing variant
  copies the other, and each problem is listed once (as are unknown keys and a missing `schema`
  line). Files over 64 KiB are refused before they're read, and a newer `schema` is refused.
- **Saving** is atomic (a temp file in the same folder, then a rename) and private: theme files are
  `0600` and a new themes folder is `0700` (Windows keeps its default permissions).
- **Picking (`F5`):** moving through the list previews each theme live; `Enter` keeps it, `Esc`
  goes back. `--theme <name>` picks one for a single run without saving it (a missing or broken
  theme stops before the UI starts); `--list-themes` prints the list and exits.
- **`/theme export <name> [path]`** copies a theme file (exact text) to
  `<data dir>/exports/<name>-<UTC time>.toml`, or to the path given (a folder gets `<name>.toml`).
  It never overwrites.
- **`/theme init [name|all]`** downloads `<name>.toml` from the release's `themes/` folder the same
  way (default `default`; `all` = every shipped theme, currently just `default`). Existing files
  are kept. With downloading off it writes the minimal starter instead. `--list-themes` and
  `--theme` also fetch the default first when the folder is empty (waiting up to 3 s).
  `GROKBOT_TUI_THEME_URL` replaces the download address (`<url><name>.toml`).
- **`/theme starter [name]`** always writes the minimal starter: every part is mapped to a palette
  role (title → `@accent`, errors → `@danger`, …) but every color is the terminal default. Set the
  colors in the editor.
- **Editor (`Shift-F5` or `/theme edit [name]`):** the palette and every part on the left, a live
  preview of the app on the right (drawn by the real UI code with your unsaved changes, plus sample
  chat entries and modals). The selected color or part is marked in reverse video in the preview
  (`h` toggles; the preview title says when the part isn't on screen). `Tab` flips
  dark/light, `Enter`/`e` edits (hex, 0-255, a name or `@palette`), `p` opens the color picker (tabs: 16 colors, the 216-color cube, 24 grays, `@palette` refs),
  `r`/`R` revert the item/everything, `s` saves (applied immediately if it's the theme in use),
  `d` duplicates, `n` creates a starter theme, `x` deletes (asks first; `Ctrl-D`/`Ctrl-N`/`Ctrl-X`
  work too), `t` opens another theme and `[`/`]` step to the previous/next one, `Esc` (or
  `Shift-F5`) closes (asks about unsaved changes). Text/background pairs below a 3:1 WCAG contrast
  ratio get a warning, and `⚠` marks them in the list. Mouse clicks work on the list, tabs, fields and palette.

### Responsiveness

The screen never waits on the network. Every gateway call runs on a tokio task and posts its result back over a channel.
Results that arrive after an account switch or logout are discarded.

Only the **active bot** is polled (`getAgentTranscriptTail`, last 100 entries):
- after a send: about every 1 s, backing off to 8 s while waiting for the reply ("… is replying" spinner in the body title, up to 5 min);
- otherwise: every 1 s, backing off to 15 s when nothing changes or on errors. The visible feed is re-read from local SQLite every 100 ms (no network).
