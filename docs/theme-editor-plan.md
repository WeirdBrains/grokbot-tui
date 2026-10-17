# Theme editor plan (target: 1.4.0)

Status: plan only, nothing implemented yet. It builds on 1.3.0, where `src/theme.rs` has two
hard-coded palettes (`DARK`, `LIGHT`), a `Mode` (dark / light / system) saved in the global
pref `ui.theme_mode`, and a thread-local `Variant` that every style function reads.

## 1. Goals

- Themes are data, not code: one TOML file per theme, each with a `dark` and a `light` variant.
- The built-in theme ("default") is exactly today's look and is always available, even with no
  files on disk.
- An in-app editor with a live preview: edit colors and per-part styles, then save, revert,
  duplicate, create or delete themes.
- A theme switcher in the header next to the mode switcher, plus a hotkey.
- A bad or partial theme file never breaks the UI. Missing keys fall back to the default, and
  errors are reported, not fatal.

## 2. Storage

- Directory: `<data dir>/themes/` (normally `~/.grokbot-tui/themes/`, and it follows
  `--data-dir` / `GROKBOT_TUI_APP_DIR`).
- One file per theme: `themes/<name>.toml`. Names are `[a-z0-9][a-z0-9-_]{0,31}`, and the file
  name is the theme id.
- **Default theme: embedded, written only when edited.** `default.toml` is compiled in
  (`include_str!("../themes/default.toml")`, the same file kept in the repo). Nothing is written on
  first run, so the data dir stays clean and a future release can change the default without
  fighting a stale copy. Editing "default" in the editor saves `themes/default.toml` as an
  override, and "Reset to built-in" deletes that file (after a confirmation pick list).
  - Rejected alternative: write `default.toml` on first run if missing. It's simpler to
    discover but freezes the user on old defaults after upgrades. Instead, the editor's
    "Duplicate" and `/theme export` make the file visible on demand.
- Writes are atomic (write `<name>.toml.tmp`, fsync, rename) with 0600 permissions, like the
  other data-dir files.

## 3. TOML schema (v1)

```toml
schema = 1                 # required; files with a higher version are refused, not misread
name = "Default"           # display name (free text); the file name is the id
description = "Built-in grokbot-tui theme"

# One table per variant. Each has a palette and optional per-part style overrides.
[dark.colors]              # mirrors theme.rs `Palette`
text             = "252"   # ANSI 256 index as a string, "#rrggbb", or a named color
background       = "234"
accent           = "cyan"
subtle           = "gray"
muted            = "darkgray"
header           = "237"
warn             = "yellow"
danger           = "red"
ok               = "green"
on_accent        = "black"
modal_text       = "white"
modal_background = "black"
logo_dark        = "236"   # new: today's logo_colors() constants become palette slots
logo_light       = "254"

[dark.styles]              # optional; a slot left out uses the built-in recipe for that slot
# value = { fg, bg, border_fg, bold, italic, underline, reverse, dim }, where fg/bg/border_fg
# are a color literal or "@slot" (a palette reference, e.g. "@accent")
brand          = { fg = "@on_accent", bg = "@accent", bold = true }
top_bar        = { fg = "@text", bg = "@header" }
title          = { fg = "@accent", bold = true }
account_switch = { fg = "@warn", bold = true, underline = true }

[light.colors]
text = "235"
background = "255"
# …

[light.styles]
```

Style slots (exactly the public functions in `theme.rs`; a function with a parameter
becomes one slot per case):

| Area | Slots |
|---|---|
| Screen and header | `base`, `brand`, `top_bar`, `title`, `header_label`, `account_switch`, `mode_choice.active`, `mode_choice.inactive`, `theme_choice.active`, `theme_choice.inactive` (new), `key_hint`, `mouse_on`, `mouse_off`, `mock_tag` |
| Sidebar | `selected`, `cursor_row`, `panel.focused`, `panel.unfocused` |
| Chat | `chat_panel`, `muted`, `entry.system`, `entry.error`, `entry.user`, `entry.bot`, `entry_label.*` (default: the `entry.*` slot plus bold) |
| Modals | `modal_text`, `help_frame`, `help_title`, `shell.running`, `shell.failed`, `shell.ok`, `link.selected`, `link.unselected` |

Layout properties (borders, padding, e.g. `chat_panel`'s `padding((1, 2, 0, 2))`) stay in code.
Themes control colors and text attributes only, so a theme can't break the layout.

Color literals: `"0"`–`"255"` (indexed), `"#rrggbb"` / `"#rgb"` (true color), the 16 ANSI names
(`black`, `red`, …, `darkgray`, `lightred`, …, `white`), `"reset"` (terminal default), or
`"@slot"` in styles only.

## 4. Runtime model (replacing the hard-coded palettes)

```rust
pub struct Theme { id: String, name: String, dark: VariantSpec, light: VariantSpec }
pub struct VariantSpec { colors: Palette, styles: HashMap<Slot, StyleSpec> }
pub enum Slot { Base, Brand, TopBar, /* … one per row above */ }
pub struct StyleSpec { fg: Option<ColorRef>, bg: Option<ColorRef>, border_fg: Option<ColorRef>,
                       bold: Option<bool>, italic: …, underline: …, reverse: …, dim: … }
pub enum ColorRef { Color(Color), Slot(PaletteSlot) }
```

- `thread_local! ACTIVE: RefCell<Rc<Theme>>` next to today's `VARIANT`. The style functions keep
  their names and signatures (`theme::brand()` …). Each now looks up its slot in the active
  variant, overlays the spec on the built-in recipe, and resolves `@slot` refs against that
  variant's palette. Call sites in ui.rs, picker.rs etc. don't change.
- `fn builtin() -> &'static Theme` parses the embedded `default.toml` once (`OnceLock`). A unit
  test asserts the parsed built-in reproduces the 1.3.0 `DARK` / `LIGHT` styles exactly.
- Loading: `themes::load(dir, id) -> (Theme, Vec<Warning>)`. Unknown or failed themes fall back to
  the built-in theme with a warning.

## 5. Prefs

- Existing: `ui.theme_mode` = `dark | light | system` (global).
- New: `ui.theme` = theme id (global, default `default`; added to `GLOBAL_KEYS`).
- `/prefs set ui.theme <id>` and `/theme use <id>` both validate that the theme exists before
  saving. A pref naming a theme whose file has since been deleted falls back to the built-in
  theme with a one-time notice, and the pref is kept so restoring the file brings it back.

## 6. Header theme switcher and hotkeys

- Tall header, row 2 becomes `theme  ‹default› ▾  dark  light  system  F4`. `‹name› ▾` is
  clickable and opens a theme pick list (the existing `picker` widget: one item per theme, current
  marked, plus "Edit themes…" at the bottom).
- Compact header: `‹name› ▾` goes before the mode choices, and is clipped first.
- Keys: **F5** opens the theme pick list (free today; F1/F2/F3/F4/F6 are taken). **Shift-F5** or
  `/theme edit` opens the editor for the current theme. F4 keeps cycling the mode.
- Commands: `/theme` (cycle mode, as now), `/theme dark|light|system`, `/theme use <id>`,
  `/theme list`, `/theme edit [id]`, `/theme export [id]` (writes the file, prints its path),
  `/theme reload` (re-read files after editing them by hand).

## 7. In-app editor

Opens with Shift-F5, `/theme edit [id]`, or "Edit themes…" in the theme pick list. It's a
full-screen modal, like help:

```
┌ Theme editor: Default (dark) ── modified ●───────────────────────────────────────┐
│ Variant  [dark] light          Theme  ‹Default› ▾                                │
│┌ Slots ─────────────────┐┌ Preview ──────────────────────────────────────────────┐│
││ Colors                 ││  ▄▀▀▀▀▄  GROK BOT   account  @default ▾  F3            ││
││   text          ■ 252  ││                     theme  dark light system           ││
││ › accent        ■ cyan ││ ┌ Bots ─────┐┌ Chat: Helper ──────────────────────┐    ││
││   …                    ││ │ Helper    ││ you: hi                            │    ││
││ Header                 ││ │ Coder     ││ Helper: hello!                     │    ││
││   brand    fg/bg/B     ││ └───────────┘│ ! error line                       │    ││
││   top_bar              ││              └────────────────────────────────────┘    ││
││ Sidebar / Chat / Modals││ ┌ Help ────┐ ┌ shell: ok ┐                             ││
│└────────────────────────┘└───────────────────────────────────────────────────────┘│
│ accent = cyan   [e] edit  [p] palette  [r] revert slot   ^S save  ^R revert all   │
│ [d] duplicate  [n] new  [x] delete  [Tab] variant  [Esc] close (asks if modified) │
└───────────────────────────────────────────────────────────────────────────────────┘
```

- **Left: slot list.** Palette colors first, then style slots grouped by area. Each row shows a
  swatch (`■` in the resolved color), the value, and a `●` when it differs from the saved file.
  ↑↓ / j k / mouse move the selection, and the selected slot is highlighted in the preview
  (reverse-video outline around where it's used).
- **Right: live preview.** A miniature screen rendered by the real drawing code: `ui::draw`
  takes a `PreviewApp`-style fixture (mock bots and entries), is drawn into an off-screen
  `Buffer` with the edited theme installed in the thread-local, and copied into the preview
  rect. Same code path, so the preview can't drift from the real UI. Tab switches the previewed
  variant (dark / light), independent of the app's current mode.
- **Editing a color** (Enter / `e` on a color or on a style's fg/bg):
  - A text field accepts any literal from §3. Invalid input turns the field red with the reason
    (`expected 0-255, #rrggbb or a color name`) and Enter is refused.
  - `p` opens a palette grid: 16 ANSI colors, the 6×6×6 cube and the 24 grays, navigated by arrows
    or mouse, plus "@slot" references for style fields.
  - True-color `#rrggbb` shows a warning when `COLORTERM` isn't `truecolor`/`24bit`.
- **Editing a style:** a small form with fg / bg / border_fg (only for frame slots) and toggles
  for bold / italic / underline / reverse / dim (space toggles). "Inherit" clears an override
  back to the built-in recipe.
- **Contrast check:** for slots with both fg and bg, show the WCAG contrast ratio and flag
  < 3:1 as a warning (not an error).
- **Save (^S):** validate the whole theme, then write atomically. When editing "default", save
  writes the override file (see §2), after a one-line confirmation the first time.
- **Revert:** `r` reverts the selected slot to the saved value, ^R the whole theme (with
  confirmation).
- **Duplicate (`d`) / New (`n`):** asks for a name (validated, must not exist). New starts from a
  copy of the built-in theme so it's always complete. The new theme opens in the editor.
- **Delete (`x`):** confirm pick list with Cancel as default (like `/account remove`). The
  built-in theme can't be deleted; for an overridden default this action is "Reset to built-in".
  Deleting the active theme switches to `default`.
- **Close (Esc):** if modified, a pick list asks Save / Discard / Cancel.
- **Applying:** saving the active theme applies it immediately. Editing a non-active theme
  doesn't change the app until it's selected.

## 8. Validation and edge cases

- **Invalid TOML** (syntax error): the theme isn't loaded, the built-in theme is used, and a
  notice shows `themes/foo.toml line 12: expected '='`. The file is never overwritten
  automatically. In the editor it opens read-only with the error and an option to start a fresh
  copy from the built-in theme.
- **Missing keys:** any missing color or style uses the built-in value for that variant, so
  partial files are valid. A whole missing variant table (e.g. no `[light]`) uses the built-in
  variant, and the theme list marks it "dark only (light: default)".
- **Unknown keys:** ignored with a warning listing them (helps catch typos like `acent`).
- **Bad values** (`text = "#zzz"`, `bold = "yes"`, `@nonexistent`): that one key falls back,
  and the warning names the key and file.
- **`schema` missing:** treated as 1. **Higher than supported:** refused with "made by a newer
  grokbot-tui".
- **Reference cycles:** impossible by construction, since `@slot` only refers to palette colors,
  never to other styles.
- **Large or hostile files:** reject files over 64 KiB, and accept only names matching the id
  regex (no path traversal).
- **Concurrent edits** (file changed on disk while the editor is open): on save, compare the
  mtime/hash read at open, and if it changed ask Overwrite / Reload / Cancel.
- **Read-only data dir or write failure:** an error notice, and the in-memory edits are kept.
- **16-color terminals:** indexed colors > 15 and true color degrade via the terminal. The editor
  notes it when `TERM` suggests a limited palette.
- **Deleted active theme / missing pref target:** see §5.

## 9. Migration from the hard-coded theme.rs

1. Add `themes/default.toml` to the repo, generated from the 1.3.0 `DARK` / `LIGHT` palettes and
   the style recipes (including `logo_colors` as `logo_dark` / `logo_light`).
2. Introduce `Theme` / `VariantSpec` / `Slot` and the parser (`serde` derive is already a
   dependency; add `toml` as a new direct dependency).
3. Re-implement the style functions on top of the active theme. Keep the 1.3.0 tests and add
   a golden test: for every slot × variant, the 1.3.0 hard-coded result equals the TOML-driven
   result. Then delete the `DARK` / `LIGHT` constants.
4. No user data migration is needed: there are no theme files before 1.4.0. `ui.theme_mode`
   is unchanged, and `ui.theme` defaults to `default`.

## 10. Testing

- **Parser unit tests:** each color literal form, named colors, `@slot`, invalid values per key,
  unknown keys, missing variant, `schema` handling, size limit, name validation.
- **Golden test:** built-in TOML == 1.3.0 styles (every slot, both variants).
- **Fallback tests:** partial file → missing keys equal the built-in; syntax error → built-in
  plus a warning; deleted active theme → built-in plus a notice, pref kept.
- **Store tests in a temp data dir** (never the real `~/.grokbot-tui`): save is atomic, the file
  round-trips (save → load → equal), duplicate/new/delete, reset default override, concurrent
  modification detection.
- **Editor tests with `TestBackend`:** opening via Shift-F5 / `/theme edit`; navigating slots;
  entering a valid and an invalid color (field error, save refused); that the preview's buffer
  reflects an edited `accent` before saving; save / revert / Esc-with-changes prompt; mouse
  clicks on the slot list and palette grid.
- **Header tests:** the theme name segment is clickable in tall and compact headers and opens the
  pick list; F5 opens it; picking applies and saves `ui.theme`.
- **Help/registry tests:** `/theme` subcommands get help entries (the existing registry test
  enforces this).
- **Manual:** the dark and light variants in a few real terminals (GNOME Console, kitty, tmux)
  and in a 16-color `TERM=xterm`.

## 11. Version and steps

Target **1.4.0** (new feature, backward compatible). Suggested commits:

1. `themes/default.toml` + `Theme` model + parser + unit tests (no behavior change).
2. Style functions read the active theme. Golden test, remove the hard-coded palettes.
3. Theme store in `<data dir>/themes`: load / list / save atomically / delete / reset-default,
   and warnings surfaced as notices.
4. `ui.theme` pref + `/theme use|list|reload|export` + header theme switcher + F5 pick list.
5. Editor modal: slot list, live preview (off-screen render), navigation.
6. Editor editing: color field + palette grid + style form + validation + contrast hint.
7. Editor file ops: save / revert / duplicate / new / delete / reset, and the unsaved-changes prompt.
8. Help topic "themes", README, CHANGELOG `[1.4.0]`, version bump, full verification
   (fmt, clippy -D warnings, tests, release build, `dist generate --check`), tag v1.4.0.
