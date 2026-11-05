#!/usr/bin/env bash
# Assemble the static site and fill _site/dl from the rolling prerelease.
# Exits 1 unless that release was built from this commit, so a push-triggered
# Workers build does not deploy before GitHub Actions has published the binaries.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
bash "$root/scripts/assemble-site.sh"

head=$(git rev-parse HEAD)
mkdir -p _site/dl
manifest=$(mktemp)
trap 'rm -f "$manifest"' EXIT
curl -fsSL "https://api.github.com/repos/octo-works/sinkd/releases/tags/rolling" -o "$manifest"

python3 - "$manifest" _site/dl <<'PY'
import json, os, sys, urllib.request

manifest, dest = sys.argv[1], sys.argv[2]
assets = json.load(open(manifest)).get("assets") or []
if not assets:
    sys.exit("rolling release has no assets")
for asset in assets:
    name = os.path.basename(asset["name"])
    path = os.path.join(dest, name)
    urllib.request.urlretrieve(asset["browser_download_url"], path)
    print(f"downloaded {name}")
PY

commit=$(tr -d '[:space:]' < _site/dl/COMMIT)
if [ "$commit" != "$head" ]; then
    printf 'rolling COMMIT %s does not match HEAD %s\n' "$commit" "$head" >&2
    exit 1
fi
if [ ! -s _site/dl/SHA256SUMS ]; then
    echo "SHA256SUMS missing from rolling release" >&2
    exit 1
fi
