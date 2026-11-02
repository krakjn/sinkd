_:
    @just --list

version:
    @bump update scenario/Cargo.toml
    @bump update sinkd-client/Cargo.toml
    @bump update sinkd-server/Cargo.toml
    @bump update sinkd-core/Cargo.toml
    @bump update sinkd-proto/Cargo.toml
    @bump update sinkd-reflect/Cargo.toml
    @bump update sinkd-reflect-sys/Cargo.toml

lint:
    cargo clippy --all-targets --all-features --workspace -- -D warnings

test: test-local

test-local:
    cargo test --workspace

test-scenario:
    cargo test -p scenario

test-all:
    just test-local
    just test-scenario

client HOST:
    cargo run -p sinkd -- --server {{HOST}} --anchor .

server:
    cargo run -p sinkd-srv -- --sync-root ~/sinkd-data

target_dir := env_var_or_default("CARGO_TARGET_DIR", justfile_directory() + "/target")

# Static musl binaries, copied where the Alpine image expects them.
musl:
    cargo build -p sinkd -p sinkd-srv --release --locked
    mkdir -p scenario/docker/bin
    cp {{target_dir}}/x86_64-unknown-linux-musl/release/sinkd scenario/docker/bin/sinkd
    cp {{target_dir}}/x86_64-unknown-linux-musl/release/sinkd-srv scenario/docker/bin/sinkd-srv

docker-build: musl
    docker build -t sinkd-scenario scenario/docker

# Alpine containers: pair, push, fan-out, delete, conflict, late joiner, restart, share.
test-docker: docker-build
    bash scenario/docker/run.sh

# Assemble the pages site and open it. Ctrl-C stops the server.
site:
    #!/usr/bin/env bash
    set -euo pipefail
    rm -rf _site
    mkdir -p _site/assets
    cp -R site/. _site/
    cp assets/sinkd-slim.svg assets/moonhouse.ttf _site/assets/
    echo "http://127.0.0.1:8765  (Ctrl-C to stop)"
    (sleep 0.4 && xdg-open http://127.0.0.1:8765) >/dev/null 2>&1 &
    exec python3 -m http.server 8765 --directory _site
