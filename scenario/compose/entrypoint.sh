#!/usr/bin/env bash
# Compose-scenario entrypoint. Branches on $ROLE:
#   server       -> sinkd-srv only
#   client       -> sinkd client + phase A events
#   server-client -> server + one client in the same container (1c / 2c colocated)
set -euo pipefail

case "${ROLE:-}" in
    server|server-client) export USER="${USER:-server}" ;;
    client) export USER="${USER:-${USER_NAME:-sinkd-client}}" ;;
esac

log() { printf '[entrypoint:%s] %s\n' "${ROLE:-?}" "$*"; }

SINKD_VERBOSE="${SINKD_VERBOSE:--vvv}"

# shellcheck source=/dev/null
source /usr/local/lib/sinkd/ssh-setup.sh

wait_for_log() {
    local path="$1"
    for _ in $(seq 1 50); do
        [[ -f "$path" ]] && return 0
        sleep 0.1
    done
    return 1
}

run_as_client_user() {
    : "${USER_NAME:?USER_NAME required}"
    su -s /bin/bash "${USER_NAME}" -c "$*"
}

prepare_watch_dir() {
    : "${USER_NAME:?USER_NAME required}"
    : "${WATCH:?WATCH required}"
    mkdir -p "${WATCH}"
    chown "${USER_NAME}:${USER_NAME}" "${WATCH}"
}

start_server() {
    log "init server config (location=/srv/sinkd)"
    sinkd-srv init --location /srv/sinkd --force
    log "starting server daemon"
    sinkd-srv ${SINKD_VERBOSE} start
    wait_for_log /var/log/sinkd/server.log || {
        log "server log never appeared"; exit 1;
    }
}

start_client_daemon() {
    : "${USER_NAME:?USER_NAME required for client}"
    : "${WATCH:?WATCH required for client}"
    SINKD_SERVER_ADDR="${SINKD_SERVER_ADDR:-server}"
    interval="${SINKD_INTERVAL:-1}"

    prepare_watch_dir

    log "init client configs (server_addr=${SINKD_SERVER_ADDR}, user=${USER_NAME}, watch=${WATCH})"
    sinkd init \
        --server-addr "${SINKD_SERVER_ADDR}" \
        --user "${USER_NAME}" \
        --watch "${WATCH}" \
        --interval "${interval}" \
        --force

    log "starting client daemon"
    sinkd ${SINKD_VERBOSE} start
    wait_for_log /var/log/sinkd/client.log || {
        log "client log never appeared"; exit 1;
    }
}

run_phase_a() {
    : "${USER_NAME:?USER_NAME required}"
    : "${WATCH:?WATCH required}"

    sleep 8

    log "phase A: 5 sequential writes"
    for i in 1 2 3 4 5; do
        run_as_client_user \
            "echo \"${USER_NAME} write $i $(date +%s)\" > \"${WATCH}/file_${USER_NAME}_$i.txt\""
        sleep 0.5
    done

    log "phase A: rename file_1 -> file_renamed"
    run_as_client_user \
        "mv \"${WATCH}/file_${USER_NAME}_1.txt\" \"${WATCH}/file_${USER_NAME}_renamed.txt\""
    sleep 1

    log "phase A: delete file_2"
    run_as_client_user "rm \"${WATCH}/file_${USER_NAME}_2.txt\""
    sleep 1

    log "phase A: append to file_3"
    run_as_client_user \
        "echo \"${USER_NAME} append $(date +%s)\" >> \"${WATCH}/file_${USER_NAME}_3.txt\""

    sleep 4
    log "phase A complete; dropping sentinel"
    run_as_client_user ": > \"${WATCH}/.events_done\""
}

setup_ssh

case "${ROLE:-}" in
    server)
        start_server
        log "tailing server log"
        exec tail -F /var/log/sinkd/server.log
        ;;

    client)
        start_client_daemon
        run_phase_a
        log "tailing client log"
        exec tail -F /var/log/sinkd/client.log
        ;;

    server-client)
        start_server
        start_client_daemon
        run_phase_a
        log "tailing server + client logs"
        exec tail -F /var/log/sinkd/server.log /var/log/sinkd/client.log
        ;;

    *)
        echo "unknown ROLE='${ROLE:-}'" >&2
        exit 2
        ;;
esac
