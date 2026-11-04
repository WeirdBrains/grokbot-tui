#!/usr/bin/env python3
"""Check a built site (target/site): every internal link and image resolves, every doc has a page."""
import re, sys
from pathlib import Path
ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "target" / "site"
base = "/grokbot-tui/"
if "--base" in sys.argv:
    base = sys.argv[sys.argv.index("--base") + 1]
bad = []
pages = list(OUT.rglob("*.html"))
for p in pages:
    s = p.read_text()
    for m in re.finditer(r'(?:href|src)="([^"]+)"', s):
        u = m.group(1).split("#")[0]
        if not u.startswith(base):
            continue
        t = OUT / u[len(base):]
        if u.endswith("/") or t.is_dir():
            t = t / "index.html"
        if not t.exists():
            bad.append(f"{p.relative_to(OUT)}: {m.group(1)}")
for d in (ROOT / "docs").rglob("*.md"):
    rel = d.relative_to(ROOT / "docs")
    if rel.parts[0] == "historical":
        continue
    s = str(rel.with_suffix(""))
    s = "" if s == "README" else s + "/"
    if not (OUT / "docs" / s / "index.html").exists():
        bad.append(f"no page for docs/{rel}")
themes = list((ROOT / "themes").glob("*.toml"))
tp = (OUT / "themes" / "index.html").read_text()
for t in themes:
    for v in ("dark", "light"):
        if f"theme-{t.stem}-{v}.png" not in tp:
            bad.append(f"themes page misses {t.stem} ({v})")
for need in ("chat.png", 'id="download"', "releases/latest/download/"):
    if need not in (OUT / "index.html").read_text():
        bad.append(f"front page misses {need}")
if bad:
    print("\n".join(bad))
    sys.exit(1)
print(f"ok: {len(pages)} pages, {len(themes) * 2} theme shots, links and images resolve")
