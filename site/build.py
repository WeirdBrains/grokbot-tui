#!/usr/bin/env python3
"""Build the grokbot-tui website into target/site/ (see docs/website.md)."""
import html, os, re, shutil, sys
from pathlib import Path
import markdown

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "target" / "site"
BASE = "/"
if "--base" in sys.argv:
    BASE = sys.argv[sys.argv.index("--base") + 1]
    BASE = BASE if BASE.endswith("/") else BASE + "/"
REPO = "https://github.com/WeirdBrains/grokbot-tui"
DL = REPO + "/releases/latest/download/"
VERSION = re.search(r'^version = "([^"]+)"', (ROOT / "Cargo.toml").read_text(), re.M).group(1)

def is_theme(n):
    return n.startswith("theme-") and n.endswith(("-dark", "-light"))

def shots():
    rows = []
    for line in (ROOT / "site" / "shots.tsv").read_text().splitlines():
        if line.strip():
            n, c = line.split("\t", 1)
            rows.append((n, c))
    return rows

def page(title, body, active="", desc="A terminal UI for Grok Bot"):
    nav = [("", "Home"), ("themes/", "Themes"), ("docs/", "Docs"), ("#download", "Download")]
    links = "".join(
        f'<a href="{BASE}{h}" class="{"on" if h == active else ""}">{t}</a>' if not h.startswith("#")
        else f'<a href="{BASE}{h}">{t}</a>'
        for h, t in nav
    )
    return f"""<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{html.escape(title)}</title>
<meta name="description" content="{html.escape(desc)}">
<link rel="stylesheet" href="{BASE}style.css">
</head><body>
<header class="top"><a class="brand" href="{BASE}"><span class="tag">grokbot-tui</span></a>
<nav>{links}<a href="{REPO}">GitHub</a></nav></header>
{body}
<footer>grokbot-tui {VERSION} · GPL-3.0-or-later · unofficial client for Grok Bot ·
<a href="{REPO}">source</a> · <a href="{REPO}/releases">releases</a> · <a href="{BASE}docs/">docs</a></footer>
</body></html>
"""

FEATURES = [
    ("Every bot, one terminal", "Your bots, team bots and groups in a sidebar; chat in rounded bubbles with reactions on the corner, like the desktop app.", "chats"),
    ("Log in with the desktop app", "<code>/login desktop</code> reuses the Grok Bot desktop session; no token copying. Several accounts stay apart.", "login"),
    ("Attachments", "Send files with the picker or drag and drop; a files bar on the right lists every attachment in the chat.", "attachments"),
    ("Real images", "Inline images over kitty, iTerm2, sixel or the Fuzzball protocol, half blocks everywhere else; video and GIFs with the FFmpeg build.", "images"),
    ("Search everything", "Ctrl-F searches every bot's messages and attachments locally, with a full-text index that never blocks the UI.", "search"),
    ("Themes", "Nine built-in themes in dark and light, a live picker and an in-app theme editor.", "themes"),
    ("Your keys", "Rebind anything with a keymap file, or turn on vim-like keys.", "keybindings"),
    ("Built-in browser", "Open links in a text-mode browser, even over SSH.", "browser"),
    ("Notifications", "Grok Bot notifications, toasts and rules, with a menu to catch up.", "notifications"),
    ("MCP server", "Let Claude Code, Cursor or any MCP client talk to your bots through grokbot-tui.", "mcp"),
]

DOWNLOADS = [
    ("Linux / macOS", "Installer script", f"curl --proto '=https' --tlsv1.2 -LsSf {DL}grokbot-tui-installer.sh | sh", [
        ("x86_64 .tar.xz", "grokbot-tui-x86_64-unknown-linux-gnu.tar.xz"),
        ("aarch64 .tar.xz", "grokbot-tui-aarch64-unknown-linux-gnu.tar.xz"),
        ("macOS Apple silicon .pkg", "grokbot-tui-aarch64-apple-darwin.pkg"),
        ("macOS Intel .pkg", "grokbot-tui-x86_64-apple-darwin.pkg"),
    ]),
    ("Windows", "PowerShell installer", f'powershell -ExecutionPolicy Bypass -c "irm {DL}grokbot-tui-installer.ps1 | iex"', [
        ("x64 .msi", "grokbot-tui-x86_64-pc-windows-msvc.msi"),
        ("x64 .zip", "grokbot-tui-x86_64-pc-windows-msvc.zip"),
        ("x64 with FFmpeg .zip", "grokbot-tui-ffmpeg-x86_64-pc-windows-msvc.zip"),
    ]),
    ("Debian, Ubuntu, Fedora", "Packages", "sudo apt install ./grokbot-tui_<version>-1_amd64.deb   # or: sudo dnf install ./grokbot-tui-<version>-1.x86_64.rpm", [
        ("All .deb / .rpm on the release page", None),
    ]),
]

def img(name, caption, lazy=True):
    return (f'<figure><a href="{BASE}screenshots/{name}.png"><img src="{BASE}screenshots/{name}.png" '
            f'alt="{html.escape(caption)}" width="1524" height="864"{" loading=lazy" if lazy else ""}></a>'
            f'<figcaption>{html.escape(caption)}</figcaption></figure>')

def front():
    sh = dict(shots())
    feats = "".join(f'<div class="card"><h3>{t}</h3><p>{d}</p><a href="{BASE}docs/{l}/">Read the guide →</a></div>' for t, d, l in FEATURES)
    dl = ""
    for os_, how, cmd, files in DOWNLOADS:
        fl = "".join(
            f'<li><a href="{DL}{f}">{html.escape(n)}</a></li>' if f else f'<li><a href="{REPO}/releases/latest">{html.escape(n)}</a></li>'
            for n, f in files)
        dl += f'<div class="card"><h3>{os_}</h3><p>{how}:</p><pre><code>{html.escape(cmd)}</code></pre><ul>{fl}</ul></div>'
    gallery = "".join(img(n, c) for n, c in shots() if not is_theme(n) and n != "chat")
    body = f"""<main>
<section class="hero">
<h1>Grok Bot in your terminal</h1>
<p class="lead">grokbot-tui is a fast, keyboard-first terminal client for Grok Bot: every bot and team bot,
rounded message bubbles, inline images, attachments, local search and themes. Written in Rust.</p>
<p class="cta"><a class="btn" href="#download">Download {VERSION}</a> <a class="btn ghost" href="{BASE}docs/">Read the docs</a>
<a class="btn ghost" href="{BASE}themes/">Browse themes</a></p>
{img("chat", sh.get("chat", "grokbot-tui"), lazy=False)}
<p class="note">Unofficial: grokbot-tui uses Grok Bot's undocumented gateway API, which can change at any time.
Try it offline first with <code>grokbot-tui --mock</code>.</p>
</section>
<section><h2>Features</h2><div class="grid">{feats}</div></section>
<section><h2>Screenshots</h2><div class="shots">{gallery}</div>
<p><a href="{BASE}themes/">Every theme, dark and light →</a></p></section>
<section id="download"><h2>Download</h2>
<p>The links always point at the <a href="{REPO}/releases/latest">latest release</a> (checksums next to each file).
More options in the <a href="{BASE}docs/install/">install guide</a>.</p>
<div class="grid">{dl}</div></section>
</main>"""
    return page("grokbot-tui: Grok Bot in your terminal", body, "")

def themes_page():
    th = [(n, c) for n, c in shots() if is_theme(n)]
    names = []
    for n, c in th:
        tid = n[len("theme-"):].rsplit("-", 1)[0]
        if tid not in names:
            names.append(tid)
    blocks = ""
    for tid in names:
        pair = [(n, c) for n, c in th if n.startswith(f"theme-{tid}-")]
        title = pair[0][1].rsplit(" (", 1)[0]
        blocks += (f'<section class="theme" id="{tid}"><h2>{html.escape(title)} <code>{tid}</code></h2>'
                   f'<div class="pair">{"".join(img(n, c) for n, c in pair)}</div>'
                   f'<p><code>/theme use {tid}</code> · <a href="{REPO}/blob/main/themes/{tid}.toml">{tid}.toml</a></p></section>')
    toc = " · ".join(f'<a href="#{t}">{t}</a>' for t in names)
    body = f"""<main><section><h1>Themes</h1>
<p class="lead">{len(names)} built-in themes, each with a dark and a light variant ({len(th)} screenshots).
Press <kbd>F5</kbd> or type <code>/theme use &lt;name&gt;</code> to switch, <kbd>F4</kbd> to cycle dark, light and system,
and <kbd>Shift-F5</kbd> to edit a theme. See the <a href="{BASE}docs/themes/">themes guide</a>.</p>
<p>{toc}</p></section>{blocks}</main>"""
    return page("Themes · grokbot-tui", body, "themes/")

def doc_files():
    out = []
    for p in sorted((ROOT / "docs").rglob("*.md")):
        rel = p.relative_to(ROOT / "docs")
        if rel.parts[0] in ("historical",):
            continue
        out.append(rel)
    return out

def slug(rel):
    s = str(rel.with_suffix("")).replace(os.sep, "/")
    return "" if s == "README" else s

def rewrite(href, rel):
    if re.match(r"^(https?:|mailto:|#)", href):
        return href
    path, _, frag = href.partition("#")
    frag = "#" + frag if frag else ""
    target = (ROOT / "docs" / rel.parent / path).resolve()
    try:
        drel = target.relative_to(ROOT / "docs")
    except ValueError:
        drel = None
    if drel is not None and target.suffix == ".md" and drel.parts[0] != "historical" and target.exists():
        s = slug(drel)
        return f"{BASE}docs/{s + '/' if s else ''}{frag}"
    repo_rel = target.relative_to(ROOT)
    kind = "tree" if target.is_dir() else "blob"
    return f"{REPO}/{kind}/main/{repo_rel.as_posix()}{frag}"

def nav_groups():
    groups, cur = [], None
    for line in (ROOT / "docs" / "README.md").read_text().splitlines():
        if line.startswith("## "):
            cur = (line[3:], [])
            groups.append(cur)
        m = re.match(r"- \[([^\]]+)\]\(([^)]+\.md)\)", line)
        if m and cur:
            cur[1].append((m.group(1), m.group(2)))
    return groups

def docs_pages():
    md = markdown.Markdown(extensions=["tables", "fenced_code", "toc", "sane_lists"])
    groups = nav_groups()
    for rel in doc_files():
        src = (ROOT / "docs" / rel).read_text()
        md.reset()
        h = md.convert(src)
        h = re.sub(r'(href|src)="([^"]+)"', lambda m: f'{m.group(1)}="{rewrite(html.unescape(m.group(2)), rel)}"', h)
        title = re.search(r"^# (.+)$", src, re.M)
        title = title.group(1) if title else slug(rel)
        side = "".join(
            f"<h4>{html.escape(g)}</h4><ul>" + "".join(
                f'<li><a href="{rewrite(href, Path("README.md"))}" class="{"on" if (ROOT/"docs"/href).resolve() == (ROOT/"docs"/rel).resolve() else ""}">{html.escape(t)}</a></li>'
                for t, href in items) + "</ul>"
            for g, items in groups if items)
        body = (f'<main class="docs"><aside><a href="{BASE}docs/"><b>All docs</b></a>{side}</aside>'
                f'<article>{h}<p class="edit"><a href="{REPO}/edit/main/docs/{rel.as_posix()}">Edit this page on GitHub</a></p></article></main>')
        s = slug(rel)
        d = OUT / "docs" / s
        d.mkdir(parents=True, exist_ok=True)
        (d / "index.html").write_text(page(f"{title} · grokbot-tui docs", body, "docs/"))

def main():
    if OUT.exists():
        shutil.rmtree(OUT)
    OUT.mkdir(parents=True)
    shutil.copy(ROOT / "site" / "style.css", OUT / "style.css")
    shutil.copytree(ROOT / "site" / "screenshots", OUT / "screenshots")
    (OUT / ".nojekyll").write_text("")
    # If a CNAME file exists in site/, copy it — GitHub Pages custom domains need it.
    cname = ROOT / "site" / "CNAME"
    if cname.exists():
        shutil.copy(cname, OUT / "CNAME")
    (OUT / "index.html").write_text(front())
    (OUT / "themes").mkdir()
    (OUT / "themes" / "index.html").write_text(themes_page())
    docs_pages()
    (OUT / "404.html").write_text(page("Not found · grokbot-tui", f'<main><section><h1>Not found</h1><p><a href="{BASE}">Home</a> · <a href="{BASE}docs/">Docs</a></p></section></main>'))
    n = sum(1 for _ in OUT.rglob("index.html"))
    print(f"built {n} pages into {OUT} (base {BASE})")

main()
