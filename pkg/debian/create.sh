#!/usr/bin/env bash
# Build a .deb into pkg/artifacts/.
# The binary is a static musl build with libreflect.a linked in. No runtime library deps.
# Set SINKD_BIN / SINKD_SRV_BIN to override.
set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(cd -- "$SCRIPT_DIR/../.." && pwd)

if ! command -v dpkg-deb >/dev/null 2>&1; then
  echo "dpkg-deb not found. Install dpkg-dev (e.g. apt install dpkg-dev)." >&2
  exit 1
fi

# Allow callers (CI, container builds) to inject the version directly to avoid
# requiring `bump` on the build host.
if [[ -n "${SINKD_VERSION:-}" ]]; then
  VERSION="${SINKD_VERSION}"
elif command -v bump >/dev/null 2>&1; then
  VERSION=$(bump print)
else
  # Last-resort fallback: lift the base version from Cargo.toml.
  VERSION=$(awk -F'"' '/^version[[:space:]]*=/ { split($2, a, "+"); print a[1]; exit }' "${ROOT}/sinkd-client/Cargo.toml")
fi

if [[ -z "${VERSION}" ]]; then
  echo "could not determine version (set SINKD_VERSION, install bump, or fix Cargo.toml)." >&2
  exit 1
fi

DEB_ARCH=$(dpkg --print-architecture)
case "${DEB_ARCH}" in
  amd64) MUSL_TARGET=x86_64-unknown-linux-musl ;;
  arm64) MUSL_TARGET=aarch64-unknown-linux-musl ;;
  *) echo "unsupported architecture ${DEB_ARCH}" >&2; exit 1 ;;
esac

OUT="${ROOT}/pkg/artifacts"
mkdir -p "${OUT}"

STAGE=$(mktemp -d)
cleanup() { rm -rf "${STAGE}"; }
trap cleanup EXIT

install -d "${STAGE}/usr/bin" \
  "${STAGE}/etc" \
  "${STAGE}/usr/share/sinkd" \
  "${STAGE}/usr/share/doc/sinkd" \
  "${STAGE}/etc/skel/.config/sinkd" \
  "${STAGE}/DEBIAN"

TARGET_DIR="${CARGO_TARGET_DIR:-${ROOT}/target}"
BIN="${SINKD_BIN:-${TARGET_DIR}/${MUSL_TARGET}/release/sinkd}"
BIN_SRV="${SINKD_SRV_BIN:-${TARGET_DIR}/${MUSL_TARGET}/release/sinkd-srv}"

if [[ ! -f "${BIN}" ]]; then
  echo "Missing binary at ${BIN}. cargo build -p sinkd --release --target ${MUSL_TARGET}, or set SINKD_BIN." >&2
  exit 1
fi
if [[ ! -f "${BIN_SRV}" ]]; then
  echo "Missing binary at ${BIN_SRV}. cargo build -p sinkd-srv --release --target ${MUSL_TARGET}, or set SINKD_SRV_BIN." >&2
  exit 1
fi

install "${BIN}" "${STAGE}/usr/bin/sinkd"
install "${BIN_SRV}" "${STAGE}/usr/bin/sinkd-srv"
install -m 644 LICENSE "${STAGE}/usr/share/doc/sinkd/copyright"
install -d "${STAGE}/etc/sinkd" "${STAGE}/usr/lib/systemd/system" "${STAGE}/usr/lib/systemd/user"
install -m 644 "${ROOT}/sinkd-cfg/server.toml" "${STAGE}/etc/sinkd/server.toml"
install -m 644 "${ROOT}/sinkd-cfg/client.toml" "${STAGE}/usr/share/sinkd/client.toml"
install -m 644 "${ROOT}/pkg/systemd/sinkd-srv.service" "${STAGE}/usr/lib/systemd/system/sinkd-srv.service"
install -m 644 "${ROOT}/pkg/systemd/sinkd.service" "${STAGE}/usr/lib/systemd/user/sinkd.service"

cat >"${STAGE}/DEBIAN/control" <<EOF
Package: sinkd
Version: ${VERSION}
Architecture: ${DEB_ARCH}
Maintainer: Tony <krakjn@gmail.com>
Section: utils
Priority: optional
Description: Deployable Cloud, file sync daemon.
 bring everything and the kitchen sink to your local cloud.
EOF

dpkg-deb --root-owner-group --build "${STAGE}" "${OUT}/sinkd_${VERSION}_${DEB_ARCH}.deb"
