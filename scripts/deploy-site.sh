#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
if [ -e .skip-deploy ]; then
    echo "deploy-site: rolling release is not for this commit yet; not deploying"
    exit 0
fi
exec npx wrangler deploy
