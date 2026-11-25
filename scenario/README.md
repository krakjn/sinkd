# scenario

`cargo test -p scenario` starts `sinkd-srv` and one or more `sinkd` clients as separate processes, pairs them with a fixed code, and checks push, ignore rules (including `!` re-includes), pull with delete protecting a client's ignored files, and two clients with different ignore lists syncing at once.

`just test` builds static musl binaries, puts them in an Alpine image, and runs the same checks across a server and three clients. See `scenario/docker/run.sh`. On macOS without a musl toolchain, `just test-local-docker` builds the binaries inside an Alpine container instead.
