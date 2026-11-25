#!/bin/bash
# Sync checks against the Alpine containers. Build the image first (just docker-build).
set -euo pipefail
cd "$(dirname "$0")"

compose() { docker compose "$@"; }

cleanup() {
    status=$?
    if [[ "$status" -ne 0 ]]; then
        compose --profile carol logs || true
    fi
    compose --profile carol down -v || true
    exit "$status"
}
trap cleanup EXIT

wait_log() {
    local svc=$1 needle=$2 since=${3:-}
    local i args=()
    if [[ -n "$since" ]]; then
        args+=(--since "$since")
    fi
    for i in $(seq 1 60); do
        if compose logs ${args[@]+"${args[@]}"} "$svc" 2>&1 | grep -q "$needle"; then
            return 0
        fi
        sleep 0.5
    done
    echo "timed out waiting for '$needle' on $svc" >&2
    exit 1
}

# Succeeds when any file named $2 under $3 on service $1 contains $4.
wait_has() {
    local svc=$1 name=$2 root=$3 needle=$4 tries=${5:-40}
    local i
    for i in $(seq 1 "$tries"); do
        if compose exec -T -e NAME="$name" -e ROOT="$root" -e NEEDLE="$needle" "$svc" \
            sh -c 'find "$ROOT" -name "$NAME" -type f -exec grep -F -l -- "$NEEDLE" {} + 2>/dev/null | grep -q .'; then
            return 0
        fi
        sleep 0.5
    done
    echo "timed out waiting for $root/$name containing '$needle' on $svc" >&2
    exit 1
}

wait_gone() {
    local svc=$1 name=$2 root=$3
    local i
    for i in $(seq 1 40); do
        if compose exec -T -e NAME="$name" -e ROOT="$root" "$svc" \
            sh -c 'test -z "$(find "$ROOT" -name "$NAME" -type f | head -n1)"'; then
            return 0
        fi
        sleep 0.5
    done
    echo "timed out waiting for $name to leave $root on $svc" >&2
    exit 1
}

# Fails when a file named $2 exists under $3 on service $1.
assert_absent() {
    local svc=$1 name=$2 root=$3
    if compose exec -T -e NAME="$name" -e ROOT="$root" "$svc" \
        sh -c 'test -n "$(find "$ROOT" -name "$NAME" -type f | head -n1)"'; then
        echo "$name should not be under $root on $svc" >&2
        exit 1
    fi
}

compose up -d --build server alice bob
wait_log server "zenoh tls"
wait_log alice "watching"
wait_log bob "watching"

echo "pair: wrong code is rejected"
if compose run --rm --no-deps --entrypoint sinkd alice \
    --state-dir /tmp/bad pair server --code 000000 --pair-port 7449; then
    echo "wrong pair code was accepted" >&2
    exit 1
fi

echo "push"
compose exec -T alice sh -c 'echo hello > /home/tony/sync/a.txt'
wait_has server a.txt /data hello

echo "fan-out"
wait_has bob a.txt /home/tony/sync hello

echo "edit and delete"
compose exec -T bob sh -c 'echo edited > /home/tony/sync/a.txt'
wait_has server a.txt /data edited
wait_has alice a.txt /home/tony/sync edited
compose exec -T bob sh -c 'rm /home/tony/sync/a.txt'
wait_gone server a.txt /data
wait_gone alice a.txt /home/tony/sync

echo "stale push"
compose exec -T alice sh -c 'echo base > /home/tony/sync/c.txt'
wait_has bob c.txt /home/tony/sync base
compose pause bob
compose exec -T alice sh -c 'echo alice-wins > /home/tony/sync/c.txt'
wait_has server c.txt /data alice-wins
printf 'bob-local\n' > /tmp/sinkd-bob-c.txt
docker cp /tmp/sinkd-bob-c.txt "$(compose ps -q bob):/home/tony/sync/c.txt"
rm -f /tmp/sinkd-bob-c.txt
compose unpause bob
# Bob was paused, so the write is seen by the 30s rescan, not the watcher.
wait_has bob c.txt /home/tony/sync alice-wins 100
found=0
for _ in $(seq 1 100); do
    if compose exec -T bob sh -c 'find /var/lib/sinkd/conflicts -type f 2>/dev/null | grep -q sinkd-conflict'; then
        found=1
        break
    fi
    sleep 0.5
done
[[ "$found" -eq 1 ]]

echo "late joiner"
compose exec -T alice sh -c 'echo from-hub > /home/tony/sync/from-hub.txt'
wait_has server from-hub.txt /data from-hub
compose --profile carol up -d carol
wait_log carol "watching"
compose exec -T carol sh -c 'echo carol > /home/tony/sync/carol-only.txt'
wait_has carol from-hub.txt /home/tony/sync from-hub
wait_has carol carol-only.txt /home/tony/sync carol
wait_has server carol-only.txt /data carol

echo "restart"
compose exec -T alice sh -c 'echo after > /home/tony/sync/restart.txt'
wait_has server restart.txt /data after
before=$(compose exec -T server cat /data/.sinkd/anchors.toml)
stamp=$(date -u +%Y-%m-%dT%H:%M:%SZ)
compose restart server
wait_log server "zenoh tls" "$stamp"
after=$(compose exec -T server cat /data/.sinkd/anchors.toml)
[[ "$before" == "$after" ]]
compose exec -T alice sh -c 'echo after2 > /home/tony/sync/restart.txt'
wait_has server restart.txt /data after2

echo "share"
compose exec -T alice sh -c 'echo shared > /srv/share/s.txt'
wait_has bob s.txt /srv/share shared
wait_has server s.txt /data shared

echo "ignore push"
compose exec -T alice sh -c 'printf "build/\n!build/keep/\n" > /home/tony/sync/.gitignore'
wait_has server .gitignore /data 'build/'
wait_has bob .gitignore /home/tony/sync 'build/'
compose exec -T alice sh -c 'cd /home/tony/sync && mkdir -p build/keep && echo x > build/x.txt && echo y > build/keep/y.txt'
wait_has server y.txt /data y
wait_has bob y.txt /home/tony/sync y
assert_absent server x.txt /data
assert_absent bob x.txt /home/tony/sync

echo "pull delete keeps ignored"
compose exec -T bob sh -c 'mkdir -p /home/tony/sync/build && echo bob > /home/tony/sync/build/bob.txt'
compose exec -T alice sh -c 'echo t > /home/tony/sync/tracked.txt'
wait_has bob tracked.txt /home/tony/sync t
compose exec -T alice sh -c 'rm /home/tony/sync/tracked.txt'
wait_gone bob tracked.txt /home/tony/sync
wait_gone server tracked.txt /data
compose exec -T bob grep -q bob /home/tony/sync/build/bob.txt
assert_absent server bob.txt /data

echo "hub concurrency"
compose exec -T alice sh -c 'printf "sync_skip/\n" >> /home/tony/sync/.gitignore && printf "share_skip/\n" > /srv/share/.gitignore'
wait_has server .gitignore /data 'sync_skip/'
wait_has server .gitignore /data 'share_skip/'
wait_has bob .gitignore /srv/share 'share_skip/'
wait_has bob .gitignore /home/tony/sync 'sync_skip/'
# Two anchors with different rules land on the hub at once. Each writer owns one anchor so neither is behind.
burst='mkdir -p $D/sync_skip $D/share_skip
for i in 1 2 3 4 5; do
  echo h > $D/$HIDE/$N-hidden-$i.txt
  echo s > $D/$SEND/$N-sent-$i.txt
done'
compose exec -T -e D=/home/tony/sync -e N=sync -e HIDE=sync_skip -e SEND=share_skip alice sh -c "$burst" &
compose exec -T -e D=/srv/share -e N=share -e HIDE=share_skip -e SEND=sync_skip bob sh -c "$burst" &
wait
for n in sync share; do
    for i in 1 2 3 4 5; do
        wait_has server "$n-sent-$i.txt" /data s
    done
done
for n in sync share; do
    for i in 1 2 3 4 5; do
        assert_absent server "$n-hidden-$i.txt" /data
    done
done

if compose exec -T alice sh -c 'command -v reflect >/dev/null'; then
    echo "ssh delete"
    compose exec -T server /usr/sbin/sshd
    compose exec -T alice sh -c 'mkdir -p /root/.ssh && test -f /root/.ssh/id_ed25519 || ssh-keygen -q -t ed25519 -N "" -f /root/.ssh/id_ed25519
        printf "Host server\n  StrictHostKeyChecking no\n  UserKnownHostsFile /dev/null\n  LogLevel ERROR\n" > /root/.ssh/config'
    pub=$(compose exec -T alice cat /root/.ssh/id_ed25519.pub)
    compose exec -T -e PUB="$pub" server sh -c '
        install -d -o tony -g tony -m 700 /home/tony/.ssh
        echo "$PUB" > /home/tony/.ssh/authorized_keys
        chown tony:tony /home/tony/.ssh/authorized_keys && chmod 600 /home/tony/.ssh/authorized_keys
        install -d -o tony -g tony /home/tony/dst/build
        echo extra > /home/tony/dst/extra.txt && echo remote > /home/tony/dst/build/remote.bin
        chown -R tony:tony /home/tony/dst'
    compose exec -T alice sh -c 'mkdir -p /tmp/ssh-src/build && cd /tmp/ssh-src \
        && printf "build/\n" > .gitignore && echo keep > keep.txt && echo local > build/local.bin \
        && sinkd ssh tony@server:/home/tony/dst/ --anchor /tmp/ssh-src --delete'
    wait_has server keep.txt /home/tony/dst keep
    wait_gone server extra.txt /home/tony/dst
    assert_absent server local.bin /home/tony/dst
    if ! compose exec -T server test -f /home/tony/dst/build/remote.bin; then
        echo "sinkd ssh --delete removed a remote path the local .gitignore ignores" >&2
        exit 1
    fi
else
    echo "ssh delete: skipped, no reflect binary in the image (use just test-local-docker)"
fi

echo "scenarios passed"
