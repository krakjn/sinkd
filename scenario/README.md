# Scenario testing

Behavioral tests live here. Unit tests remain in each crate (`cargo test --workspace`).

## Layout

- `specs/` — TOML scenario definitions
- `scripts/` — shell drivers (Docker compose provisioning)
- `compose/` — Docker Compose stacks for integration scenarios (1c / 2c / 3c topologies)
- `src/` — Rust business logic (`cargo run -p scenario`)
- `output/` — gitignored **host-side** artifacts (manifests, logs, reports). Not mounted into containers; the compose driver writes here after `docker compose cp` on the host.

## Topologies

Three compose specs exercise production-like layouts over a Docker bridge network with **rsync over SSH** (no zenoh router, no shared watch volumes):

| Spec | Layout |
|------|--------|
| `compose-1c` | Single container: server + alice client |
| `compose-2c` | Server + alice colocated; bob in a second container |
| `compose-3c` | Separate server, client-alice, client-bob |

Set `COMPOSE_SPEC=compose-1c|compose-2c|compose-3c` when invoking `scripts/compose.sh` directly (defaults to `compose-3c`).

## Architecture

**Shell scripts** handle operational plumbing:

- `scripts/build-musl.sh` — compiles musl binaries in the workspace `target/` tree via a Rust container (bind-mounts `target/` and `scenario/output/compose/cargo/` for incremental builds)
- `scripts/compose.sh` — `docker compose` up/down/cp/exec, SSH key setup, log capture, teardown

**Rust** owns scenario business logic:

- Directory manifests (`scenario compose snapshot-dir`)
- Tree comparison (`scenario compose compare-manifests`)
- Generation state checks (`scenario compose check-generation`, `get-generation`)
- Phase-B event definitions (`scenario compose phase-b-ops`)

The compose driver [`scripts/compose.sh`](scripts/compose.sh) calls into the Rust binary for all checks and comparisons.

## Running locally

```bash
# All specs (smoke + compose when Docker is available)
cargo run -p scenario

# List specs
cargo run -p scenario -- --list

# Single spec
cargo run -p scenario -- --spec scenario/specs/smoke.toml

# All compose topologies (requires Docker)
just test-compose

# One topology
cargo run -p scenario -- --spec scenario/specs/compose-3c.toml

# Compose helpers directly
cargo run -p scenario -- compose snapshot-dir /path/to/tree
cargo run -p scenario -- compose compare-manifests --client a.manifest --server b.manifest --label A --user alice
```

Set `COMPOSE_KEEP=1` to leave the Docker stack running after a failed compose scenario.
Set `SCENARIO_BIN=/path/to/scenario` to use a pre-built binary instead of `cargo run`.
Set `COMPOSE_BUILD_PULL=1` to pass `--pull` when building the runtime image.

Musl build cache lives under `target/` (Linux musl triple) and `scenario/output/compose/cargo/` (registry/git).

## CI

Every PR runs `cargo run -p scenario`. The smoke spec validates the runner without Docker; the three compose specs build the integration image and run the full sync flow when Docker is available on the runner.

Artifacts are uploaded from `scenario/output/`.
