#!/bin/sh
# Install sinkd: curl -fsSL https://sinkd.octoworks.dev/install.sh | sh
#
#   PREFIX=PATH        tarball install root (default /usr/local)
#   SINKD_VERSION=vX   a GitHub release tag instead of the site
#   SINKD_BASE=URL     download directory instead of the site
#   DRY_RUN=1          print what would be installed and exit
set -eu

repo="https://github.com/krakjn/sinkd"
prefix="${PREFIX:-/usr/local}"
prefix=${prefix%/}

# Color when the stream is a terminal. NO_COLOR disables it. curl | sh still has a tty on stdout.
c_dim= c_ok= c_off=
if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
    c_dim=$(printf '\033[2m')
    c_ok=$(printf '\033[32m')
    c_off=$(printf '\033[0m')
fi
c_err= c_eoff=
if [ -t 2 ] && [ -z "${NO_COLOR:-}" ]; then
    c_err=$(printf '\033[31m')
    c_eoff=$(printf '\033[0m')
fi
say() { printf '%ssinkd:%s %s\n' "$c_dim" "$c_off" "$*"; }
ok() { printf '%ssinkd:%s %s\n' "$c_ok" "$c_off" "$*"; }
die() { printf '%ssinkd: error:%s %s\n' "$c_err" "$c_eoff" "$*" >&2; exit 1; }

fetch() {
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$1" -o "$2"
    elif command -v wget >/dev/null 2>&1; then
        wget -q "$1" -O "$2"
    else
        die "need curl or wget"
    fi
}

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        die "need sha256sum or shasum to verify the download"
    fi
}

# True when an existing ancestor of $1 is writable, so we can mkdir the rest.
can_write() {
    dir=$1
    while [ ! -e "$dir" ]; do
        dir=$(dirname "$dir")
        [ "$dir" != "/" ] || break
    done
    [ -w "$dir" ]
}

run_as_root() {
    if [ "$(id -u)" = 0 ]; then
        "$@"
    elif command -v sudo >/dev/null 2>&1; then
        sudo "$@"
    else
        return 1
    fi
}

sum_for() {
    awk -v f="$1" '$2 == f || $2 == "*" f { print $1; exit }' "$tmp/SHA256SUMS"
}

case "$(uname -s)" in
    Linux) os=linux ;;
    Darwin) os=macos ;;
    *) die "unsupported OS $(uname -s) (Linux and macOS only; on Windows use WSL)" ;;
esac
case "$(uname -m)" in
    x86_64 | amd64) arch=x86_64; debarch=amd64; pkgarch=x86_64 ;;
    aarch64 | arm64) arch=arm64; debarch=arm64; pkgarch=aarch64 ;;
    *) die "unsupported architecture $(uname -m)" ;;
esac
platform="${os}-${arch}"

if [ -n "${SINKD_BASE:-}" ]; then
    base=${SINKD_BASE%/}
elif [ -n "${SINKD_VERSION:-}" ]; then
    ver=$SINKD_VERSION
    case "$ver" in
        v*) ;;
        *) ver="v$ver" ;;
    esac
    base="$repo/releases/download/$ver"
else
    base="https://sinkd.octoworks.dev/dl"
fi

tmp=$(mktemp -d 2>/dev/null || mktemp -d -t sinkd)
trap 'rm -rf "$tmp"' EXIT INT TERM

fetch "$base/SHA256SUMS" "$tmp/SHA256SUMS" || die "no SHA256SUMS at $base"

deb=""
if [ "$os" = linux ] && command -v dpkg >/dev/null 2>&1; then
    deb=$(awk -v arch="$debarch" '{
      name = $2
      sub(/^\*/, "", name)
      if (name ~ ("^sinkd_.*_" arch "[.]deb$")) { print name; exit }
    }' "$tmp/SHA256SUMS")
fi

pkg=""
if [ "$os" = linux ] && command -v pacman >/dev/null 2>&1; then
    pkg=$(awk -v arch="$pkgarch" '{
      name = $2
      sub(/^\*/, "", name)
      if (name ~ ("^sinkd-.*-" arch "[.]pkg[.]tar[.]zst$")) { print name; exit }
    }' "$tmp/SHA256SUMS")
fi

use_deb=0
use_pkg=0
if [ -n "$deb" ] && { [ "$(id -u)" = 0 ] || command -v sudo >/dev/null 2>&1; }; then
    use_deb=1
elif [ -n "$deb" ]; then
    say "no sudo; unpacking the tarball instead"
elif [ -n "$pkg" ] && { [ "$(id -u)" = 0 ] || command -v sudo >/dev/null 2>&1; }; then
    use_pkg=1
elif [ -n "$pkg" ]; then
    say "no sudo; unpacking the tarball instead"
fi

tarname="sinkd-${platform}.tar.gz"
dest="$prefix/bin"

if [ "$use_deb" -eq 1 ]; then
    if [ -n "${DRY_RUN:-}" ]; then
        say "would install $deb with dpkg"
        exit 0
    fi
    say "downloading $deb"
    fetch "$base/$deb" "$tmp/$deb" || die "download failed: $base/$deb"
    expected=$(sum_for "$deb")
    [ -n "$expected" ] || die "$deb is missing from SHA256SUMS"
    actual=$(sha256_of "$tmp/$deb")
    [ "$expected" = "$actual" ] || die "checksum mismatch for $deb"
    run_as_root dpkg -i "$tmp/$deb" || die "dpkg failed"
    ok "installed $deb"
elif [ "$use_pkg" -eq 1 ]; then
    if [ -n "${DRY_RUN:-}" ]; then
        say "would install $pkg with pacman"
        exit 0
    fi
    say "downloading $pkg"
    fetch "$base/$pkg" "$tmp/$pkg" || die "download failed: $base/$pkg"
    expected=$(sum_for "$pkg")
    [ -n "$expected" ] || die "$pkg is missing from SHA256SUMS"
    actual=$(sha256_of "$tmp/$pkg")
    [ "$expected" = "$actual" ] || die "checksum mismatch for $pkg"
    run_as_root pacman -U --noconfirm "$tmp/$pkg" || die "pacman failed"
    ok "installed $pkg"
else
    if [ -z "$(sum_for "$tarname")" ]; then
        die "no $platform build in this release ($base)"
    fi
    if [ -n "${DRY_RUN:-}" ]; then
        say "would install $tarname into $dest"
        exit 0
    fi
    if ! can_write "$dest" && [ "$(id -u)" != 0 ] && ! command -v sudo >/dev/null 2>&1; then
        die "cannot write $dest (try sudo, or PREFIX to a directory you own)"
    fi
    say "downloading $tarname"
    fetch "$base/$tarname" "$tmp/$tarname" || die "download failed: $base/$tarname"
    expected=$(sum_for "$tarname")
    actual=$(sha256_of "$tmp/$tarname")
    [ "$expected" = "$actual" ] || die "checksum mismatch for $tarname"
    mkdir -p "$tmp/extract"
    tar -xzf "$tmp/$tarname" -C "$tmp/extract"
    [ -f "$tmp/extract/sinkd" ] && [ -f "$tmp/extract/sinkd-srv" ] || die "archive did not contain sinkd and sinkd-srv"
    if [ "$(id -u)" = 0 ] || can_write "$dest"; then
        mkdir -p "$dest"
        cp "$tmp/extract/sinkd" "$tmp/extract/sinkd-srv" "$dest/"
        chmod 755 "$dest/sinkd" "$dest/sinkd-srv"
    else
        run_as_root mkdir -p "$dest" || die "cannot create $dest"
        run_as_root cp "$tmp/extract/sinkd" "$tmp/extract/sinkd-srv" "$dest/"
        run_as_root chmod 755 "$dest/sinkd" "$dest/sinkd-srv"
    fi
    if [ -f "$tmp/extract/sinkd.1" ]; then
        mandir="$prefix/share/man/man1"
        if [ "$(id -u)" = 0 ] || can_write "$mandir"; then
            mkdir -p "$mandir"
            cp "$tmp/extract/sinkd.1" "$tmp/extract/sinkd-srv.1" "$mandir/"
        else
            run_as_root mkdir -p "$mandir" || die "cannot create $mandir"
            run_as_root cp "$tmp/extract/sinkd.1" "$tmp/extract/sinkd-srv.1" "$mandir/"
        fi
    fi
    ok "installed sinkd and sinkd-srv -> $dest"
    case ":$PATH:" in
        *":$dest:"*) ;;
        *) say "note: $dest is not on PATH" ;;
    esac
fi

say "hub:    sinkd-srv"
say "client: sinkd pair <hub>"
say "docs:   https://sinkd.octoworks.dev/docs.html"
