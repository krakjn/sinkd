#!/usr/bin/env bash
# Build static musl sinkd + sinkd-srv into the workspace target/ tree and pack a .deb.
# Invoked from the repo root by build-musl.sh (Docker) or directly on Linux with musl-tools.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT}"

if ! command -v dpkg-deb >/dev/null 2>&1; then
  echo "build-musl-inner: dpkg-deb not found (install dpkg-dev)" >&2
  exit 1
fi

case "$(dpkg --print-architecture 2>/dev/null || uname -m)" in
  amd64|x86_64) MUSL_TARGET=x86_64-unknown-linux-musl ;;
  arm64|aarch64) MUSL_TARGET=aarch64-unknown-linux-musl ;;
  *)
    echo "build-musl-inner: unsupported architecture" >&2
    exit 1
    ;;
esac

export SINKD_VERSION="${SINKD_VERSION:-0.0.0-compose}"

if command -v rustup >/dev/null 2>&1; then
  rustup target add "${MUSL_TARGET}"
fi

echo "[build-musl] target=${MUSL_TARGET} (incremental cache: ${ROOT}/target)"
cargo build --release --target "${MUSL_TARGET}" -p sinkd -p sinkd-srv

SINKD_BIN="${ROOT}/target/${MUSL_TARGET}/release/sinkd" \
SINKD_SRV_BIN="${ROOT}/target/${MUSL_TARGET}/release/sinkd-srv" \
  bash pkg/debian/create.sh

echo "[build-musl] packaged $(ls -1 "${ROOT}"/pkg/artifacts/sinkd_"${SINKD_VERSION}"_*.deb)"
