#!/usr/bin/env bash
# Assemble the static site and fill _site/dl from dist/.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
bash "$root/scripts/assemble-site.sh"

if [ ! -d dist ] || [ -z "$(ls -A dist)" ]; then
    echo "prepare-site: dist/ is empty" >&2
    exit 1
fi
mkdir -p _site/dl
cp -a dist/. _site/dl/
if [ ! -s _site/dl/SHA256SUMS ]; then
    echo "SHA256SUMS missing from dist" >&2
    exit 1
fi
if [ ! -s _site/dl/VERSION ]; then
    echo "VERSION missing from dist" >&2
    exit 1
fi

python3 - _site <<'PY'
import sys
from pathlib import Path

root = Path(sys.argv[1])
dl = root / "dl"
version = (dl / "VERSION").read_text().strip()
if not version:
    sys.exit("VERSION is empty")

rows = []

def add(name, label):
    if (dl / name).is_file():
        rows.append((label, name))

add("sinkd-linux-x86_64.tar.gz", "Linux x86_64 tarball")
add("sinkd-linux-arm64.tar.gz", "Linux arm64 tarball")
add("sinkd-macos-arm64.tar.gz", "macOS Apple silicon tarball")
add("sinkd-macos-x86_64.tar.gz", "macOS Intel tarball")
for name in sorted(p.name for p in dl.glob("sinkd_*_amd64.deb")):
    add(name, "Debian/Ubuntu amd64 package")
for name in sorted(p.name for p in dl.glob("sinkd_*_arm64.deb")):
    add(name, "Debian/Ubuntu arm64 package")
add("SHA256SUMS", "SHA256SUMS")
if not rows:
    sys.exit("no downloadable files in _site/dl")

items = "\n".join(
    f'          <li><a href="dl/{name}"><span>{label}</span><code>{name}</code></a></li>'
    for label, name in rows
)
page = root / "downloads.html"
raw = page.read_text()
if "__VERSION__" not in raw or "<!-- downloads -->" not in raw:
    sys.exit("downloads page is missing stamp markers")
page.write_text(raw.replace("__VERSION__", version).replace("<!-- downloads -->", items))
index = root / "index.html"
index_raw = index.read_text()
if "__VERSION__" not in index_raw:
    sys.exit("index.html is missing the version marker")
index.write_text(index_raw.replace("__VERSION__", version))
print(f"stamped version {version}")
PY
