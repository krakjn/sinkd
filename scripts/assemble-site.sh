#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
rm -rf _site
mkdir -p _site/assets
cp -R site/. _site/
cp assets/sinkd-slim.svg assets/moonhouse.ttf _site/assets/
