IMAGE := "ghcr.io/ballast-dev/sinkd:0.1.0"
export ARCH := if `uname -m` == "x86_64" { "amd64" } else { "arm64" }

_: 
    @just --list

version:
    @bump update scenario/Cargo.toml
    @bump update sinkd-client/Cargo.toml
    @bump update sinkd-server/Cargo.toml
    @bump update sinkd-core/Cargo.toml

# ensure to run this only on native architecture
lint:
    cargo clippy --all-targets --all-features --workspace -- \
      -D warnings

# Default / CI-parity: workspace unit tests only (seconds; no Docker).
test: test-local

# fast local lane (alias for default `test`)
test-local:
    cargo test --workspace

# Behavioral scenario tests (no Docker; runs all specs in scenario/specs/).
test-scenario:
    cargo run -p scenario

# Containerized integration scenarios (requires Docker + docker compose).
# Can take many minutes on cold cache (image build + sync waits); use when validating integration.
test-compose:
    cargo run -p scenario -- --spec scenario/specs/compose-1c.toml
    cargo run -p scenario -- --spec scenario/specs/compose-2c.toml
    cargo run -p scenario -- --spec scenario/specs/compose-3c.toml

# Full lane: unit tests then all scenario specs.
test-all:
    just test-local
    just test-scenario

# optional Zenoh router (see docker-compose.yml); use when debugging cross-host peers
zenoh:
    docker compose up -d zenoh

zenoh-down:
    docker compose down

# the following commands are purely for debugging
client:
    cargo run -p sinkd -- -d -s cfg/system/sinkd.conf -u cfg/user/sinkd.conf start

client-log:
    tail -f /tmp/sinkd/client.log

server:
    cargo run -p sinkd-srv -- -d start

server-log:
    tail -f /tmp/sinkd/server.log


img:
    @docker buildx build \
    --platform linux/amd64,linux/arm64 \
    -t {{IMAGE}} \
    -< Dockerfile


sh *ARGS:
    @docker run -it --rm \
        --hostname sinkd \
        -e WORKDIR=$(pwd) \
        -v $(pwd):$(pwd) \
        -w $(pwd) \
        {{ARGS}} \
        {{IMAGE}}
