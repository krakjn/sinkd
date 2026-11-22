#!/usr/bin/env bash
# Assemble the static site and fill _site/dl from the rolling prerelease.
# When that release is from an older commit (the push-triggered Workers build
# runs before GitHub Actions publishes), writes .skip-deploy and exits 0 so
# scripts/deploy-site.sh skips wrangler; the deploy-hook build ships it.
#
# Private repo: set GITHUB_TOKEN (or GH_TOKEN) in Workers Builds variables.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
rm -f .skip-deploy
bash "$root/scripts/assemble-site.sh"

gh_token="${GITHUB_TOKEN:-${GH_TOKEN:-}}"
gh_api=(curl -fsSL)
if [ -n "$gh_token" ]; then
    gh_api+=(-H "Authorization: Bearer $gh_token" -H "Accept: application/vnd.github+json")
fi

head=$(git rev-parse HEAD)
mkdir -p _site/dl
manifest=$(mktemp)
trap 'rm -f "$manifest"' EXIT

if ! "${gh_api[@]}" "https://api.github.com/repos/octo-works/sinkd/releases/tags/rolling" -o "$manifest"; then
    if [ -z "$gh_token" ]; then
        echo "prepare-site: could not fetch rolling release (404 often means the repo is private; set GITHUB_TOKEN in Workers Builds variables)" >&2
    else
        echo "prepare-site: could not fetch rolling release from GitHub API" >&2
    fi
    exit 1
fi

export GITHUB_TOKEN="${gh_token}"
python3 - "$manifest" _site/dl <<'PY'
import json, os, sys, urllib.request

manifest, dest = sys.argv[1], sys.argv[2]
token = os.environ.get("GITHUB_TOKEN", "")
data = json.load(open(manifest))
if data.get("message") and not data.get("assets"):
    sys.exit(data["message"])
assets = data.get("assets") or []
if not assets:
    sys.exit("rolling release has no assets")

def fetch(url, path):
    req = urllib.request.Request(url)
    if token:
        req.add_header("Authorization", f"Bearer {token}")
        req.add_header("Accept", "application/octet-stream")
    with urllib.request.urlopen(req) as resp, open(path, "wb") as out:
        out.write(resp.read())

for asset in assets:
    name = os.path.basename(asset["name"])
    path = os.path.join(dest, name)
    url = asset["url"] if token else asset["browser_download_url"]
    fetch(url, path)
    print(f"downloaded {name}")
PY

commit=$(tr -d '[:space:]' < _site/dl/COMMIT)
if [ "$commit" != "$head" ]; then
    printf 'rolling COMMIT %s does not match HEAD %s; skipping deploy until the deploy hook runs\n' "$commit" "$head"
    touch .skip-deploy
    exit 0
fi
if [ ! -s _site/dl/SHA256SUMS ]; then
    echo "SHA256SUMS missing from rolling release" >&2
    exit 1
fi
if [ ! -s _site/dl/VERSION ]; then
    echo "VERSION missing from rolling release" >&2
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
