#!/usr/bin/env bash
# Assemble the static site and fill _site/dl from the rolling prerelease.
# Exits 1 unless that release was built from this commit, so a push-triggered
# Workers build does not deploy before GitHub Actions has published the binaries.
#
# Private repo: set GITHUB_TOKEN (or GH_TOKEN) in Workers Builds variables.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
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
    printf 'rolling COMMIT %s does not match HEAD %s (wait for publish job or rebuild after main catches up)\n' "$commit" "$head" >&2
    exit 1
fi
if [ ! -s _site/dl/SHA256SUMS ]; then
    echo "SHA256SUMS missing from rolling release" >&2
    exit 1
fi
