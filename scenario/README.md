# scenario

`cargo test -p scenario` starts `sinkd-srv` and `sinkd` as two processes, pairs them with a fixed code, and checks that a file written on the client shows up in the server store.

The two sides are separate processes because libreflect's filter hook is process-global. One process cannot run the send and the receive at the same time.

`just test-docker` builds static musl binaries, puts them in an Alpine image, and runs the same checks across a server and three clients. See `scenario/docker/run.sh`.
