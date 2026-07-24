#!/usr/bin/env bash
# Build musl binaries in the mounted workspace target/ tree using a Rust container.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CARGO_CACHE="${REPO_ROOT}/scenario/output/compose/cargo"
RUST_IMAGE="${SINKD_RUST_IMAGE:-rust:1.92-bookworm}"

mkdir -p "${REPO_ROOT}/target" "${REPO_ROOT}/pkg/artifacts"
mkdir -p "${CARGO_CACHE}/registry" "${CARGO_CACHE}/git"

docker run --rm \
  -v "${REPO_ROOT}:/src" \
  -v "${REPO_ROOT}/target:/src/target" \
  -v "${CARGO_CACHE}/registry:/usr/local/cargo/registry" \
  -v "${CARGO_CACHE}/git:/usr/local/cargo/git" \
  -e SINKD_VERSION="${SINKD_VERSION:-0.0.0-compose}" \
  -w /src \
  "${RUST_IMAGE}" \
  bash -euxo pipefail -c '
    apt-get update
    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      musl-tools dpkg-dev ca-certificates pkg-config
    rm -rf /var/lib/apt/lists/*
    bash scenario/scripts/build-musl-inner.sh
  '
