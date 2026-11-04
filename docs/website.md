# Website

The site at <https://weirdbrains.github.io/grokbot-tui/> is static HTML built from this
repository: a front page (hero screenshot, features, downloads, screenshots), a theme
gallery with a screenshot of every built-in theme in dark and light, and these docs as
pages under `/docs/`. GitHub Pages serves it from the `gh-pages` branch, the same flow as
Muninn's site.

| file | what |
|------|------|
| `site/build.py` | builds the site into `target/site/` (front page, `/themes/`, `/docs/<slug>/`) |
| `site/check.py` | checks a build: every page, every internal link and image resolves |
| `site/style.css` | the layout and colours (the default theme's dark palette) |
| `site/shots.tsv` | the screenshot list (name, caption), written by `scripts/screenshots.sh` |
| `site/screenshots/` | the PNGs (1524x864, 256 colours) |
| `scripts/screenshots.sh` | regenerates the screenshots |

Every `docs/**/*.md` except `docs/historical/` is published: `docs/README.md` is `/docs/`,
`docs/search.md` is `/docs/search/`, `docs/reference/help.md` is
`/docs/reference/help/`. Links between docs become links between pages; links to other
files in the repository (the man page, `CHANGELOG.md`, …) go to GitHub. Each page has an
"Edit this page on GitHub" link. The pages are generated from `docs/` on every build, so
they never drift.

## Building

Needs Python 3 with the `markdown` package (`python3-markdown` on Debian/Ubuntu, or
`pip install --user markdown`). Nothing else; the Rust build and the release workflows
never run it.

```sh
python3 site/build.py          # -> target/site/
python3 site/check.py          # links and images
python3 -m http.server -d target/site 8080   # preview at http://localhost:8080/grokbot-tui/ (see below)
```

The site is built for the `/grokbot-tui/` path prefix (`BASE` in `build.py`). For a local
preview, `python3 site/build.py --base /` builds it for the server root instead.

## Screenshots

`scripts/screenshots.sh` redoes every screenshot. It runs the ignored test
`app::screenshots::site_screenshots`, which drives the TUI with the offline `--mock` API
in a temp data dir (never `~/.grokbot-tui`, never a real account or message) and renders
each screen (via ratatui's `TestBackend`) to an HTML grid of coloured cells. Headless
Chromium turns each page into a PNG (no window opens) and ImageMagick shrinks it into
`site/screenshots/`; `site/shots.tsv` is rewritten. It needs `chromium` and `magick`.

The shots: the chat with the bot list on the left and the attachments bar pulled out on
the right, the same chat with that bar tucked away, search, help, the theme picker, and
every theme in `themes/` in dark and light. Run it whenever a view or a theme changes,
then rebuild and redeploy the site. To add a shot, add a `s.shot(...)` call in
`src/screenshots.rs`.

## Deploying

The `gh-pages` branch holds only the built site and `.nojekyll`. To deploy after a change:

```sh
python3 site/build.py && python3 site/check.py
git worktree add /tmp/gbt-pages gh-pages
rsync -a --delete --exclude .git --exclude CNAME --exclude .nojekyll target/site/ /tmp/gbt-pages/
git -C /tmp/gbt-pages add -A
git -C /tmp/gbt-pages commit -m "site: build of main $(git rev-parse --short HEAD)"
git -C /tmp/gbt-pages push origin gh-pages
git worktree remove /tmp/gbt-pages
```

Pages rebuilds within a minute or two
(`gh api repos/WeirdBrains/grokbot-tui/pages/builds/latest`). Pages is set to "Deploy
from a branch: `gh-pages` / root"; the repository's homepage points at the site.

First-time setup (already done): create an orphan `gh-pages` branch with the build and
`.nojekyll`, push it, then
`gh api -X POST repos/WeirdBrains/grokbot-tui/pages -f 'source[branch]=gh-pages' -f 'source[path]=/'`.

## Adding a custom domain later

1. DNS: a `CNAME` record from the subdomain (e.g. `grokbot-tui.weirdbrains.com`) to
   `weirdbrains.github.io` (DNS only, no proxy). For an apex domain use GitHub's A records
   (185.199.108.153, 185.199.109.153, 185.199.110.153, 185.199.111.153) instead.
2. Build with `python3 site/build.py --base /` (the site moves from `/grokbot-tui/` to the
   root) and add a `CNAME` file containing the domain to the `gh-pages` branch (the deploy
   above keeps it).
3. `gh api -X PUT repos/WeirdBrains/grokbot-tui/pages -f cname=grokbot-tui.weirdbrains.com`,
   then turn on "Enforce HTTPS" once GitHub has issued the certificate.
4. Change `BASE` in `site/build.py`, the homepage in `Cargo.toml`, the links in
   `README.md` and `docs/`, and `gh repo edit --homepage`.
