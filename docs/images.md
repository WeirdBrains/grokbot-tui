# Images and terminal graphics

Inline images (kitty, iTerm2, sixel, halfblocks) and the Fuzzball keyboard and graphics protocol.

## Images

Images attached to a transcript message (your uploads and files the bot sent) draw
inside that chat bubble, scaled to the bubble width, using the same kitty / sixel /
iTerm2 / halfblocks path as markdown image URLs. A `[🖼 name]` placeholder stands in
until the image loads; click it to open the viewer.

Images in replies (`![alt](url)`, bare image URLs and paths) show as a placeholder,
`[🖼 alt · cat.png · 12.0 KB]` (size for local files), followed by the URL. **Click the
placeholder** (or `/view`, vim `O`) to open the image viewer: the image as large as the
terminal allows, aspect ratio kept, with its name, pixel size, file size and position
(`2/5`) in the title. The graphics protocol is detected at start-up with ratatui-image (kitty,
sixel, iTerm2; halfblocks everywhere else, e.g. the Windows console or Alacritty);
**Fuzzball (1.36.0).** In [Mouser](https://github.com/WeirdBrains/mouser), images use the
Fuzzball graphics protocol (`docs/protocol/graphics.md`): grokbot-tui sends one capability
query plus DA1 at startup (no delay on other terminals; tmux gets a short extra wait), uploads
each picture as base64 RGBA in `max_chunk` pieces and places it at its cells. tmux/screen
passthrough wrapping is applied when `TMUX`/`STY` is set. The order is
`images.protocol = auto` → Fuzzball, then kitty, then native (sixel / iTerm2 / halfblocks);
set `fuzzball`, `kitty` or `native` to force a step (unavailable choices fall through with a
notice). `/graphics` shows what answered; `/graphics native` etc. sets the pref.
`GROKBOT_TUI_FUZZBALL=off` skips the Fuzzball query. Keyboard: `keys.protocol = auto |
fuzzball | kitty | legacy` (`keys.kitty_protocol` still works). With `auto`/`fuzzball`,
Fuzzball keyboard events are used when the terminal offers them (not through tmux/screen),
then the kitty keyboard protocol, then legacy keys. Every startup query is bounded
(`graphics.detect_timeout_ms`, clamped to 1–5 s), so a terminal that never answers still
gets the UI, with halfblocks images and legacy keys.

`GROKBOT_TUI_IMAGES=halfblocks` skips the terminal query. Viewer keys: `Esc`/`q` or a click
outside close, `o`/`Enter` open in the system viewer, `←`/`→` (`n`/`p`, `h`/`l`) step through
the chat's images. Closing or resizing clears the screen (and deletes kitty images) so no image
ghosts stay. Images are read from disk or downloaded (http/https, up to 25 MB) only when you
open the viewer. `/open` opens images and links externally; `/open list` numbers them.

**Attaching an image to a prompt isn't possible:** grokbot_api 1.1's `sendPrompt` takes text
only (agent id, prompt, nonce, reply-to) and there is no upload method. Theme parts:
`image_placeholder`, `image_viewer_info` (the viewer uses `help_frame` / `help_title`).

## Keyboard protocol

On terminals with the kitty keyboard protocol (kitty, Ghostty, foot, WezTerm, recent
Alacritty/iTerm2), grokbot-tui turns it on (disambiguate, event types and alternate keys),
so Alt, Ctrl-Enter and Shift-Enter are reliable and Esc has no delay. Elsewhere the ESC-prefix
Alt handling is used. Pref `keys.kitty_protocol = auto|on|off` (default `auto`);
`/kitty` shows or changes it. The flags are popped on exit and on suspend.
