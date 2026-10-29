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
        if compose logs "${args[@]}" "$svc" 2>&1 | grep -q "$needle"; then
            return 0
        fi
        sleep 0.5
    done
    echo "timed out waiting for '$needle' on $svc" >&2
    exit 1
}

# Succeeds when a file named $2 under $3 on service $1 contains $4.
wait_has() {
    local svc=$1 name=$2 root=$3 needle=$4 tries=${5:-40}
    local i
    for i in $(seq 1 "$tries"); do
        if compose exec -T -e NAME="$name" -e ROOT="$root" -e NEEDLE="$needle" "$svc" \
            sh -c 'f=$(find "$ROOT" -name "$NAME" -type f | head -n1); test -n "$f" && grep -F -q "$NEEDLE" "$f"'; then
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

echo "scenarios passed"
