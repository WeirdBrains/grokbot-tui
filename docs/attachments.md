# Attachments and the file picker

Sending files, the attach button, downloads, and media playback.

## Attachments

`/attach <path…>` (alias `/file`) attaches files to your next prompt, up to 6. Tab completes
paths (cycling replaces the path, it never appends), and pasting a file path attaches the file.
Uploads start right away. Chips above the input show each file's icon, name, size and
progress (✓ done, ✗ failed). Click a chip to preview an image, and click ✕ to remove it and
abort its upload. `/attach remove <n>`, `/attach clear` and `/detach [n|all]` remove from the
keyboard. Sending waits for uploads to finish. A failed upload keeps your prompt in the input.

Any file type can be attached. The client checks sizes up front: images 25 MiB, video 1 GiB,
everything else 250 MiB. The server has the final say and its refusal reason is shown. A bot
that only takes files through the desktop app says so. Attachments need a Cursor session
(`/login desktop` or `/login cursor`); plain gateway logins can't upload.

Sent prompts show their files as `[🖼 name size ⤓]` chips. Click the name to open an image in
the viewer, or ⤓ for the download modal. The link is resolved when you click and is never
shown. The gateway transcript doesn't record attachments, so the chips only appear on prompts
sent from this grokbot-tui. Theme parts: `attach_chip`, `attach_chip_error`.

## Downloads

Bot replies with images or file links (`.pdf`, `.zip`, `.csv`…; not web pages) get a row of
`[⤓] name` buttons (theme part `download_button`). Click one, run `/download [n|list]`
(alias `/save`), or press vim `d` (`message.download`) to open the download modal. Its path
is prefilled with `~/Downloads/<name>` (or `~/<name>` without a Downloads folder). Tab
completes paths, and Down moves into a folder browser (arrows/hjkl, Enter/l opens a folder,
h goes to the parent). Overwriting a file and creating missing folders are confirmed first.
Remote files stream in the background into `<name>.part` and are renamed when done. Local
files are copied. The size is capped by `downloads.max_mb` (default 25). The result shows as
a notification. Theme parts: `download_path`, `download_dir`.

## Attach button (1.35.0)

The **[📎 Attach]** button on the input box's top border starts `/attach ` and lists files
to pick (Tab completes paths). If you're mid-draft it keeps your text; paste a file path
or use `/attach <path>` instead. Theme part `attach_button`.

## Video, GIFs and music (1.24.0)

Videos and animated GIFs play **inside the bubble**, scaled like images (capped at
`images.inline_max_rows`), with a one-row bar under them: **[▶]/[⏸]** toggles playback,
clicking the bar seeks, and the label shows elapsed/total time (GIFs: frame position).
Clicking the picture opens the full-size player (Space play/pause, ←/→ seek 5 s, Home,
Esc/q close). Music links and attachments get the same bar. Only one video or track plays
at a time, and it pauses when scrolled out of view; GIFs loop on their own.

| Build | Decodes | Download |
|---|---|---|
| standard | GIF; H.264 MP4 (OpenH264, compiled in); MP3, AAC/M4A, FLAC, Vorbis, WAV. Other video codecs use a system `ffmpeg` if one is on PATH | `grokbot-tui-installer.sh/.ps1`, .deb/.rpm/.msi/.pkg, Homebrew, AUR |
| -ffmpeg | the above, plus FFmpeg linked in statically: nearly every codec and container, nothing to install | `grokbot-tui-ffmpeg-<target>.tar.xz/.zip` |

`grokbot-tui-setup.sh` / `grokbot-tui-setup.ps1` (release assets) ask which build to
install (`--variant standard|ffmpeg` / `-Variant` to skip the question).

Prefs: `media.inline` (`on`), `media.fps` (`10`, 1–30), `media.decoder`
(`auto`|`builtin`|`ffmpeg`|`system`), `media.gif_autoplay` (`on`). `/media
pause|stop|decoders`. Theme part: `media_bar`. On Linux, sound needs ALSA (libasound2).
