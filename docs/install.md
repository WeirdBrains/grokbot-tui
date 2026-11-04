# Install and download

How to get grokbot-tui on Linux, macOS and Windows, and how to start it.

## Downloads by OS

Every [release](https://github.com/WeirdBrains/grokbot-tui/releases/latest) has these
files; the links below always point at the latest one.

| OS | file | how |
|----|------|-----|
| Linux, macOS (installer) | [`grokbot-tui-installer.sh`](https://github.com/WeirdBrains/grokbot-tui/releases/latest/download/grokbot-tui-installer.sh) | `curl --proto '=https' --tlsv1.2 -LsSf https://github.com/WeirdBrains/grokbot-tui/releases/latest/download/grokbot-tui-installer.sh \| sh` |
| Windows (installer) | [`grokbot-tui-installer.ps1`](https://github.com/WeirdBrains/grokbot-tui/releases/latest/download/grokbot-tui-installer.ps1) | `powershell -ExecutionPolicy Bypass -c "irm https://github.com/WeirdBrains/grokbot-tui/releases/latest/download/grokbot-tui-installer.ps1 \| iex"` |
| Windows (MSI) | [`grokbot-tui-x86_64-pc-windows-msvc.msi`](https://github.com/WeirdBrains/grokbot-tui/releases/latest/download/grokbot-tui-x86_64-pc-windows-msvc.msi) | double-click; zip: [`…windows-msvc.zip`](https://github.com/WeirdBrains/grokbot-tui/releases/latest/download/grokbot-tui-x86_64-pc-windows-msvc.zip) |
| macOS Apple silicon | [`grokbot-tui-aarch64-apple-darwin.pkg`](https://github.com/WeirdBrains/grokbot-tui/releases/latest/download/grokbot-tui-aarch64-apple-darwin.pkg) | unsigned pkg; or [`.tar.xz`](https://github.com/WeirdBrains/grokbot-tui/releases/latest/download/grokbot-tui-aarch64-apple-darwin.tar.xz) |
| macOS Intel | [`grokbot-tui-x86_64-apple-darwin.pkg`](https://github.com/WeirdBrains/grokbot-tui/releases/latest/download/grokbot-tui-x86_64-apple-darwin.pkg) | unsigned pkg; or [`.tar.xz`](https://github.com/WeirdBrains/grokbot-tui/releases/latest/download/grokbot-tui-x86_64-apple-darwin.tar.xz) |
| Linux x86_64 | [`grokbot-tui-x86_64-unknown-linux-gnu.tar.xz`](https://github.com/WeirdBrains/grokbot-tui/releases/latest/download/grokbot-tui-x86_64-unknown-linux-gnu.tar.xz) | `.deb`/`.rpm` (`grokbot-tui_<ver>-1_amd64.deb`, `grokbot-tui-<ver>-1.x86_64.rpm`) on the release page |
| Linux aarch64 | [`grokbot-tui-aarch64-unknown-linux-gnu.tar.xz`](https://github.com/WeirdBrains/grokbot-tui/releases/latest/download/grokbot-tui-aarch64-unknown-linux-gnu.tar.xz) | `.deb`/`.rpm` (`arm64` / `aarch64`) on the release page |

The `-ffmpeg` archives (`grokbot-tui-ffmpeg-<target>…`) add video playback with FFmpeg
linked in. Each file has a `.sha256` next to it, and `sha256.sum` lists them all.

After installing, run `grokbot-tui --mock` to try the UI offline, then `grokbot-tui` and
[log in](login.md). `man grokbot-tui` (from the `.deb`/`.rpm` or `make install`) lists every
flag and environment variable.

## Install

Prebuilt packages are attached to each [GitHub Release](https://github.com/WeirdBrains/grokbot-tui/releases)
(see [PACKAGING.md](../PACKAGING.md) for how releases are made and what is published where):

- **Shell installer (macOS/Linux):** the `grokbot-tui-installer.sh` command shown in the release notes
- **PowerShell (Windows):** the `grokbot-tui-installer.ps1` command shown in the release notes
- **Windows:** `grokbot-tui-x86_64-pc-windows-msvc.msi` · **macOS:** `grokbot-tui-<arch>.pkg` (unsigned)
- **Debian/Ubuntu:** `sudo apt install ./grokbot-tui_<ver>-1_<arch>.deb`
- **Fedora:** `sudo dnf install ./grokbot-tui-<ver>-1.<arch>.rpm`
- **Homebrew, AUR, winget:** once published (see PACKAGING.md), `brew install WeirdBrains/tap/grokbot-tui`,
  `yay -S grokbot-tui-bin`, `winget install WeirdBrains.grokbot-tui`
- **From source:** `make install` from a checkout (see [Install from a local checkout](./install.md#install-from-a-local-checkout))

### Install from a local checkout

These build the code in front of you and copy the result into place. Nothing is
fetched from crates.io as an install, and no prebuilt binary is downloaded. Building
still needs a Rust toolchain (`cargo`/`rustc`, from rustup or your distro), plus
`libasound2-dev`/`alsa-lib-devel` on Linux.

```sh
make                         # build (release)
make install                 # install to ~/.local (or /usr/local as root)
make install PREFIX=/opt/gbt # any prefix; DESTDIR=... for staged/package installs
make install FFMPEG=1        # -ffmpeg variant (FFmpeg built and linked statically)
make uninstall               # same PREFIX/DESTDIR as the install
make vendor && make install OFFLINE=1   # vendor crates once, then build fully offline
```

Without `make`, run the script directly: `./install-local.sh [--prefix DIR] [--destdir DIR]
[--ffmpeg] [--vendor] [--offline] [--no-build] [--uninstall]`. On Windows:
`.\install-local.ps1 [-Prefix DIR] [-Ffmpeg] [-Vendor] [-Offline] [-Uninstall]` (default
`%LOCALAPPDATA%\Programs\grokbot-tui`, added to your user PATH; `-Ffmpeg` needs vcpkg FFmpeg).

The install puts the binary in `PREFIX/bin`, docs and `THIRD_PARTY.md` in
`PREFIX/share/doc/grokbot-tui`, the licence in `PREFIX/share/licenses/grokbot-tui`, and
the example themes, keymap and notify rules in `PREFIX/share/grokbot-tui`. Uninstall
removes only those; your settings and history stay.

## Run

```sh
cargo run -- --mock                        # offline: fake bots, delayed echo replies
cargo run                                  # real gateway
make install                               # builds and installs the `grokbot-tui` binary
grokbot-tui --data-dir /tmp/gbtest         # different data directory (holds context.db)
grokbot-tui --account work                 # start in an account (or GROKBOT_TUI_ACCOUNT=work)
grokbot-tui --theme solar                  # use a theme for this run only (not saved)
grokbot-tui --list-themes                  # list the theme files (● = in use) and exit
```

In mock mode any token logs in except `bad`. Sending `reject` simulates a 4xx and `unknown` simulates a 5xx after the prompt has landed.
