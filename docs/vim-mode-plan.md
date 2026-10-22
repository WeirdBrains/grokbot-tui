# Vim-like modes and keymap (1.9.0)

## Source

Taken from Lenin's design plan in **acp-rs-scraper**:

- Path: `architecture/PLAN-MODES-KEYMAP.md`
- Commit: `3c84eb2ee3b8d6db7b8a4137829158ea3c1a350a` (HEAD of
  https://github.com/m3talsmith/acp-rs-scraper when this was adopted)
- Status there: **design** (not yet implemented in Red Star)

We follow the gist, not a literal port. Deviations are listed below.

## Gist of the source plan

- Modal input: one current mode, selected by focus / object kind.
- Global actions are Ctrl chords; other keys are mode hotkeys.
- Bindings live in a TOML keymap (`~/.acp/keymap.toml` there); bad files error
  without crashing and keep the previous map.
- Action ids are a fixed set; duplicate keys in one table are conflicts.
- Help / tooltips render the active binding, not a hardcoded letter.
- Reload through one function (`keymap.reload`).

## What grokbot-tui implements

| Area | Behaviour |
|---|---|
| Opt-in | Pref `ui.keys = default \| vim` (default keeps today's bindings). |
| Modes | `insert` (typing), `normal` (motions / panes), `command` (`:` → slash), `visual` (selection). |
| Indicator | Status / input title shows the mode; theme parts `mode_normal`, `mode_insert`, `mode_command`, `mode_visual` with starter `@ref` fallbacks to `mode_active` / `mode_inactive`. |
| Motions | `hjkl` / arrows move in normal; `Tab`/`Shift-Tab` switch panes; `i`/`a` enter insert; `v` enters visual (wired to G selection); `Esc` returns to normal. |
| Command | `:` opens a `:` prompt that maps onto slash commands and reuses Tab fuzzy completion from 1.8.0. |
| Keymap | `~/.grokbot-tui/keymap.toml` (shipped default at `keymap/default.toml`). Validation, conflict detection, graceful fallback + notice. `/keymap` lists or reloads (`/keys` alias). |
| Globals | Ctrl chords (Ctrl-C resolver, Ctrl-R history, Ctrl-Q, …) stay global and are not remapped away by mode tables. |

## Deviations from PLAN-MODES-KEYMAP.md

1. **Opt-in, not mandatory.** Red Star plans to teach keymap keys first. We keep
   `ui.keys = default` so existing Ctrl-C / history / selection / theme-editor
   bindings keep working until the user opts into vim.
2. **Modes are editor-shaped, not object-shaped.** Red Star maps focus →
   `post` / `nick` / `menu` / …. We use Neovim-like `normal` / `insert` /
   `command` / `visual` because grokbot-tui is a chat composer, not a feed of
   posts with service capabilities.
3. **Path:** `~/.grokbot-tui/keymap.toml` (and `keymap/default.toml` in the
   repo) instead of `~/.acp/keymap.toml`.
4. **Action ids** are grokbot-tui actions (`app.quit`, `input.history_search`,
   `view.sidebar`, `selection.copy`, slash command names, …), not Red Star
   capability ids (`reply`, `quote`, `plugins.reload`).
5. **`:foo` → `/foo`.** Red Star does not describe a colon command line; we map
   `:` onto our slash command registry so Tab completion from 1.8.0 is shared.
6. **Visual mode integrates with feature G** (body selection + `keyctx`
   Ctrl-C), which Red Star does not have.
7. **No Preferences › Keymap screen yet** — edit the TOML and `/keymap reload`.
8. **Counts (`10j`) and which-key popups** stay out of scope (open questions in
   the source plan §6).

## What Lenin should mirror back in acp-rs-scraper

To keep the two apps aligned when Red Star implements the plan:

- **Keymap TOML shape:** `[global]` plus `[mode.<name>]` tables; `"key" = "action.id"`;
  comments naming the old fixed key; reload action id `keymap.reload`.
- **Theme parts** for the mode indicator: `mode_normal`, `mode_insert`,
  `mode_command`, `mode_visual` (and keep existing `mode_active` /
  `mode_inactive` for the dark/light/system switcher). Starter rules should
  `@ref` something already in the palette so old themes keep working.
- **Conflict / error behaviour:** name the file and line; keep the previous
  map; never exit.
- Optional: adopt `ui.keys = default|vim` (or equivalent) if Red Star wants a
  migration period before keymap becomes mandatory.
