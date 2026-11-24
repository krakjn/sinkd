#!/usr/bin/env bash
# Build a .deb into pkg/artifacts/.
# The binary is a static musl build with libreflect.a linked in. No runtime library deps.
# Set SINKD_BIN / SINKD_SRV_BIN to override.
# Version is `bump print` from https://github.com/krakjn/bump (bump.toml).
# Homepage is https://sinkd.octoworks.dev: that DNS name is this repo's GitHub Pages site.
set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(cd -- "$SCRIPT_DIR/../.." && pwd)

if ! command -v dpkg-deb >/dev/null 2>&1; then
  echo "dpkg-deb not found. Install dpkg-dev (e.g. apt install dpkg-dev)." >&2
  exit 1
fi

if ! command -v bump >/dev/null 2>&1; then
  echo "bump not found. Install bump (https://github.com/krakjn/bump) and ensure it is on PATH." >&2
  exit 1
fi

VERSION=$(bump print)

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
install -d "${STAGE}/etc/sinkd" "${STAGE}/usr/lib/systemd/system" \
  "${STAGE}/usr/lib/sysusers.d" "${STAGE}/usr/lib/tmpfiles.d"
install -m 644 "${ROOT}/sinkd-cfg/server.toml" "${STAGE}/etc/sinkd/server.toml"
install -m 644 "${ROOT}/sinkd-cfg/client.toml" "${STAGE}/usr/share/sinkd/client.toml"
install -m 755 "${ROOT}/pkg/install-client-config" "${STAGE}/usr/share/sinkd/install-client-config"
install -m 644 "${ROOT}/sinkd-cfg/client.toml" "${STAGE}/etc/skel/.config/sinkd/client.toml"
install -m 644 "${ROOT}/pkg/systemd/sinkd-srv.service" "${STAGE}/usr/lib/systemd/system/sinkd-srv.service"
install -m 644 "${ROOT}/pkg/systemd/sinkd@.service" "${STAGE}/usr/lib/systemd/system/sinkd@.service"
install -m 644 "${ROOT}/pkg/sysusers.d/sinkd.conf" "${STAGE}/usr/lib/sysusers.d/sinkd.conf"
install -m 644 "${ROOT}/pkg/tmpfiles.d/sinkd.conf" "${STAGE}/usr/lib/tmpfiles.d/sinkd.conf"
install -d "${STAGE}/usr/share/man/man1"
install -m 644 "${ROOT}/pkg/man/sinkd.1" "${ROOT}/pkg/man/sinkd-srv.1" "${STAGE}/usr/share/man/man1/"

cat >"${STAGE}/DEBIAN/postinst" <<'EOF'
#!/bin/sh
set -e
if command -v systemd-sysusers >/dev/null 2>&1; then
  systemd-sysusers sinkd.conf || true
fi
if command -v systemd-tmpfiles >/dev/null 2>&1; then
  systemd-tmpfiles --create sinkd.conf || true
fi
/usr/share/sinkd/install-client-config
EOF
chmod 755 "${STAGE}/DEBIAN/postinst"

cat >"${STAGE}/DEBIAN/control" <<EOF
Package: sinkd
Version: ${VERSION}
Architecture: ${DEB_ARCH}
Maintainer: Tony <krakjn@gmail.com>
Homepage: https://sinkd.octoworks.dev
Section: utils
Priority: optional
Description: Deployable Cloud, file sync daemon.
 bring everything and the kitchen sink to your local cloud.
EOF

dpkg-deb --root-owner-group --build "${STAGE}" "${OUT}/sinkd_${VERSION}_${DEB_ARCH}.deb"
